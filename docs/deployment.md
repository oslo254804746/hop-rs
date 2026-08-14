# Hop v0.2 deployment and recovery

This guide covers Linux binaries, systemd, Docker, backup, and recovery.

## Runtime files

Keep these files together under a service-owned directory such as `/var/lib/hop`:

| File | Purpose | Backup |
|---|---|---|
| `hop.db` | v0.2 Catalog and runtime state | Required |
| `hop.secret` | Master Key used to encrypt credentials | Required; losing it makes stored credentials unusable |
| `hop_host_key` | SSH gateway identity | Required to avoid client host-key changes |
| `config.toml` or `config.yaml` | Startup-only settings | Required |
| API token file | Optional Control API credential | Required only when API is enabled |
| manifest, public-key, and secret files | Declarative source material | Required when used |

The startup file owns listeners, paths, timeouts, watcher policy, and runtime settings. SQLite is the only runtime truth for credentials, assets, Access Keys, and allowlists. Manifests, the local CLI, and the API only submit Catalog changes.

## Linux binary

Build from source:

```bash
cargo build --release --locked -p hop-server
sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
sudo install -d -o hop -g hop -m 0700 /var/lib/hop
sudo install -d -o root -g hop -m 0750 /etc/hop
sudo install -m 0640 -o root -g hop config.example.toml /etc/hop/config.toml
```

Use absolute paths in `/etc/hop/config.toml`:

```toml
[server]
ssh_listen = "0.0.0.0:2222"

[database]
path = "/var/lib/hop/hop.db"

[api]
enabled = false
listen = "127.0.0.1:8083"
token_file = "/var/lib/hop/control-api.token"
cors_allowlist = []

[ssh]
host_key_file = "/var/lib/hop/hop_host_key"
host_key_type = "ed25519"
banner = "Welcome to Hop"
keepalive_interval = 30
connect_timeout = 10
proxy_policy = "assets_only"

[security]
master_key_file = "/var/lib/hop/hop.secret"

[inventory]
sources = []

[runtime]
temp_dir = "/tmp/hop"
log_level = "info"
session_retention_days = 30
```

Listener addresses and non-loopback API/CORS policy are validated before the
Catalog is opened. On `serve`, Hop creates `runtime.temp_dir`, uses
`runtime.log_level` unless `RUST_LOG` overrides it, and removes ended session
records older than `session_retention_days`; setting the retention to `0`
disables automatic cleanup. Cleanup runs at startup rather than through an idle
write loop.

The sample [systemd unit](../systemd/hop.service) runs the process as `hop`. Start it only after the paths and ownership are correct:

```bash
sudo install -m 0644 systemd/hop.service /etc/systemd/system/hop.service
sudo systemctl daemon-reload
sudo systemctl enable --now hop
sudo journalctl -u hop -f
```

First startup creates a new v0.2 database, Master Key, and SSH Host Key. It does not create an administrator password or an HTTP listener.

## Initial resources with the local CLI

Run CLI writes as the same service user so the database and key permissions remain consistent:

```bash
sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml key add \
  --name laptop --public-key-file /etc/hop/laptop.pub

printf '%s' 'target-password' | sudo -u hop \
  /usr/local/bin/hop-server --config /etc/hop/config.toml credential add \
  --name server-root --username root --auth-type password --password-stdin

sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml \
  credential list
sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml \
  asset add --name server --hostname 192.0.2.10 --port 22 \
  --credential-id <credential-id>
```

New local keys default to all assets. Restrict one key by internal asset IDs:

```bash
hop-server --config /etc/hop/config.toml key access set <key-id> \
  --mode restricted --asset-id <asset-id>
hop-server --config /etc/hop/config.toml key access set <key-id> \
  --mode restricted
hop-server --config /etc/hop/config.toml key access set <key-id> --mode all
```

The second command creates an empty allowlist; the third restores all current and future assets. Changes affect new connections and do not terminate existing sessions.

## Declarative source and watcher

Validate and preview a complete source scope before applying it:

```bash
hop-server config validate -f '/etc/hop/resources.d/*.yaml' --offline --json
hop-server --config /etc/hop/config.toml config diff \
  -f '/etc/hop/resources.d/*.yaml' --source home --json
hop-server --config /etc/hop/config.toml apply \
  -f '/etc/hop/resources.d/*.yaml' --source home --base-revision <revision> --json
```

Quotes prevent the shell from expanding the glob so Hop can detect a temporarily empty or incomplete scope.

Enable startup loading and stable-snapshot watching with:

```toml
[[inventory.sources]]
id = "home"
path = "/etc/hop/resources.d/*.yaml"
watch = true
prune = false
```

Every configured source is applied once at startup. A watched source is rescanned only after its path/size/mtime snapshot remains stable across two polling intervals. Failed scans and parses record a non-sensitive error in `config status` and retain the previous Catalog. `prune` is false unless explicitly enabled for that source.

Local CRUD cannot edit a declarative resource. The API returns `409 managed_by_source`; the CLI returns the same ownership boundary as a validation error. Hop does not implement field-level or last-file-wins ownership.

## Optional Control API

Create a high-entropy token file before enabling the API:

```bash
umask 077
openssl rand -hex 32 | sudo tee /var/lib/hop/control-api.token >/dev/null
sudo chown hop:hop /var/lib/hop/control-api.token
```

Then set `api.enabled = true` and restart. Keep the default loopback listener behind a local agent or authenticated TLS reverse proxy. If `api.listen` is not loopback, Hop requires a non-empty `cors_allowlist`; CORS is not a replacement for TLS or network access control.

```bash
token=$(sudo cat /var/lib/hop/control-api.token)
curl -H "Authorization: Bearer $token" http://127.0.0.1:8083/api/v1/status
curl -H "Authorization: Bearer $token" http://127.0.0.1:8083/api/v1/config/status
```

The API uses one equal-privilege management token. It has no administrator accounts, login form, role, capability, Cookie, or CSRF protocol. Credential reads return secret status only.

## Docker

The image stores mutable files under `/data`; the bundled config keeps the API off:

```bash
docker build -t hop:0.2.0 .
mkdir -p data
docker run -d --name hop --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/data:/data" \
  hop:0.2.0

docker exec hop hop-server --config /data/config.toml key list
```

Do not publish port 8083 unless the API is intentionally enabled and protected. Docker is an optional distribution path, not a runtime dependency.

## Backup

The simplest consistent backup stops writes, copies the complete trust material, and restarts:

```bash
sudo systemctl stop hop
sudo install -d -m 0700 /srv/backups/hop-$(date +%F)
sudo cp -a /var/lib/hop/hop.db /var/lib/hop/hop.secret \
  /var/lib/hop/hop_host_key /srv/backups/hop-$(date +%F)/
sudo cp -a /etc/hop /srv/backups/hop-$(date +%F)/etc-hop
sudo systemctl start hop
```

If the API token, manifest secrets, or public-key files live elsewhere, include them. Do not back up only `hop.db`: encrypted credentials require the matching Master Key.

## Restore and rebuild

For an exact restore, stop Hop and restore the database, Master Key, Host Key, startup config, and source files with their original service ownership. Start Hop and verify:

```bash
sudo -u hop hop-server --config /etc/hop/config.toml config status --json
sudo -u hop hop-server --config /etc/hop/config.toml asset list
sudo -u hop hop-server --config /etc/hop/config.toml key list
```

For a clean rebuild, keep the source manifests and secret files, choose an empty database path, then run `apply`. Runtime sessions, health, audit history, and TOFU Known Hosts are intentionally not reconstructed from manifests.

## OpenWrt

The separate `luci-app-hop` repository provides an architecture-independent LuCI/procd control package. It does not compile or embed Rust. On the first enabled start it downloads the matching static `hop-server` release archive, verifies it against `SHA256SUMS`, self-checks it, and installs it atomically. UCI does not own Catalog resources. See [OpenWrt distribution and footprint](openwrt.md).

## Troubleshooting

| Symptom | Check |
|---|---|
| `Permission denied (publickey)` | The ingress public key fingerprint is active in `key list` |
| Asset missing from TUI | Inspect `key access show <key-id>` and `config status` orphans |
| Direct/exec/SFTP rejected | The key must reach the asset and an SSH asset must reference a credential |
| Proxy target rejected | The target must resolve to an authorized Catalog asset by name or host/port |
| `managed_by_source` | Modify the owning manifest source instead of local CRUD |
| `revision_conflict` | Fetch the current revision, recompute diff, and retry intentionally |
| Watcher did not delete | Default is orphan; use explicit absent or opt-in prune |
| API port absent | Expected while `api.enabled = false` |
| Database decrypt failures | Restore the Master Key paired with that database |
