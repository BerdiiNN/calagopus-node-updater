// common: synced from extension-common/health.rs — edit there, then run scripts/sync-ext.ps1 -Common
//! Health reports for the PineRiver Extension Hub admin page (`com.pineriver.exthub`).
//!
//! An extension upserts one row per component ("listener-http", "ingest", "jobs", ...) from
//! wherever it already does periodic work; the hub lists the rows, shows `detail`, and marks
//! a row stale when it has not been refreshed for a while. Reports are best-effort: when the
//! hub is not installed (no table) they are dropped and not retried for ten minutes, and a
//! failed report never propagates to the caller.

use shared::State;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const RETRY_AFTER_MISSING_SECS: i64 = 600;

// sqlx only accepts literal SQL; the table is `com_pineriver_hub_health` (hub migration).
const UPSERT: &str = "INSERT INTO com_pineriver_hub_health (extension, component, ok, detail, meta, updated) \
     VALUES ($1, $2, $3, $4, $5, NOW()) \
     ON CONFLICT (extension, component) DO UPDATE \
     SET ok = EXCLUDED.ok, detail = EXCLUDED.detail, meta = EXCLUDED.meta, updated = NOW()";

static TABLE_MISSING_UNTIL: AtomicI64 = AtomicI64::new(0);

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record the state of one component. `meta` is free-form JSON shown by the hub (counts,
/// addresses, ...); pass `serde_json::Value::Null` when there is nothing to add.
pub async fn report(
    state: &State,
    extension: &str,
    component: &str,
    ok: bool,
    detail: impl Into<String>,
    meta: serde_json::Value,
) {
    let now = now_secs();
    if TABLE_MISSING_UNTIL.load(Ordering::Relaxed) > now {
        return;
    }

    let detail: String = detail.into();
    let result = sqlx::query(UPSERT)
        .bind(extension)
    .bind(component)
    .bind(ok)
    .bind(detail.chars().take(1000).collect::<String>())
    .bind(if meta.is_null() { None } else { Some(meta.to_string()) })
    .execute(state.database.write())
    .await;

    if let Err(err) = result {
        let text = err.to_string();
        if text.contains("does not exist") {
            // The hub extension is not installed; do not hammer the database.
            TABLE_MISSING_UNTIL.store(now + RETRY_AFTER_MISSING_SECS, Ordering::Relaxed);
        } else {
            tracing::debug!(
                target: "extension.health",
                extension,
                component,
                "health report failed: {err}"
            );
        }
    }
}
