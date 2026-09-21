//! The shell that runs on a node: a detection script that reports how the daemon was
//! installed, and one update script per strategy, generated from the official updating
//! guides (`/docs/wings/updating`, `/docs/db-agent/updating`).
//!
//! Scripts are piped to `bash -s` over SSH, so operator-supplied values (paths, service
//! names) are single-quoted with [`quote`] and validated before they get here.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Names that differ between the two daemons.
pub struct Names {
    pub package: &'static str,
    pub units: &'static [&'static str],
    pub binaries: &'static [&'static str],
    pub image: &'static str,
    pub default_unit: &'static str,
    pub default_binary: &'static str,
    pub display: &'static str,
}

pub fn names(kind: &str) -> Names {
    match kind {
        crate::db::KIND_DB_AGENT => Names {
            package: "calagopus-db-agent",
            units: &["db-agent", "calagopus-db-agent"],
            binaries: &[
                "/usr/local/bin/calagopus-db-agent",
                "/usr/bin/calagopus-db-agent",
                "/usr/local/bin/db-agent",
            ],
            image: "calagopus/db-agent",
            default_unit: "db-agent",
            default_binary: "/usr/local/bin/calagopus-db-agent",
            display: "DB Agent",
        },
        _ => Names {
            package: "calagopus-wings",
            units: &["wings", "calagopus-wings"],
            binaries: &[
                "/usr/local/bin/wings",
                "/usr/bin/calagopus-wings",
                "/usr/local/bin/calagopus-wings",
            ],
            image: "calagopus/wings",
            default_unit: "wings",
            default_binary: "/usr/local/bin/wings",
            display: "Wings",
        },
    }
}

/// Single-quote a value for bash.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Reject values that would break out of a quoted context or make no sense as a path/name.
pub fn valid_token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.chars().any(|c| c.is_control())
        && !value.contains('\0')
}

const HEADER: &str = r#"set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
log() { printf '[node-updater] %s\n' "$*"; }
service_ctl() {
  # service_ctl <action> <unit>
  if command -v systemctl >/dev/null 2>&1; then systemctl "$1" "$2"
  elif command -v rc-service >/dev/null 2>&1; then rc-service "$2" "$1"
  else log "no systemd or OpenRC found; cannot $1 $2" >&2; return 1; fi
}
"#;

/// Everything the detection script reports, parsed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub os: String,
    pub architecture: String,
    pub kernel: String,
    pub init: String,
    pub package_manager: String,
    pub package_installed: String,
    pub unit: String,
    pub unit_active: String,
    pub exec_start: String,
    pub binary: String,
    pub binary_version: String,
    pub docker_container: String,
    pub docker_image: String,
    pub compose_dir: String,
    pub compose_service: String,
    pub compose_tool: String,
    pub whoami: String,
    pub sudo_ok: bool,
    /// The strategy the findings point at.
    pub suggested_strategy: String,
    pub suggestion_reason: String,
}

/// Script that reports how the daemon is installed on the node. Read-only.
pub fn detect_script(kind: &str) -> String {
    let n = names(kind);
    let units = n.units.join(" ");
    let binaries = n.binaries.join(" ");
    let package = n.package;
    let image = n.image;
    format!(
        r#"set -u
echo "WHOAMI=$(id -un 2>/dev/null || echo unknown)"
if [ "$(id -u)" -eq 0 ]; then echo "SUDO_OK=1"; elif sudo -n true >/dev/null 2>&1; then echo "SUDO_OK=1"; else echo "SUDO_OK=0"; fi
if [ -r /etc/os-release ]; then . /etc/os-release; echo "OS=${{PRETTY_NAME:-${{ID:-unknown}}}}"; else echo "OS=$(uname -s)"; fi
echo "ARCH=$(uname -m)"
echo "KERNEL=$(uname -r)"
if command -v systemctl >/dev/null 2>&1; then echo "INIT=systemd"; elif command -v rc-service >/dev/null 2>&1; then echo "INIT=openrc"; else echo "INIT=unknown"; fi
if command -v dpkg-query >/dev/null 2>&1; then
  echo "PKG=apt"
  V=$(dpkg-query -W -f='${{Version}}' {package} 2>/dev/null || true); [ -n "$V" ] && echo "PKG_INSTALLED=$V"
elif command -v rpm >/dev/null 2>&1; then
  if command -v dnf >/dev/null 2>&1; then echo "PKG=dnf"; else echo "PKG=yum"; fi
  V=$(rpm -q --qf '%{{VERSION}}' {package} 2>/dev/null || true); case "$V" in *"not installed"*) V="";; esac; [ -n "$V" ] && echo "PKG_INSTALLED=$V"
elif command -v apk >/dev/null 2>&1; then
  echo "PKG=apk"
  V=$(apk info -e {package} 2>/dev/null || true); [ -n "$V" ] && echo "PKG_INSTALLED=$(apk version {package} 2>/dev/null | tail -n1 | awk '{{print $1}}')"
fi
for u in {units}; do
  if command -v systemctl >/dev/null 2>&1 && systemctl cat "$u" >/dev/null 2>&1; then
    echo "UNIT=$u"; echo "UNIT_ACTIVE=$(systemctl is-active "$u" 2>/dev/null || true)"
    echo "EXEC_START=$(systemctl show -p ExecStart --value "$u" 2>/dev/null | sed -n 's/.*path=\([^ ;]*\).*/\1/p' | head -n1)"
    break
  elif [ -x "/etc/init.d/$u" ]; then
    echo "UNIT=$u"; echo "UNIT_ACTIVE=$(rc-service "$u" status >/dev/null 2>&1 && echo active || echo inactive)"; break
  fi
done
for b in {binaries}; do
  if [ -x "$b" ]; then echo "BIN=$b"; echo "BIN_VERSION=$(timeout 10 "$b" version 2>/dev/null | head -n1 || true)"; break; fi
done
if command -v docker >/dev/null 2>&1; then
  CID=$(docker ps --format '{{{{.ID}}}} {{{{.Image}}}}' 2>/dev/null | grep -i '{image}' | head -n1 | cut -d' ' -f1 || true)
  if [ -n "$CID" ]; then
    echo "DOCKER_CONTAINER=$CID"
    echo "DOCKER_IMAGE=$(docker inspect -f '{{{{.Config.Image}}}}' "$CID" 2>/dev/null)"
    echo "COMPOSE_DIR=$(docker inspect -f '{{{{index .Config.Labels "com.docker.compose.project.working_dir"}}}}' "$CID" 2>/dev/null)"
    echo "COMPOSE_SERVICE=$(docker inspect -f '{{{{index .Config.Labels "com.docker.compose.service"}}}}' "$CID" 2>/dev/null)"
  fi
  if docker compose version >/dev/null 2>&1; then echo "COMPOSE=plugin"; elif command -v docker-compose >/dev/null 2>&1; then echo "COMPOSE=standalone"; fi
fi
exit 0
"#
    )
}

/// Turn the `KEY=value` lines of [`detect_script`] into a [`Detected`] with a suggestion.
pub fn parse_detection(output: &str) -> Detected {
    let mut d = Detected::default();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().to_string();
        match key.trim() {
            "OS" => d.os = value,
            "ARCH" => d.architecture = value,
            "KERNEL" => d.kernel = value,
            "INIT" => d.init = value,
            "PKG" => d.package_manager = value,
            "PKG_INSTALLED" => d.package_installed = value,
            "UNIT" => d.unit = value,
            "UNIT_ACTIVE" => d.unit_active = value,
            "EXEC_START" => d.exec_start = value,
            "BIN" => d.binary = value,
            "BIN_VERSION" => d.binary_version = value,
            "DOCKER_CONTAINER" => d.docker_container = value,
            "DOCKER_IMAGE" => d.docker_image = value,
            "COMPOSE_DIR" => d.compose_dir = value,
            "COMPOSE_SERVICE" => d.compose_service = value,
            "COMPOSE" => d.compose_tool = value,
            "WHOAMI" => d.whoami = value,
            "SUDO_OK" => d.sudo_ok = value == "1",
            _ => {}
        }
    }

    if !d.docker_container.is_empty() {
        d.suggested_strategy = crate::db::STRATEGY_SSH_DOCKER.to_string();
        d.suggestion_reason = if d.compose_dir.is_empty() {
            format!(
                "container {} runs image {}, but it was not started by Docker Compose — set the compose directory by hand",
                d.docker_container, d.docker_image
            )
        } else {
            format!(
                "container {} runs image {} from the compose project in {}",
                d.docker_container, d.docker_image, d.compose_dir
            )
        };
    } else if !d.package_installed.is_empty() {
        d.suggested_strategy = crate::db::STRATEGY_SSH_PACKAGE.to_string();
        d.suggestion_reason = format!(
            "installed from the {} repository (package version {})",
            d.package_manager, d.package_installed
        );
    } else if !d.binary.is_empty() || !d.exec_start.is_empty() {
        d.suggested_strategy = crate::db::STRATEGY_SSH_BINARY.to_string();
        let path = if d.binary.is_empty() { &d.exec_start } else { &d.binary };
        d.suggestion_reason = format!(
            "plain binary at {path}{} — the daemon's own upgrade API (strategy \"api\") also works for this install",
            if d.unit.is_empty() { String::new() } else { format!(" managed by the {} service", d.unit) }
        );
    } else {
        d.suggested_strategy = String::new();
        d.suggestion_reason = "no Docker container, package or binary of this daemon was found on the host".to_string();
    }
    d
}

pub struct UpdatePlan<'a> {
    pub kind: &'a str,
    pub version: &'a semver::Version,
    /// `None` when the node's architecture is unknown: the script then uses `uname -m`.
    pub architecture: Option<&'a str>,
    pub sha256: Option<&'a str>,
    pub compose_dir: &'a str,
    pub compose_service: &'a str,
    pub service_name: &'a str,
    pub binary_path: &'a str,
}

/// `docker compose pull && docker compose up -d` in the compose project directory.
pub fn docker_script(plan: &UpdatePlan<'_>) -> String {
    let n = names(plan.kind);
    let service = if plan.compose_service.is_empty() { "" } else { plan.compose_service };
    format!(
        r#"{HEADER}
DIR={dir}
SVC={svc}
cd "$DIR"
if docker compose version >/dev/null 2>&1; then DC="docker compose"; elif command -v docker-compose >/dev/null 2>&1; then DC="docker-compose"; else log "neither 'docker compose' nor 'docker-compose' is available" >&2; exit 2; fi
log "pulling the {display} image in $DIR (service: ${{SVC:-all}})"
$DC pull ${{SVC:+"$SVC"}}
log "recreating the container"
$DC up -d ${{SVC:+"$SVC"}}
sleep 3
$DC ps ${{SVC:+"$SVC"}} || true
log "done — the panel now waits for the daemon to report version {version}"
"#,
        dir = quote(plan.compose_dir),
        svc = quote(service),
        display = n.display,
        version = plan.version,
    )
}

/// Upgrade the distribution package and restart the service.
pub fn package_script(plan: &UpdatePlan<'_>) -> String {
    let n = names(plan.kind);
    let unit = if plan.service_name.is_empty() { n.default_unit } else { plan.service_name };
    format!(
        r#"{HEADER}
PKG={package}
UNIT={unit}
if command -v apt-get >/dev/null 2>&1; then
  log "apt-get update"; apt-get update -qq
  log "upgrading $PKG"; apt-get install -y -qq --only-upgrade "$PKG"
elif command -v dnf >/dev/null 2>&1; then
  log "dnf upgrade $PKG"; dnf -y upgrade "$PKG"
elif command -v yum >/dev/null 2>&1; then
  log "yum update $PKG"; yum -y update "$PKG"
elif command -v apk >/dev/null 2>&1; then
  log "apk upgrade $PKG"; apk update; apk upgrade "$PKG"
else
  log "no supported package manager (apt, dnf, yum, apk) found" >&2; exit 2
fi
log "restarting $UNIT"
service_ctl restart "$UNIT"
sleep 3
for b in {binaries}; do if [ -x "$b" ]; then log "installed: $($b version 2>/dev/null | head -n1)"; break; fi; done
log "done — the panel now waits for the daemon to report version {version}"
"#,
        package = quote(n.package),
        unit = quote(unit),
        binaries = n.binaries.join(" "),
        version = plan.version,
    )
}

/// Stop, replace the binary with the exact release asset, start.
pub fn binary_script(plan: &UpdatePlan<'_>) -> String {
    let n = names(plan.kind);
    let unit = if plan.service_name.is_empty() { n.default_unit } else { plan.service_name };
    let binary = if plan.binary_path.is_empty() { n.default_binary } else { plan.binary_path };
    let repo = crate::versions::repository(plan.kind).unwrap_or("calagopus/wings");
    let asset_prefix = match plan.kind {
        crate::db::KIND_DB_AGENT => "db-agent",
        _ => "wings-rs",
    };
    let arch_line = match plan.architecture.and_then(crate::versions::normalise_arch) {
        Some(arch) => format!("ARCH={}", quote(arch)),
        None => "ARCH=$(uname -m)".to_string(),
    };
    format!(
        r#"{HEADER}
UNIT={unit}
BIN={binary}
SHA={sha}
{arch_line}
URL="https://github.com/{repo}/releases/download/release-{version}/{asset_prefix}-${{ARCH}}-linux"
TMP="$BIN.new"
log "downloading $URL"
curl -fsSL --retry 3 -o "$TMP" "$URL"
if [ -n "$SHA" ]; then
  echo "$SHA  $TMP" | sha256sum -c - >/dev/null && log "checksum verified" || {{ log "checksum mismatch" >&2; rm -f "$TMP"; exit 3; }}
else
  log "no checksum available for this architecture; skipping verification"
fi
chmod 0755 "$TMP"
log "new binary reports: $("$TMP" version 2>/dev/null | head -n1 || echo unknown)"
log "stopping $UNIT"
# From here on the service is down: whatever happens (error, lost SSH session, cancel), start
# it again on exit so the node is never left without its daemon.
trap 'service_ctl start "$UNIT" >/dev/null 2>&1 || true' EXIT
service_ctl stop "$UNIT" || true
mv -f "$TMP" "$BIN"
log "starting $UNIT"
service_ctl start "$UNIT"
trap - EXIT
sleep 3
if command -v systemctl >/dev/null 2>&1; then log "service state: $(systemctl is-active "$UNIT" 2>/dev/null || true)"; fi
log "done — the panel now waits for the daemon to report version {version}"
"#,
        unit = quote(unit),
        binary = quote(binary),
        sha = quote(plan.sha256.unwrap_or("")),
        version = plan.version,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_single_quotes() {
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn detect_script_renders_literal_braces() {
        let script = detect_script("wings");
        assert!(script.contains("--format '{{.ID}} {{.Image}}'"));
        assert!(script.contains("${PRETTY_NAME:-${ID:-unknown}}"));
        assert!(script.contains("-f='${Version}' calagopus-wings"));
        assert!(script.contains("--qf '%{VERSION}' calagopus-wings"));
        assert!(script.contains("awk '{print $1}'"));
        assert!(script.contains("grep -i 'calagopus/wings'"));
        assert!(script.contains("com.docker.compose.project.working_dir"));
        let db = detect_script("db_agent");
        assert!(db.contains("for u in db-agent calagopus-db-agent; do"));
    }

    #[test]
    fn parses_detection_output() {
        let d = parse_detection("OS=Ubuntu 24.04\nARCH=x86_64\nPKG=apt\nPKG_INSTALLED=1.1.5-1\nUNIT=wings\nBIN=/usr/bin/calagopus-wings\n");
        assert_eq!(d.suggested_strategy, "ssh_package");
        assert_eq!(d.architecture, "x86_64");
        let d = parse_detection("DOCKER_CONTAINER=abc\nDOCKER_IMAGE=ghcr.io/calagopus/wings:latest\nCOMPOSE_DIR=/opt/wings\nCOMPOSE_SERVICE=wings\n");
        assert_eq!(d.suggested_strategy, "ssh_docker");
        let d = parse_detection("BIN=/usr/local/bin/wings\nUNIT=wings\n");
        assert_eq!(d.suggested_strategy, "ssh_binary");
    }

    #[test]
    fn scripts_embed_values() {
        let v = semver::Version::parse("1.2.0").unwrap();
        let plan = UpdatePlan {
            kind: "wings",
            version: &v,
            architecture: Some("x86_64"),
            sha256: Some("abc"),
            compose_dir: "/opt/wings",
            compose_service: "wings",
            service_name: "",
            binary_path: "",
        };
        let d = docker_script(&plan);
        assert!(d.contains("DIR='/opt/wings'"));
        assert!(d.contains("SVC='wings'"));
        // Quoted when set, no argument at all when empty (= every service).
        assert!(d.contains(r#"$DC pull ${SVC:+"$SVC"}"#));
        assert!(!d.contains("$DC pull $SVC"));
        let all = docker_script(&UpdatePlan { compose_service: "", ..plan });
        assert!(all.contains("SVC=''"));
        assert!(package_script(&plan).contains("PKG='calagopus-wings'"));
        let b = binary_script(&plan);
        assert!(b.contains("release-1.2.0/wings-rs-${ARCH}-linux"));
        assert!(b.contains("ARCH='x86_64'"));
        assert!(b.contains("BIN='/usr/local/bin/wings'"));
    }
}
