//! Data access for `com_pineriver_nodeupdater_*`.
//!
//! Five tables: `_targets` (how a node gets updated), `_profiles` (reusable SSH credentials a
//! target can point at), `_runs` (update attempts with their streamed log), `_state`
//! (key/value: latest-version cache, daemon snapshot, notification markers) and `_checksums`
//! (SHA-256 of release assets the panel already hashed).
//!
//! Every statement is a `const &'static str` (sqlx 0.9 rejects `query(&format!(..))`).
//! All reads go to the primary: production runs a read replica and every read here follows
//! a write closely (claim → log → finish), and the tables are tiny.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use shared::State;
use sqlx::Row;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

pub const KIND_WINGS: &str = "wings";
pub const KIND_DB_AGENT: &str = "db_agent";
pub const KIND_PANEL: &str = "panel";

pub const STRATEGY_AUTO: &str = "auto";
pub const STRATEGY_API: &str = "api";
pub const STRATEGY_SSH_DOCKER: &str = "ssh_docker";
pub const STRATEGY_SSH_PACKAGE: &str = "ssh_package";
pub const STRATEGY_SSH_BINARY: &str = "ssh_binary";
pub const STRATEGIES: &[&str] = &[
    STRATEGY_AUTO,
    STRATEGY_API,
    STRATEGY_SSH_DOCKER,
    STRATEGY_SSH_PACKAGE,
    STRATEGY_SSH_BINARY,
];

pub const AUTH_NONE: &str = "none";
pub const AUTH_PASSWORD: &str = "password";
pub const AUTH_KEY: &str = "key";

pub const RUN_QUEUED: &str = "queued";
pub const RUN_RUNNING: &str = "running";
pub const RUN_SUCCEEDED: &str = "succeeded";
pub const RUN_FAILED: &str = "failed";
pub const RUN_CANCELLED: &str = "cancelled";

/// Logs are capped so a chatty `apt` cannot grow a row without bound.
const MAX_LOG_BYTES: i32 = 200_000;

pub fn is_kind(kind: &str) -> bool {
    matches!(kind, KIND_WINGS | KIND_DB_AGENT)
}

pub fn run_is_active(status: &str) -> bool {
    matches!(status, RUN_QUEUED | RUN_RUNNING)
}

/* ----------------------------------------------------------------------------- profiles */

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRow {
    pub id: Uuid,
    pub name: String,
    pub ssh_user: String,
    /// password | key
    pub ssh_auth: String,
    /// Encrypted secret blob; decrypted only when connecting. Never serialised.
    #[serde(skip)]
    pub ssh_secret: Option<String>,
    pub has_secret: bool,
    pub use_sudo: bool,
    /// How many targets point at this profile.
    pub targets: i64,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

fn row_to_profile(row: &sqlx::postgres::PgRow) -> ProfileRow {
    let ssh_secret: Option<String> = row.get("ssh_secret");
    ProfileRow {
        id: row.get("id"),
        name: row.get("name"),
        ssh_user: row.get("ssh_user"),
        ssh_auth: row.get("ssh_auth"),
        has_secret: ssh_secret.as_deref().is_some_and(|s| !s.is_empty()),
        ssh_secret,
        use_sudo: row.get("use_sudo"),
        targets: row.get("targets"),
        created: row.get("created"),
        updated: row.get("updated"),
    }
}

pub async fn profiles_all(state: &State) -> Result<Vec<ProfileRow>> {
    const SQL: &str = "SELECT p.id, p.name, p.ssh_user, p.ssh_auth, p.ssh_secret, p.use_sudo, p.created, p.updated, \
        (SELECT COUNT(*) FROM com_pineriver_nodeupdater_targets t WHERE t.profile_id = p.id) AS targets \
        FROM com_pineriver_nodeupdater_profiles p ORDER BY p.name";
    let rows = sqlx::query(SQL)
        .fetch_all(state.database.write())
        .await
        .context("listing credential profiles")?;
    Ok(rows.iter().map(row_to_profile).collect())
}

pub async fn profile_get(state: &State, id: Uuid) -> Result<Option<ProfileRow>> {
    const SQL: &str = "SELECT p.id, p.name, p.ssh_user, p.ssh_auth, p.ssh_secret, p.use_sudo, p.created, p.updated, \
        (SELECT COUNT(*) FROM com_pineriver_nodeupdater_targets t WHERE t.profile_id = p.id) AS targets \
        FROM com_pineriver_nodeupdater_profiles p WHERE p.id = $1";
    let row = sqlx::query(SQL)
        .bind(id)
        .fetch_optional(state.database.write())
        .await
        .context("loading credential profile")?;
    Ok(row.as_ref().map(row_to_profile))
}

pub async fn profile_by_name(state: &State, name: &str) -> Result<Option<ProfileRow>> {
    const SQL: &str = "SELECT p.id, p.name, p.ssh_user, p.ssh_auth, p.ssh_secret, p.use_sudo, p.created, p.updated, \
        (SELECT COUNT(*) FROM com_pineriver_nodeupdater_targets t WHERE t.profile_id = p.id) AS targets \
        FROM com_pineriver_nodeupdater_profiles p WHERE lower(p.name) = lower($1)";
    let row = sqlx::query(SQL)
        .bind(name)
        .fetch_optional(state.database.write())
        .await
        .context("looking up credential profile by name")?;
    Ok(row.as_ref().map(row_to_profile))
}

/// What the admin form submits for a profile. `set_secret = false` keeps the stored secret.
pub struct ProfileInput<'a> {
    pub name: &'a str,
    pub ssh_user: &'a str,
    pub ssh_auth: &'a str,
    pub set_secret: bool,
    /// Already encrypted (base64).
    pub secret: Option<String>,
    pub use_sudo: bool,
}

pub async fn profile_create(state: &State, input: ProfileInput<'_>) -> Result<ProfileRow> {
    const SQL: &str = "INSERT INTO com_pineriver_nodeupdater_profiles (name, ssh_user, ssh_auth, ssh_secret, use_sudo) \
        VALUES ($1, $2, $3, $4, $5) RETURNING id";
    let row = sqlx::query(SQL)
        .bind(input.name)
        .bind(input.ssh_user)
        .bind(input.ssh_auth)
        .bind(input.secret)
        .bind(input.use_sudo)
        .fetch_one(state.database.write())
        .await
        .context("creating credential profile")?;
    let id: Uuid = row.get("id");
    profile_get(state, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the profile vanished right after it was created"))
}

pub async fn profile_update(state: &State, id: Uuid, input: ProfileInput<'_>) -> Result<Option<ProfileRow>> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_profiles SET name = $2, ssh_user = $3, ssh_auth = $4, \
        ssh_secret = CASE WHEN $5 THEN $6 ELSE ssh_secret END, use_sudo = $7, updated = NOW() WHERE id = $1";
    let result = sqlx::query(SQL)
        .bind(id)
        .bind(input.name)
        .bind(input.ssh_user)
        .bind(input.ssh_auth)
        .bind(input.set_secret)
        .bind(input.secret)
        .bind(input.use_sudo)
        .execute(state.database.write())
        .await
        .context("updating credential profile")?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    profile_get(state, id).await
}

/// Targets that used the profile fall back to their own (usually empty) credentials.
pub async fn profile_delete(state: &State, id: Uuid) -> Result<bool> {
    const SQL: &str = "DELETE FROM com_pineriver_nodeupdater_profiles WHERE id = $1";
    let result = sqlx::query(SQL)
        .bind(id)
        .execute(state.database.write())
        .await
        .context("deleting credential profile")?;
    Ok(result.rows_affected() > 0)
}

/* ------------------------------------------------------------------------------ targets */

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetRow {
    pub id: Uuid,
    pub kind: String,
    pub ref_uuid: Uuid,
    pub strategy: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub ssh_user: String,
    pub ssh_auth: String,
    /// Encrypted secret blob; decrypted only by the runner. Never serialised.
    #[serde(skip)]
    pub ssh_secret: Option<String>,
    /// Whether a secret is stored (the UI shows "keep existing").
    pub has_secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_host_key: Option<String>,
    pub use_sudo: bool,
    pub compose_dir: String,
    pub compose_service: String,
    pub service_name: String,
    pub binary_path: String,
    pub restart_command: String,
    pub auto_update: bool,
    /// Credential profile the SSH login comes from; its user / auth / secret / sudo override
    /// the target's own fields (see [`resolve_credentials`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

impl TargetRow {
    /// Whether an update can be attempted at all with this configuration.
    pub fn uses_ssh(&self) -> bool {
        self.strategy.starts_with("ssh_") || (self.strategy == STRATEGY_AUTO && !self.ssh_host.is_empty())
    }

    /// A copy whose login fields come from `profile`.
    pub fn with_profile(&self, profile: &ProfileRow) -> TargetRow {
        let mut resolved = self.clone();
        resolved.ssh_user = profile.ssh_user.clone();
        resolved.ssh_auth = profile.ssh_auth.clone();
        resolved.ssh_secret = profile.ssh_secret.clone();
        resolved.has_secret = profile.has_secret;
        resolved.use_sudo = profile.use_sudo;
        resolved.profile_name = Some(profile.name.clone());
        resolved
    }
}

fn row_to_target(row: &sqlx::postgres::PgRow) -> TargetRow {
    let ssh_secret: Option<String> = row.get("ssh_secret");
    TargetRow {
        id: row.get("id"),
        kind: row.get("kind"),
        ref_uuid: row.get("ref_uuid"),
        strategy: row.get("strategy"),
        ssh_host: row.get("ssh_host"),
        ssh_port: row.get("ssh_port"),
        ssh_user: row.get("ssh_user"),
        ssh_auth: row.get("ssh_auth"),
        has_secret: ssh_secret.as_deref().is_some_and(|s| !s.is_empty()),
        ssh_secret,
        ssh_host_key: row.get("ssh_host_key"),
        use_sudo: row.get("use_sudo"),
        compose_dir: row.get("compose_dir"),
        compose_service: row.get("compose_service"),
        service_name: row.get("service_name"),
        binary_path: row.get("binary_path"),
        restart_command: row.get("restart_command"),
        auto_update: row.get("auto_update"),
        profile_id: row.get("profile_id"),
        profile_name: row.get("profile_name"),
        detected: row.get("detected"),
        detected_at: row.get("detected_at"),
        created: row.get("created"),
        updated: row.get("updated"),
    }
}

pub async fn targets_all(state: &State) -> Result<Vec<TargetRow>> {
    const SQL: &str = "SELECT t.id, t.kind, t.ref_uuid, t.strategy, t.ssh_host, t.ssh_port, t.ssh_user, t.ssh_auth, t.ssh_secret, \
        t.ssh_host_key, t.use_sudo, t.compose_dir, t.compose_service, t.service_name, t.binary_path, t.restart_command, \
        t.auto_update, t.profile_id, p.name AS profile_name, t.detected, t.detected_at, t.created, t.updated \
        FROM com_pineriver_nodeupdater_targets t \
        LEFT JOIN com_pineriver_nodeupdater_profiles p ON p.id = t.profile_id \
        ORDER BY t.kind, t.created";
    let rows = sqlx::query(SQL)
        .fetch_all(state.database.write())
        .await
        .context("listing update targets")?;
    Ok(rows.iter().map(row_to_target).collect())
}

pub async fn target_get(state: &State, kind: &str, ref_uuid: Uuid) -> Result<Option<TargetRow>> {
    const SQL: &str = "SELECT t.id, t.kind, t.ref_uuid, t.strategy, t.ssh_host, t.ssh_port, t.ssh_user, t.ssh_auth, t.ssh_secret, \
        t.ssh_host_key, t.use_sudo, t.compose_dir, t.compose_service, t.service_name, t.binary_path, t.restart_command, \
        t.auto_update, t.profile_id, p.name AS profile_name, t.detected, t.detected_at, t.created, t.updated \
        FROM com_pineriver_nodeupdater_targets t \
        LEFT JOIN com_pineriver_nodeupdater_profiles p ON p.id = t.profile_id \
        WHERE t.kind = $1 AND t.ref_uuid = $2";
    let row = sqlx::query(SQL)
        .bind(kind)
        .bind(ref_uuid)
        .fetch_optional(state.database.write())
        .await
        .context("loading update target")?;
    Ok(row.as_ref().map(row_to_target))
}

/// The target with its login fields taken from the attached profile, when there is one.
pub async fn resolve_credentials(state: &State, target: &TargetRow) -> Result<TargetRow> {
    match target.profile_id {
        Some(profile_id) => match profile_get(state, profile_id).await? {
            Some(profile) => Ok(target.with_profile(&profile)),
            None => Ok(target.clone()),
        },
        None => Ok(target.clone()),
    }
}

/// Everything the admin form submits. `set_secret` / `set_host_key` = false keeps the stored value.
pub struct TargetInput<'a> {
    pub kind: &'a str,
    pub ref_uuid: Uuid,
    pub strategy: &'a str,
    pub ssh_host: &'a str,
    pub ssh_port: i32,
    pub ssh_user: &'a str,
    pub ssh_auth: &'a str,
    pub set_secret: bool,
    /// Already encrypted (base64) — or `None` to clear.
    pub secret: Option<String>,
    pub set_host_key: bool,
    pub host_key: Option<&'a str>,
    pub use_sudo: bool,
    pub compose_dir: &'a str,
    pub compose_service: &'a str,
    pub service_name: &'a str,
    pub binary_path: &'a str,
    pub restart_command: &'a str,
    pub auto_update: bool,
    pub profile_id: Option<Uuid>,
}

pub async fn target_upsert(state: &State, input: TargetInput<'_>) -> Result<TargetRow> {
    const SQL: &str = "INSERT INTO com_pineriver_nodeupdater_targets \
        (kind, ref_uuid, strategy, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_secret, ssh_host_key, use_sudo, \
         compose_dir, compose_service, service_name, binary_path, restart_command, auto_update, profile_id) \
        VALUES ($1, $2, $3, $4, $5, $6, $7, $9, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
        ON CONFLICT (kind, ref_uuid) DO UPDATE SET \
          strategy = EXCLUDED.strategy, ssh_host = EXCLUDED.ssh_host, ssh_port = EXCLUDED.ssh_port, \
          ssh_user = EXCLUDED.ssh_user, ssh_auth = EXCLUDED.ssh_auth, \
          ssh_secret = CASE WHEN $8 THEN EXCLUDED.ssh_secret ELSE com_pineriver_nodeupdater_targets.ssh_secret END, \
          ssh_host_key = CASE WHEN $10 THEN EXCLUDED.ssh_host_key ELSE com_pineriver_nodeupdater_targets.ssh_host_key END, \
          use_sudo = EXCLUDED.use_sudo, compose_dir = EXCLUDED.compose_dir, compose_service = EXCLUDED.compose_service, \
          service_name = EXCLUDED.service_name, binary_path = EXCLUDED.binary_path, \
          restart_command = EXCLUDED.restart_command, auto_update = EXCLUDED.auto_update, \
          profile_id = EXCLUDED.profile_id, updated = NOW()";
    sqlx::query(SQL)
        .bind(input.kind)
        .bind(input.ref_uuid)
        .bind(input.strategy)
        .bind(input.ssh_host)
        .bind(input.ssh_port)
        .bind(input.ssh_user)
        .bind(input.ssh_auth)
        .bind(input.set_secret)
        .bind(input.secret)
        .bind(input.set_host_key)
        .bind(input.host_key)
        .bind(input.use_sudo)
        .bind(input.compose_dir)
        .bind(input.compose_service)
        .bind(input.service_name)
        .bind(input.binary_path)
        .bind(input.restart_command)
        .bind(input.auto_update)
        .bind(input.profile_id)
        .execute(state.database.write())
        .await
        .context("saving update target")?;
    target_get(state, input.kind, input.ref_uuid)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the target vanished right after it was saved"))
}

pub async fn target_delete(state: &State, kind: &str, ref_uuid: Uuid) -> Result<bool> {
    const SQL: &str = "DELETE FROM com_pineriver_nodeupdater_targets WHERE kind = $1 AND ref_uuid = $2";
    let result = sqlx::query(SQL)
        .bind(kind)
        .bind(ref_uuid)
        .execute(state.database.write())
        .await
        .context("deleting update target")?;
    Ok(result.rows_affected() > 0)
}

pub async fn target_set_detected(state: &State, id: Uuid, detected: &serde_json::Value) -> Result<()> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_targets SET detected = $2, detected_at = NOW(), updated = NOW() WHERE id = $1";
    sqlx::query(SQL)
        .bind(id)
        .bind(detected)
        .execute(state.database.write())
        .await
        .context("storing detection result")?;
    Ok(())
}

/// Drop target rows whose node / DB Agent host no longer exists.
pub async fn targets_prune(state: &State, live: &[(String, Uuid)]) -> Result<usize> {
    let mut removed = 0;
    for target in targets_all(state).await? {
        let alive = live
            .iter()
            .any(|(kind, uuid)| *kind == target.kind && *uuid == target.ref_uuid);
        if !alive && target_delete(state, &target.kind, target.ref_uuid).await? {
            removed += 1;
        }
    }
    Ok(removed)
}

/* --------------------------------------------------------------------------------- runs */

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    pub id: Uuid,
    pub kind: String,
    pub ref_uuid: Uuid,
    pub ref_name: String,
    pub strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    pub to_version: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub triggered_by: String,
    pub force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    #[serde(flatten)]
    #[schema(inline)]
    pub run: RunRow,
    pub log: String,
}

fn row_to_run(row: &sqlx::postgres::PgRow) -> RunRow {
    RunRow {
        id: row.get("id"),
        kind: row.get("kind"),
        ref_uuid: row.get("ref_uuid"),
        ref_name: row.get("ref_name"),
        strategy: row.get("strategy"),
        from_version: row.get("from_version"),
        to_version: row.get("to_version"),
        status: row.get("status"),
        error: row.get("error"),
        triggered_by: row.get("triggered_by"),
        force: row.get("force"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        created: row.get("created"),
        updated: row.get("updated"),
    }
}

pub struct NewRun<'a> {
    pub kind: &'a str,
    pub ref_uuid: Uuid,
    pub ref_name: &'a str,
    pub strategy: &'a str,
    pub from_version: Option<&'a str>,
    pub to_version: &'a str,
    pub triggered_by: &'a str,
    pub force: bool,
}

/// Whether a `run_create` error is the unique index refusing a second active run for the
/// same node (`com_pineriver_nodeupdater_runs_one_active`).
pub fn is_duplicate_active_run(err: &anyhow::Error) -> bool {
    let text = format!("{err:#}");
    text.contains("com_pineriver_nodeupdater_runs_one_active")
}

pub async fn run_create(state: &State, new: NewRun<'_>) -> Result<RunRow> {
    const SQL: &str = "INSERT INTO com_pineriver_nodeupdater_runs \
        (kind, ref_uuid, ref_name, strategy, from_version, to_version, triggered_by, force) \
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
        RETURNING id, kind, ref_uuid, ref_name, strategy, from_version, to_version, status, error, \
          triggered_by, force, started_at, finished_at, created, updated";
    let row = sqlx::query(SQL)
        .bind(new.kind)
        .bind(new.ref_uuid)
        .bind(new.ref_name)
        .bind(new.strategy)
        .bind(new.from_version)
        .bind(new.to_version)
        .bind(new.triggered_by)
        .bind(new.force)
        .fetch_one(state.database.write())
        .await
        .context("queueing update run")?;
    Ok(row_to_run(&row))
}

pub async fn runs_list(state: &State, limit: i64, offset: i64) -> Result<Vec<RunRow>> {
    const SQL: &str = "SELECT id, kind, ref_uuid, ref_name, strategy, from_version, to_version, status, error, \
          triggered_by, force, started_at, finished_at, created, updated \
        FROM com_pineriver_nodeupdater_runs ORDER BY created DESC LIMIT $1 OFFSET $2";
    let rows = sqlx::query(SQL)
        .bind(limit)
        .bind(offset)
        .fetch_all(state.database.write())
        .await
        .context("listing update runs")?;
    Ok(rows.iter().map(row_to_run).collect())
}

pub async fn run_get(state: &State, id: Uuid) -> Result<Option<RunDetail>> {
    const SQL: &str = "SELECT id, kind, ref_uuid, ref_name, strategy, from_version, to_version, status, error, \
          triggered_by, force, started_at, finished_at, created, updated, log \
        FROM com_pineriver_nodeupdater_runs WHERE id = $1";
    let row = sqlx::query(SQL)
        .bind(id)
        .fetch_optional(state.database.write())
        .await
        .context("loading update run")?;
    Ok(row.map(|row| RunDetail {
        run: row_to_run(&row),
        log: row.get("log"),
    }))
}

/// Whether a run for this component is queued or running.
pub async fn run_active_for(state: &State, kind: &str, ref_uuid: Uuid) -> Result<bool> {
    const SQL: &str = "SELECT COUNT(*) AS n FROM com_pineriver_nodeupdater_runs \
        WHERE kind = $1 AND ref_uuid = $2 AND status IN ('queued', 'running')";
    let row = sqlx::query(SQL)
        .bind(kind)
        .bind(ref_uuid)
        .fetch_one(state.database.write())
        .await
        .context("checking for active runs")?;
    Ok(row.get::<i64, _>("n") > 0)
}

pub async fn runs_count_active(state: &State) -> Result<i64> {
    const SQL: &str = "SELECT COUNT(*) AS n FROM com_pineriver_nodeupdater_runs WHERE status IN ('queued', 'running')";
    let row = sqlx::query(SQL)
        .fetch_one(state.database.write())
        .await
        .context("counting active runs")?;
    Ok(row.get::<i64, _>("n"))
}

/// Atomically take the oldest queued run (several panel instances may share the table).
pub async fn run_claim_next(state: &State) -> Result<Option<RunRow>> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs SET status = 'running', started_at = NOW(), updated = NOW() \
        WHERE id = (SELECT id FROM com_pineriver_nodeupdater_runs WHERE status = 'queued' \
                    ORDER BY created LIMIT 1 FOR UPDATE SKIP LOCKED) \
        RETURNING id, kind, ref_uuid, ref_name, strategy, from_version, to_version, status, error, \
          triggered_by, force, started_at, finished_at, created, updated";
    let row = sqlx::query(SQL)
        .fetch_optional(state.database.write())
        .await
        .context("claiming the next update run")?;
    Ok(row.as_ref().map(row_to_run))
}

pub async fn run_append_log(state: &State, id: Uuid, text: &str) -> Result<()> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs \
        SET log = left(log || $2, $3), updated = NOW() WHERE id = $1";
    sqlx::query(SQL)
        .bind(id)
        .bind(text)
        .bind(MAX_LOG_BYTES)
        .execute(state.database.write())
        .await
        .context("appending to the run log")?;
    Ok(())
}

pub async fn run_set_strategy(state: &State, id: Uuid, strategy: &str, from_version: Option<&str>) -> Result<()> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs SET strategy = $2, \
        from_version = COALESCE($3, from_version), updated = NOW() WHERE id = $1";
    sqlx::query(SQL)
        .bind(id)
        .bind(strategy)
        .bind(from_version)
        .execute(state.database.write())
        .await
        .context("updating the run strategy")?;
    Ok(())
}

pub async fn run_finish(state: &State, id: Uuid, status: &str, error: Option<&str>) -> Result<()> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs \
        SET status = $2, error = $3, finished_at = NOW(), updated = NOW() WHERE id = $1";
    sqlx::query(SQL)
        .bind(id)
        .bind(status)
        .bind(error.map(|e| e.chars().take(4000).collect::<String>()))
        .execute(state.database.write())
        .await
        .context("finishing the run")?;
    Ok(())
}

/// Cancel a queued run. Running runs are cancelled cooperatively through [`crate::runner::cancel`].
pub async fn run_cancel_queued(state: &State, id: Uuid) -> Result<bool> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs \
        SET status = 'cancelled', finished_at = NOW(), updated = NOW() WHERE id = $1 AND status = 'queued'";
    let result = sqlx::query(SQL)
        .bind(id)
        .execute(state.database.write())
        .await
        .context("cancelling the run")?;
    Ok(result.rows_affected() > 0)
}

pub async fn run_delete(state: &State, id: Uuid) -> Result<bool> {
    const SQL: &str = "DELETE FROM com_pineriver_nodeupdater_runs WHERE id = $1 AND status NOT IN ('queued', 'running')";
    let result = sqlx::query(SQL)
        .bind(id)
        .execute(state.database.write())
        .await
        .context("deleting the run")?;
    Ok(result.rows_affected() > 0)
}

/// A panel restart kills the in-process runner; leave no run stuck in `running`.
pub async fn runs_reconcile_interrupted(state: &State) -> Result<u64> {
    const SQL: &str = "UPDATE com_pineriver_nodeupdater_runs \
        SET status = 'failed', error = 'interrupted by a panel restart', finished_at = NOW(), updated = NOW() \
        WHERE status = 'running'";
    let result = sqlx::query(SQL)
        .execute(state.database.write())
        .await
        .context("reconciling interrupted runs")?;
    Ok(result.rows_affected())
}

/// The most recent run per component.
pub async fn runs_last_per_component(state: &State) -> Result<HashMap<(String, Uuid), RunRow>> {
    const SQL: &str = "SELECT DISTINCT ON (kind, ref_uuid) id, kind, ref_uuid, ref_name, strategy, from_version, \
          to_version, status, error, triggered_by, force, started_at, finished_at, created, updated \
        FROM com_pineriver_nodeupdater_runs ORDER BY kind, ref_uuid, created DESC";
    let rows = sqlx::query(SQL)
        .fetch_all(state.database.write())
        .await
        .context("loading last runs")?;
    Ok(rows
        .iter()
        .map(row_to_run)
        .map(|run| ((run.kind.clone(), run.ref_uuid), run))
        .collect())
}

pub async fn runs_prune(state: &State, keep_days: i64) -> Result<u64> {
    const SQL: &str = "DELETE FROM com_pineriver_nodeupdater_runs \
        WHERE status NOT IN ('queued', 'running') AND created < NOW() - ($1 || ' days')::interval";
    let result = sqlx::query(SQL)
        .bind(keep_days.to_string())
        .execute(state.database.write())
        .await
        .context("pruning old runs")?;
    Ok(result.rows_affected())
}

/* -------------------------------------------------------------------------------- state */

pub async fn state_get<T: DeserializeOwned>(state: &State, key: &str) -> Result<Option<T>> {
    const SQL: &str = "SELECT value FROM com_pineriver_nodeupdater_state WHERE key = $1";
    let row = sqlx::query(SQL)
        .bind(key)
        .fetch_optional(state.database.write())
        .await
        .with_context(|| format!("reading state '{key}'"))?;
    match row {
        Some(row) => {
            let value: serde_json::Value = row.get("value");
            Ok(Some(serde_json::from_value(value).with_context(|| format!("decoding state '{key}'"))?))
        }
        None => Ok(None),
    }
}

pub async fn state_updated(state: &State, key: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    const SQL: &str = "SELECT updated FROM com_pineriver_nodeupdater_state WHERE key = $1";
    let row = sqlx::query(SQL)
        .bind(key)
        .fetch_optional(state.database.write())
        .await
        .with_context(|| format!("reading state timestamp '{key}'"))?;
    Ok(row.map(|row| row.get("updated")))
}

pub async fn state_set(state: &State, key: &str, value: &impl Serialize) -> Result<()> {
    const SQL: &str = "INSERT INTO com_pineriver_nodeupdater_state (key, value, updated) VALUES ($1, $2, NOW()) \
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated = NOW()";
    let value = serde_json::to_value(value).context("encoding state value")?;
    sqlx::query(SQL)
        .bind(key)
        .bind(value)
        .execute(state.database.write())
        .await
        .with_context(|| format!("writing state '{key}'"))?;
    Ok(())
}

pub async fn state_delete_prefix(state: &State, prefix: &str) -> Result<u64> {
    const SQL: &str = "DELETE FROM com_pineriver_nodeupdater_state WHERE key LIKE $1 || '%'";
    let result = sqlx::query(SQL)
        .bind(prefix)
        .execute(state.database.write())
        .await
        .context("clearing state")?;
    Ok(result.rows_affected())
}

/* ---------------------------------------------------------------------------- checksums */

pub async fn checksum_get(state: &State, kind: &str, version: &str, architecture: &str) -> Result<Option<String>> {
    const SQL: &str = "SELECT sha256 FROM com_pineriver_nodeupdater_checksums \
        WHERE kind = $1 AND version = $2 AND architecture = $3";
    let row = sqlx::query(SQL)
        .bind(kind)
        .bind(version)
        .bind(architecture)
        .fetch_optional(state.database.write())
        .await
        .context("reading cached checksum")?;
    Ok(row.map(|row| row.get("sha256")))
}

pub async fn checksum_put(state: &State, kind: &str, version: &str, architecture: &str, sha256: &str, size: i64) -> Result<()> {
    const SQL: &str = "INSERT INTO com_pineriver_nodeupdater_checksums (kind, version, architecture, sha256, size) \
        VALUES ($1, $2, $3, $4, $5) ON CONFLICT (kind, version, architecture) DO UPDATE SET sha256 = EXCLUDED.sha256, size = EXCLUDED.size";
    sqlx::query(SQL)
        .bind(kind)
        .bind(version)
        .bind(architecture)
        .bind(sha256)
        .bind(size)
        .execute(state.database.write())
        .await
        .context("caching checksum")?;
    Ok(())
}
