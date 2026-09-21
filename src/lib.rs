//! Node Updater for Calagopus.
//!
//! Keeps the fleet current without touching panel core or Wings source:
//!
//! * **Release discovery** — GitHub releases of the panel, Wings and DB Agent plus the
//!   `calagopus.com/api/latest` feed the panel itself uses (`versions.rs`).
//! * **Inventory** — every node and DB Agent host is asked for its version, architecture
//!   and container type through its own system-overview endpoint (`inventory.rs`).
//! * **Notifications** — e-mail to administrators, Discord, generic webhook and the
//!   Extension Hub, once per new version with optional reminders (`notify.rs`, `checker.rs`).
//! * **Updates** — the daemon's own `POST /api/system/upgrade` for bare-metal binaries, or a
//!   script over SSH for Docker Compose, package-manager and binary installs, with a streamed
//!   log per run and a version re-check afterwards (`runner.rs`, `scripts.rs`, `ssh.rs`).
//!
//! Admin API under `/api/admin/extensions/com.pineriver.nodeupdater/`, permission group
//! `node-updater` (`read`, `update`, `settings`).

pub mod checker;
pub mod db;
pub mod health;
pub mod inventory;
pub mod notify;
pub mod routes;
pub mod runner;
pub mod scripts;
pub mod settings;
pub mod ssh;
pub mod versions;

use indexmap::IndexMap;
use shared::{
    State,
    extensions::{
        Extension, ExtensionPermissionsBuilder, ExtensionRouteBuilder, background_tasks::BackgroundTaskBuilder,
        settings::ExtensionSettingsDeserializer,
    },
    permissions::PermissionGroup,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const PACKAGE: &str = "com.pineriver.nodeupdater";
pub const LOG: &str = "extension.nodeupdater";

/// Finished runs are kept this long.
const KEEP_RUNS_DAYS: i64 = 90;

#[derive(Default)]
pub struct ExtensionStruct;

static FIRST_CHECK_DONE: AtomicBool = AtomicBool::new(false);

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn settings_deserializer(&self, _state: State) -> ExtensionSettingsDeserializer {
        Arc::new(settings::ExtensionSettingsDeserializer)
    }

    async fn initialize(&mut self, state: State) {
        match db::runs_reconcile_interrupted(&state).await {
            Ok(0) => {}
            Ok(n) => tracing::warn!(target: LOG, "marked {n} interrupted update run(s) as failed after a restart"),
            Err(err) => tracing::warn!(target: LOG, "could not reconcile interrupted runs: {err:#}"),
        }
        let settings = settings::current(&state).await;
        tracing::info!(
            target: LOG,
            "Node Updater ready — checking every {} min, e-mail {}, discord {}, webhook {}",
            settings.check_interval_minutes,
            if settings.notify_email { "on" } else { "off" },
            if settings.notify_discord_url.is_empty() { "off" } else { "on" },
            if settings.notify_webhook_url.is_empty() { "off" } else { "on" },
        );
    }

    async fn initialize_permissions(
        &mut self,
        _state: State,
        builder: ExtensionPermissionsBuilder,
    ) -> ExtensionPermissionsBuilder {
        builder.add_admin_permission_group(
            "node-updater",
            PermissionGroup {
                description: "Permissions for the Node Updater extension.",
                permissions: IndexMap::from([
                    ("read", "Allows viewing available releases, node versions, update targets (hosts and user names, never secrets) and update run logs."),
                    ("update", "Allows starting, cancelling and deleting update runs and forcing a re-check. Update runs replace the daemon binary on nodes."),
                    ("settings", "Allows configuring update targets, SSH credentials and profiles, restart commands, notification endpoints and settings. Equivalent to root access on the configured nodes."),
                ]),
            },
        )
    }

    async fn initialize_router(&mut self, state: State, builder: ExtensionRouteBuilder) -> ExtensionRouteBuilder {
        builder.add_admin_api_router(move |routes| {
            routes.nest("/extensions/com.pineriver.nodeupdater", routes::admin::router(&state))
        })
    }

    async fn initialize_background_tasks(&mut self, _state: State, builder: BackgroundTaskBuilder) -> BackgroundTaskBuilder {
        builder
            .add_task("pineriver.nodeupdater.checker", |state| async move {
                // Soon after boot, then at the configured interval.
                if FIRST_CHECK_DONE.swap(true, Ordering::Relaxed) {
                    let interval = settings::current(&state).await.check_interval();
                    tokio::time::sleep(interval).await;
                } else {
                    tokio::time::sleep(Duration::from_secs(45)).await;
                }
                let outcome = checker::run(&state, true).await?;
                let outdated = outcome.inventory.components.iter().filter(|c| c.outdated).count();
                tracing::info!(
                    target: LOG,
                    components = outcome.inventory.components.len(),
                    outdated,
                    notified = outcome.notified,
                    queued = outcome.queued,
                    "update check finished"
                );
                Ok(())
            })
            .await;
        builder
            .add_task("pineriver.nodeupdater.runner", |state| async move {
                match runner::tick(&state).await? {
                    true => {}
                    false => tokio::time::sleep(Duration::from_secs(3)).await,
                }
                Ok(())
            })
            .await;
        builder
            .add_task("pineriver.nodeupdater.housekeeping", |state| async move {
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
                let removed = db::runs_prune(&state, KEEP_RUNS_DAYS).await?;
                if removed > 0 {
                    tracing::info!(target: LOG, removed, "dropped update runs older than {KEEP_RUNS_DAYS} days");
                }
                Ok(())
            })
            .await;
        builder
    }
}
