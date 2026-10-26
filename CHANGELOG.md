# Changelog

## 1.0.1 (2026-10-26)

- Builds on panel 1.2.7, whose API client no longer serializes the daemon upgrade response.
- The `api` strategy speaks both daemon upgrade protocols on every panel version: Wings 1.2.7 and DB Agent 1.2.4 onwards are asked for a version and fetch it from their own release provider; older daemons still get the download URL, SHA-256 and restart command. Panel 1.2.7 can therefore update nodes that still run Wings 1.2.6 or older, and older panels can update nodes that already run the new Wings.
- The upgrade call waits up to 15 minutes, because both daemon generations download the release before they answer.
- No architecture check for daemons that pick their own release asset.

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
