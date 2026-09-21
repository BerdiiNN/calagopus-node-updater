//! Executes queued update runs, one at a time, streaming the log into the run row.
//!
//! A run resolves its strategy at execution time (so "auto" picks up the latest detection),
//! refuses to move a daemon past the panel's own version unless forced or allowed in the
//! settings, performs the update (daemon upgrade API or SSH script) and then polls the
//! daemon until it reports the requested version.

use crate::db::{self, RunRow, TargetRow};
use crate::settings::{self, ExtensionSettings};
use crate::{inventory, notify, scripts, ssh, versions};
use anyhow::{Context, Result, anyhow};
use shared::State;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use uuid::Uuid;

const VERIFY_POLL: Duration = Duration::from_secs(5);

fn cancels() -> &'static Mutex<HashMap<Uuid, Arc<AtomicBool>>> {
    static CANCELS: OnceLock<Mutex<HashMap<Uuid, Arc<AtomicBool>>>> = OnceLock::new();
    CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ask a running run to stop. Returns whether a run with that id is executing here.
pub fn cancel(id: Uuid) -> bool {
    let guard = cancels().lock().unwrap_or_else(|p| p.into_inner());
    match guard.get(&id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

fn register(id: Uuid) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    cancels()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id, flag.clone());
    flag
}

fn unregister(id: Uuid) {
    cancels().lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
}

async fn wait_cancel(flag: Arc<AtomicBool>) {
    loop {
        if flag.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Buffered run log: lines are appended in memory and flushed to the row roughly once a
/// second, so a chatty package manager does not turn into hundreds of tiny updates.
#[derive(Clone)]
pub struct Logger {
    buffer: Arc<Mutex<String>>,
}

impl Logger {
    fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(String::new())),
        }
    }

    /// A panel-side status line, time-stamped.
    pub fn line(&self, text: impl AsRef<str>) {
        let stamp = chrono::Utc::now().format("%H:%M:%S");
        let mut guard = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        guard.push_str(&format!("[{stamp}] {}\n", text.as_ref().trim_end()));
    }

    /// Raw remote output, appended as-is.
    pub fn raw(&self, text: &str) {
        let mut guard = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        guard.push_str(text);
    }

    fn take(&self) -> String {
        let mut guard = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut *guard)
    }

    async fn flush(&self, state: &State, id: Uuid) {
        let text = self.take();
        if !text.is_empty()
            && let Err(err) = db::run_append_log(state, id, &text).await
        {
            tracing::warn!(target: crate::LOG, run = %id, "could not append to the run log: {err:#}");
        }
    }
}

/// One scheduler pass: claim the oldest queued run and execute it. `Ok(false)` = nothing to do.
pub async fn tick(state: &State) -> Result<bool> {
    let Some(run) = db::run_claim_next(state).await? else {
        return Ok(false);
    };
    execute(state, run).await;
    Ok(true)
}

async fn execute(state: &State, run: RunRow) {
    let settings = settings::current(state).await;
    let flag = register(run.id);
    let logger = Logger::new();
    logger.line(format!(
        "update run {} for {} {} → {} (triggered by {})",
        run.id,
        scripts::names(&run.kind).display,
        run.ref_name,
        run.to_version,
        run.triggered_by
    ));

    // Flush the buffer while the run works.
    let flusher = {
        let state = state.clone();
        let logger = logger.clone();
        let id = run.id;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(900)).await;
                logger.flush(&state, id).await;
            }
        })
    };

    let outcome = tokio::select! {
        result = tokio::time::timeout(settings.update_timeout(), perform(state, &run, &settings, &logger)) => match result {
            Ok(result) => result,
            Err(_) => Err(anyhow!("the run did not finish within {} seconds", settings.update_timeout().as_secs())),
        },
        _ = wait_cancel(flag.clone()) => Err(anyhow!("cancelled by an administrator")),
    };

    flusher.abort();
    let cancelled = flag.load(Ordering::Relaxed);
    let (status, error) = match (&outcome, cancelled) {
        (_, true) => (db::RUN_CANCELLED, Some("cancelled by an administrator".to_string())),
        (Ok(()), false) => (db::RUN_SUCCEEDED, None),
        (Err(err), false) => (db::RUN_FAILED, Some(format!("{err:#}"))),
    };
    match status {
        db::RUN_SUCCEEDED => logger.line("update finished successfully"),
        _ => logger.line(format!("update {status}: {}", error.as_deref().unwrap_or(""))),
    }
    logger.flush(state, run.id).await;
    if let Err(err) = db::run_finish(state, run.id, status, error.as_deref()).await {
        tracing::error!(target: crate::LOG, run = %run.id, "could not finish the run: {err:#}");
    }
    unregister(run.id);

    if status == db::RUN_FAILED || (status == db::RUN_SUCCEEDED && settings.notify_on_success) {
        let detail = db::run_get(state, run.id).await.ok().flatten();
        if let Some(detail) = detail {
            let tail: String = detail
                .log
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            notify::send(
                state,
                &settings,
                notify::Event::RunFinished {
                    run: &detail.run,
                    log_tail: &tail,
                },
            )
            .await;
        }
    }
}

/// Build the SSH session for a target: takes the login from its credential profile when one
/// is attached, decrypts the stored secret and connects.
pub async fn connect(state: &State, target: &TargetRow, timeout: Duration) -> Result<ssh::Session> {
    let resolved = db::resolve_credentials(state, target).await?;
    let target = &resolved;
    let encrypted = target
        .ssh_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("no SSH credentials are stored for this target"))?;
    let decrypted = state
        .database
        .decrypt_base64(encrypted)
        .await
        .context("decrypting the stored SSH credentials")?;
    let secret: serde_json::Value = serde_json::from_str(&decrypted).context("reading the stored SSH credentials")?;
    let auth = match target.ssh_auth.as_str() {
        db::AUTH_PASSWORD => ssh::RemoteAuth::Password(
            secret
                .get("password")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        ),
        db::AUTH_KEY => ssh::RemoteAuth::Key {
            pem: secret.get("pem").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            passphrase: secret
                .get("passphrase")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        },
        other => return Err(anyhow!("unsupported SSH authentication '{other}'")),
    };
    let port = u16::try_from(target.ssh_port).unwrap_or(22);
    ssh::Session::connect(
        &target.ssh_host,
        port,
        &target.ssh_user,
        auth,
        target.ssh_host_key.as_deref().unwrap_or(""),
        target.use_sudo,
        timeout,
    )
    .await
}

async fn perform(state: &State, run: &RunRow, settings: &ExtensionSettings, logger: &Logger) -> Result<()> {
    let target = match db::target_get(state, &run.kind, run.ref_uuid).await? {
        Some(target) => Some(db::resolve_credentials(state, &target).await?),
        None => None,
    };
    let instance = inventory::instance(state, &run.kind, run.ref_uuid)
        .await?
        .ok_or_else(|| anyhow!("the node no longer exists in the panel"))?;
    let to_version = semver::Version::parse(&run.to_version).context("parsing the requested version")?;

    let overview = match instance.overview().await {
        Ok(overview) => {
            logger.line(format!(
                "daemon reports version {} on {} (container: {})",
                if overview.version_raw.is_empty() { "unknown" } else { &overview.version_raw },
                if overview.architecture.is_empty() { "unknown arch" } else { &overview.architecture },
                if overview.container_type.is_empty() { "unknown" } else { &overview.container_type }
            ));
            Some(overview)
        }
        Err(err) => {
            logger.line(format!("daemon did not answer its overview endpoint: {err:#}"));
            None
        }
    };
    let from_version = overview.as_ref().and_then(|o| o.version.clone());
    if from_version.as_ref() == Some(&to_version) {
        logger.line(format!("already on {to_version}; nothing to do"));
        return Ok(());
    }

    let panel = inventory::panel_version();
    if to_version > panel && !settings.allow_ahead_of_panel && !run.force {
        return Err(anyhow!(
            "{to_version} is newer than this panel ({panel}) — update the panel first, or start the update with \"force\" / enable \"allow ahead of panel\" in the settings"
        ));
    }
    if to_version > panel {
        logger.line(format!("warning: {to_version} is newer than the panel ({panel}); proceeding as requested"));
    }

    let (strategy, can_update, blocked, _) = inventory::plan(
        &run.kind,
        overview.as_ref(),
        target.as_ref(),
        Some(&to_version),
        settings.allow_ahead_of_panel,
    );
    if !can_update {
        return Err(anyhow!(
            "cannot update: {}",
            blocked.unwrap_or_else(|| "no usable strategy".to_string())
        ));
    }
    db::run_set_strategy(state, run.id, &strategy, from_version.as_ref().map(|v| v.to_string()).as_deref()).await?;
    logger.line(format!("strategy: {strategy}"));

    match strategy.as_str() {
        db::STRATEGY_API => {
            let overview = overview.as_ref().ok_or_else(|| anyhow!("the daemon is unreachable"))?;
            let names = scripts::names(&run.kind);
            let restart = target
                .as_ref()
                .map(|t| t.restart_command.trim().to_string())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| {
                    let unit = target
                        .as_ref()
                        .map(|t| t.service_name.trim().to_string())
                        .filter(|u| !u.is_empty())
                        .unwrap_or_else(|| names.default_unit.to_string());
                    format!("systemctl restart {unit}")
                });
            let mut parts = restart.split_whitespace().map(str::to_string);
            let command = parts.next().ok_or_else(|| anyhow!("empty restart command"))?;
            let args: Vec<String> = parts.collect();

            logger.line(format!("computing the SHA-256 of the {} {to_version} release asset for {} (downloaded once, cached)", names.display, overview.architecture));
            let sha256 = versions::asset_sha256(state, &run.kind, &to_version, &overview.architecture).await?;
            let url = versions::download_url(&run.kind, &to_version, &overview.architecture)
                .ok_or_else(|| anyhow!("no release asset for architecture '{}'", overview.architecture))?;
            logger.line(format!("asking the daemon to download {url} (sha256 {sha256}) and run `{restart}`"));
            let applied = instance.upgrade(&url, &sha256, &command, &args).await?;
            if !applied {
                return Err(anyhow!(
                    "the daemon refused: remote upgrades are disabled in its config (ignore_panel_wings_upgrades); use an SSH strategy or enable them"
                ));
            }
            logger.line("the daemon accepted the upgrade and is replacing its binary");
        }
        db::STRATEGY_SSH_DOCKER | db::STRATEGY_SSH_PACKAGE | db::STRATEGY_SSH_BINARY => {
            let target = target.as_ref().ok_or_else(|| anyhow!("no SSH target configured"))?;
            let architecture = overview
                .as_ref()
                .map(|o| o.architecture.clone())
                .filter(|a| !a.is_empty())
                .or_else(|| {
                    target
                        .detected
                        .as_ref()
                        .and_then(|d| d.get("architecture"))
                        .and_then(|a| a.as_str())
                        .map(str::to_string)
                });
            let sha256 = match (&strategy[..], architecture.as_deref()) {
                (db::STRATEGY_SSH_BINARY, Some(arch)) if versions::normalise_arch(arch).is_some() => {
                    logger.line(format!("computing the SHA-256 of the release asset for {arch} (downloaded once, cached)"));
                    match versions::asset_sha256(state, &run.kind, &to_version, arch).await {
                        Ok(sha) => Some(sha),
                        Err(err) => {
                            logger.line(format!("could not compute the checksum, the node will skip verification: {err:#}"));
                            None
                        }
                    }
                }
                _ => None,
            };
            let plan = scripts::UpdatePlan {
                kind: &run.kind,
                version: &to_version,
                architecture: architecture.as_deref(),
                sha256: sha256.as_deref(),
                compose_dir: &target.compose_dir,
                compose_service: &target.compose_service,
                service_name: &target.service_name,
                binary_path: &target.binary_path,
            };
            let script = match strategy.as_str() {
                db::STRATEGY_SSH_DOCKER => scripts::docker_script(&plan),
                db::STRATEGY_SSH_PACKAGE => scripts::package_script(&plan),
                _ => scripts::binary_script(&plan),
            };

            logger.line(format!(
                "connecting to {}@{}:{}{}",
                target.ssh_user,
                target.ssh_host,
                target.ssh_port,
                if target.use_sudo { " (sudo)" } else { "" }
            ));
            let session = connect(state, target, settings.ssh_timeout()).await?;
            logger.line("connected; running the update script");
            let script_budget = settings
                .update_timeout()
                .saturating_sub(settings.verify_timeout())
                .max(Duration::from_secs(60));
            let output = session
                .run_script(&script, script_budget, |chunk| logger.raw(chunk))
                .await;
            session.close().await;
            let output = output?;
            if !output.success() {
                return Err(anyhow!(
                    "the remote script exited with status {}",
                    output
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }
        }
        other => return Err(anyhow!("unknown strategy '{other}'")),
    }

    verify(&instance, &to_version, settings.verify_timeout(), logger).await
}

/// Poll the daemon until it reports at least `to_version`.
async fn verify(instance: &inventory::Instance, to_version: &semver::Version, budget: Duration, logger: &Logger) -> Result<()> {
    logger.line(format!(
        "waiting up to {} s for the daemon to come back with version {to_version}",
        budget.as_secs()
    ));
    let deadline = tokio::time::Instant::now() + budget;
    let mut last_seen = String::new();
    let mut last_error = String::new();
    loop {
        tokio::time::sleep(VERIFY_POLL).await;
        match instance.overview().await {
            Ok(overview) => {
                last_seen = overview.version_raw.clone();
                match overview.version {
                    Some(version) if version >= *to_version => {
                        logger.line(format!("the daemon reports {} — update verified", overview.version_raw));
                        return Ok(());
                    }
                    Some(version) => {
                        last_error.clear();
                        tracing::debug!(target: crate::LOG, "daemon still on {version}");
                    }
                    None => last_error = format!("unparseable version '{}'", overview.version_raw),
                }
            }
            Err(err) => last_error = format!("{err:#}"),
        }
        if tokio::time::Instant::now() >= deadline {
            let mut detail = String::new();
            if !last_seen.is_empty() {
                detail.push_str(&format!(" (last reported version: {last_seen})"));
            }
            if !last_error.is_empty() {
                detail.push_str(&format!(" (last error: {last_error})"));
            }
            return Err(anyhow!(
                "the daemon did not report version {to_version} within {} seconds{detail}",
                budget.as_secs()
            ));
        }
    }
}
