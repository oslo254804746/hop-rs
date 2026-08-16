# Hop administrator guide

Use the official panel for day-to-day local resources. Resources declared in `hop.yaml` must be changed in that file and applied by restarting Hop.

## Web panel

Open the Compose panel at `http://<host>:8080` and enter `api.token`. The panel calls same-origin `/api/v1` by default. The Token stays only in page memory and is cleared on refresh. A separate remote API URL is available under the advanced connection disclosure.

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
hop-server --config /etc/hop/hop.yaml credential add-password \
  --name nas-root --username root
```

Interactive secret input is not echoed. Direct secrets in YAML require `0600` permissions.

## Assets

```bash
hop-server --config /etc/hop/hop.yaml asset add-ssh \
  --name nas --host 192.168.1.20 --port 22 --credential nas-root
hop-server --config /etc/hop/hop.yaml asset add-tcp \
  --name metrics --host 192.168.1.30 --port 9090
hop-server --config /etc/hop/hop.yaml asset list
```

Connect using the asset name:

```bash
ssh -p 2222 nas@hop.example.com
```

## Sessions

The panel shows the latest 100 session records. Termination can signal only a `started` session with an active in-memory transport. A stale record may return not active while remaining in history.

## Host trust (Known Hosts)

Hop records an SSH target host-key fingerprint through TOFU on the first managed connection and verifies it on every later connection. Reinstalling a target or intentionally rotating its SSH host key makes the stored record reject the new key.

Verify the new fingerprint through an independent trusted path first. Then open **Host trust** in the panel, select the exact host, port, and key algorithm, and choose **Reset trust**. The server requires an explicit confirmation field. The next managed connection establishes the new TOFU fingerprint automatically.

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
