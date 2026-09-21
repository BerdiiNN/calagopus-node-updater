//! Telling people. Four channels: e-mail to every administrator (through the panel's own
//! mail transport), a Discord webhook, a generic JSON webhook, and the Extension Hub health
//! row (see `checker.rs`). Sending never fails the caller; problems are logged.
//!
//! Two guards matter here:
//!
//! * The panel renders e-mail subject and body as minijinja templates with the mail settings
//!   in scope, so nothing from the outside world (node names, daemon error text) is ever
//!   interpolated into a template string; it all travels through the template context and is
//!   auto-escaped.
//! * Webhook destinations are operator-supplied URLs. They are posted through a dedicated
//!   HTTP client that resolves the host first, refuses addresses the panel blocks for outbound
//!   traffic (or the well-known internal ranges on panels without that list), pins the
//!   connection to the vetted addresses and follows no redirects — the panel must not become a
//!   way to poke at services on its own network.

use crate::db::RunRow;
use crate::settings::ExtensionSettings;
use anyhow::{Context, Result, anyhow};
use shared::State;
use sqlx::Row;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::versions::USER_AGENT;
const DISCORD_HOSTS: &[&str] = &["discord.com", "discordapp.com", "canary.discord.com", "ptb.discord.com"];

pub struct OutdatedInstance {
    pub name: String,
    pub installed: String,
}

pub enum Event<'a> {
    /// A newer release exists for `kind`.
    UpdateAvailable {
        kind: &'a str,
        latest: &'a semver::Version,
        recommended: Option<&'a semver::Version>,
        release_url: &'a str,
        /// Instances still on an older version (empty for the panel itself).
        outdated: &'a [OutdatedInstance],
        /// The panel runs an older version than `latest` — Wings/DB Agent should wait.
        panel_behind: bool,
        reminder: bool,
    },
    /// An update run ended.
    RunFinished { run: &'a RunRow, log_tail: &'a str },
}

fn display_kind(kind: &str) -> &'static str {
    match kind {
        crate::db::KIND_PANEL => "Panel",
        crate::db::KIND_DB_AGENT => "DB Agent",
        _ => "Wings",
    }
}

struct Message {
    subject: String,
    /// Plain text, one paragraph per line.
    lines: Vec<String>,
    colour: u32,
    payload: serde_json::Value,
}

fn build(event: &Event<'_>, panel_url: &str) -> Message {
    let page = format!("{}/admin/node-updater", panel_url.trim_end_matches('/'));
    match event {
        Event::UpdateAvailable {
            kind,
            latest,
            recommended,
            release_url,
            outdated,
            panel_behind,
            reminder,
        } => {
            let name = display_kind(kind);
            let mut subject = format!("{name} {latest} is available");
            if !outdated.is_empty() {
                subject.push_str(&format!(" ({} to update)", outdated.len()));
            }
            if *reminder {
                subject.insert_str(0, "Reminder: ");
            }
            let mut lines = vec![format!("A new {name} release, {latest}, has been published.")];
            if let Some(recommended) = recommended
                && *recommended != *latest
            {
                lines.push(format!("calagopus.com currently recommends {recommended}."));
            }
            if *kind == crate::db::KIND_PANEL {
                lines.push(format!("This panel runs {}.", shared::VERSION));
                lines.push("Update the panel before updating Wings or DB Agent.".to_string());
            } else if !outdated.is_empty() {
                lines.push("Still on an older version:".to_string());
                for instance in outdated.iter() {
                    lines.push(format!("  • {} — {}", instance.name, instance.installed));
                }
            }
            if *panel_behind && *kind != crate::db::KIND_PANEL {
                lines.push(format!(
                    "The panel runs {} — update the panel first; the {name} update needs confirmation until then.",
                    shared::VERSION
                ));
            }
            if release_url.starts_with("https://github.com/") {
                lines.push(format!("Release notes: {release_url}"));
            }
            lines.push(format!("Update from the panel: {page}"));
            let payload = serde_json::json!({
                "event": if *reminder { "update_reminder" } else { "update_available" },
                "component": kind,
                "latest": latest.to_string(),
                "recommended": recommended.map(|v| v.to_string()),
                "panel_version": shared::VERSION,
                "panel_behind": panel_behind,
                "release_url": release_url,
                "outdated": outdated.iter().map(|i| serde_json::json!({ "name": i.name, "installed": i.installed })).collect::<Vec<_>>(),
                "page": page,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            Message {
                subject,
                lines,
                colour: if *reminder { 0xf59e0b } else { 0x3b82f6 },
                payload,
            }
        }
        Event::RunFinished { run, log_tail } => {
            let name = display_kind(&run.kind);
            let ok = run.status == crate::db::RUN_SUCCEEDED;
            let subject = if ok {
                format!("{name} on {} updated to {}", run.ref_name, run.to_version)
            } else {
                format!("{name} update on {} {}", run.ref_name, run.status)
            };
            let mut lines = vec![format!(
                "Strategy: {}. From {} to {}. Triggered by {}.",
                run.strategy,
                run.from_version.as_deref().unwrap_or("unknown"),
                run.to_version,
                run.triggered_by
            )];
            if let Some(error) = &run.error {
                lines.push(format!("Error: {error}"));
            }
            if !log_tail.trim().is_empty() {
                lines.push("Last log lines:".to_string());
                for line in log_tail.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
                    lines.push(format!("  {line}"));
                }
            }
            lines.push(format!("Details: {page}"));
            let payload = serde_json::json!({
                "event": "run_finished",
                "component": run.kind,
                "node": run.ref_name,
                "node_uuid": run.ref_uuid,
                "run_id": run.id,
                "status": run.status,
                "strategy": run.strategy,
                "from_version": run.from_version,
                "to_version": run.to_version,
                "error": run.error,
                "triggered_by": run.triggered_by,
                "page": page,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            Message {
                subject,
                lines,
                colour: if ok { 0x22c55e } else { 0xef4444 },
                payload,
            }
        }
    }
}

async fn panel_url(state: &State) -> String {
    state
        .settings
        .get()
        .await
        .ok()
        .map(|s| s.app.url.to_string())
        .unwrap_or_default()
}

async fn mail_enabled(state: &State) -> bool {
    match state.settings.get().await {
        Ok(settings) => !matches!(settings.mail_mode, shared::settings::MailMode::None),
        Err(_) => false,
    }
}

async fn admin_emails(state: &State) -> Result<Vec<String>> {
    const SQL: &str = "SELECT email FROM users WHERE admin = TRUE AND email <> '' ORDER BY email";
    let rows = sqlx::query(SQL)
        .fetch_all(state.database.read())
        .await
        .context("listing administrators")?;
    Ok(rows.iter().map(|row| row.get::<String, _>("email")).collect())
}

/// Dispatch to every configured channel.
pub async fn send(state: &State, settings: &ExtensionSettings, event: Event<'_>) {
    let url = panel_url(state).await;
    let message = build(&event, &url);

    if settings.notify_email {
        send_email(state, &message).await;
    }
    if !settings.notify_discord_url.is_empty() {
        post(state, &settings.notify_discord_url, discord_payload(&message), None).await;
    }
    if !settings.notify_webhook_url.is_empty() {
        let secret = settings.notify_webhook_secret.as_str();
        post(
            state,
            &settings.notify_webhook_url,
            message.payload.clone(),
            if secret.is_empty() { None } else { Some(secret) },
        )
        .await;
    }
}

/// Subject and body are minijinja templates in the panel's mail transport; both are constant
/// here and every value arrives through the context (HTML auto-escaped).
const SUBJECT_TEMPLATE: &str = "{{ subject }}";
const BODY_TEMPLATE: &str = "<h2>{{ subject }}</h2>{% for line in lines %}<p style=\"margin:0 0 6px 0\">{{ line }}</p>{% endfor %}<p style=\"color:#888;font-size:12px;margin-top:16px\">Sent by the Node Updater extension.</p>";

async fn send_email(state: &State, message: &Message) {
    if !mail_enabled(state).await {
        tracing::debug!(target: crate::LOG, "mail is disabled in the panel settings; skipping e-mail notification");
        return;
    }
    let recipients = match admin_emails(state).await {
        Ok(recipients) => recipients,
        Err(err) => {
            tracing::warn!(target: crate::LOG, "could not list administrators for e-mail: {err:#}");
            return;
        }
    };
    if recipients.is_empty() {
        return;
    }
    let context = minijinja::context! {
        subject => message.subject.clone(),
        lines => message.lines.clone(),
    };
    // A CompactString compiles on both panel generations: 1.2.0 takes CompactString itself,
    // 1.2.1 takes impl Into<MailRecipient> (From<CompactString>) where a bare .into() is ambiguous.
    for recipient in recipients {
        state
            .mail
            .send(compact_str::CompactString::from(recipient), SUBJECT_TEMPLATE, BODY_TEMPLATE, context.clone())
            .await;
    }
}

fn discord_payload(message: &Message) -> serde_json::Value {
    let description: String = message.lines.join("\n").chars().take(3800).collect();
    serde_json::json!({
        "username": "Node Updater",
        // Node names and error text end up in the embed; never let them ping anyone.
        "allowed_mentions": { "parse": [] },
        "embeds": [{
            "title": message.subject.chars().take(240).collect::<String>(),
            "description": description,
            "color": message.colour,
            "footer": { "text": "PineRiver Node Updater" },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }]
    })
}

/* ------------------------------------------------------------------- outbound guard */

/// Resolve `host`, dropping every address the panel blocks for outbound connections
/// (`APP_BLOCKED_CIDRS`).
#[cfg(panel_net_filter)]
async fn allowed_addresses(env: &std::sync::Arc<shared::env::Env>, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let addresses = shared::net::resolve_allowed_addresses(env, host, port, "node updater webhook")
        .await
        .with_context(|| format!("resolving {host}"))?;
    if addresses.is_empty() {
        return Err(anyhow!("{host} resolves only to addresses the panel blocks for outbound requests"));
    }
    Ok(addresses)
}

/// Panels older than the shared `net` module (release 1.1.5) have no operator-configurable
/// block list, so the well-known internal ranges are refused instead.
#[cfg(not(panel_net_filter))]
async fn allowed_addresses(_env: &std::sync::Arc<shared::env::Env>, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolving {host}"))?
        .collect();
    if resolved.is_empty() {
        return Err(anyhow!("could not resolve {host}"));
    }
    let addresses: Vec<_> = resolved.into_iter().filter(|a| !is_internal(&a.ip())).collect();
    if addresses.is_empty() {
        return Err(anyhow!("{host} resolves only to internal addresses, which webhooks may not reach"));
    }
    Ok(addresses)
}

/// Loopback, private, link-local, CGNAT and unspecified ranges.
#[cfg(not(panel_net_filter))]
fn is_internal(ip: &IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || a == 0
                || (a == 100 && (b & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || v6.is_unspecified() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Parse and vet a webhook URL: http(s), no embedded credentials, a host that does not
/// resolve to a blocked address. Returns a client pinned to the vetted addresses that
/// follows no redirects.
async fn guarded_client(state: &State, url: &str) -> Result<(reqwest::Client, reqwest::Url), String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http:// and https:// URLs are allowed".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs with embedded credentials are not allowed".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "the URL has no host".to_string())?
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);

    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());

    match host.parse::<IpAddr>() {
        Ok(ip) => {
            // Literal address: vet it directly.
            allowed_addresses(&state.env, &ip.to_string(), port)
                .await
                .map_err(|e| format!("{e:#}"))?;
        }
        Err(_) => {
            let addresses = allowed_addresses(&state.env, &host, port).await.map_err(|e| format!("{e:#}"))?;
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
    }
    let client = builder.build().map_err(|e| format!("building the HTTP client: {e}"))?;
    Ok((client, parsed))
}

/// Is this a Discord webhook URL (https, a Discord host, the webhooks API path)?
pub fn is_discord_webhook_url(url: &str) -> bool {
    match reqwest::Url::parse(url.trim()) {
        Ok(parsed) => {
            parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed
                    .host_str()
                    .is_some_and(|h| DISCORD_HOSTS.iter().any(|d| h.eq_ignore_ascii_case(d)))
                && parsed.path().starts_with("/api/webhooks/")
        }
        Err(_) => false,
    }
}

/// Validate a generic webhook URL the way sending will: shape and destination. Nothing is sent.
pub async fn validate_webhook_url(state: &State, url: &str) -> Result<(), String> {
    guarded_client(state, url).await.map(|_| ())
}

async fn post(state: &State, url: &str, payload: serde_json::Value, secret: Option<&str>) {
    if let Err(err) = post_checked(state, url, payload, secret).await {
        tracing::warn!(target: crate::LOG, url, "notification webhook failed: {err}");
    }
}

async fn post_checked(state: &State, url: &str, payload: serde_json::Value, secret: Option<&str>) -> Result<(), String> {
    let (client, parsed) = guarded_client(state, url).await?;
    let mut request = client.post(parsed).json(&payload);
    if let Some(secret) = secret {
        request = request.header("X-NodeUpdater-Secret", secret);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => Err(format!("the endpoint answered HTTP {}", response.status())),
        Err(err) => Err(format!("request failed: {err}")),
    }
}

/// Send a test message through one channel; the error text is shown to the operator.
pub async fn test_channel(state: &State, settings: &ExtensionSettings, channel: &str) -> Result<(), String> {
    let url = panel_url(state).await;
    let page = format!("{}/admin/node-updater", url.trim_end_matches('/'));
    let message = Message {
        subject: "Node Updater test notification".to_string(),
        lines: vec![
            "If you can read this, the channel works.".to_string(),
            format!("Panel: {page}"),
        ],
        colour: 0x3b82f6,
        payload: serde_json::json!({
            "event": "test",
            "page": page,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }),
    };
    match channel {
        "discord" => {
            if settings.notify_discord_url.is_empty() {
                return Err("no Discord webhook URL is configured".to_string());
            }
            if !is_discord_webhook_url(&settings.notify_discord_url) {
                return Err("the Discord URL is not a https://discord.com/api/webhooks/… URL".to_string());
            }
            post_checked(state, &settings.notify_discord_url, discord_payload(&message), None).await
        }
        "webhook" => {
            if settings.notify_webhook_url.is_empty() {
                return Err("no webhook URL is configured".to_string());
            }
            let secret = settings.notify_webhook_secret.as_str();
            post_checked(
                state,
                &settings.notify_webhook_url,
                message.payload.clone(),
                if secret.is_empty() { None } else { Some(secret) },
            )
            .await
        }
        "email" => {
            if !mail_enabled(state).await {
                return Err("e-mail is disabled in the panel settings".to_string());
            }
            let recipients = admin_emails(state).await.map_err(|e| format!("{e:#}"))?;
            if recipients.is_empty() {
                return Err("no administrator has an e-mail address".to_string());
            }
            send_email(state, &message).await;
            Ok(())
        }
        _ => Err("unknown channel".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::is_discord_webhook_url;

    #[test]
    fn recognises_discord_webhooks_only() {
        assert!(is_discord_webhook_url("https://discord.com/api/webhooks/123/abc"));
        assert!(is_discord_webhook_url("https://ptb.discord.com/api/webhooks/123/abc"));
        assert!(!is_discord_webhook_url("http://discord.com/api/webhooks/123/abc"));
        assert!(!is_discord_webhook_url("https://discord.com.evil.example/api/webhooks/1/a"));
        assert!(!is_discord_webhook_url("https://discord.com/api/users/@me"));
        assert!(!is_discord_webhook_url("https://user:pw@discord.com/api/webhooks/1/a"));
    }
}
