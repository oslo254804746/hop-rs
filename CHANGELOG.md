# Changelog

All notable user-visible changes to Hop are recorded here.

## [Unreleased]

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

[Unreleased]: https://github.com/oslo254804746/hop-rs/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/oslo254804746/hop-rs/releases/tag/v0.1.5

