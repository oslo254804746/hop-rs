# Changelog

All notable user-visible changes to Hop are recorded here.

## [Unreleased]

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

[Unreleased]: https://github.com/oslo254804746/hop-rs/compare/v0.1.7...HEAD
[0.1.7]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.7
[0.1.6]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.6
[0.1.5]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.5
