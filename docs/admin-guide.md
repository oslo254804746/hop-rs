# Admin Web Guide

[简体中文](admin-guide.zh-CN.md) | **English**

Hop's Admin Web is designed for a personal jump server or a small operations
team. A single administrator gets a password-only login and a compact settings
page. Team controls appear only when an Owner chooses to add another
administrator.

For deployment, network exposure, backup, and recovery, use the
[deployment guide](deployment.md).

## Open Admin Web safely

Admin Web listens on `127.0.0.1:8080` by default. Keep that default and use an
SSH tunnel when administering a remote host:

```bash
ssh -N -L 8080:127.0.0.1:8080 root@hop-host
```

Then open `http://127.0.0.1:8080`.

If a reverse proxy terminates HTTPS, set `security.admin_cookie_secure = true`.
Do not publish the Admin port directly to an untrusted network.

## Login behavior

- With one active administrator, the login asks only for a password.
- After a second active administrator is added, the login also asks for the
  account name.
- Account names are matched case-insensitively.
- New teammates receive a temporary password of at least 12 characters and
  must replace it on first login before making changes.
- Admin sessions expire after 30 minutes of inactivity and are stored in
  memory, so restarting Hop signs administrators out.

The administrator migrated from v0.1.4 keeps the existing password and becomes
an Owner.

## Dashboard

The Dashboard uses live runtime and database data:

- **Gateway status** checks the Admin and SSH listeners, SQLite health,
  running version, start time, and uptime.
- **Recent SSH access** summarizes identities, targets, modes, results, and
  durations.
- **Recent admin changes** comes from the separate `audit_events` store.
- **Asset health** uses results recorded by real SSH or TCP connection
  attempts. A newly added or never-used asset remains `unknown`.
- **Action items** identify failed or unknown targets, missing managed
  credentials, missing active SSH access, or a failed Dashboard data source.
- **Coverage** explains managed-credential, restricted-access, and Known Hosts
  coverage with both counts and percentages.

A failed data source degrades only its own Dashboard section instead of taking
down the full page.

## Assets

The Assets page is the primary operational inventory.

- Search across names, addresses, descriptions, protocols, presets, and tags.
- Filter by tag and apply tags to multiple selected assets.
- Copy `hostname:port` without opening the edit form.
- Edit an asset in a focused drawer while keeping the list context.
- Assign a managed credential to SSH assets.
- Use RDP, VNC, MySQL, PostgreSQL, Redis, or Generic TCP presets for tunnel
  defaults and client guidance.

Presets do not make Hop an application-layer proxy. Generic forwarding is TCP
only.

## Credentials

Credentials describe how Hop authenticates to an SSH target for managed TUI,
direct SSH, and SFTP connections.

- Create or edit credentials in a drawer.
- The selected authentication mode shows only its relevant secret fields.
- Saved secrets are encrypted and are never rendered back to the browser.
- On edit, blank secret fields retain the existing encrypted value; enter a
  value only when rotating that field.
- The list shows how many assets use each credential.
- A credential cannot be deleted while an asset still references it.
- CSV and JSON exports contain metadata only. Passwords, private keys, and
  passphrases are never exported.

Back up `hop.secret` with `hop.db`. Losing `hop.secret` makes every stored
credential unrecoverable.

## People and SSH access

An SSH public key controls who may enter Hop and which assets that key may
reach. It is separate from a target credential.

- `all` grants access to every current and future asset.
- `restricted` grants access only to explicitly selected asset IDs.
- An empty `restricted` assignment allows authentication to Hop but exposes no
  assets.

TUI, direct SSH, SFTP, ProxyJump, and generic `direct-tcpip` forwarding enforce
the same per-key asset assignment.

## Known Hosts

Known Hosts stores TOFU trust for target SSH servers.

- Review the recorded target, key type, and fingerprint.
- Copy a fingerprint for out-of-band comparison.
- See whether a trust record matches an existing asset.
- Reset trust only after confirming that the target was intentionally
  reinstalled or its host key was intentionally rotated.

Resetting a record removes the stored trust decision. The next connection
creates a new TOFU record, so verify the new fingerprint before proceeding.

## Audit Logs

Audit Logs combines two evidence streams:

- SSH sessions record identity, mode, target, client address, result, start,
  end, and error context.
- Admin events record the actor, action, target, result, source address, and
  allowlisted structured metadata.

Passwords, private keys, passphrases, and uploaded secret contents are not
written to audit event details.

v0.1.5 displays the most recent 100 SSH sessions and 100 admin events. Query
filters, pagination, retention controls, and audit export are planned but are
not part of v0.1.5.

## Team access without a heavy RBAC UI

Hop exposes three task-oriented access levels:

| Access level | Can do |
|--------------|--------|
| Owner | Everything, including administrators and access levels |
| Operator | View inventory and audit evidence; manage assets, credentials, SSH access, and Known Hosts |
| Viewer | View inventory, Dashboard, and audit evidence without changing configuration |

Only an Owner can add administrators or change their access and active status.
Hop prevents disabling or demoting the final active Owner. Changing an
administrator's access or active status immediately invalidates that
administrator's current sessions.

## Current v0.1.5 boundaries

- Local password authentication only; no OIDC or external identity provider.
- Three fixed access levels; no custom policy editor.
- No aggregated person record containing multiple SSH keys.
- No audit filter, pagination, export, or retention UI yet.
- Asset health is event-driven by real connection attempts, not a continuous
  background monitoring system.
