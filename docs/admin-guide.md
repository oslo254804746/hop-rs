# Hop administrator guide

Use the official panel for day-to-day local resources. Resources declared in `hop.yaml` must be changed in that file and applied by restarting Hop.

## Web panel

Open the Compose panel at `http://<host>:8080` and enter `api.token`. The panel calls same-origin `/api/v1` by default. After a successful validation, the Token is held in the current tab's `sessionStorage`; a refresh verifies it and reconnects automatically. Closing the tab or choosing **Forget connection** clears the Token. A separate remote API URL is available under the advanced connection disclosure.

Both backend and panel warn when `change-me` is used. After rotating the Token, reopen the connection dialog and authenticate again.

The panel checks API `ownership` before rendering mutations:

- `local`: editable, removable, and rotatable;
- `config`: read-only, labelled as managed by `hop.yaml`, with mutation actions absent.

## Ingress public keys

Ingress keys decide who can connect to Hop. Add them in the panel or with the CLI:

```bash
hop-server --config /etc/hop/hop.yaml key add \
  --name oslo-laptop \
  --public-key-file ./oslo-laptop.pub
hop-server --config /etc/hop/hop.yaml key list
```

In `hop.yaml`, choose exactly one of `public_key` and `public_key_file`. Omitting `assets` grants all assets, `[]` grants none, and a non-empty list grants exactly those assets.

## Target credentials

Target credentials let Hop log into SSH assets. The panel and API return only configured/missing secret status, never stored values.

```bash
hop-server --config /etc/hop/hop.yaml credential list
printf '%s\n' "$TARGET_PASSWORD" | hop-server --config /etc/hop/hop.yaml credential add \
  --name nas-root --username root --auth-type password --password-stdin
```

Direct secrets in YAML require `0600` permissions.

## Assets, multi-hop chaining, and `~/.ssh/config`

```bash
hop-server --config /etc/hop/hop.yaml asset add \
  --name nas --protocol ssh --hostname 192.168.1.20 --port 22 \
  --display-name "Home NAS" --tags home,storage --credential nas-root
hop-server --config /etc/hop/hop.yaml asset add \
  --name internal-db --protocol tcp --hostname 10.0.0.50 --port 5432 \
  --via-asset nas
hop-server --config /etc/hop/hop.yaml asset list
hop-server --config /etc/hop/hop.yaml asset probe
```

Use `--via-asset <name_or_id>` to chain private-subnet SSH/TCP targets through an upstream managed SSH jump host (up to 4 hops), and `--allow-tcp-forwarding <true|false>` to control managed-mode `-L` / `-D` port forwarding per asset. Connect directly using the asset name or generate `~/.ssh/config` blocks for managed vault relay (`--mode relay`, supporting `-L` / `-D` and Cursor / VS Code Remote-SSH without local target keys) or end-to-end `ProxyJump`:

```bash
ssh -p 2222 nas@hop.example.com
ssh -p 2222 -L 15432:127.0.0.1:5432 nas@hop.example.com
hop-server --config /etc/hop/hop.yaml ssh-config --host hop.example.com --mode relay
hop-server --config /etc/hop/hop.yaml ssh-config --host hop.example.com --mode proxyjump
```

## One-click SSH key bootstrap

For newly provisioned password-based Linux servers, trigger **Bootstrap SSH key** in the panel (`POST /api/v1/assets/{id}/bootstrap-key`) or run the CLI:

```bash
printf '%s\n' "$ROOT_PASSWORD" | hop-server --config /etc/hop/hop.yaml \
  asset bootstrap-key nas --username root --password-stdin --disable-password-auth
```

Hop generates an Ed25519 keypair in memory, installs `~/.ssh/authorized_keys` idempotently with `0700`/`0600` permissions, verifies a fresh second SSH connection with the new private key before rotating the asset's encrypted vault credential (never deleting the existing password credential if verification fails), and hardens `sshd` `PasswordAuthentication no` only when `--disable-password-auth` is explicitly requested.

## Scheduled probing and alerts

Alongside on-demand probing (`POST /api/v1/assets/probe`), configure background probing and encrypted alert channels on the panel's **Alerts** page (or via `probe` and `notifications` in `hop.yaml`):

- `GET / PUT /api/v1/probe/settings`: configure background scheduler enablement, interval (30–86400s), timeout, consecutive failure threshold, and recovery notifications;
- `GET /api/v1/assets/probe-history` and `GET /api/v1/assets/{id}/probe-history`: fetch recent RTT samples for sparklines;
- `GET / POST /api/v1/notifications/channels`, `PUT / DELETE /api/v1/notifications/channels/{id}`, and `POST /api/v1/notifications/channels/{id}/test`: manage and test **Telegram Bot**, **Bark**, and **HMAC-SHA256 Webhook** channels.

## Sessions and audit events

The panel shows the latest 100 session records and recent audit events (`GET /api/v1/audit-events` or `hop-server audit list`). Termination can signal only a `started` session with an active in-memory transport. A stale record may return not active while remaining in history. Completed sessions and audit events older than `runtime.session_retention_days` are pruned at startup and every 6 hours while running.

When Hop starts, any `started` record left by an earlier process is closed as
`failed` with an error explaining that the server restarted. This recovery runs
before the SSH and Control API listeners open, so sessions created by the
current process are not selected. On SIGINT or SIGTERM, Hop stops accepting
new HTTP/SSH connections, signals active transports, and allows up to five
seconds for SSH connections to drain before terminating anything remaining.

## Host trust (Known Hosts)

Hop records an SSH target host-key fingerprint through TOFU on the first managed connection and verifies it on every later connection. Reinstalling a target or intentionally rotating its SSH host key makes the stored record reject the new key.

Verify the new fingerprint through an independent trusted path first. Then open **Host trust** in the panel (or run `hop-server known-hosts reset --hostname <host> --port <port> --key-type <type>`), select the exact host, port, and key algorithm, and choose **Reset trust**. The server requires an explicit confirmation field. The next managed connection establishes the new TOFU fingerprint automatically.

Control API operations:

- `GET /api/v1/known-hosts` lists trusted records.
- `DELETE /api/v1/known-hosts` requires `hostname`, `port`, `key_type`, and `confirm_reset: true` in its JSON body.

## Backup and restore

Back up together:

- `hop.yaml` and referenced public-key files;
- `data_dir/hop.db`;
- `data_dir/hop.secret`;
- `data_dir/hop_host_key`.

The database and `hop.secret` must be restored as a pair or stored target secrets cannot be decrypted. Stop Hop, restore complete files and permissions, then restart and inspect logs.

## Security checklist

- YAML is `0600` and excluded from version control.
- Compose does not publish the Control API port.
- Non-Compose remote management is behind a TLS reverse proxy.
- Webpage Token and target credentials are rotated.
- Unregistered keys receive only `Permission denied (publickey)`.
- Tokens and target secrets never appear in logs, API responses, or frontend assets.
