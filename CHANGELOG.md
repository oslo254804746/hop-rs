# Changelog

All notable user-visible changes to Hop are recorded here.

## [Unreleased]

## [0.3.0] - 2026-11-13

### Added

- Added managed-mode SSH TCP port forwarding (`direct-tcpip` over vault-authenticated
  target sessions), enabling `ssh -L`, `ssh -D`, and Cursor / VS Code Remote-SSH
  over `--mode relay` without storing target credentials locally. Controlled via
  `ssh.allow_managed_tcp_forwarding` (default `true`) and per-asset
  `allow_tcp_forwarding` (default `true`, CLI `--allow-tcp-forwarding`).
- Added chained multi-hop SSH/TCP relay (`via_asset` in `hop.yaml`, `--via-asset`
  in CLI, `via_asset_id` in Control API and Web Panel) up to 4 hops deep with
  cycle validation and per-hop TOFU host-key verification across shell, `exec`,
  SFTP, port forwarding, and zero-agent probes.
- Added TUI-selected target SFTP follow and parent-bound lifecycle management for
  `hop@<hop-host>` (`menu`, `hop`, `+tui`): multiplexed and concurrent same-key /
  same-IP SFTP, `exec`, and `direct-tcpip` channels wait cleanly for TUI target
  selection, attach to the chosen asset, and close automatically as soon as the
  parent TUI target session exits or disconnects.
- Added zero-agent one-click SSH key bootstrap and password rotation
  (`POST /api/v1/assets/{id}/bootstrap-key` and `hop-server asset bootstrap-key`),
  verifying a fresh Ed25519 SSH login before rotating the encrypted vault
  credential and optionally disabling `sshd` `PasswordAuthentication`.
- Added a background probe scheduler (`GET/PUT /api/v1/probe/settings` and
  `probe` in `hop.yaml`), per-asset RTT sample history
  (`GET /api/v1/assets/probe-history`, `GET /api/v1/assets/{id}/probe-history`),
  and encrypted **Telegram**, **Bark**, and **HMAC-SHA256 Webhook** alert
  channels (`GET/POST/PUT/DELETE /api/v1/notifications/channels`,
  `POST /api/v1/notifications/channels/{id}/test`, and `notifications` in
  `hop.yaml`) with failure-threshold and recovery state tracking.
- Recorded granular access and port-forwarding denial reasons on `sessions.error`
  and `audit_events` (`session.access_denied`, `session.forward_denied`).

### Fixed

- Terminated follower SFTP, `exec`, and port-forwarding sessions immediately when
  an interactive TUI target shell exits back to the selector or disconnects,
  preventing orphaned GUI SFTP panels in clients such as Netcatty.
- Removed runtime inline style mutations in the Web Panel so sparklines and
  dialogs satisfy strict `style-src 'self'` Content-Security-Policy rules with
  zero browser console CSP violations.
- Synchronized `document.title` with the active locale and wrapped SHA256
  fingerprints cleanly on mobile viewports.

### Compatibility and upgrade

- Existing `v0.2.x` SQLite databases are upgraded automatically inside a startup
  transaction (`via_asset_id`, `allow_tcp_forwarding`, `asset_probe_samples`,
  `probe_settings`, and `notification_channels`), preserving all existing assets,
  encrypted credentials, access keys, Known Hosts, sessions, and audit events.
- Back up `hop.yaml` and `data_dir` (`hop.db`, `hop.secret`, `hop_host_key`) as a
  pair before upgrading, then update `hop-rs`, `hop-rs-frontend`, and
  `luci-app-hop` together to `v0.3.0`. See the
  [full v0.3.0 release notes](docs/releases/v0.3.0.md).

## [0.2.9] - 2026-11-11

### Added

- Surfaced stored `asset_health` telemetry across Catalog queries, Control API
  asset responses, CLI (`asset list`, `asset probe`), SSH TUI (`r` to probe),
  and the official Web Panel.
- Added zero-agent on-demand network and protocol banner latency probing
  (`POST /api/v1/assets/probe`, `POST /api/v1/assets/{id}/probe`, and
  `hop-server asset probe [target]`), verifying TCP reachability and RFC 4253
  `SSH-` banners with bounded concurrency.
- Wired `audit_events` recording across Control API mutations and local CLI
  commands, exposed `GET /api/v1/audit-events` and `hop-server audit list`, and
  added startup plus periodic 6-hour retention pruning.
- Added `tags` support to declarative `hop.yaml` / Manifest assets and exposed
  `display_name` across local asset CRUD and CLI (`--display-name`).
- Added `hop-server ssh-config` (`--mode relay` and `--mode proxyjump`),
  `hop-server key delete`, `hop-server known-hosts list|reset`, and
  `--credential <id_or_name>` resolution on `asset add`.
- Exposed `ssh_listen` on `GET /api/v1/status` so the Web Panel generates
  ready-to-copy `ssh` and `~/.ssh/config` snippets with the active SSH port.

### Fixed

- Enforced exact target port matching on `.hop` `ProxyJump` / `direct-tcpip`
  authorization (`a.name = ?4 AND a.port = ?3`), matching host-based rules.
- Allowed `ProxyJump` gateway usernames (e.g. `proxy`) and reserved `menu` /
  `+tui` usernames without failing authentication when a user matches a
  non-managed or TCP asset name.
- Forwarded the SSH client's negotiated PTY `TERM` value to target shells and
  PTY exec sessions instead of hardcoding `xterm-256color`.
- Populated `key_id` and `asset_id` on runtime `sessions` records.
- Removed the unmounted remote HTTP manifest routes
  (`/api/v1/config/{validate,diff,apply}`) so `SecretSource::File` remains
  strictly local to CLI and startup config.
- Enabled multi-arch (`linux/amd64`, `linux/arm64`) GHCR container image builds
  and aligned `systemd/hop.service` and CLI documentation with `/etc/hop/hop.yaml`.

### Compatibility and upgrade

- No SQLite schema migration is required from `v0.2.8` (`asset_health`,
  `audit_events`, `display_name`, `key_id`, and `asset_id` columns already exist
  in the `v0.2` baseline schema). Upgrade `hop-rs`, `hop-rs-frontend`, and
  `luci-app-hop` together to `v0.2.9`.

## [0.2.8] - 2026-08-27

### Changed

- Added graceful shutdown handling for SIGINT and SIGTERM. The HTTP listener
  stops accepting requests, while SSH connections receive a short drain window
  before remaining transports are terminated.
- Session records left in `started` state by an interrupted process are
  recovered as `failed` during startup with an explanatory error message.
- Bumped the workspace version to `0.2.8`.

### Compatibility and upgrade

- No SQLite schema or configuration-key change is included. Existing
  deployments can be upgraded in place after backing up persistent data.

## [0.2.5] - 2026-08-25

### Fixed

- Fixed SSH key-exchange interoperability with Termius on iOS. russh now
  correctly ignores the guessed first KEX packet when `first_kex_packet_follows`
  names an unsupported algorithm, instead of parsing that packet as the next
  SSH message and failing with `SshEncoding: character encoding invalid`.

### Changed

- Upgraded russh from 0.61.1 to the exact 0.63.1 release. This includes the
  upstream KEXINIT regression test and security fixes for invalid channel IDs
  and MAC-required cipher negotiation.

### Compatibility and upgrade

- No SQLite migration, configuration-key change, or database-format change is
  included. Back up persistent data and configuration, then deploy the backend
  with `HOP_VERSION=v0.2.5`.
- The panel has no functional changes in this release. Compose continues to
  use the same matching `HOP_VERSION` tag for the backend and panel images.
- If a rollback is needed, stop Hop, restore the backup if necessary, and set
  `HOP_VERSION=v0.2.4` to use the previous matching backend/panel images.

## [0.2.4] - 2026-08-16

### Added

- Restored Known Hosts inspection and explicit TOFU trust reset through the
  Control API and official panel. Resets require the exact host, port, and key
  type plus an explicit server-side confirmation flag.
- Added matching desktop/mobile host-trust views with complete SHA256
  fingerprints, first-seen timestamps, matching SSH assets, and a guarded
  rebuild/key-rotation workflow.

## [0.2.3] - 2026-08-16

### Fixed

- Classified wrapped SSH EOF/reset errors as client disconnects instead of
  session failures, and added the client address to disconnect, failure, and
  ingress-authentication logs.
- Refreshed the active panel route immediately after first authentication, so
  instance status and Catalog resources no longer require a tab change.

## [0.2.2] - 2026-08-16

### Fixed

- Made the container default command honor `HOP_CONFIG`, so a read-only config
  mounted outside `/data` is used without repeating CLI arguments.

### Changed

- Made the official Compose file pull released backend and panel images without
  source build contexts.
- Reduced deployment to one ignored runtime `hop.yaml`; panel-first and
  config-first YAML files now live under `examples/` as alternative templates.

## [0.2.1] - 2026-08-15

### Added

- Added one strict startup YAML for runtime settings, direct target secrets,
  assets, and ingress public keys, applied atomically before listeners start.
- Added the official `hop` + `panel` Compose deployment with a private Control
  API, same-origin panel proxy, read-only configuration, and persistent data.
- Added minimal `local`/`config` ownership to resource API responses so the
  panel can make configuration-managed records read-only before rendering
  mutation actions.

### Fixed

- Made `api.cors_allowlist = ["*"]` select Tower HTTP's wildcard origin mode
  instead of panicking, and reject ambiguous wildcard-plus-origin lists with a
  configuration error.
- Limited the SSH ingress authentication methods advertised to clients to
  `publickey`, avoiding misleading password prompts for an ingress that has no
  password authentication.

### Changed

- Replaced `api.token_file` with the direct, redacted `api.token`. A missing or
  empty token disables only the optional Control API; SSH continues to run.
- Removed startup inventory lists, file watching, the separate Docker resource
  manifest, and the reload endpoint from the supported startup workflow.
- Removed the non-loopback requirement for a non-empty CORS list. Complete
  Origins remain supported for direct cross-origin clients, while Compose uses
  same-origin proxying and publishes no backend API port.
- Limited the asset type contract to `ssh` and `tcp`; service names such as RDP
  and MySQL no longer appear as protocol aliases or presets.
- Added the production graphical management panel as the recommended path;
  the LuCI package remains a separate service/core-download integration.
- Removed superseded Admin Web roadmaps and implementation plans from the
  active repository documentation.
- Removed unused bulk-session registry code and replaced v0.1-specific
  administrator/legacy-database names with v0.2 management/schema terms.
- Added configuration, proxying, Linux deployment, and `luci-app-hop` guides.

## [0.2.0] - 2026-08-14

### Changed

- Replaced the v0.1 migration chain with a fresh `hop/v0.2` SQLite Catalog
  baseline. Existing non-v0.2 databases are rejected read-only with explicit
  backup/delete/new-path guidance.
- Added strict YAML/TOML validate, diff, dry-run, atomic apply, revision
  conflicts, source ownership, orphan status, and opt-in prune.
- Unified TUI, direct SSH, exec, SCP/SFTP, ProxyJump, TCP forwarding, and TCP
  presets on the same all/restricted Access Key boundary.
- Added a default-off, loopback Control API with one management token, local
  resource CRUD, session termination, and declarative operations.
- Removed the built-in Admin Web, its Node/Docker/release-asset pipeline,
  administrator accounts, cookies, CSRF, Owner/Operator/Viewer roles,
  capability code, and `reset-admin`.
- Added stable-snapshot inventory watching through the same Apply engine and
  an OpenWrt core-package boundary with procd/UCI service integration.

### Compatibility

- v0.2 is intentionally incompatible with v0.1 data and provides no migration,
  import, compatibility reader, or dual-version mode.

## [0.1.7] - 2026-08-09

### Fixed

- Managed PTY, remote-command, and SFTP setup now ignores informational SSH
  channel-window adjustments while waiting for the target's request reply.
- Remote commands now work with OpenSSH targets that increase their channel
  window immediately after opening a session.

### Compatibility

- No database migration or configuration change is required from v0.1.6.
- Configured SSH authentication banners still appear on stderr before the
  client selects interactive shell, exec, or subsystem mode. Set
  `ssh.banner = ""` when automation requires clean stderr.

See the [full v0.1.7 release notes](docs/releases/v0.1.7.md).

## [0.1.6] - 2026-08-09

### Added

- Managed SSH remote-command execution with stdin, stdout, stderr, exit status,
  exit signal, and optional PTY forwarding.
- Live SSH session controls in Admin Web for terminating one connection or all
  connections registered by the gateway.
- Administrative audit events for session termination actions.

### Changed

- Remote commands use the managed target credential path and are recorded as
  `exec` sessions without displaying the interactive Hop banner.
- Managed SSH and direct TCP connection outcomes continue to update asset
  health while supporting administrator-initiated termination.

### Compatibility

- No database migration or configuration change is required from v0.1.5.
- Existing interactive shell, SFTP, ProxyJump, and generic TCP flows remain
  supported.

See the [full v0.1.6 release notes](docs/releases/v0.1.6.md).

## [0.1.5] - 2026-07-29

### Added

- Real-data Admin Dashboard with gateway status, asset health, recent
  activity, action items, and coverage indicators.
- Admin audit events for sign-in, password, asset, credential, SSH access,
  Known Hosts, import, and administrator changes.
- Progressive multi-administrator access with Owner, Operator, and Viewer
  profiles.
- Per-administrator passwords, temporary-password rotation, last-owner
  protection, and session revocation after access changes.
- Inline asset editing, target-address copy, credential create/edit drawers,
  credential usage guards, and safer Known Hosts reset flows.

### Changed

- The one-administrator login remains password-only. The account field appears
  only after a second active administrator is added.
- Admin navigation and responsive layouts were redesigned around frequent
  operational tasks.

### Upgrade notes

- v0.1.4 data is migrated automatically on first v0.1.5 startup.
- Existing assets, credentials, SSH keys, Known Hosts, and the local admin
  password are preserved.
- A database opened by v0.1.5 cannot be opened directly by v0.1.4. Rolling
  back requires restoring the pre-upgrade data backup.

See the [full v0.1.5 release notes](docs/releases/v0.1.5.md).

[Unreleased]: https://github.com/oslo254804746/hop-rs/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/oslo254804746/hop-rs/compare/v0.2.9...v0.3.0
[0.2.9]: https://github.com/oslo254804746/hop-rs/compare/v0.2.8...v0.2.9
[0.2.8]: https://github.com/oslo254804746/hop-rs/compare/v0.2.5...v0.2.8
[0.2.5]: https://github.com/oslo254804746/hop-rs/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/oslo254804746/hop-rs/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/oslo254804746/hop-rs/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/oslo254804746/hop-rs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/oslo254804746/hop-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.2.0
[0.1.7]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.7
[0.1.6]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.6
[0.1.5]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.5
