//! Build script: detect which panel generation this extension is compiled against.
//!
//! * `russh_cert_hosts` — `client::Handler::check_server_key` takes `&PublicKey` up to russh
//!   0.62 and `&PublicKeyOrCertificate` from 0.63; a trait impl can only spell one of them.
//!   The panel workspace `Cargo.lock` is the source of truth.
//! * `panel_net_filter` — `shared::net` (operator-configurable outbound block list,
//!   `APP_BLOCKED_CIDRS`) arrived after release 1.1.5; older panels get the extension's
//!   built-in internal-range filter for webhook destinations instead.
//!
//! The extension sits at `<panel>/backend-extensions/<pkg>`, so the panel sources are a few
//! levels up.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(russh_cert_hosts)");
    println!("cargo::rustc-check-cfg=cfg(panel_net_filter)");

    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut version = None;
    let mut dir = manifest_dir.as_path();
    for _ in 0..4 {
        let net = dir.join("shared").join("src").join("net.rs");
        if net.is_file() {
            println!("cargo::rerun-if-changed={}", net.display());
            println!("cargo::rustc-cfg=panel_net_filter");
        }
        let lock = dir.join("Cargo.lock");
        if lock.is_file() {
            println!("cargo::rerun-if-changed={}", lock.display());
            if let Ok(text) = std::fs::read_to_string(&lock) {
                version = russh_version(&text);
                if version.is_some() {
                    break;
                }
            }
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    // When the lock cannot be read, assume the current generation.
    let (major, minor) = version.unwrap_or((0, 63));
    if (major, minor) >= (0, 63) {
        println!("cargo::rustc-cfg=russh_cert_hosts");
    }
}

/// `(major, minor)` of the `russh` package recorded in a `Cargo.lock`.
fn russh_version(lock: &str) -> Option<(u32, u32)> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "name = \"russh\"" {
            continue;
        }
        let version = lines
            .next()?
            .trim()
            .strip_prefix("version = \"")?
            .trim_end_matches('"');
        let mut parts = version.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        return Some((major, minor));
    }
    None
}
