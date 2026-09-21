//! Operator settings, stored in the panel settings table under this extension's prefix.
//!
//! | Key                        | Default | Meaning                                                        |
//! |----------------------------|---------|----------------------------------------------------------------|
//! | `check_interval_minutes`   | 60      | how often latest releases and node versions are compared (15–1440) |
//! | `include_prereleases`      | false   | treat GitHub pre-releases as candidates                         |
//! | `allow_ahead_of_panel`     | false   | let Wings / DB Agent be updated past the panel's own version    |
//! | `notify_email`             | true    | e-mail every administrator when a new release appears          |
//! | `notify_on_success`        | true    | also notify when an update run succeeds (failures always notify) |
//! | `notify_discord_url`       | ""      | Discord webhook URL                                            |
//! | `notify_webhook_url`       | ""      | generic JSON webhook URL                                       |
//! | `notify_webhook_secret`    | ""      | sent as `X-NodeUpdater-Secret` (stored encrypted)              |
//! | `remind_after_hours`       | 168     | re-notify while still outdated; 0 = only once per version      |
//! | `ssh_connect_timeout_secs` | 30      | SSH connect / auth budget (5–300)                              |
//! | `update_timeout_secs`      | 900     | whole update run budget incl. the version re-check (60–7200)   |
//! | `verify_timeout_secs`      | 180     | how long to wait for the daemon to report the new version (30–1800) |

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shared::State;
use shared::extensions::settings::{
    ExtensionSettings as ExtensionSettingsTrait, SettingsDeserializeExt, SettingsDeserializer,
    SettingsSerializeExt, SettingsSerializer,
};
use utoipa::ToSchema;

const KEY_INTERVAL: &str = "check_interval_minutes";
const KEY_PRERELEASES: &str = "include_prereleases";
const KEY_AHEAD: &str = "allow_ahead_of_panel";
const KEY_EMAIL: &str = "notify_email";
const KEY_ON_SUCCESS: &str = "notify_on_success";
const KEY_DISCORD: &str = "notify_discord_url";
const KEY_WEBHOOK: &str = "notify_webhook_url";
const KEY_WEBHOOK_SECRET: &str = "notify_webhook_secret";
const KEY_REMIND: &str = "remind_after_hours";
const KEY_SSH_TIMEOUT: &str = "ssh_connect_timeout_secs";
const KEY_UPDATE_TIMEOUT: &str = "update_timeout_secs";
const KEY_VERIFY_TIMEOUT: &str = "verify_timeout_secs";

pub const DEFAULT_INTERVAL: i64 = 60;
pub const DEFAULT_REMIND: i64 = 168;
pub const DEFAULT_SSH_TIMEOUT: i64 = 30;
pub const DEFAULT_UPDATE_TIMEOUT: i64 = 900;
pub const DEFAULT_VERIFY_TIMEOUT: i64 = 180;

fn clamp_interval(v: i64) -> i64 {
    v.clamp(15, 1440)
}
fn clamp_remind(v: i64) -> i64 {
    v.clamp(0, 24 * 90)
}
fn clamp_ssh_timeout(v: i64) -> i64 {
    v.clamp(5, 300)
}
fn clamp_update_timeout(v: i64) -> i64 {
    v.clamp(60, 7200)
}
fn clamp_verify_timeout(v: i64) -> i64 {
    v.clamp(30, 1800)
}
fn clamp_url(v: &str) -> String {
    let v = v.trim();
    if v.starts_with("https://") || v.starts_with("http://") {
        v.chars().take(2000).collect()
    } else {
        String::new()
    }
}

#[derive(Debug, Clone, ToSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettings {
    pub check_interval_minutes: i64,
    pub include_prereleases: bool,
    pub allow_ahead_of_panel: bool,
    pub notify_email: bool,
    pub notify_on_success: bool,
    pub notify_discord_url: String,
    pub notify_webhook_url: String,
    /// Never serialised to the client; the GET route reports `webhookSecretSet` instead.
    #[serde(skip_serializing)]
    pub notify_webhook_secret: String,
    pub remind_after_hours: i64,
    pub ssh_connect_timeout_secs: i64,
    pub update_timeout_secs: i64,
    pub verify_timeout_secs: i64,
}

impl Default for ExtensionSettings {
    fn default() -> Self {
        Self {
            check_interval_minutes: DEFAULT_INTERVAL,
            include_prereleases: false,
            allow_ahead_of_panel: false,
            notify_email: true,
            notify_on_success: true,
            notify_discord_url: String::new(),
            notify_webhook_url: String::new(),
            notify_webhook_secret: String::new(),
            remind_after_hours: DEFAULT_REMIND,
            ssh_connect_timeout_secs: DEFAULT_SSH_TIMEOUT,
            update_timeout_secs: DEFAULT_UPDATE_TIMEOUT,
            verify_timeout_secs: DEFAULT_VERIFY_TIMEOUT,
        }
    }
}

impl ExtensionSettings {
    /// Clamp every value into its documented range.
    pub fn normalised(mut self) -> Self {
        self.check_interval_minutes = clamp_interval(self.check_interval_minutes);
        self.remind_after_hours = clamp_remind(self.remind_after_hours);
        self.ssh_connect_timeout_secs = clamp_ssh_timeout(self.ssh_connect_timeout_secs);
        self.update_timeout_secs = clamp_update_timeout(self.update_timeout_secs);
        self.verify_timeout_secs = clamp_verify_timeout(self.verify_timeout_secs);
        self.notify_discord_url = clamp_url(&self.notify_discord_url);
        self.notify_webhook_url = clamp_url(&self.notify_webhook_url);
        self.notify_webhook_secret = self.notify_webhook_secret.trim().chars().take(500).collect();
        self
    }

    pub fn ssh_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(clamp_ssh_timeout(self.ssh_connect_timeout_secs) as u64)
    }

    pub fn update_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(clamp_update_timeout(self.update_timeout_secs) as u64)
    }

    pub fn verify_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(clamp_verify_timeout(self.verify_timeout_secs) as u64)
    }

    pub fn check_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(clamp_interval(self.check_interval_minutes) as u64 * 60)
    }
}

#[async_trait::async_trait]
impl SettingsSerializeExt for ExtensionSettings {
    async fn serialize(&self, serializer: SettingsSerializer) -> Result<SettingsSerializer> {
        let s = self.clone().normalised();
        let serializer = serializer
            .write_raw_setting(KEY_INTERVAL, s.check_interval_minutes.to_string())
            .write_raw_setting(KEY_PRERELEASES, s.include_prereleases.to_string())
            .write_raw_setting(KEY_AHEAD, s.allow_ahead_of_panel.to_string())
            .write_raw_setting(KEY_EMAIL, s.notify_email.to_string())
            .write_raw_setting(KEY_ON_SUCCESS, s.notify_on_success.to_string())
            .write_raw_setting(KEY_DISCORD, s.notify_discord_url.clone())
            .write_raw_setting(KEY_WEBHOOK, s.notify_webhook_url.clone())
            .write_raw_setting(KEY_REMIND, s.remind_after_hours.to_string())
            .write_raw_setting(KEY_SSH_TIMEOUT, s.ssh_connect_timeout_secs.to_string())
            .write_raw_setting(KEY_UPDATE_TIMEOUT, s.update_timeout_secs.to_string())
            .write_raw_setting(KEY_VERIFY_TIMEOUT, s.verify_timeout_secs.to_string());
        Ok(serializer
            .write_raw_encrypted_setting(KEY_WEBHOOK_SECRET, s.notify_webhook_secret.clone())
            .await?)
    }
}

pub struct ExtensionSettingsDeserializer;

fn parse_bool(v: Option<compact_str::CompactString>, default: bool) -> bool {
    match v.as_deref().map(str::trim) {
        Some("true") | Some("1") => true,
        Some("false") | Some("0") => false,
        _ => default,
    }
}

fn parse_i64(v: Option<compact_str::CompactString>, default: i64) -> i64 {
    v.and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(default)
}

#[async_trait::async_trait]
impl SettingsDeserializeExt for ExtensionSettingsDeserializer {
    async fn deserialize_boxed(
        &self,
        mut deserializer: SettingsDeserializer<'_>,
    ) -> Result<ExtensionSettingsTrait> {
        let secret = deserializer
            .read_raw_encrypted_setting(KEY_WEBHOOK_SECRET)
            .await
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_default();
        // Remove the key so the panel does not complain about an unconsumed setting.
        let _ = deserializer.take_raw_setting(KEY_WEBHOOK_SECRET);

        let settings = ExtensionSettings {
            check_interval_minutes: parse_i64(deserializer.take_raw_setting(KEY_INTERVAL), DEFAULT_INTERVAL),
            include_prereleases: parse_bool(deserializer.take_raw_setting(KEY_PRERELEASES), false),
            allow_ahead_of_panel: parse_bool(deserializer.take_raw_setting(KEY_AHEAD), false),
            notify_email: parse_bool(deserializer.take_raw_setting(KEY_EMAIL), true),
            notify_on_success: parse_bool(deserializer.take_raw_setting(KEY_ON_SUCCESS), true),
            notify_discord_url: deserializer
                .take_raw_setting(KEY_DISCORD)
                .map(|s| s.to_string())
                .unwrap_or_default(),
            notify_webhook_url: deserializer
                .take_raw_setting(KEY_WEBHOOK)
                .map(|s| s.to_string())
                .unwrap_or_default(),
            notify_webhook_secret: secret,
            remind_after_hours: parse_i64(deserializer.take_raw_setting(KEY_REMIND), DEFAULT_REMIND),
            ssh_connect_timeout_secs: parse_i64(deserializer.take_raw_setting(KEY_SSH_TIMEOUT), DEFAULT_SSH_TIMEOUT),
            update_timeout_secs: parse_i64(deserializer.take_raw_setting(KEY_UPDATE_TIMEOUT), DEFAULT_UPDATE_TIMEOUT),
            verify_timeout_secs: parse_i64(deserializer.take_raw_setting(KEY_VERIFY_TIMEOUT), DEFAULT_VERIFY_TIMEOUT),
        }
        .normalised();

        Ok(Box::new(settings))
    }
}

/// The current settings (defaults when nothing is stored yet).
pub async fn current(state: &State) -> ExtensionSettings {
    state
        .settings
        .get()
        .await
        .ok()
        .and_then(|s| s.find_extension_settings::<ExtensionSettings>().ok().cloned())
        .unwrap_or_default()
        .normalised()
}

/// Persist new settings. `keep_secret` leaves the stored webhook secret untouched (the UI
/// sends an empty string when the operator did not type a new one).
pub async fn store(state: &State, mut next: ExtensionSettings, keep_secret: bool) -> Result<()> {
    let mut settings = state
        .settings
        .get_mut()
        .await
        .context("acquiring the panel settings write lock")?;
    let ext: &mut ExtensionSettings = settings
        .find_mut_extension_settings()
        .context("locating this extension's settings")?;

    if keep_secret {
        next.notify_webhook_secret = ext.notify_webhook_secret.clone();
    }
    *ext = next.normalised();

    settings.save().await.context("writing settings")?;
    Ok(())
}
