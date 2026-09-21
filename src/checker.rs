//! The periodic check: refresh the latest releases, poll every daemon, notify about new
//! versions (once per version, with optional reminders), queue auto-updates and report to
//! the Extension Hub.

use crate::db;
use crate::inventory::{self, Inventory};
use crate::notify::{self, OutdatedInstance};
use crate::settings;
use crate::versions::{self, LatestInfo};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::State;

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Manual "check now" requests hit GitHub and every daemon; keep them this far apart.
const MANUAL_COOLDOWN_SECS: i64 = 15;
static LAST_MANUAL: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Claim a manual check slot. `Err(seconds)` = how long to wait.
pub fn try_begin_manual() -> Result<(), i64> {
    let now = chrono::Utc::now().timestamp();
    let last = LAST_MANUAL.load(std::sync::atomic::Ordering::Relaxed);
    if now - last < MANUAL_COOLDOWN_SECS {
        return Err(MANUAL_COOLDOWN_SECS - (now - last));
    }
    LAST_MANUAL.store(now, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Latest releases plus a fresh daemon poll, with no notifications and no auto-update
/// queueing — what a read-only viewer may trigger when nothing is cached yet.
pub async fn quiet(state: &State) -> Result<(LatestInfo, Inventory)> {
    let _guard = LOCK.lock().await;
    let settings = settings::current(state).await;
    let latest = versions::cached_or_refresh(state, settings.include_prereleases, settings.check_interval()).await?;
    let inventory = inventory::collect(state, &latest, settings.allow_ahead_of_panel).await?;
    Ok((latest, inventory))
}

pub struct Outcome {
    pub latest: LatestInfo,
    pub inventory: Inventory,
    pub notified: usize,
    pub queued: usize,
}

#[derive(Serialize, Deserialize, Default)]
struct Marker {
    first: chrono::DateTime<chrono::Utc>,
    last: chrono::DateTime<chrono::Utc>,
    count: u32,
}

fn marker_key(kind: &str, version: &semver::Version) -> String {
    format!("notified:{kind}:{version}")
}

/// One full check. `refresh_latest` forces a GitHub / feed round-trip; otherwise the cached
/// result is reused while it is younger than the check interval.
pub async fn run(state: &State, refresh_latest: bool) -> Result<Outcome> {
    let _guard = LOCK.lock().await;
    let settings = settings::current(state).await;

    let latest = if refresh_latest {
        versions::refresh(state, settings.include_prereleases).await?
    } else {
        versions::cached_or_refresh(state, settings.include_prereleases, settings.check_interval()).await?
    };
    let inventory = inventory::collect(state, &latest, settings.allow_ahead_of_panel).await?;

    // Targets whose node vanished.
    let live: Vec<(String, uuid::Uuid)> = inventory
        .components
        .iter()
        .map(|c| (c.kind.clone(), c.ref_uuid))
        .collect();
    match db::targets_prune(state, &live).await {
        Ok(0) | Err(_) => {}
        Ok(n) => tracing::info!(target: crate::LOG, removed = n, "dropped targets of deleted nodes"),
    }

    let notified = notify_new_versions(state, &settings, &latest, &inventory).await;
    let queued = queue_auto_updates(state, &settings, &inventory).await;
    report_health(state, &latest, &inventory).await;

    Ok(Outcome {
        latest,
        inventory,
        notified,
        queued,
    })
}

async fn notify_new_versions(
    state: &State,
    settings: &settings::ExtensionSettings,
    latest: &LatestInfo,
    inventory: &Inventory,
) -> usize {
    let mut sent = 0;
    let panel_version = inventory::panel_version();
    let now = chrono::Utc::now();

    for kind in [db::KIND_PANEL, db::KIND_WINGS, db::KIND_DB_AGENT] {
        let Some(component) = latest.for_kind(kind) else { continue };
        let Some(release) = component.latest.as_ref() else { continue };

        let outdated: Vec<OutdatedInstance> = if kind == db::KIND_PANEL {
            Vec::new()
        } else {
            inventory
                .components
                .iter()
                .filter(|c| c.kind == kind && c.outdated)
                .map(|c| OutdatedInstance {
                    name: c.name.clone(),
                    installed: c
                        .installed
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| c.installed_raw.clone()),
                })
                .collect()
        };
        let anything_outdated = if kind == db::KIND_PANEL {
            inventory.panel.outdated
        } else {
            !outdated.is_empty()
        };
        if !anything_outdated {
            continue;
        }

        let key = marker_key(kind, &release.version);
        let marker = db::state_get::<Marker>(state, &key).await.ok().flatten();
        let reminder = match &marker {
            None => false,
            Some(marker) => {
                if settings.remind_after_hours <= 0 {
                    continue;
                }
                let due = marker.last + chrono::Duration::hours(settings.remind_after_hours);
                if now < due {
                    continue;
                }
                true
            }
        };

        notify::send(
            state,
            settings,
            notify::Event::UpdateAvailable {
                kind,
                latest: &release.version,
                recommended: component.recommended.as_ref(),
                release_url: &release.html_url,
                outdated: &outdated,
                panel_behind: release.version > panel_version,
                reminder,
            },
        )
        .await;
        sent += 1;

        let next = Marker {
            first: marker.as_ref().map(|m| m.first).unwrap_or(now),
            last: now,
            count: marker.as_ref().map(|m| m.count + 1).unwrap_or(1),
        };
        if let Err(err) = db::state_set(state, &key, &next).await {
            tracing::warn!(target: crate::LOG, "could not store the notification marker: {err:#}");
        }
    }
    sent
}

async fn queue_auto_updates(state: &State, settings: &settings::ExtensionSettings, inventory: &Inventory) -> usize {
    let mut queued = 0;
    for component in &inventory.components {
        let Some(target) = component.target.as_ref() else { continue };
        if !target.auto_update || !component.outdated || !component.can_update || component.active_run {
            continue;
        }
        let Some(latest) = component.latest.as_ref() else { continue };
        if component.needs_force && !settings.allow_ahead_of_panel {
            tracing::info!(
                target: crate::LOG,
                node = %component.name,
                "auto-update skipped: {latest} is newer than the panel; update the panel first"
            );
            continue;
        }
        let result = db::run_create(
            state,
            db::NewRun {
                kind: &component.kind,
                ref_uuid: component.ref_uuid,
                ref_name: &component.name,
                strategy: &component.effective_strategy,
                from_version: component.installed.as_ref().map(|v| v.to_string()).as_deref(),
                to_version: &latest.to_string(),
                triggered_by: "auto",
                force: false,
            },
        )
        .await;
        match result {
            Ok(_) => {
                queued += 1;
                tracing::info!(target: crate::LOG, node = %component.name, "auto-update to {latest} queued");
            }
            Err(err) if db::is_duplicate_active_run(&err) => {
                tracing::debug!(target: crate::LOG, node = %component.name, "auto-update skipped: a run is already active");
            }
            Err(err) => tracing::warn!(target: crate::LOG, node = %component.name, "could not queue auto-update: {err:#}"),
        }
    }
    queued
}

async fn report_health(state: &State, latest: &LatestInfo, inventory: &Inventory) {
    let total = inventory.components.len();
    let outdated = inventory.components.iter().filter(|c| c.outdated).count();
    let unreachable = inventory.components.iter().filter(|c| c.error.is_some()).count();
    let mut parts = vec![format!("{} of {total} daemons up to date", total - outdated - unreachable)];
    if inventory.panel.outdated
        && let Some(latest) = &inventory.panel.latest
    {
        parts.push(format!("panel {latest} available"));
    }
    if unreachable > 0 {
        parts.push(format!("{unreachable} unreachable"));
    }
    if let Some(err) = &latest.feed_error {
        parts.push(format!("feed: {err}"));
    }
    for kind in [db::KIND_WINGS, db::KIND_DB_AGENT, db::KIND_PANEL] {
        if let Some(err) = latest.for_kind(kind).and_then(|c| c.error.as_deref()) {
            parts.push(format!("{kind}: {err}"));
        }
    }
    let ok = outdated == 0 && unreachable == 0 && !inventory.panel.outdated;
    crate::health::report(
        state,
        crate::PACKAGE,
        "updates",
        ok,
        parts.join("; "),
        serde_json::json!({
            "outdated": outdated,
            "unreachable": unreachable,
            "total": total,
            "panel": inventory.panel.installed.to_string(),
            "panelLatest": inventory.panel.latest.as_ref().map(|v| v.to_string()),
            "wingsLatest": latest.latest_version(db::KIND_WINGS).map(|v| v.to_string()),
            "dbAgentLatest": latest.latest_version(db::KIND_DB_AGENT).map(|v| v.to_string()),
        }),
    )
    .await;
}
