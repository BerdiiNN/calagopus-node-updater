# Node Updater for Calagopus

An extension for [Calagopus Panel](https://calagopus.com) that keeps your Wings nodes and DB Agent hosts up to date from inside the panel. It watches for new Panel, Wings and DB Agent releases, shows which version every node actually runs, tells you when something is out of date and can update the nodes for you.

It adds a **Node Updater** page to the admin area (under Infrastructure), an update notice on the dashboard and admin home, and an update card on the admin Health page.

- **Package:** `com.pineriver.nodeupdater`
- **Panel:** Calagopus 1.1.0 or newer
- **Nodes:** Linux (systemd or OpenRC; apt, dnf, yum or apk; Docker Compose)
- **Languages:** English and Danish

## Features

| Area | How |
|------|-----|
| Release discovery | GitHub releases of `calagopus/panel`, `calagopus/wings` and `calagopus/db-agent` (newest first, pre-releases optional), plus `https://calagopus.com/api/latest`, shown as "recommended". |
| Inventory | Every node and DB Agent host is asked for its `version`, `architecture` and `container_type` through its own system-overview endpoint. |
| Notifications | E-mail to administrators, a Discord webhook, a generic JSON webhook, an in-panel notice and an admin Health card. Sent once per new version, with optional reminders, and always when an update run fails. |
| Updates | `api`: the daemon's own `POST /api/system/upgrade` (bare-metal binaries). `ssh_docker`, `ssh_package`, `ssh_binary`: a generated bash script over SSH. After every run the panel polls the daemon until it reports the requested version. |
| Auto-updates | Per node: queue an update automatically when a new release appears. |
| Credential profiles | Shared SSH logins (user, password or key, sudo) that several nodes can use. Host, port and host key stay per node. |

## Installation

Installing extensions requires the Calagopus **heavy** image (`:heavy` / `:nightly-heavy`) or a development environment, because the panel compiles extensions when you install them. See [Installing Extensions](https://calagopus.com/docs/panel/extensions/installing-extensions).

1. Download `com_pineriver_nodeupdater.c7s.zip` from the [latest release](https://github.com/BerdiiNN/calagopus-node-updater/releases/latest).
2. In the panel, open **Admin → Extensions** and drop the file into the upload area. The panel installs it, runs its database migrations, compiles it and loads it.
3. After the build finishes, reload the page.

Alternatively, copy the `.c7s.zip` into the panel's `extensions/` data directory (`./build/extensions` with the default heavy compose stack) and run `docker compose restart web`.

In a development environment:

```bash
panel-rs extensions add path/to/com_pineriver_nodeupdater.c7s.zip
panel-rs extensions apply --profile balanced
```

## Getting started

1. Open **Admin → Node Updater**. The overview lists every node and DB Agent host with its installed version and the newest release. Nothing is updated until you set it up.
2. To let the panel update a node, open its target settings and choose a strategy:
   - **API** works for Wings and DB Agent installed as a plain binary. No SSH needed. Wings 1.2.7 and DB Agent 1.2.4 onwards fetch the requested version from their own release provider and restart themselves; older daemons download the asset the panel points them at, check the SHA-256 the panel computed and run the restart command. The extension speaks both protocols on every panel version.
   - **SSH (Docker Compose / package / binary)** runs a short script on the node. Add the node's SSH host and port and a login (or a credential profile), then click **Probe host key**. Check that the `SHA256:…` fingerprint matches the node before you confirm it. **Detect installation** then works out how the daemon was installed.
   - **Auto-detect** uses the detected installation, or the API strategy when no SSH host is set.
3. Under **Settings**, turn on the notifications you want and set how often to check.
4. Update a node from its row, or several at once. Every run has a live log.

Update the panel before the nodes. Calagopus release notes ask for this, so the extension refuses to put a node on a version newer than the panel unless you confirm the run explicitly or enable **Allow updating Wings / DB Agent past the panel version**. The panel itself is only checked and reported, never updated by this extension.

## Permissions (admin group `node-updater`)

| Permission | Grants | Effective power |
|------------|--------|-----------------|
| `read` | Overview, release info, node versions, targets (hosts, user names, fingerprints; never secrets), run history and logs. | Read-only. Run logs contain package-manager output from nodes. |
| `update` | Start, cancel and delete update runs, bulk updates, "Check now". | Replaces the daemon binary on nodes with a published Calagopus release. Can pick any release the panel has seen, including older ones. Triggers notifications and configured auto-updates. |
| `settings` | Targets, SSH credentials, credential profiles, restart commands, notification endpoints, settings. | **Root on every configured node**: the SSH login and the restart command are executed as-is. Grant it the way you would grant SSH root. |

## Settings

| Setting | Default | Range |
|---------|---------|-------|
| Check interval | 60 minutes | 15 minutes to 24 hours |
| Include GitHub pre-releases | off | |
| Allow updating Wings / DB Agent past the panel version | off | |
| E-mail every administrator (uses the panel's mail settings) | on | |
| Also notify when an update run succeeds (failed runs always notify) | on | |
| Discord webhook URL, generic webhook URL and secret | empty | |
| Remind again after | 168 hours | 0 (never) to 90 days |
| SSH connect timeout | 30 s | 5 to 300 s |
| Update timeout | 900 s | 60 to 7200 s |
| Verify timeout (waiting for the new version to report) | 180 s | 30 to 1800 s |

## Security model

- **Authentication.** Every route is under the panel's admin API and checks one of the three permissions above. There are no unauthenticated endpoints.
- **Secrets.** SSH passwords, private keys, passphrases and the webhook secret are encrypted with the panel's `APP_ENCRYPTION_KEY` before they reach the database and are never returned by any endpoint; the UI only learns whether a secret exists. Activity log entries carry no secrets.
- **SSH.** Host keys are trust-on-first-use: the operator probes the host, sees the `SHA256:…` fingerprint and confirms it; any later key change aborts the connection. Scripts are piped to `bash -s` (optionally `sudo -n bash -s`); every operator-supplied value is single-quoted and validated (no control characters, absolute paths). Connect, run and verification phases have timeouts. The binary strategy restarts the service on any exit once it has been stopped, so a lost session cannot leave a node without its daemon.
- **Node targets are admin-supplied hosts.** SSH destinations are not filtered against `APP_BLOCKED_CIDRS` on purpose (nodes live on private networks); loopback, unspecified, multicast and broadcast addresses are refused, so the panel can never be pointed at itself. Only `settings` holders can set them, and that permission is root-equivalent anyway.
- **Webhooks are filtered.** Notification URLs are posted through a dedicated client that resolves the host, refuses addresses the panel blocks for outbound traffic (or the internal ranges on panels without `APP_BLOCKED_CIDRS`), pins the connection to the vetted addresses and follows no redirects. Discord URLs must be `https://discord.com/api/webhooks/…` (or the canary / ptb / discordapp hosts). URLs are validated on save and again on send. Discord embeds are sent with `allowed_mentions: []`.
- **E-mail.** The panel renders mail subject and body as templates with the mail settings in scope; this extension passes every value (node names, error text) through the template context so nothing from a node can be interpreted as template syntax.
- **Supply chain.** Binaries come only from the pinned GitHub repositories over TLS. For daemons that predate their own release provider the panel downloads the asset once, hashes it and hands the SHA-256 to the daemon, which refuses a mismatch (newer daemons download and smoke-test the release themselves); the SSH binary strategy verifies the same hash on the node. The hash proves the node got the bytes the panel saw, not that GitHub itself is honest. A version newer than the panel is refused unless forced or allowed in the settings. Manual updates can only target releases the panel has seen in the release list.
- **Concurrency.** One queued or running run per node is enforced by a partial unique index; runs execute one at a time on the primary panel instance; interrupted runs are marked failed on restart.
- **Rate limits.** Periodic checks run at most every 15 minutes; manual checks have a 15-second cooldown; GitHub is queried three times per check.

## Operational notes

- `ignore_panel_wings_upgrades: true` in a Wings config disables the `api` strategy for that node; use an SSH strategy instead. Wings also refuses the `api` strategy when it runs inside a container.
- Cancelling a running SSH update closes the session. The `docker` and `package` scripts are safe to interrupt; the `binary` script restarts the service on exit.
- If the PineRiver Extension Hub is installed, the checker also reports its status there. Without it, those reports are skipped.

## Building from source

Clone the repository into the `backend-extensions` directory of a [Calagopus development environment](https://calagopus.com/docs/panel/extensions/dev-environment). The directory name must be the package identifier:

```bash
cd panel
git clone https://github.com/BerdiiNN/calagopus-node-updater.git backend-extensions/com_pineriver_nodeupdater
panel-rs extensions resync
panel-rs extensions apply --profile dev
```

Run the unit tests (script generation and quoting, detection parsing, SSH host checks, webhook URL checks, version handling) with:

```bash
cargo test -p com_pineriver_nodeupdater
```

Build an installable archive with:

```bash
panel-rs extensions export com.pineriver.nodeupdater
```

The package version is set in three places that must match: `Metadata.toml`, `Cargo.toml` and `frontend/package.json`. The panel caches builds by version, so bump the version for every release. Migrations that have shipped are never edited; schema changes go into a new `migrations/<timestamp>_<name>/` directory.

### Layout

```
Metadata.toml            package metadata (name, version, panel requirement)
Cargo.toml               backend crate
migrations/              database tables for targets, profiles, runs and state
src/
  lib.rs                 extension entry point: permissions, routes, background checker
  versions.rs            GitHub and calagopus.com release discovery
  inventory.rs           per-node version poll and overview
  checker.rs             periodic check, notifications, auto-update queue
  notify.rs              e-mail, Discord and webhook delivery
  runner.rs              update runs (API and SSH strategies)
  scripts.rs             the bash scripts that run on nodes
  ssh.rs                 SSH client with host key pinning
  db.rs                  targets, profiles, runs, encrypted secrets
  routes/                admin HTTP API
  settings.rs            extension settings
  health.rs              optional Extension Hub reporting
frontend/
  src/index.ts           registers the admin page, notices and Health card
  src/pages/             the Node Updater page
  src/components/        overview, runs, profiles and settings tabs
  src/translations.ts    English strings
  public/translations/   Danish translation
```

## License

[MIT](LICENSE) © 2026 PineRiver. You're free to use, modify and redistribute this extension, including commercially, as long as you keep the copyright notice.
