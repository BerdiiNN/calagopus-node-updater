//! Where releases come from.
//!
//! Two feeds are consulted and both are shown to the operator:
//!
//! * **GitHub releases** of `calagopus/panel`, `calagopus/wings` and `calagopus/db-agent`
//!   (tags `release-X.Y.Z`, assets `wings-rs-<arch>-linux`, `db-agent-<arch>-linux`,
//!   `panel-rs-<arch>-linux`). This is the newest published build.
//! * **`https://calagopus.com/api/latest`** — the feed the panel's own update checker uses.
//!   It can lag behind GitHub; it is shown as the *recommended* version.
//!
//! "Latest" for update decisions is the GitHub release (pre-releases only when the operator
//! opted in). Results are cached in the `_state` table so the page never waits for GitHub.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use shared::State;
use std::time::Duration;
use utoipa::ToSchema;

pub const STATE_KEY_LATEST: &str = "latest";
/// Sent with every outbound request (GitHub, calagopus.com, webhooks).
pub(crate) const USER_AGENT: &str = concat!(
    "PineRiver-NodeUpdater/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/BerdiiNN/calagopus-node-updater)"
);
const GITHUB_TIMEOUT: Duration = Duration::from_secs(30);

/// GitHub repository per component kind.
pub fn repository(kind: &str) -> Option<&'static str> {
    match kind {
        crate::db::KIND_WINGS => Some("calagopus/wings"),
        crate::db::KIND_DB_AGENT => Some("calagopus/db-agent"),
        crate::db::KIND_PANEL => Some("calagopus/panel"),
        _ => None,
    }
}

/// Release asset name for a kind + machine architecture (`uname -m`).
pub fn asset_name(kind: &str, architecture: &str) -> Option<String> {
    let arch = normalise_arch(architecture)?;
    match kind {
        crate::db::KIND_WINGS => Some(format!("wings-rs-{arch}-linux")),
        crate::db::KIND_DB_AGENT => Some(format!("db-agent-{arch}-linux")),
        crate::db::KIND_PANEL => Some(format!("panel-rs-{arch}-linux")),
        _ => None,
    }
}

/// Map the architecture strings daemons report onto the release asset suffixes.
pub fn normalise_arch(architecture: &str) -> Option<&'static str> {
    match architecture.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        "riscv64" | "riscv64gc" => Some("riscv64"),
        "ppc64le" | "powerpc64le" => Some("ppc64le"),
        _ => None,
    }
}

/// Download URL of one release asset.
pub fn download_url(kind: &str, version: &semver::Version, architecture: &str) -> Option<String> {
    let repo = repository(kind)?;
    let asset = asset_name(kind, architecture)?;
    Some(format!("https://github.com/{repo}/releases/download/release-{version}/{asset}"))
}

/// Parse the version string a daemon reports (`1.1.5`, `1.1.5:abc1234@main`, `v1.1.5`).
pub fn parse_reported(raw: &str) -> Option<semver::Version> {
    let trimmed = raw.trim().trim_start_matches('v');
    let core = trimmed.split([':', ' ']).next().unwrap_or(trimmed);
    semver::Version::parse(core).ok()
}

/// Parse a release tag (`release-1.2.0`, `1.2.0`, `v1.2.0`).
pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    let trimmed = tag.trim();
    let stripped = trimmed
        .strip_prefix("release-")
        .or_else(|| trimmed.strip_prefix('v'))
        .unwrap_or(trimmed);
    semver::Version::parse(stripped).ok()
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    #[schema(value_type = String)]
    pub version: semver::Version,
    pub tag: String,
    pub prerelease: bool,
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    /// First lines of the release notes.
    #[serde(default)]
    pub notes: String,
    /// Asset names, so the UI can tell which architectures exist.
    #[serde(default)]
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComponentLatest {
    /// Newest GitHub release (respecting the pre-release setting).
    #[schema(inline)]
    pub latest: Option<Release>,
    /// Version the calagopus.com feed points at.
    #[schema(value_type = Option<String>)]
    pub recommended: Option<semver::Version>,
    /// A few recent releases for the version picker (newest first).
    #[schema(inline)]
    pub recent: Vec<Release>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct LatestInfo {
    #[schema(inline)]
    pub panel: ComponentLatest,
    #[schema(inline)]
    pub wings: ComponentLatest,
    #[schema(inline)]
    pub db_agent: ComponentLatest,
    pub checked_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub feed_error: Option<String>,
}

impl LatestInfo {
    pub fn for_kind(&self, kind: &str) -> Option<&ComponentLatest> {
        match kind {
            crate::db::KIND_WINGS => Some(&self.wings),
            crate::db::KIND_DB_AGENT => Some(&self.db_agent),
            crate::db::KIND_PANEL => Some(&self.panel),
            _ => None,
        }
    }

    pub fn latest_version(&self, kind: &str) -> Option<&semver::Version> {
        self.for_kind(kind)?.latest.as_ref().map(|r| &r.version)
    }

    /// The release matching `version`, when it is among the ones fetched.
    pub fn release(&self, kind: &str, version: &semver::Version) -> Option<&Release> {
        let component = self.for_kind(kind)?;
        component
            .recent
            .iter()
            .chain(component.latest.iter())
            .find(|r| &r.version == version)
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
}

async fn github_releases(state: &State, repo: &str, include_prereleases: bool) -> Result<Vec<Release>> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=15");
    let response = state
        .client
        .get(&url)
        .timeout(GITHUB_TIMEOUT)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let remaining = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        return Err(anyhow!(
            "GitHub answered HTTP {status} for {repo} (rate-limit remaining: {remaining})"
        ));
    }
    let releases: Vec<GitHubRelease> = response.json().await.context("decoding the GitHub release list")?;

    let mut out: Vec<Release> = releases
        .into_iter()
        .filter(|r| !r.draft && (include_prereleases || !r.prerelease))
        .filter_map(|r| {
            let version = parse_tag(&r.tag_name)?;
            let notes: String = r
                .body
                .unwrap_or_default()
                .lines()
                .take(12)
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(1200)
                .collect();
            Some(Release {
                version,
                tag: r.tag_name,
                prerelease: r.prerelease,
                html_url: r.html_url,
                published_at: r.published_at,
                notes,
                assets: r.assets.into_iter().map(|a| a.name).collect(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(out)
}

#[derive(Deserialize)]
struct Feed {
    versions: FeedVersions,
}

#[derive(Deserialize)]
struct FeedVersions {
    #[serde(default)]
    panel: Option<String>,
    #[serde(default)]
    wings: Option<String>,
    #[serde(default)]
    db_agent: Option<String>,
}

async fn calagopus_feed(state: &State) -> Result<FeedVersions> {
    let feed: Feed = state
        .client
        .get("https://calagopus.com/api/latest")
        .timeout(GITHUB_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("requesting calagopus.com/api/latest")?
        .error_for_status()
        .context("calagopus.com/api/latest")?
        .json()
        .await
        .context("decoding calagopus.com/api/latest")?;
    Ok(feed.versions)
}

async fn component(state: &State, kind: &str, include_prereleases: bool, recommended: Option<&str>) -> ComponentLatest {
    let repo = repository(kind).expect("known kind");
    let mut out = ComponentLatest {
        recommended: recommended.and_then(parse_reported),
        ..Default::default()
    };
    match github_releases(state, repo, include_prereleases).await {
        Ok(releases) => {
            out.latest = releases.first().cloned();
            out.recent = releases.into_iter().take(8).collect();
        }
        Err(err) => out.error = Some(format!("{err:#}")),
    }
    out
}

/// Query both feeds and store the result. Errors per component are recorded, not raised, so
/// one unreachable feed never hides the others.
pub async fn refresh(state: &State, include_prereleases: bool) -> Result<LatestInfo> {
    let (feed, feed_error) = match calagopus_feed(state).await {
        Ok(feed) => (Some(feed), None),
        Err(err) => (None, Some(format!("{err:#}"))),
    };
    let feed = feed.unwrap_or(FeedVersions {
        panel: None,
        wings: None,
        db_agent: None,
    });

    let (panel, wings, db_agent) = tokio::join!(
        component(state, crate::db::KIND_PANEL, include_prereleases, feed.panel.as_deref()),
        component(state, crate::db::KIND_WINGS, include_prereleases, feed.wings.as_deref()),
        component(state, crate::db::KIND_DB_AGENT, include_prereleases, feed.db_agent.as_deref()),
    );

    let info = LatestInfo {
        panel,
        wings,
        db_agent,
        checked_at: chrono::Utc::now(),
        feed_error,
    };
    crate::db::state_set(state, STATE_KEY_LATEST, &info).await?;
    Ok(info)
}

/// The cached result of the last [`refresh`], if any.
pub async fn cached(state: &State) -> Option<LatestInfo> {
    crate::db::state_get::<LatestInfo>(state, STATE_KEY_LATEST)
        .await
        .ok()
        .flatten()
}

/// Cached info, refreshed when missing or older than `max_age`.
pub async fn cached_or_refresh(state: &State, include_prereleases: bool, max_age: Duration) -> Result<LatestInfo> {
    if let Some(info) = cached(state).await {
        let age = chrono::Utc::now() - info.checked_at;
        if age.to_std().map(|age| age < max_age).unwrap_or(true) {
            return Ok(info);
        }
    }
    refresh(state, include_prereleases).await
}

/// SHA-256 of a release asset, downloaded once by the panel and cached. The Wings / DB Agent
/// upgrade API insists on a checksum and GitHub does not publish one for these assets.
pub async fn asset_sha256(state: &State, kind: &str, version: &semver::Version, architecture: &str) -> Result<String> {
    use sha2::Digest;

    let arch = normalise_arch(architecture)
        .ok_or_else(|| anyhow!("unsupported architecture '{architecture}' — no release asset exists for it"))?;
    let version_text = version.to_string();
    if let Some(cached) = crate::db::checksum_get(state, kind, &version_text, arch).await? {
        return Ok(cached);
    }

    let url = download_url(kind, version, arch).ok_or_else(|| anyhow!("no download URL for {kind}"))?;
    let mut response = state
        .client
        .get(&url)
        .timeout(Duration::from_secs(600))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .with_context(|| format!("downloading {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {url}"))?;

    // Release binaries are 15–40 MB; refuse anything absurd so a bad URL cannot fill memory or disk.
    const MAX_ASSET: u64 = 512 * 1024 * 1024;
    let mut hasher = sha2::Sha256::new();
    let mut size: u64 = 0;
    while let Some(chunk) = response.chunk().await.context("reading the release asset")? {
        size += chunk.len() as u64;
        if size > MAX_ASSET {
            return Err(anyhow!("the release asset at {url} is larger than {MAX_ASSET} bytes"));
        }
        hasher.update(&chunk);
    }
    if size == 0 {
        return Err(anyhow!("the release asset at {url} is empty"));
    }
    let sha256 = hex::encode(hasher.finalize());
    crate::db::checksum_put(state, kind, &version_text, arch, &sha256, size as i64).await?;
    Ok(sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reported_versions() {
        assert_eq!(parse_reported("1.1.5").unwrap().to_string(), "1.1.5");
        assert_eq!(parse_reported("1.1.5:abc1234@main").unwrap().to_string(), "1.1.5");
        assert_eq!(parse_reported("v1.2.0").unwrap().to_string(), "1.2.0");
        assert!(parse_reported("nightly").is_none());
    }

    #[test]
    fn parses_tags_and_builds_urls() {
        assert_eq!(parse_tag("release-1.2.0").unwrap().to_string(), "1.2.0");
        let v = semver::Version::parse("1.2.0").unwrap();
        assert_eq!(
            download_url("wings", &v, "x86_64").unwrap(),
            "https://github.com/calagopus/wings/releases/download/release-1.2.0/wings-rs-x86_64-linux"
        );
        assert_eq!(
            download_url("db_agent", &v, "arm64").unwrap(),
            "https://github.com/calagopus/db-agent/releases/download/release-1.2.0/db-agent-aarch64-linux"
        );
        assert!(download_url("wings", &v, "mips").is_none());
    }
}
