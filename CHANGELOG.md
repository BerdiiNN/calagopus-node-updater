# Changelog

## 1.0.0 (2026-09-21)

First public release.

- Release discovery for Calagopus Panel, Wings and DB Agent from GitHub and calagopus.com.
- Version inventory of every Wings node and DB Agent host.
- Notifications by e-mail, Discord, generic webhook, in-panel notice and admin Health card, with optional reminders.
- Remote updates through the daemon upgrade API or over SSH (Docker Compose, package manager or binary installs), with a live run log and a version check afterwards.
- Per-node auto-updates and shared SSH credential profiles.
- `node-updater` admin permission group (`read`, `update`, `settings`).
- English and Danish translations.
- Compatible with panel 1.2.1, where the mail API changed how recipients are passed.
- The User-Agent now carries the real extension version and points to this repository.
- The Docker Compose update script passes the configured service name as one quoted argument.
