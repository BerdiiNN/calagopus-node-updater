//! What runs where: every Wings node and DB Agent host, the version and architecture it
//! reports through its own system-overview endpoint, and how that compares with the
//! latest release. Also the panel itself.
//!
//! Polling the daemons is the slow part, so only its result (a [`Snapshot`]) is stored.
//! Everything that changes through the admin UI — targets, credential profiles, runs — is
//! merged in at request time by [`view`], so a save shows up immediately.

use crate::db::{self, RunRow, TargetRow};
use crate::versions::{self, LatestInfo};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use shared::State;
use shared::models::ByUuid;
use shared::models::database_agent_host::DatabaseAgentHost;
use shared::models::node::Node;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

/// Older versions stored the fully merged inventory under `inventory`; this key holds only
/// the daemon poll results.
pub const STATE_KEY_SNAPSHOT: &str = "inventory.v2";
const OVERVIEW_TIMEOUT: Duration = Duration::from_secs(12);

/// Both daemon generations download (and the newer ones smoke-test) the release before they
/// answer the upgrade call, so it gets far longer than the overview poll.
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

static UPGRADE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("Calagopus Panel")
        .connect_timeout(Duration::from_secs(30))
        .timeout(UPGRADE_TIMEOUT)
        .build()
        .expect("building the upgrade HTTP client")
});

/// A Wings node or DB Agent host as the panel knows it, plus the API client to ask it.
pub struct Instance {
    pub kind: String,
    pub uuid: Uuid,
    pub name: String,
    pub url: String,
    pub location: Option<String>,
    /// The daemon token in plain text, for the upgrade call.
    token: String,
    client: Client,
}

pub enum Client {
    Wings(wings_api::client::WingsClient),
    DbAgent(db_agent_api::client::DbAgentClient),
}

/// The fields we read from a system overview, generation-independent (read through JSON so
/// a field added or renamed in a later API client never breaks the build).
#[derive(Debug, Clone, Default)]
pub struct Overview {
    pub version_raw: String,
    pub version: Option<semver::Version>,
    pub architecture: String,
    pub container_type: String,
    /// The daemon names its own release provider (`upgrade` in the overview; Wings 1.2.7 and
    /// DB Agent 1.2.4 onwards) and installs a version by itself. Only an API client that
    /// knows the field (panel 1.2.7 onwards) can see it; older panels always read `false`.
    pub self_upgrade: bool,
}

fn overview_from_value(value: serde_json::Value) -> Overview {
    let version_raw = value
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Overview {
        version: versions::parse_reported(&version_raw),
        version_raw,
        architecture: value
            .get("architecture")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        container_type: value
            .get("container_type")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        self_upgrade: value.get("upgrade").is_some_and(|u| u.is_object()),
    }
}

/// What the daemon's `POST /api/system/upgrade` is asked to install.
pub struct UpgradeRequest {
    pub version: semver::Version,
    /// Where daemons that predate their own release provider (Wings before 1.2.7, DB Agent
    /// before 1.2.4) download the binary from; they check the SHA-256 and run the restart
    /// command. `None` when the daemon fetches `version` itself.
    pub legacy: Option<LegacyUpgrade>,
}

pub struct LegacyUpgrade {
    pub url: String,
    pub sha256: String,
    pub restart_command: String,
    pub restart_args: Vec<String>,
}

pub struct UpgradeOutcome {
    /// `false` when the daemon has panel-driven upgrades disabled in its config.
    pub applied: bool,
    /// The version the daemon resolved, when it reports one.
    pub version: Option<String>,
}

/// The upgrade body. With `legacy` it carries the keys of both protocols; each daemon
/// generation ignores the other's.
fn upgrade_body(request: &UpgradeRequest) -> serde_json::Value {
    let mut body = serde_json::json!({ "version": request.version.to_string(), "token": null });
    if let Some(legacy) = &request.legacy {
        body["url"] = legacy.url.clone().into();
        body["headers"] = serde_json::json!({});
        body["sha256"] = legacy.sha256.clone().into();
        body["restart_command"] = legacy.restart_command.clone().into();
        body["restart_command_args"] = legacy.restart_args.clone().into();
    }
    body
}

/// The message of a daemon error response (`{"errors": [..]}`).
fn daemon_error(body: &serde_json::Value) -> String {
    body.get("errors")
        .and_then(|e| e.as_array())
        .map(|e| e.iter().filter_map(|m| m.as_str()).collect::<Vec<_>>().join("; "))
        .filter(|m| !m.is_empty())
        .or_else(|| body.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| "no error message".to_string())
}

/// The token columns are `Vec<u8>` up to panel 1.1.5 and `EncryptedString` since; both are
/// `AsRef<[u8]>`, which every generation's `Database::decrypt` accepts.
async fn plain_token(state: &State, token: &impl AsRef<[u8]>) -> Result<String> {
    Ok(state.database.decrypt(token.as_ref().to_vec()).await?.to_string())
}

impl Instance {
    pub async fn overview(&self) -> Result<Overview> {
        let value = tokio::time::timeout(OVERVIEW_TIMEOUT, async {
            match &self.client {
                Client::Wings(client) => {
                    let response = client.get_system_overview().await?;
                    Ok::<serde_json::Value, anyhow::Error>(serde_json::to_value(response)?)
                }
                Client::DbAgent(client) => {
                    let response = client.get_system_overview().await?;
                    Ok(serde_json::to_value(response)?)
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out after {} s", OVERVIEW_TIMEOUT.as_secs()))??;
        Ok(overview_from_value(value))
    }

    /// Ask the daemon to replace its binary with `request.version` and restart.
    ///
    /// Sent directly rather than through the API client: panel 1.2.7 changed the client's
    /// request body to the new protocol (`{version, token}` instead of `{url, sha256,
    /// restart_command, ..}`), and every panel generation has to upgrade every daemon
    /// generation.
    pub async fn upgrade(&self, request: &UpgradeRequest) -> Result<UpgradeOutcome> {
        let response = UPGRADE_CLIENT
            .post(format!("{}/api/system/upgrade", self.url.trim_end_matches('/')))
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&upgrade_body(request))
            .send()
            .await
            .map_err(|err| anyhow!("calling the upgrade API: {}", err.without_url()))?;
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            return Err(anyhow!("the daemon answered {status}: {}", daemon_error(&body)));
        }
        Ok(UpgradeOutcome {
            applied: body.get("applied").and_then(|v| v.as_bool()).unwrap_or(false),
            version: body.get("version").and_then(|v| v.as_str()).map(str::to_string),
        })
    }

    async fn wings(state: &State, node: &Node) -> Result<Self> {
        let token = plain_token(state, &node.token).await.context("decrypting the node token")?;
        Ok(Self {
            kind: db::KIND_WINGS.to_string(),
            uuid: node.uuid,
            name: node.name.to_string(),
            url: node.url.to_string(),
            location: Some(node.location.name.to_string()),
            client: Client::Wings(wings_api::client::WingsClient::new(node.url.to_string(), token.clone())),
            token,
        })
    }

    async fn db_agent(state: &State, host: &DatabaseAgentHost) -> Result<Self> {
        let token = plain_token(state, &host.token).await.context("decrypting the DB Agent token")?;
        Ok(Self {
            kind: db::KIND_DB_AGENT.to_string(),
            uuid: host.uuid,
            name: host.name.to_string(),
            url: host.url.to_string(),
            location: None,
            client: Client::DbAgent(db_agent_api::client::DbAgentClient::new(host.url.to_string(), token.clone())),
            token,
        })
    }

    /// Host name of the daemon URL, the natural default for the SSH host.
    pub fn host(&self) -> Option<String> {
        url_host(&self.url)
    }
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or(host).to_string()
    };
    if host.is_empty() { None } else { Some(host) }
}

async fn wings_instances(state: &State) -> Result<Vec<Instance>> {
    let nodes = Node::all_with_pagination(&state.database, 1, 500, None)
        .await
        .context("listing nodes")?;
    let mut out = Vec::with_capacity(nodes.data.len());
    for node in nodes.data {
        out.push(Instance::wings(state, &node).await?);
    }
    Ok(out)
}

async fn db_agent_instances(state: &State) -> Result<Vec<Instance>> {
    let hosts = DatabaseAgentHost::all_with_pagination(&state.database, 1, 500, None)
        .await
        .context("listing DB Agent hosts")?;
    let mut out = Vec::with_capacity(hosts.data.len());
    for host in hosts.data {
        out.push(Instance::db_agent(state, &host).await?);
    }
    Ok(out)
}

/// Every node and DB Agent host.
pub async fn instances(state: &State) -> Result<Vec<Instance>> {
    let mut all = wings_instances(state).await?;
    match db_agent_instances(state).await {
        Ok(hosts) => all.extend(hosts),
        // Panels without the DB Agent tables (or without hosts) simply have none.
        Err(err) => tracing::debug!(target: crate::LOG, "no DB Agent hosts: {err:#}"),
    }
    Ok(all)
}

/// One node / host by kind and uuid.
pub async fn instance(state: &State, kind: &str, uuid: Uuid) -> Result<Option<Instance>> {
    match kind {
        db::KIND_WINGS => {
            let Some(node) = Node::by_uuid_optional(&state.database, uuid).await.context("loading node")? else {
                return Ok(None);
            };
            Ok(Some(Instance::wings(state, &node).await?))
        }
        db::KIND_DB_AGENT => {
            let Some(host) = DatabaseAgentHost::by_uuid_optional(&state.database, uuid)
                .await
                .context("loading DB Agent host")?
            else {
                return Ok(None);
            };
            Ok(Some(Instance::db_agent(state, &host).await?))
        }
        _ => Ok(None),
    }
}

/* --------------------------------------------------------------------------- snapshot */

/// What one daemon answered (or did not) during the last poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonEntry {
    pub kind: String,
    pub ref_uuid: Uuid,
    pub name: String,
    pub url: String,
    pub location: Option<String>,
    pub host: Option<String>,
    pub installed: Option<semver::Version>,
    pub installed_raw: String,
    pub architecture: String,
    pub container_type: String,
    /// Absent in snapshots stored before 1.0.1.
    #[serde(default)]
    pub self_upgrade: bool,
    pub error: Option<String>,
}

impl DaemonEntry {
    fn overview(&self) -> Option<Overview> {
        if self.error.is_some() {
            return None;
        }
        Some(Overview {
            version_raw: self.installed_raw.clone(),
            version: self.installed.clone(),
            architecture: self.architecture.clone(),
            container_type: self.container_type.clone(),
            self_upgrade: self.self_upgrade,
        })
    }
}

/// The stored result of the last daemon poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub entries: Vec<DaemonEntry>,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

/// Ask every daemon for its overview and store the answers.
pub async fn poll(state: &State) -> Result<Snapshot> {
    let instances = instances(state).await?;
    let overviews = futures_util::future::join_all(instances.iter().map(|i| i.overview())).await;
    let entries = instances
        .iter()
        .zip(overviews)
        .map(|(instance, overview)| {
            let (overview, error) = match overview {
                Ok(o) => (Some(o), None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };
            DaemonEntry {
                kind: instance.kind.clone(),
                ref_uuid: instance.uuid,
                name: instance.name.clone(),
                url: instance.url.clone(),
                location: instance.location.clone(),
                host: instance.host(),
                installed: overview.as_ref().and_then(|o| o.version.clone()),
                installed_raw: overview.as_ref().map(|o| o.version_raw.clone()).unwrap_or_default(),
                architecture: overview.as_ref().map(|o| o.architecture.clone()).unwrap_or_default(),
                container_type: overview.as_ref().map(|o| o.container_type.clone()).unwrap_or_default(),
                self_upgrade: overview.as_ref().is_some_and(|o| o.self_upgrade),
                error,
            }
        })
        .collect();
    let snapshot = Snapshot {
        entries,
        checked_at: chrono::Utc::now(),
    };
    db::state_set(state, STATE_KEY_SNAPSHOT, &snapshot).await?;
    Ok(snapshot)
}

/// The last stored poll, if any.
pub async fn snapshot(state: &State) -> Option<Snapshot> {
    db::state_get::<Snapshot>(state, STATE_KEY_SNAPSHOT).await.ok().flatten()
}

/* ------------------------------------------------------------------------------ views */

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentView {
    pub kind: String,
    pub ref_uuid: Uuid,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Host name from the daemon URL — the default SSH host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[schema(value_type = Option<String>)]
    pub installed: Option<semver::Version>,
    pub installed_raw: String,
    pub architecture: String,
    pub container_type: String,
    /// Why the daemon could not be reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[schema(value_type = Option<String>)]
    pub latest: Option<semver::Version>,
    pub outdated: bool,
    /// The strategy that would be used right now (`auto` resolved), or empty when the
    /// component cannot be updated from the panel.
    pub effective_strategy: String,
    pub can_update: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// Updating past the panel's own version needs the operator's confirmation.
    pub needs_force: bool,
    /// The target as stored (its own login fields, plus the attached profile's name).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub target: Option<TargetRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub last_run: Option<RunRow>,
    pub active_run: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PanelView {
    #[schema(value_type = String)]
    pub installed: semver::Version,
    pub installed_raw: String,
    #[schema(value_type = Option<String>)]
    pub latest: Option<semver::Version>,
    pub outdated: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    #[schema(inline)]
    pub panel: PanelView,
    #[schema(inline)]
    pub components: Vec<ComponentView>,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

/// The panel's own version. `shared::VERSION` is plain `X.Y.Z` on release builds.
pub fn panel_version() -> semver::Version {
    versions::parse_reported(shared::VERSION).unwrap_or_else(|| semver::Version::new(0, 0, 0))
}

/// Resolve `auto` and decide whether an update can be started, without touching the node.
/// `target` must already carry the login fields of its profile (see
/// [`db::resolve_credentials`]).
pub fn plan(
    kind: &str,
    overview: Option<&Overview>,
    target: Option<&TargetRow>,
    latest: Option<&semver::Version>,
    allow_ahead_of_panel: bool,
) -> (String, bool, Option<String>, bool) {
    let mut needs_force = false;
    if let Some(latest) = latest
        && !allow_ahead_of_panel
        && *latest > panel_version()
    {
        needs_force = true;
    }

    let Some(target) = target else {
        // No row: the daemon's own upgrade API is still usable for a bare-metal install.
        return match overview {
            Some(o) if o.container_type.is_empty() || o.container_type == "none" => {
                if !o.self_upgrade && versions::normalise_arch(&o.architecture).is_none() {
                    (
                        String::new(),
                        false,
                        Some(format!("no release asset exists for architecture '{}'", o.architecture)),
                        needs_force,
                    )
                } else {
                    (db::STRATEGY_API.to_string(), true, None, needs_force)
                }
            }
            Some(_) => (
                String::new(),
                false,
                Some(format!(
                    "the {} runs in a container; configure SSH with the Docker Compose strategy",
                    crate::scripts::names(kind).display
                )),
                needs_force,
            ),
            None => (String::new(), false, Some("the daemon is unreachable".to_string()), needs_force),
        };
    };

    let mut strategy = target.strategy.clone();
    if strategy == db::STRATEGY_AUTO {
        let detected = target
            .detected
            .as_ref()
            .and_then(|d| d.get("suggestedStrategy"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        strategy = if !detected.is_empty() {
            detected
        } else if target.ssh_host.is_empty() {
            db::STRATEGY_API.to_string()
        } else {
            match overview {
                Some(o) if !(o.container_type.is_empty() || o.container_type == "none") => db::STRATEGY_SSH_DOCKER.to_string(),
                _ => db::STRATEGY_SSH_BINARY.to_string(),
            }
        };
    }

    if strategy.starts_with("ssh_") {
        if target.ssh_host.is_empty() {
            return (strategy, false, Some("SSH host is not set".to_string()), needs_force);
        }
        if target.ssh_auth == db::AUTH_NONE || !target.has_secret {
            let hint = match target.profile_name.as_deref() {
                Some(profile) => format!("credential profile \"{profile}\" has no secret stored"),
                None => "SSH credentials are not set (set them or attach a credential profile)".to_string(),
            };
            return (strategy, false, Some(hint), needs_force);
        }
        if target.ssh_host_key.as_deref().unwrap_or("").is_empty() {
            return (strategy, false, Some("SSH host key has not been confirmed".to_string()), needs_force);
        }
        if strategy == db::STRATEGY_SSH_DOCKER && target.compose_dir.is_empty() {
            return (strategy, false, Some("compose directory is not set".to_string()), needs_force);
        }
        return (strategy, true, None, needs_force);
    }

    match overview {
        Some(o) if !(o.container_type.is_empty() || o.container_type == "none") => (
            strategy,
            false,
            Some("the daemon runs in a container; its upgrade API refuses, use an SSH strategy".to_string()),
            needs_force,
        ),
        Some(o) if !o.self_upgrade && versions::normalise_arch(&o.architecture).is_none() => (
            strategy,
            false,
            Some(format!("no release asset exists for architecture '{}'", o.architecture)),
            needs_force,
        ),
        Some(_) => (strategy, true, None, needs_force),
        None => (strategy, false, Some("the daemon is unreachable".to_string()), needs_force),
    }
}

/// Merge the stored daemon poll with the current targets, profiles and runs.
pub async fn view(state: &State, snapshot: &Snapshot, latest: &LatestInfo, allow_ahead_of_panel: bool) -> Result<Inventory> {
    let profiles: HashMap<Uuid, db::ProfileRow> = db::profiles_all(state)
        .await?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    let targets: HashMap<(String, Uuid), TargetRow> = db::targets_all(state)
        .await?
        .into_iter()
        .map(|t| ((t.kind.clone(), t.ref_uuid), t))
        .collect();
    let last_runs = db::runs_last_per_component(state).await?;

    let mut components = Vec::with_capacity(snapshot.entries.len());
    for entry in &snapshot.entries {
        let key = (entry.kind.clone(), entry.ref_uuid);
        let target = targets.get(&key);
        let effective = target.map(|t| match t.profile_id.and_then(|id| profiles.get(&id)) {
            Some(profile) => t.with_profile(profile),
            None => t.clone(),
        });
        let last_run = last_runs.get(&key).cloned();
        let active_run = last_run.as_ref().is_some_and(|r| db::run_is_active(&r.status));
        let latest_version = latest.latest_version(&entry.kind).cloned();
        let overview = entry.overview();
        let outdated = match (&entry.installed, &latest_version) {
            (Some(installed), Some(latest)) => installed < latest,
            _ => false,
        };
        let (effective_strategy, can_update, blocked_reason, needs_force) = plan(
            &entry.kind,
            overview.as_ref(),
            effective.as_ref(),
            latest_version.as_ref(),
            allow_ahead_of_panel,
        );
        components.push(ComponentView {
            kind: entry.kind.clone(),
            ref_uuid: entry.ref_uuid,
            name: entry.name.clone(),
            url: entry.url.clone(),
            location: entry.location.clone(),
            host: entry.host.clone(),
            installed: entry.installed.clone(),
            installed_raw: entry.installed_raw.clone(),
            architecture: entry.architecture.clone(),
            container_type: entry.container_type.clone(),
            error: entry.error.clone(),
            latest: latest_version,
            outdated,
            effective_strategy,
            can_update: can_update && !active_run,
            blocked_reason,
            needs_force,
            target: target.cloned(),
            last_run,
            active_run,
        });
    }

    let panel_installed = panel_version();
    let panel_latest = latest.latest_version(db::KIND_PANEL).cloned();
    let panel = PanelView {
        outdated: panel_latest.as_ref().is_some_and(|l| *l > panel_installed),
        installed: panel_installed,
        installed_raw: shared::VERSION.to_string(),
        latest: panel_latest,
    };

    Ok(Inventory {
        panel,
        components,
        checked_at: snapshot.checked_at,
    })
}

/// Poll every daemon and assemble the full picture.
pub async fn collect(state: &State, latest: &LatestInfo, allow_ahead_of_panel: bool) -> Result<Inventory> {
    let snapshot = poll(state).await?;
    view(state, &snapshot, latest, allow_ahead_of_panel).await
}

/// The picture from the last poll with live targets and runs, or `None` before the first poll.
pub async fn cached(state: &State, latest: &LatestInfo, allow_ahead_of_panel: bool) -> Result<Option<Inventory>> {
    match snapshot(state).await {
        Some(snapshot) => Ok(Some(view(state, &snapshot, latest, allow_ahead_of_panel).await?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonEntry, LegacyUpgrade, UpgradeRequest, daemon_error, overview_from_value, upgrade_body, url_host};
    use serde_json::json;

    #[test]
    fn upgrade_body_for_self_upgrading_daemons_is_the_new_protocol() {
        let request = UpgradeRequest {
            version: semver::Version::new(1, 2, 7),
            legacy: None,
        };
        assert_eq!(upgrade_body(&request), json!({ "version": "1.2.7", "token": null }));
    }

    #[test]
    fn legacy_upgrade_body_carries_both_protocols() {
        let request = UpgradeRequest {
            version: semver::Version::new(1, 2, 7),
            legacy: Some(LegacyUpgrade {
                url: "https://example.com/wings".into(),
                sha256: "ab".into(),
                restart_command: "systemctl".into(),
                restart_args: vec!["restart".into(), "wings".into()],
            }),
        };
        assert_eq!(
            upgrade_body(&request),
            json!({
                "version": "1.2.7",
                "token": null,
                "url": "https://example.com/wings",
                "headers": {},
                "sha256": "ab",
                "restart_command": "systemctl",
                "restart_command_args": ["restart", "wings"],
            })
        );
    }

    #[test]
    fn reads_the_release_provider_from_the_overview() {
        let base = json!({ "version": "1.2.7", "architecture": "x86_64", "container_type": "none" });
        assert!(!overview_from_value(base.clone()).self_upgrade);
        let mut none = base.clone();
        none["upgrade"] = serde_json::Value::Null;
        assert!(!overview_from_value(none).self_upgrade);
        let mut provider = base;
        provider["upgrade"] = json!({ "provider": "github", "repository": "calagopus/wings", "api_url": "https://api.github.com" });
        assert!(overview_from_value(provider).self_upgrade);
    }

    #[test]
    fn daemon_errors_are_joined() {
        assert_eq!(daemon_error(&json!({ "errors": ["a", "b"] })), "a; b");
        assert_eq!(daemon_error(&json!({ "error": "c" })), "c");
        assert_eq!(daemon_error(&serde_json::Value::Null), "no error message");
    }

    #[test]
    fn snapshots_from_1_0_0_still_load() {
        let entry: DaemonEntry = serde_json::from_value(json!({
            "kind": "wings", "ref_uuid": "123e4567-e89b-12d3-a456-426614174000", "name": "n", "url": "https://n",
            "location": null, "host": null, "installed": null, "installed_raw": "", "architecture": "", "container_type": "", "error": null,
        }))
        .unwrap();
        assert!(!entry.self_upgrade);
    }

    #[test]
    fn extracts_hosts() {
        assert_eq!(url_host("https://node1.example.com:8080/").as_deref(), Some("node1.example.com"));
        assert_eq!(url_host("http://10.0.0.5:8080").as_deref(), Some("10.0.0.5"));
        assert_eq!(url_host("https://[2001:db8::1]:8080").as_deref(), Some("[2001:db8::1]"));
    }
}
