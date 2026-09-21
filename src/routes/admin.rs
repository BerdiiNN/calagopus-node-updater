//! `/api/admin/extensions/com.pineriver.nodeupdater/...`
//!
//! Permissions (admin group `node-updater`): `read` for everything that only looks,
//! `update` to start / cancel update runs, `settings` for targets (SSH credentials) and the
//! extension settings.

use super::State;
use utoipa_axum::{router::OpenApiRouter, routes};

const PERM_READ: &str = "node-updater.read";
const PERM_UPDATE: &str = "node-updater.update";
const PERM_SETTINGS: &str = "node-updater.settings";
const RUNS_LIST_MAX: i64 = 200;

/// Everything the page needs in one response.
mod overview_shape {
    use crate::inventory::{ComponentView, PanelView};
    use crate::versions::LatestInfo;
    use serde::Serialize;
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    pub struct Response {
        #[schema(inline)]
        pub panel: PanelView,
        #[schema(inline)]
        pub latest: LatestInfo,
        #[schema(inline)]
        pub components: Vec<ComponentView>,
        pub checked_at: chrono::DateTime<chrono::Utc>,
        pub runs_active: i64,
        pub allow_ahead_of_panel: bool,
        pub check_interval_minutes: i64,
        /// Background work only runs on the primary panel instance.
        pub panel_primary: bool,
        pub strategies: Vec<&'static str>,
    }

    pub async fn build(state: &shared::State, refresh: bool) -> Result<Response, anyhow::Error> {
        let settings = crate::settings::current(state).await;
        let (latest, inventory) = if refresh {
            let outcome = crate::checker::run(state, true).await?;
            (outcome.latest, outcome.inventory)
        } else {
            // The stored daemon poll, merged with the current targets, profiles and runs.
            let cached = match crate::versions::cached(state).await {
                Some(latest) => crate::inventory::cached(state, &latest, settings.allow_ahead_of_panel)
                    .await?
                    .map(|inventory| (latest, inventory)),
                None => None,
            };
            match cached {
                Some(pair) => pair,
                // Nothing polled yet: fetch quietly (no notifications, no auto-updates).
                None => crate::checker::quiet(state).await?,
            }
        };
        Ok(Response {
            panel: inventory.panel,
            latest,
            components: inventory.components,
            checked_at: inventory.checked_at,
            runs_active: crate::db::runs_count_active(state).await.unwrap_or(0),
            allow_ahead_of_panel: settings.allow_ahead_of_panel,
            check_interval_minutes: settings.check_interval_minutes,
            panel_primary: state.env.app_primary,
            strategies: crate::db::STRATEGIES.to_vec(),
        })
    }
}

mod overview {
    use super::{PERM_READ, overview_shape};
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };

    /// Latest releases, every node with its version, update targets and last runs (cached).
    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(overview_shape::Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_READ)?;
        let response = overview_shape::build(&state, false).await?;
        ApiResponse::new_serialized(response).ok()
    }
}

mod check {
    use super::{PERM_UPDATE, overview_shape};
    use crate::routes::err;
    use axum::http::StatusCode;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };

    /// Re-check GitHub / calagopus.com and every daemon right now. This also sends the
    /// notifications and queues configured auto-updates, hence the `update` permission and
    /// a short cooldown.
    #[utoipa::path(post, path = "/", responses(
        (status = OK, body = inline(overview_shape::Response)),
        (status = TOO_MANY_REQUESTS, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_UPDATE)?;
        if let Err(wait) = crate::checker::try_begin_manual() {
            return Err(err(
                StatusCode::TOO_MANY_REQUESTS,
                format!("A check just ran; try again in {wait} seconds."),
            )
            .into());
        }
        let response = overview_shape::build(&state, true).await?;
        ApiResponse::new_serialized(response).ok()
    }
}

/* ------------------------------------------------------------------------------ targets */

#[derive(serde::Deserialize)]
pub struct TargetParams {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: uuid::Uuid,
}

mod targets_list {
    use super::PERM_READ;
    use crate::db;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[schema(inline)]
        targets: Vec<db::TargetRow>,
    }

    /// Every configured update target.
    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_READ)?;
        let targets = db::targets_all(&state).await?;
        ApiResponse::new_serialized(Response { targets }).ok()
    }
}

mod target_put {
    use super::{PERM_SETTINGS, TargetParams};
    use crate::{db, routes::err, scripts};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::Deserialize;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    fn default_port() -> i32 {
        22
    }
    fn default_user() -> String {
        "root".to_string()
    }
    fn default_strategy() -> String {
        db::STRATEGY_AUTO.to_string()
    }
    fn default_auth() -> String {
        db::AUTH_NONE.to_string()
    }

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        #[serde(default = "default_strategy")]
        strategy: String,
        #[serde(default)]
        ssh_host: String,
        #[serde(default = "default_port")]
        ssh_port: i32,
        #[serde(default = "default_user")]
        ssh_user: String,
        /// none | password | key
        #[serde(default = "default_auth")]
        ssh_auth: String,
        /// New password (auth = password). Empty = keep the stored one.
        #[serde(default)]
        password: String,
        /// New OpenSSH private key (auth = key). Empty = keep the stored one.
        #[serde(default)]
        private_key: String,
        #[serde(default)]
        passphrase: String,
        /// Forget the stored credentials.
        #[serde(default)]
        clear_secret: bool,
        /// `SHA256:…` from the probe endpoint. `null` = keep, empty string = clear.
        #[serde(default)]
        host_key_fingerprint: Option<String>,
        #[serde(default)]
        use_sudo: bool,
        #[serde(default)]
        compose_dir: String,
        #[serde(default)]
        compose_service: String,
        #[serde(default)]
        service_name: String,
        #[serde(default)]
        binary_path: String,
        #[serde(default)]
        restart_command: String,
        #[serde(default)]
        auto_update: bool,
        /// Credential profile whose user / auth / secret / sudo apply to this node; `null` = own credentials.
        #[serde(default)]
        profile_id: Option<uuid::Uuid>,
    }

    /// Create or update how a node gets updated. Credentials are stored encrypted.
    #[utoipa::path(put, path = "/{kind}/{ref}", responses(
        (status = OK, body = inline(db::TargetRow)),
        (status = BAD_REQUEST, body = ApiError),
        (status = NOT_FOUND, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(
        ("kind" = String, description = "wings or db_agent"),
        ("ref" = uuid::Uuid, description = "Node or DB Agent host UUID"),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(params): Path<TargetParams>,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;

        if !db::is_kind(&params.kind) {
            return Err(err(StatusCode::BAD_REQUEST, "kind must be wings or db_agent").into());
        }
        let instance = crate::inventory::instance(&state, &params.kind, params.reference)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "No such node or DB Agent host."))?;

        if !db::STRATEGIES.contains(&data.strategy.as_str()) {
            return Err(err(StatusCode::BAD_REQUEST, "Unknown update strategy.").into());
        }
        if !matches!(data.ssh_auth.as_str(), db::AUTH_NONE | db::AUTH_PASSWORD | db::AUTH_KEY) {
            return Err(err(StatusCode::BAD_REQUEST, "ssh_auth must be none, password or key.").into());
        }
        if !(1..=65535).contains(&data.ssh_port) {
            return Err(err(StatusCode::BAD_REQUEST, "The SSH port must be 1–65535.").into());
        }
        let ssh_host = data.ssh_host.trim();
        let ssh_user = data.ssh_user.trim();
        for (label, value, max) in [
            ("SSH host", ssh_host, 253),
            ("SSH user", ssh_user, 64),
            ("compose directory", data.compose_dir.trim(), 1024),
            ("compose service", data.compose_service.trim(), 128),
            ("service name", data.service_name.trim(), 128),
            ("binary path", data.binary_path.trim(), 1024),
            ("restart command", data.restart_command.trim(), 512),
        ] {
            if !value.is_empty() && !scripts::valid_token(value, max) {
                return Err(err(StatusCode::BAD_REQUEST, format!("The {label} contains invalid characters or is too long.")).into());
            }
        }
        if data.strategy.starts_with("ssh_") && ssh_host.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "An SSH strategy needs an SSH host.").into());
        }
        if data.strategy == db::STRATEGY_SSH_DOCKER && data.compose_dir.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "The Docker Compose strategy needs the compose directory.").into());
        }
        if !data.compose_dir.trim().is_empty() && !data.compose_dir.trim().starts_with('/') {
            return Err(err(StatusCode::BAD_REQUEST, "The compose directory must be an absolute path.").into());
        }
        if !data.binary_path.trim().is_empty() && !data.binary_path.trim().starts_with('/') {
            return Err(err(StatusCode::BAD_REQUEST, "The binary path must be an absolute path.").into());
        }
        if let Some(profile_id) = data.profile_id
            && db::profile_get(&state, profile_id).await?.is_none()
        {
            return Err(err(StatusCode::BAD_REQUEST, "The selected credential profile does not exist.").into());
        }

        // Secrets: encrypt whatever was typed; otherwise keep (or clear on request).
        let (set_secret, secret) = if data.clear_secret {
            (true, None)
        } else {
            match data.ssh_auth.as_str() {
                db::AUTH_PASSWORD if !data.password.is_empty() => {
                    let json = serde_json::json!({ "password": data.password }).to_string();
                    (true, Some(state.database.encrypt_base64(json).await?.to_string()))
                }
                db::AUTH_KEY if !data.private_key.trim().is_empty() => {
                    let pem = data.private_key.trim().to_string();
                    // Validate before storing so a paste error shows up now, not at update time.
                    shared::git::parse_private_key(&pem, if data.passphrase.is_empty() { None } else { Some(&data.passphrase) })
                        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("The private key could not be read: {e}")))?;
                    let json = serde_json::json!({ "pem": pem, "passphrase": data.passphrase }).to_string();
                    (true, Some(state.database.encrypt_base64(json).await?.to_string()))
                }
                db::AUTH_NONE => (true, None),
                _ => (false, None),
            }
        };
        let (set_host_key, host_key) = match data.host_key_fingerprint.as_deref() {
            None => (false, None),
            Some("") => (true, None),
            Some(fp) => {
                if !fp.starts_with("SHA256:") || fp.len() > 120 {
                    return Err(err(StatusCode::BAD_REQUEST, "The host key fingerprint must be the SHA256:… value from the probe.").into());
                }
                (true, Some(fp))
            }
        };

        let row = db::target_upsert(
            &state,
            db::TargetInput {
                kind: &params.kind,
                ref_uuid: params.reference,
                strategy: &data.strategy,
                ssh_host,
                ssh_port: data.ssh_port,
                ssh_user: if ssh_user.is_empty() { "root" } else { ssh_user },
                ssh_auth: &data.ssh_auth,
                set_secret,
                secret,
                set_host_key,
                host_key,
                use_sudo: data.use_sudo,
                compose_dir: data.compose_dir.trim(),
                compose_service: data.compose_service.trim(),
                service_name: data.service_name.trim(),
                binary_path: data.binary_path.trim(),
                restart_command: data.restart_command.trim(),
                auto_update: data.auto_update,
                profile_id: data.profile_id,
            },
        )
        .await?;

        activity_logger
            .log(
                "nodeupdater:target.update",
                serde_json::json!({
                    "kind": params.kind,
                    "ref": params.reference,
                    "name": instance.name,
                    "strategy": data.strategy,
                    "sshHost": ssh_host,
                    "autoUpdate": data.auto_update,
                    "profileId": data.profile_id,
                }),
            )
            .await;

        ApiResponse::new_serialized(row).ok()
    }
}

mod target_delete {
    use super::{PERM_SETTINGS, TargetParams};
    use crate::db;
    use axum::extract::Path;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    struct Response {
        deleted: bool,
    }

    /// Forget a target (the node is still version-checked).
    #[utoipa::path(delete, path = "/{kind}/{ref}", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(
        ("kind" = String, description = "wings or db_agent"),
        ("ref" = uuid::Uuid, description = "Node or DB Agent host UUID"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(params): Path<TargetParams>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let deleted = db::target_delete(&state, &params.kind, params.reference).await?;
        if deleted {
            activity_logger
                .log(
                    "nodeupdater:target.delete",
                    serde_json::json!({ "kind": params.kind, "ref": params.reference }),
                )
                .await;
        }
        ApiResponse::new_serialized(Response { deleted }).ok()
    }
}

mod target_probe {
    use super::{PERM_SETTINGS, TargetParams};
    use crate::{routes::err, settings, ssh};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::{Deserialize, Serialize};
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    fn default_port() -> u16 {
        22
    }

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        #[serde(default)]
        host: String,
        #[serde(default = "default_port")]
        port: u16,
    }

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        fingerprint: String,
        algorithm: String,
    }

    /// Connect to the host and return its SSH host-key fingerprint for confirmation.
    #[utoipa::path(post, path = "/{kind}/{ref}/probe", responses(
        (status = OK, body = inline(Response)),
        (status = BAD_REQUEST, body = ApiError),
        (status = BAD_GATEWAY, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(
        ("kind" = String, description = "wings or db_agent"),
        ("ref" = uuid::Uuid, description = "Node or DB Agent host UUID"),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        Path(_params): Path<TargetParams>,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let host = data.host.trim();
        if host.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "Enter the SSH host.").into());
        }
        if !crate::scripts::valid_token(host, 253) || host.chars().any(char::is_whitespace) {
            return Err(err(StatusCode::BAD_REQUEST, "The SSH host contains invalid characters.").into());
        }
        if data.port == 0 {
            return Err(err(StatusCode::BAD_REQUEST, "The port must be 1–65535.").into());
        }
        let settings = settings::current(&state).await;
        let key = ssh::probe_host_key(host, data.port, settings.ssh_timeout())
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        ApiResponse::new_serialized(Response {
            fingerprint: key.fingerprint,
            algorithm: key.algorithm,
        })
        .ok()
    }
}

mod target_test {
    use super::{PERM_SETTINGS, TargetParams};
    use crate::{db, routes::err, runner, settings};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        ok: bool,
        output: String,
    }

    /// Open the SSH connection with the stored credentials and run a harmless command.
    #[utoipa::path(post, path = "/{kind}/{ref}/test", responses(
        (status = OK, body = inline(Response)),
        (status = NOT_FOUND, body = ApiError),
        (status = BAD_GATEWAY, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(
        ("kind" = String, description = "wings or db_agent"),
        ("ref" = uuid::Uuid, description = "Node or DB Agent host UUID"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        Path(params): Path<TargetParams>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let target = db::target_get(&state, &params.kind, params.reference)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "No update target is configured for this node."))?;
        let settings = settings::current(&state).await;
        let session = runner::connect(&state, &target, settings.ssh_timeout())
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let output = session
            .run("echo \"user: $(id -un)\"; echo \"host: $(hostname)\"; uname -srm; if [ \"$(id -u)\" -eq 0 ]; then echo 'root: yes'; else echo 'root: no'; fi", settings.ssh_timeout())
            .await;
        session.close().await;
        let output = output.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        ApiResponse::new_serialized(Response {
            ok: output.success(),
            output: output.combined(),
        })
        .ok()
    }
}

mod target_detect {
    use super::{PERM_SETTINGS, TargetParams};
    use crate::{db, routes::err, runner, scripts, settings};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[schema(inline)]
        detected: scripts::Detected,
        raw: String,
    }

    /// Inspect the node over SSH: package, service, binary or Docker container, and the
    /// strategy those findings suggest. The result is stored on the target.
    #[utoipa::path(post, path = "/{kind}/{ref}/detect", responses(
        (status = OK, body = inline(Response)),
        (status = NOT_FOUND, body = ApiError),
        (status = BAD_GATEWAY, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(
        ("kind" = String, description = "wings or db_agent"),
        ("ref" = uuid::Uuid, description = "Node or DB Agent host UUID"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        Path(params): Path<TargetParams>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let target = db::target_get(&state, &params.kind, params.reference)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "No update target is configured for this node."))?;
        let settings = settings::current(&state).await;
        let session = runner::connect(&state, &target, settings.ssh_timeout())
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let output = session
            .run(&scripts::detect_script(&params.kind), std::time::Duration::from_secs(90))
            .await;
        session.close().await;
        let output = output.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let detected = scripts::parse_detection(&output.stdout);
        let json = serde_json::to_value(&detected)?;
        db::target_set_detected(&state, target.id, &json).await?;
        ApiResponse::new_serialized(Response {
            detected,
            raw: output.combined(),
        })
        .ok()
    }
}

/* ------------------------------------------------------------------------------ updates */

mod update_queue {
    use super::PERM_UPDATE;
    use crate::{db, inventory, routes::err, settings, versions};
    use axum::http::StatusCode;
    use serde::Deserialize;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::{GetPermissionManager, GetUser}},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        kind: String,
        ref_uuid: uuid::Uuid,
        /// Defaults to the latest release.
        #[serde(default)]
        version: Option<String>,
        /// Allow a version newer than the panel's own.
        #[serde(default)]
        force: bool,
    }

    /// Queue an update run for one node.
    #[utoipa::path(post, path = "/", responses(
        (status = OK, body = inline(db::RunRow)),
        (status = BAD_REQUEST, body = ApiError),
        (status = NOT_FOUND, body = ApiError),
        (status = CONFLICT, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        user: GetUser,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_UPDATE)?;
        if !db::is_kind(&data.kind) {
            return Err(err(StatusCode::BAD_REQUEST, "kind must be wings or db_agent").into());
        }
        let settings = settings::current(&state).await;
        let latest = versions::cached_or_refresh(&state, settings.include_prereleases, settings.check_interval()).await?;
        let instance = inventory::instance(&state, &data.kind, data.ref_uuid)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "No such node or DB Agent host."))?;

        let to_version = match data.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => {
                let version = versions::parse_tag(v).ok_or_else(|| err(StatusCode::BAD_REQUEST, "The version must look like 1.2.0."))?;
                // Only releases the panel has seen on GitHub: no typos, no arbitrary downgrades.
                if latest.release(&data.kind, &version).is_none() {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        format!("{version} is not among the known releases; pick one from the release list or check for updates first."),
                    )
                    .into());
                }
                version
            }
            None => latest
                .latest_version(&data.kind)
                .cloned()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "The latest release is not known yet; check for updates first."))?,
        };
        if to_version > inventory::panel_version() && !settings.allow_ahead_of_panel && !data.force {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!(
                    "{to_version} is newer than this panel ({}). Update the panel first, or confirm the update with force.",
                    shared::VERSION
                ),
            )
            .into());
        }
        if db::run_active_for(&state, &data.kind, data.ref_uuid).await? {
            return Err(err(StatusCode::CONFLICT, "An update for this node is already queued or running.").into());
        }

        let overview = instance.overview().await.ok();
        let target = match db::target_get(&state, &data.kind, data.ref_uuid).await? {
            Some(target) => Some(db::resolve_credentials(&state, &target).await?),
            None => None,
        };
        let (strategy, can_update, blocked, _) = inventory::plan(
            &data.kind,
            overview.as_ref(),
            target.as_ref(),
            Some(&to_version),
            settings.allow_ahead_of_panel,
        );
        if !can_update {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("This node cannot be updated from the panel: {}", blocked.unwrap_or_default()),
            )
            .into());
        }

        let triggered_by = format!("manual:{}", user.username);
        let run = db::run_create(
            &state,
            db::NewRun {
                kind: &data.kind,
                ref_uuid: data.ref_uuid,
                ref_name: &instance.name,
                strategy: &strategy,
                from_version: overview.as_ref().and_then(|o| o.version.as_ref()).map(|v| v.to_string()).as_deref(),
                to_version: &to_version.to_string(),
                triggered_by: &triggered_by,
                force: data.force,
            },
        )
        .await
        .map_err(|e| {
            if db::is_duplicate_active_run(&e) {
                err(StatusCode::CONFLICT, "An update for this node is already queued or running.")
            } else {
                e
            }
        })?;

        activity_logger
            .log(
                "nodeupdater:update.queue",
                serde_json::json!({
                    "kind": data.kind,
                    "ref": data.ref_uuid,
                    "name": instance.name,
                    "toVersion": to_version.to_string(),
                    "strategy": strategy,
                    "force": data.force,
                    "run": run.id,
                }),
            )
            .await;

        ApiResponse::new_serialized(run).ok()
    }
}

mod update_bulk {
    use super::PERM_UPDATE;
    use crate::{db, inventory, settings, versions};
    use serde::{Deserialize, Serialize};
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::{GetPermissionManager, GetUser}},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        /// Restrict to wings or db_agent; omit for both.
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        force: bool,
    }

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Skipped {
        name: String,
        reason: String,
    }

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        queued: usize,
        #[schema(inline)]
        skipped: Vec<Skipped>,
    }

    /// Queue an update for every outdated node that can be updated.
    #[utoipa::path(post, path = "/bulk", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        user: GetUser,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_UPDATE)?;
        let settings = settings::current(&state).await;
        let latest = versions::cached_or_refresh(&state, settings.include_prereleases, settings.check_interval()).await?;
        let inventory = inventory::collect(&state, &latest, settings.allow_ahead_of_panel).await?;
        let triggered_by = format!("bulk:{}", user.username);

        let mut queued = 0;
        let mut skipped = Vec::new();
        for component in inventory.components {
            if let Some(kind) = data.kind.as_deref()
                && component.kind != kind
            {
                continue;
            }
            if !component.outdated {
                continue;
            }
            let Some(latest) = component.latest.as_ref() else { continue };
            if component.active_run {
                skipped.push(Skipped { name: component.name, reason: "an update is already queued or running".into() });
                continue;
            }
            if !component.can_update {
                skipped.push(Skipped {
                    name: component.name,
                    reason: component.blocked_reason.unwrap_or_else(|| "cannot be updated from the panel".into()),
                });
                continue;
            }
            if component.needs_force && !data.force {
                skipped.push(Skipped {
                    name: component.name,
                    reason: format!("{latest} is newer than the panel; confirm with force"),
                });
                continue;
            }
            let created = db::run_create(
                &state,
                db::NewRun {
                    kind: &component.kind,
                    ref_uuid: component.ref_uuid,
                    ref_name: &component.name,
                    strategy: &component.effective_strategy,
                    from_version: component.installed.as_ref().map(|v| v.to_string()).as_deref(),
                    to_version: &latest.to_string(),
                    triggered_by: &triggered_by,
                    force: data.force,
                },
            )
            .await;
            match created {
                Ok(_) => queued += 1,
                Err(e) if db::is_duplicate_active_run(&e) => {
                    skipped.push(Skipped { name: component.name, reason: "an update is already queued or running".into() });
                }
                Err(e) => return Err(e.into()),
            }
        }

        activity_logger
            .log(
                "nodeupdater:update.bulk",
                serde_json::json!({ "kind": data.kind, "queued": queued, "skipped": skipped.len(), "force": data.force }),
            )
            .await;

        ApiResponse::new_serialized(Response { queued, skipped }).ok()
    }
}

/* --------------------------------------------------------------------------------- runs */

#[derive(serde::Deserialize)]
pub struct RunParams {
    pub run: uuid::Uuid,
}

mod runs_list {
    use super::{PERM_READ, RUNS_LIST_MAX};
    use crate::db;
    use axum::extract::Query;
    use serde::{Deserialize, Serialize};
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Deserialize)]
    pub struct Params {
        #[serde(default)]
        limit: Option<i64>,
        #[serde(default)]
        offset: Option<i64>,
    }

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[schema(inline)]
        runs: Vec<db::RunRow>,
    }

    /// Update runs, newest first (without logs).
    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager, Query(params): Query<Params>) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_READ)?;
        let limit = params.limit.unwrap_or(50).clamp(1, RUNS_LIST_MAX);
        let offset = params.offset.unwrap_or(0).max(0);
        let runs = db::runs_list(&state, limit, offset).await?;
        ApiResponse::new_serialized(Response { runs }).ok()
    }
}

mod run_get {
    use super::{PERM_READ, RunParams};
    use crate::{db, routes::err};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };

    /// One run with its full log (the page polls this while it runs).
    #[utoipa::path(get, path = "/{run}", responses(
        (status = OK, body = inline(db::RunDetail)),
        (status = NOT_FOUND, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(("run" = uuid::Uuid, description = "The run ID")))]
    pub async fn route(state: GetState, permissions: GetPermissionManager, Path(params): Path<RunParams>) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_READ)?;
        let run = db::run_get(&state, params.run)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Run not found."))?;
        ApiResponse::new_serialized(run).ok()
    }
}

mod run_cancel {
    use super::{PERM_UPDATE, RunParams};
    use crate::{db, routes::err, runner};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    struct Response {
        cancelled: bool,
    }

    /// Cancel a queued run, or ask a running one to stop between steps.
    #[utoipa::path(post, path = "/{run}/cancel", responses(
        (status = OK, body = inline(Response)),
        (status = NOT_FOUND, body = ApiError),
        (status = CONFLICT, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(("run" = uuid::Uuid, description = "The run ID")))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(params): Path<RunParams>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_UPDATE)?;
        let run = db::run_get(&state, params.run)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Run not found."))?;
        let cancelled = match run.run.status.as_str() {
            db::RUN_QUEUED => db::run_cancel_queued(&state, params.run).await?,
            db::RUN_RUNNING => {
                if !runner::cancel(params.run) {
                    return Err(err(
                        StatusCode::CONFLICT,
                        "This run executes on another panel instance and cannot be cancelled from here.",
                    )
                    .into());
                }
                true
            }
            _ => return Err(err(StatusCode::CONFLICT, "The run has already finished.").into()),
        };
        activity_logger
            .log("nodeupdater:update.cancel", serde_json::json!({ "run": params.run, "cancelled": cancelled }))
            .await;
        ApiResponse::new_serialized(Response { cancelled }).ok()
    }
}

mod run_delete {
    use super::{PERM_UPDATE, RunParams};
    use crate::{db, routes::err};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    struct Response {
        deleted: bool,
    }

    /// Delete a finished run and its log.
    #[utoipa::path(delete, path = "/{run}", responses(
        (status = OK, body = inline(Response)),
        (status = CONFLICT, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(("run" = uuid::Uuid, description = "The run ID")))]
    pub async fn route(state: GetState, permissions: GetPermissionManager, Path(params): Path<RunParams>) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_UPDATE)?;
        let deleted = db::run_delete(&state, params.run).await?;
        if !deleted {
            return Err(err(StatusCode::CONFLICT, "The run is still active or does not exist.").into());
        }
        ApiResponse::new_serialized(Response { deleted }).ok()
    }
}

/* ----------------------------------------------------------------------------- profiles */

#[derive(serde::Deserialize)]
pub struct ProfileParams {
    pub profile: uuid::Uuid,
}

mod profile_shared {
    use crate::{db, routes::err};
    use axum::http::StatusCode;
    use serde::Deserialize;
    use utoipa::ToSchema;

    fn default_user() -> String {
        "root".to_string()
    }
    fn default_auth() -> String {
        db::AUTH_KEY.to_string()
    }

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        #[serde(default)]
        pub name: String,
        #[serde(default = "default_user")]
        pub ssh_user: String,
        /// password | key
        #[serde(default = "default_auth")]
        pub ssh_auth: String,
        /// New password (auth = password). Empty = keep the stored one.
        #[serde(default)]
        pub password: String,
        /// New OpenSSH private key (auth = key). Empty = keep the stored one.
        #[serde(default)]
        pub private_key: String,
        #[serde(default)]
        pub passphrase: String,
        #[serde(default)]
        pub use_sudo: bool,
    }

    /// Validate the fields and encrypt a typed secret. Returns `(set_secret, secret)`.
    pub async fn prepare(state: &shared::State, data: &Payload) -> Result<(bool, Option<String>), anyhow::Error> {
        let name = data.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(|c| c.is_control()) {
            return Err(err(StatusCode::BAD_REQUEST, "The profile name must be 1–64 characters."));
        }
        if !matches!(data.ssh_auth.as_str(), db::AUTH_PASSWORD | db::AUTH_KEY) {
            return Err(err(StatusCode::BAD_REQUEST, "ssh_auth must be password or key."));
        }
        let user = data.ssh_user.trim();
        if user.is_empty() || !crate::scripts::valid_token(user, 64) {
            return Err(err(StatusCode::BAD_REQUEST, "The SSH user is missing or invalid."));
        }
        match data.ssh_auth.as_str() {
            db::AUTH_PASSWORD if !data.password.is_empty() => {
                let json = serde_json::json!({ "password": data.password }).to_string();
                Ok((true, Some(state.database.encrypt_base64(json).await?.to_string())))
            }
            db::AUTH_KEY if !data.private_key.trim().is_empty() => {
                let pem = data.private_key.trim().to_string();
                shared::git::parse_private_key(&pem, if data.passphrase.is_empty() { None } else { Some(&data.passphrase) })
                    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("The private key could not be read: {e}")))?;
                let json = serde_json::json!({ "pem": pem, "passphrase": data.passphrase }).to_string();
                Ok((true, Some(state.database.encrypt_base64(json).await?.to_string())))
            }
            _ => Ok((false, None)),
        }
    }
}

mod profiles_list {
    use super::PERM_SETTINGS;
    use crate::db;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[schema(inline)]
        profiles: Vec<db::ProfileRow>,
    }

    /// Every credential profile (secrets are never returned).
    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let profiles = db::profiles_all(&state).await?;
        ApiResponse::new_serialized(Response { profiles }).ok()
    }
}

mod profile_create {
    use super::{PERM_SETTINGS, profile_shared};
    use crate::{db, routes::err};
    use axum::http::StatusCode;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };

    /// Create a credential profile. A password or private key is required.
    #[utoipa::path(post, path = "/", responses(
        (status = OK, body = inline(db::ProfileRow)),
        (status = BAD_REQUEST, body = ApiError),
        (status = CONFLICT, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), request_body = inline(profile_shared::Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<profile_shared::Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let (set_secret, secret) = profile_shared::prepare(&state, &data).await?;
        if !set_secret {
            return Err(err(StatusCode::BAD_REQUEST, "Enter a password or a private key for the profile.").into());
        }
        let name = data.name.trim();
        if db::profile_by_name(&state, name).await?.is_some() {
            return Err(err(StatusCode::CONFLICT, "A profile with that name already exists.").into());
        }
        let row = db::profile_create(
            &state,
            db::ProfileInput {
                name,
                ssh_user: data.ssh_user.trim(),
                ssh_auth: &data.ssh_auth,
                set_secret,
                secret,
                use_sudo: data.use_sudo,
            },
        )
        .await?;
        activity_logger
            .log(
                "nodeupdater:profile.create",
                serde_json::json!({ "profile": row.id, "name": row.name, "sshUser": row.ssh_user, "sshAuth": row.ssh_auth }),
            )
            .await;
        ApiResponse::new_serialized(row).ok()
    }
}

mod profile_update {
    use super::{PERM_SETTINGS, ProfileParams, profile_shared};
    use crate::{db, routes::err};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };

    /// Update a credential profile. An empty password / key keeps the stored secret unless the
    /// authentication method changes.
    #[utoipa::path(put, path = "/{profile}", responses(
        (status = OK, body = inline(db::ProfileRow)),
        (status = BAD_REQUEST, body = ApiError),
        (status = NOT_FOUND, body = ApiError),
        (status = CONFLICT, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(("profile" = uuid::Uuid, description = "The profile ID")), request_body = inline(profile_shared::Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(params): Path<ProfileParams>,
        shared::Payload(data): shared::Payload<profile_shared::Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let existing = db::profile_get(&state, params.profile)
            .await?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Profile not found."))?;
        let (set_secret, secret) = profile_shared::prepare(&state, &data).await?;
        if !set_secret && existing.ssh_auth != data.ssh_auth {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Enter the new password or private key when changing the authentication method.",
            )
            .into());
        }
        let name = data.name.trim();
        if let Some(other) = db::profile_by_name(&state, name).await?
            && other.id != existing.id
        {
            return Err(err(StatusCode::CONFLICT, "A profile with that name already exists.").into());
        }
        let row = db::profile_update(
            &state,
            existing.id,
            db::ProfileInput {
                name,
                ssh_user: data.ssh_user.trim(),
                ssh_auth: &data.ssh_auth,
                set_secret,
                secret,
                use_sudo: data.use_sudo,
            },
        )
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Profile not found."))?;
        activity_logger
            .log(
                "nodeupdater:profile.update",
                serde_json::json!({ "profile": row.id, "name": row.name, "sshUser": row.ssh_user, "sshAuth": row.ssh_auth, "secretChanged": set_secret }),
            )
            .await;
        ApiResponse::new_serialized(row).ok()
    }
}

mod profile_delete {
    use super::{PERM_SETTINGS, ProfileParams};
    use crate::db;
    use axum::extract::Path;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    struct Response {
        deleted: bool,
    }

    /// Delete a profile. Targets that used it fall back to their own credentials.
    #[utoipa::path(delete, path = "/{profile}", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ), params(("profile" = uuid::Uuid, description = "The profile ID")))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(params): Path<ProfileParams>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let deleted = db::profile_delete(&state, params.profile).await?;
        if deleted {
            activity_logger
                .log("nodeupdater:profile.delete", serde_json::json!({ "profile": params.profile }))
                .await;
        }
        ApiResponse::new_serialized(Response { deleted }).ok()
    }
}

/* ----------------------------------------------------------------------------- settings */

mod settings_get {
    use super::PERM_SETTINGS;
    use crate::settings;
    use serde::Serialize;
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Serialize, ToSchema)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[serde(flatten)]
        #[schema(inline)]
        settings: settings::ExtensionSettings,
        webhook_secret_set: bool,
        panel_version: String,
    }

    /// The extension settings (the webhook secret is never returned).
    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let settings = settings::current(&state).await;
        ApiResponse::new_serialized(Response {
            webhook_secret_set: !settings.notify_webhook_secret.is_empty(),
            settings,
            panel_version: shared::VERSION.to_string(),
        })
        .ok()
    }
}

mod settings_put {
    use super::PERM_SETTINGS;
    use crate::{routes::err, settings};
    use axum::http::StatusCode;
    use serde::{Deserialize, Serialize};
    use shared::{
        ApiError, GetState,
        models::{admin_activity::GetAdminActivityLogger, user::GetPermissionManager},
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        #[serde(default)]
        check_interval_minutes: Option<i64>,
        #[serde(default)]
        include_prereleases: Option<bool>,
        #[serde(default)]
        allow_ahead_of_panel: Option<bool>,
        #[serde(default)]
        notify_email: Option<bool>,
        #[serde(default)]
        notify_on_success: Option<bool>,
        #[serde(default)]
        notify_discord_url: Option<String>,
        #[serde(default)]
        notify_webhook_url: Option<String>,
        /// Empty / absent keeps the stored secret; send `clear_webhook_secret` to remove it.
        #[serde(default)]
        notify_webhook_secret: Option<String>,
        #[serde(default)]
        clear_webhook_secret: bool,
        #[serde(default)]
        remind_after_hours: Option<i64>,
        #[serde(default)]
        ssh_connect_timeout_secs: Option<i64>,
        #[serde(default)]
        update_timeout_secs: Option<i64>,
        #[serde(default)]
        verify_timeout_secs: Option<i64>,
    }

    #[derive(Serialize, ToSchema)]
    struct Response {
        ok: bool,
    }

    /// Update the settings (values are clamped to safe ranges).
    #[utoipa::path(put, path = "/", responses(
        (status = OK, body = inline(Response)),
        (status = UNAUTHORIZED, body = ApiError),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;

        let mut next = settings::current(&state).await;
        if let Some(v) = data.check_interval_minutes {
            next.check_interval_minutes = v;
        }
        if let Some(v) = data.include_prereleases {
            next.include_prereleases = v;
        }
        if let Some(v) = data.allow_ahead_of_panel {
            next.allow_ahead_of_panel = v;
        }
        if let Some(v) = data.notify_email {
            next.notify_email = v;
        }
        if let Some(v) = data.notify_on_success {
            next.notify_on_success = v;
        }
        if let Some(v) = data.notify_discord_url {
            next.notify_discord_url = v;
        }
        if let Some(v) = data.notify_webhook_url {
            next.notify_webhook_url = v;
        }
        if let Some(v) = data.remind_after_hours {
            next.remind_after_hours = v;
        }
        if let Some(v) = data.ssh_connect_timeout_secs {
            next.ssh_connect_timeout_secs = v;
        }
        if let Some(v) = data.update_timeout_secs {
            next.update_timeout_secs = v;
        }
        if let Some(v) = data.verify_timeout_secs {
            next.verify_timeout_secs = v;
        }
        let typed_secret = data.notify_webhook_secret.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let keep_secret = !data.clear_webhook_secret && typed_secret.is_none();
        if data.clear_webhook_secret {
            next.notify_webhook_secret.clear();
        } else if let Some(secret) = typed_secret {
            next.notify_webhook_secret = secret.to_string();
        }

        // Destinations: Discord must be a real Discord webhook; the generic webhook must not
        // point at an address the panel blocks for outbound requests.
        let discord = next.notify_discord_url.trim();
        if !discord.is_empty() && !crate::notify::is_discord_webhook_url(discord) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "The Discord URL must be a https://discord.com/api/webhooks/… URL.",
            )
            .into());
        }
        let webhook = next.notify_webhook_url.trim();
        if !webhook.is_empty()
            && let Err(reason) = crate::notify::validate_webhook_url(&state, webhook).await
        {
            return Err(err(StatusCode::BAD_REQUEST, format!("The webhook URL was rejected: {reason}")).into());
        }

        settings::store(&state, next.clone(), keep_secret).await?;

        activity_logger
            .log(
                "nodeupdater:settings.update",
                serde_json::json!({
                    "checkIntervalMinutes": next.check_interval_minutes,
                    "includePrereleases": next.include_prereleases,
                    "allowAheadOfPanel": next.allow_ahead_of_panel,
                    "notifyEmail": next.notify_email,
                    "notifyOnSuccess": next.notify_on_success,
                    "discord": !next.notify_discord_url.is_empty(),
                    "webhook": !next.notify_webhook_url.is_empty(),
                    "remindAfterHours": next.remind_after_hours,
                }),
            )
            .await;

        ApiResponse::new_serialized(Response { ok: true }).ok()
    }
}

mod settings_test {
    use super::PERM_SETTINGS;
    use crate::{notify, routes::err, settings};
    use axum::http::StatusCode;
    use serde::{Deserialize, Serialize};
    use shared::{
        ApiError, GetState,
        models::user::GetPermissionManager,
        response::{ApiResponse, ApiResponseResult},
    };
    use utoipa::ToSchema;

    #[derive(Deserialize, ToSchema)]
    pub struct Payload {
        /// email | discord | webhook
        channel: String,
    }

    #[derive(Serialize, ToSchema)]
    struct Response {
        ok: bool,
    }

    /// Send a test notification through one channel.
    #[utoipa::path(post, path = "/test", responses(
        (status = OK, body = inline(Response)),
        (status = BAD_GATEWAY, body = ApiError),
        (status = UNAUTHORIZED, body = ApiError),
    ), request_body = inline(Payload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        shared::Payload(data): shared::Payload<Payload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission(PERM_SETTINGS)?;
        let settings = settings::current(&state).await;
        notify::test_channel(&state, &settings, &data.channel)
            .await
            .map_err(|message| err(StatusCode::BAD_GATEWAY, message))?;
        ApiResponse::new_serialized(Response { ok: true }).ok()
    }
}

pub fn router(state: &State) -> OpenApiRouter<State> {
    OpenApiRouter::new()
        .nest("/overview", {
            OpenApiRouter::new()
                .routes(routes!(overview::route))
                .with_state(state.clone())
        })
        .nest("/check", {
            OpenApiRouter::new()
                .routes(routes!(check::route))
                .with_state(state.clone())
        })
        .nest("/targets", {
            OpenApiRouter::new()
                .routes(routes!(targets_list::route))
                .routes(routes!(target_put::route, target_delete::route))
                .routes(routes!(target_probe::route))
                .routes(routes!(target_test::route))
                .routes(routes!(target_detect::route))
                .with_state(state.clone())
        })
        .nest("/updates", {
            OpenApiRouter::new()
                .routes(routes!(update_queue::route))
                .routes(routes!(update_bulk::route))
                .with_state(state.clone())
        })
        .nest("/runs", {
            OpenApiRouter::new()
                .routes(routes!(runs_list::route))
                .routes(routes!(run_get::route, run_delete::route))
                .routes(routes!(run_cancel::route))
                .with_state(state.clone())
        })
        .nest("/profiles", {
            OpenApiRouter::new()
                .routes(routes!(profiles_list::route, profile_create::route))
                .routes(routes!(profile_update::route, profile_delete::route))
                .with_state(state.clone())
        })
        .nest("/settings", {
            OpenApiRouter::new()
                .routes(routes!(settings_get::route, settings_put::route))
                .routes(routes!(settings_test::route))
                .with_state(state.clone())
        })
        .with_state(state.clone())
}
