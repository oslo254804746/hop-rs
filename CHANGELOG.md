# Changelog

All notable user-visible changes to Hop are recorded here.

## [Unreleased]

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

[Unreleased]: https://github.com/oslo254804746/hop-rs/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/oslo254804746/hop-rs/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/oslo254804746/hop-rs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/oslo254804746/hop-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.2.0
[0.1.7]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.7
[0.1.6]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.6
[0.1.5]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.5
