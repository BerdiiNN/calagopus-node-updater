//! SSH to a node: connect, authenticate, run a script, stream its output.
//!
//! Connect / auth boilerplate mirrors `shared/src/git.rs` and the SFTP Import extension.
//! The host key is trust-on-first-use: [`probe_host_key`] shows the key, the operator
//! confirms it in the UI, and every later connection refuses any other key.
//!
//! No outbound address filter is applied here on purpose: update targets are configured by
//! administrators holding `node-updater.settings`, the same trust level as the node URL the
//! panel already talks to, and nodes routinely live on private networks that the panel's
//! default `APP_BLOCKED_CIDRS` would refuse.

use anyhow::{Context, Result, anyhow};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub enum RemoteAuth {
    Password(String),
    Key { pem: String, passphrase: Option<String> },
}

/// What the remote host identified itself with.
#[derive(Debug, Clone)]
pub struct HostKey {
    /// `SHA256:<base64>` — the format `ssh-keygen -l` prints.
    pub fingerprint: String,
    /// e.g. `ssh-ed25519`.
    pub algorithm: String,
}

/// Host-key policy for one connection: records the key the server presents and, when a
/// fingerprint is expected, rejects any other.
struct HostKeyPolicy {
    expected: Option<String>,
    seen: Arc<Mutex<Option<HostKey>>>,
}

impl HostKeyPolicy {
    fn accept(&self, key: HostKey) -> bool {
        let ok = self
            .expected
            .as_deref()
            .is_none_or(|expected| expected == key.fingerprint);
        *self.seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(key);
        ok
    }
}

fn describe_key(key: &russh::keys::PublicKey) -> HostKey {
    HostKey {
        fingerprint: key.fingerprint(russh::keys::HashAlg::Sha256).to_string(),
        algorithm: key.algorithm().to_string(),
    }
}

impl russh::client::Handler for HostKeyPolicy {
    type Error = russh::Error;

    // russh 0.63 hands the host key over as a `PublicKeyOrCertificate`; panels built on
    // russh 0.62 pass a `PublicKey`. `src/build.rs` picks the signature from the panel's
    // Cargo.lock so the same source compiles on both.
    #[cfg(russh_cert_hosts)]
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        use russh::keys::PublicKeyOrCertificate;

        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => describe_key(key),
            PublicKeyOrCertificate::Certificate(certificate) => {
                let key = certificate.public_key();
                HostKey {
                    fingerprint: key.fingerprint(russh::keys::HashAlg::Sha256).to_string(),
                    algorithm: key.algorithm().to_string(),
                }
            }
        };
        Ok(self.accept(key))
    }

    #[cfg(not(russh_cert_hosts))]
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(self.accept(describe_key(server_public_key)))
    }
}

fn ssh_config() -> Arc<russh::client::Config> {
    Arc::new(russh::client::Config {
        keepalive_interval: Some(Duration::from_secs(20)),
        ..Default::default()
    })
}

/// Addresses that can never be a node: the panel host itself, the unspecified address,
/// multicast and broadcast. Private ranges stay allowed — nodes live there.
fn is_never_a_node(ip: &std::net::IpAddr) -> bool {
    match ip.to_canonical() {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_multicast(),
    }
}

async fn resolve(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>> {
    let resolved: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolving {host}:{port}"))?
        .collect();
    if resolved.is_empty() {
        return Err(anyhow!("could not resolve {host}"));
    }
    let addresses: Vec<_> = resolved.into_iter().filter(|a| !is_never_a_node(&a.ip())).collect();
    if addresses.is_empty() {
        return Err(anyhow!(
            "{host} resolves only to loopback, unspecified or multicast addresses, which cannot be a node"
        ));
    }
    Ok(addresses)
}

/// Connect far enough to learn the host key, then hang up. Nothing is authenticated.
pub async fn probe_host_key(host: &str, port: u16, timeout: Duration) -> Result<HostKey> {
    let probe = async {
        let addresses = resolve(host, port).await?;
        let seen = Arc::new(Mutex::new(None));
        let policy = HostKeyPolicy {
            expected: None,
            seen: seen.clone(),
        };
        let session = russh::client::connect(ssh_config(), addresses.as_slice(), policy)
            .await
            .with_context(|| format!("connecting to {host}:{port}"))?;
        drop(session);
        let key = seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        key.ok_or_else(|| anyhow!("the server at {host}:{port} did not present a host key"))
    };
    tokio::time::timeout(timeout, probe)
        .await
        .map_err(|_| anyhow!("timed out connecting to {host}:{port}"))?
}

/// An authenticated SSH session. `Send` but not `Sync` — keep it on one task.
pub struct Session {
    handle: russh::client::Handle<HostKeyPolicy>,
    pub use_sudo: bool,
}

/// Result of one remote command.
pub struct Output {
    pub exit_code: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Both streams, stderr last, for error messages.
    pub fn combined(&self) -> String {
        let mut text = self.stdout.trim_end().to_string();
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim_end());
        }
        text
    }
}

impl Session {
    /// Open an SSH connection and authenticate. `expected_fingerprint` is the `SHA256:…`
    /// value the operator confirmed; any other host key aborts the connection.
    pub async fn connect(
        host: &str,
        port: u16,
        user: &str,
        auth: RemoteAuth,
        expected_fingerprint: &str,
        use_sudo: bool,
        timeout: Duration,
    ) -> Result<Self> {
        tokio::time::timeout(
            timeout,
            Self::connect_inner(host, port, user, auth, expected_fingerprint, use_sudo),
        )
        .await
        .map_err(|_| anyhow!("timed out connecting to {host}:{port}"))?
    }

    async fn connect_inner(
        host: &str,
        port: u16,
        user: &str,
        auth: RemoteAuth,
        expected_fingerprint: &str,
        use_sudo: bool,
    ) -> Result<Self> {
        if expected_fingerprint.trim().is_empty() {
            return Err(anyhow!(
                "no confirmed host key for {host} — probe the host key in the target settings first"
            ));
        }
        let addresses = resolve(host, port).await?;

        let seen = Arc::new(Mutex::new(None));
        let policy = HostKeyPolicy {
            expected: Some(expected_fingerprint.to_string()),
            seen: seen.clone(),
        };

        let mut handle = match russh::client::connect(ssh_config(), addresses.as_slice(), policy).await {
            Ok(handle) => handle,
            Err(err) => {
                let presented = seen
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(presented) = presented
                    && presented.fingerprint != expected_fingerprint
                {
                    return Err(anyhow!(
                        "the host key of {host} does not match the confirmed one (expected {expected_fingerprint}, the server presented {} {}) — re-probe the host in the target settings if the key legitimately changed",
                        presented.algorithm,
                        presented.fingerprint
                    ));
                }
                return Err(anyhow::Error::from(err).context(format!("connecting to {host}:{port}")));
            }
        };

        let authenticated = match auth {
            RemoteAuth::Password(password) => handle.authenticate_password(user, password).await?,
            RemoteAuth::Key { pem, passphrase } => {
                let key = shared::git::parse_private_key(&pem, passphrase.as_deref())
                    .context("reading the private key")?;
                let hash_alg = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(
                        user,
                        russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                    )
                    .await?
            }
        };

        if !authenticated.success() {
            return Err(anyhow!(
                "SSH authentication was rejected for {user}@{host} — check the username and password / key"
            ));
        }

        Ok(Self { handle, use_sudo })
    }

    /// Run `script` through `bash -s` on the remote host (piped over stdin, so no quoting
    /// problems), streaming every output chunk to `on_output` as it arrives. With
    /// `use_sudo` the shell is started through `sudo -n` (non-interactive: the account
    /// needs passwordless sudo for the commands involved).
    pub async fn run_script<F>(&self, script: &str, timeout: Duration, mut on_output: F) -> Result<Output>
    where
        F: FnMut(&str) + Send,
    {
        let shell = if self.use_sudo { "sudo -n bash -s" } else { "bash -s" };
        let run = async {
            let mut channel = self
                .handle
                .channel_open_session()
                .await
                .context("opening an SSH channel")?;
            channel
                .exec(true, shell)
                .await
                .context("starting the remote shell")?;
            channel
                .data(script.as_bytes())
                .await
                .context("sending the script")?;
            channel.eof().await.context("closing the script stream")?;

            let mut stdout = String::new();
            let mut stderr = String::new();
            let mut exit_code = None;
            let mut pending_stdout: Vec<u8> = Vec::new();
            let mut pending_stderr: Vec<u8> = Vec::new();

            while let Some(message) = channel.wait().await {
                match message {
                    russh::ChannelMsg::Data { data } => {
                        pending_stdout.extend_from_slice(data.as_ref());
                        for line in drain_lines(&mut pending_stdout) {
                            on_output(&line);
                            stdout.push_str(&line);
                        }
                    }
                    russh::ChannelMsg::ExtendedData { data, .. } => {
                        pending_stderr.extend_from_slice(data.as_ref());
                        for line in drain_lines(&mut pending_stderr) {
                            on_output(&line);
                            stderr.push_str(&line);
                        }
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
                    russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {
                        // Either can arrive before ExitStatus; wait for the channel to close.
                        if matches!(message, russh::ChannelMsg::Close) {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            for (pending, sink) in [(&mut pending_stdout, &mut stdout), (&mut pending_stderr, &mut stderr)] {
                if !pending.is_empty() {
                    let tail = String::from_utf8_lossy(pending).to_string();
                    on_output(&tail);
                    sink.push_str(&tail);
                    pending.clear();
                }
            }
            Ok::<Output, anyhow::Error>(Output {
                exit_code,
                stdout,
                stderr,
            })
        };
        tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| anyhow!("the remote command did not finish within {} seconds", timeout.as_secs()))?
    }

    /// Run a short one-liner and collect its output.
    pub async fn run(&self, command: &str, timeout: Duration) -> Result<Output> {
        self.run_script(command, timeout, |_| {}).await
    }

    pub async fn close(self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await;
    }
}

/// Split complete lines (with their newline) off the front of a byte buffer.
fn drain_lines(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(position) = buffer.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=position).collect();
        lines.push(String::from_utf8_lossy(&line).to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{drain_lines, is_never_a_node};

    #[test]
    fn refuses_addresses_that_cannot_be_nodes() {
        let never = ["127.0.0.1", "0.0.0.0", "::1", "::", "224.0.0.1", "255.255.255.255", "::ffff:127.0.0.1"];
        for address in never {
            assert!(is_never_a_node(&address.parse().unwrap()), "{address} should be refused");
        }
        let allowed = ["10.0.0.5", "192.168.1.20", "172.16.3.4", "203.0.113.9", "fd00::1", "2001:db8::1"];
        for address in allowed {
            assert!(!is_never_a_node(&address.parse().unwrap()), "{address} should be allowed");
        }
    }

    #[test]
    fn drains_complete_lines_only() {
        let mut buffer = b"one\ntwo\nthr".to_vec();
        let lines = drain_lines(&mut buffer);
        assert_eq!(lines, vec!["one\n".to_string(), "two\n".to_string()]);
        assert_eq!(buffer, b"thr".to_vec());
    }
}
