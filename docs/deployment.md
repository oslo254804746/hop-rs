# Deployment

Production deployment guide for Hop. Covers binary, systemd, Docker, upgrades, backup, and troubleshooting.

> **Target platform:** Linux (amd64). Build on Windows/macOS for dev; deploy on Linux, WSL, or Docker.

---

## Ports & Data Files

| File | Purpose | Loss impact |
|------|---------|-------------|
| `hop.db` | Assets, keys, sessions, encrypted credentials | Data loss |
| `hop.secret` | ChaCha20-Poly1305 master key | **All credentials unrecoverable** |
| `hop_host_key` | Ed25519 SSH host identity | Client host-key warnings |
| `config.toml` | Listener, path, cookie, timeout, and proxy settings | Service may start with unsafe or incorrect defaults |

Default network:

```toml
[server]
ssh_bind   = "0.0.0.0:2222"
admin_bind = "127.0.0.1:8080"   # Loopback only — see "Admin Web Access"
```

---

## Binary + systemd

### 1. Get the binary

```bash
# Download a release binary
HOP_VERSION=vX.Y.Z
curl -fL -o hop-server \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-server-linux-amd64"
curl -fL -o hop-admin-web-static.tar.gz \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-admin-web-static.tar.gz"
chmod 0755 hop-server

# Or build from source
cargo build --release -p hop-server
cp target/release/hop-server ./hop-server
(cd web/admin && npm ci && npm run build)
tar -C web/admin/dist -czf hop-admin-web-static.tar.gz .
```

### 2. Install

```bash
sudo useradd --system --home-dir /var/lib/hop --shell /usr/sbin/nologin hop
sudo install -d -o hop -g hop -m 0750 /var/lib/hop
sudo install -d -m 0755 /etc/hop
sudo install -d -m 0755 /usr/share/hop/admin-web

sudo install -m 0755 hop-server /usr/local/bin/hop-server
sudo install -m 0644 config.example.toml /etc/hop/config.toml
sudo tar -C /usr/share/hop/admin-web -xzf hop-admin-web-static.tar.gz
```

### 3. Enable service

```bash
sudo install -m 0644 systemd/hop.service /etc/systemd/system/hop.service
sudo systemctl daemon-reload
sudo systemctl enable --now hop
```

First boot prints the admin password:

```bash
sudo journalctl -u hop -n 50 --no-pager
```

After signing in, change the admin password from Admin Web -> Settings. If it is forgotten, run `hop-server reset-admin` to generate a new random password.

Continue with "Initial Data Setup" before the first SSH verification.

---

## Docker

### Linux: host network (recommended)

Host network lets the container bind `127.0.0.1:8080` directly on the host — no exposure gymnastics needed.

```bash
HOP_VERSION=vX.Y.Z
docker run -d \
  --name hop \
  --restart unless-stopped \
  --network host \
  -v "$PWD/data:/data" \
  -e RUST_LOG=info \
  ghcr.io/oslo254804746/hop-rs:${HOP_VERSION}
```

### Docker Desktop (macOS/Windows): bridge network

`--network host` on Docker Desktop doesn't give real host access. Use bridge with explicit loopback mapping:

1. First run to generate config:
   ```bash
   mkdir -p data
   HOP_VERSION=vX.Y.Z
   docker run --rm -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:${HOP_VERSION} hop-server --help >/dev/null
   ```

2. Edit `data/config.toml`:
   ```toml
   [server]
   admin_bind = "0.0.0.0:8080"
   ```

3. Start with loopback-only publish:
   ```bash
   HOP_VERSION=vX.Y.Z
   docker run -d \
     --name hop \
     --restart unless-stopped \
     -p 2222:2222 \
     -p 127.0.0.1:8080:8080 \
     -v "$PWD/data:/data" \
     -e RUST_LOG=info \
     ghcr.io/oslo254804746/hop-rs:${HOP_VERSION}
   ```

The security boundary is `-p 127.0.0.1:8080:8080` — never use `-p 8080:8080`.

### Docker CLI management

```bash
docker exec hop hop-server --config /data/config.toml key list
docker exec hop hop-server --config /data/config.toml asset list
docker exec hop hop-server --config /data/config.toml credential list
docker exec hop hop-server --config /data/config.toml reset-admin

# stdin for secrets
printf '%s' 'secret' | docker exec -i hop hop-server --config /data/config.toml \
  credential add --name deploy --username deploy --auth-type password --password-stdin
```

Use Admin Web -> Settings to change the password after the first login; keep `reset-admin` for password recovery.

---

## Admin Web Access

Admin Web binds to `127.0.0.1:8080` by default. Remote access options:

```bash
# SSH tunnel (recommended)
ssh -N -L 8080:127.0.0.1:8080 root@hop-host
# Then open http://127.0.0.1:8080 locally
```

If you change `admin_bind` to `0.0.0.0:8080`, the server logs a warning. Protect with firewall / VPN / trusted management network.

For Dashboard, assets, credentials, SSH access, Known Hosts, audit evidence,
and progressive team administration, see the [Admin Web guide](admin-guide.md).

---

## Initial Data Setup

Run this before the first SSH login. Hop entry auth and target auth are separate:

Two types of auth data — don't confuse them:

| Command | Controls | Used for |
|---------|----------|----------|
| `key add` | Who can SSH into Hop on `:2222` | Hop entry (pubkey whitelist) |
| `credential add` | How Hop connects to targets | Managed connections (TUI/direct) |

```bash
hop-server --config /etc/hop/config.toml key add \
  --name "alice laptop" \
  --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'secret' | hop-server --config /etc/hop/config.toml credential add \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin

hop-server --config /etc/hop/config.toml asset add \
  --name web-prod-01 \
  --hostname 10.0.1.10 \
  --port 22 \
  --tags prod,web \
  --credential-id <id>
```

`credential add` prints the credential ID for `--credential-id`. Assets without a credential are proxy-only: they can be reached through ProxyJump, but TUI and direct-connect mode require a managed credential.

Verify SSH after adding data:

```bash
ssh -p 2222 hop-host             # TUI
ssh -p 2222 web-prod-01@hop-host # Direct connect with managed credential
sftp -P 2222 web-prod-01@hop-host # Managed SFTP subsystem
ssh -J hop-host:2222 web-prod-01.hop # ProxyJump TCP relay
```

Each active Hop SSH key uses one of two asset access modes. `all` includes every
current and future asset. `restricted` includes only explicitly assigned asset
IDs; an empty assignment grants no asset access. Existing keys migrate to `all`,
so an upgrade preserves their current behavior.

Use Admin Web → Keys → Edit to choose a mode and filter/check assets, or use:

```bash
hop-server key access show <key-id>
hop-server key access set <key-id> --mode restricted \
  --asset-id <asset-id> --asset-id <asset-id>
hop-server key access set <key-id> --mode restricted # Revoke all asset access
hop-server key access set <key-id> --mode all        # Include future assets
```

Entry-key authorization and target credentials are separate. The key grants
access to an asset; the asset's credential authenticates managed SSH and SFTP
connections to the target. TUI, direct SSH, SFTP, ProxyJump, and generic
`direct-tcpip` forwarding enforce the same per-key policy. RDP, VNC, MySQL,
PostgreSQL, Redis, and Generic TCP presets only provide port defaults and
examples.

---

## Bulk Import/Export

```bash
# Export assets to CSV
hop-server export --kind assets --format csv --output assets.csv

# Import with conflict handling
hop-server import --file assets.csv --on-conflict skip|overwrite|error

# Also works for credentials
hop-server export --kind credentials --format json --output creds.json
hop-server import --kind credentials --file creds.json
```

Credential import/export is metadata-only. It transfers `name`, `username`, and `auth_type`, but never exports passwords, private keys, or passphrases.
Key-to-asset assignments are not part of these transfer formats. They are stored
in `hop.db`, so the normal database backup includes them without an extra file.

---

## Upgrade

Hop applies pending SQLite migrations during startup. Treat an upgrade as a
data migration, not only a binary or image replacement.

If Hop is the only path to the machines behind it, do not perform a remote
upgrade until you have an alternate management path such as a host-level SSH
route, VPN, second jump host, or local console.

Before every upgrade:

1. Download or pull the new version before the maintenance window.
2. Keep the previous binary or container image.
3. Stop Hop before copying SQLite data.
4. Back up the complete persistent data directory and configuration.
5. Record `asset list`, `key list`, and `credential list` for comparison.
6. After startup, verify Admin login and one real end-to-end SSH path.

### v0.1.4 to v0.1.5 compatibility

v0.1.5 adds migrations 005 and 006. They preserve existing assets,
credentials, authorized SSH keys, key-to-asset assignments, Known Hosts,
sessions, and the local admin password. The existing local administrator
becomes an Owner and the one-admin login remains password-only.

After v0.1.5 migrates `hop.db`, v0.1.4 rejects that database because it does
not know migrations 005 and 006. A rollback must restore the pre-upgrade data
backup as well as the v0.1.4 binary or image. Changing only the version tag is
not a valid rollback.

### Binary

Download and verify both release artifacts before stopping the service:

```bash
HOP_VERSION=v0.1.5
BACKUP_DIR="$PWD/hop-backup-before-${HOP_VERSION}-$(date +%Y%m%d%H%M%S)"
mkdir -m 0700 "$BACKUP_DIR"

curl -fLO \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-server-linux-amd64"
curl -fLO \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-server-linux-amd64.sha256"
curl -fLO \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-admin-web-static.tar.gz"
curl -fLO \
  "https://github.com/oslo254804746/hop-rs/releases/download/${HOP_VERSION}/hop-admin-web-static.tar.gz.sha256"

sha256sum -c hop-server-linux-amd64.sha256
sha256sum -c hop-admin-web-static.tar.gz.sha256

sudo systemctl stop hop
sudo tar -C /var/lib -czf "$BACKUP_DIR/hop-data.tgz" hop
sudo tar -C /etc -czf "$BACKUP_DIR/hop-config.tgz" hop
sudo tar -C /usr/share -czf "$BACKUP_DIR/hop-admin-web.tgz" hop/admin-web
sudo cp -a /usr/local/bin/hop-server "$BACKUP_DIR/hop-server.previous"

sudo install -m 0755 hop-server-linux-amd64 /usr/local/bin/hop-server
sudo tar -C /usr/share/hop/admin-web -xzf hop-admin-web-static.tar.gz
sudo systemctl start hop

sudo journalctl -u hop -n 50 --no-pager
sudo -u hop sh -c 'cd /var/lib/hop && /usr/local/bin/hop-server --config /etc/hop/config.toml asset list'
sudo -u hop sh -c 'cd /var/lib/hop && /usr/local/bin/hop-server --config /etc/hop/config.toml key list'
sudo -u hop sh -c 'cd /var/lib/hop && /usr/local/bin/hop-server --config /etc/hop/config.toml credential list'
```

If verification fails, restore the stopped-service snapshot instead of opening
the migrated database with the old binary:

```bash
FAILED_SUFFIX="$(date +%Y%m%d%H%M%S)"
sudo systemctl stop hop
sudo mv /var/lib/hop "/var/lib/hop.failed-${FAILED_SUFFIX}"
sudo mv /etc/hop "/etc/hop.failed-${FAILED_SUFFIX}"
sudo mv /usr/share/hop/admin-web "/usr/share/hop/admin-web.failed-${FAILED_SUFFIX}"
sudo tar -C /var/lib -xzf "$BACKUP_DIR/hop-data.tgz"
sudo tar -C /etc -xzf "$BACKUP_DIR/hop-config.tgz"
sudo tar -C /usr/share -xzf "$BACKUP_DIR/hop-admin-web.tgz"
sudo install -m 0755 "$BACKUP_DIR/hop-server.previous" /usr/local/bin/hop-server
sudo systemctl start hop
sudo journalctl -u hop -n 50 --no-pager
```

### Docker

Pull first, then stop the container before archiving the bind-mounted data:

```bash
HOP_VERSION=v0.1.5
BACKUP_DIR="$PWD/hop-backup-before-${HOP_VERSION}-$(date +%Y%m%d%H%M%S)"
mkdir -m 0700 "$BACKUP_DIR"

docker pull "ghcr.io/oslo254804746/hop-rs:${HOP_VERSION}"
docker inspect hop > "$BACKUP_DIR/hop-container.json"
docker stop hop
tar -C "$PWD" -czf "$BACKUP_DIR/hop-data.tgz" data
docker image inspect ghcr.io/oslo254804746/hop-rs:v0.1.4 \
  > "$BACKUP_DIR/hop-image-v0.1.4.json"
docker rm hop

# Re-run the original docker run command with only the image tag changed:
# ghcr.io/oslo254804746/hop-rs:v0.1.5

docker logs --tail 100 hop
docker exec hop hop-server --config /data/config.toml asset list
docker exec hop hop-server --config /data/config.toml key list
docker exec hop hop-server --config /data/config.toml credential list
```

To roll back:

```bash
FAILED_SUFFIX="$(date +%Y%m%d%H%M%S)"
docker stop hop
docker rm hop
mv data "data.failed-${FAILED_SUFFIX}"
tar -C "$PWD" -xzf "$BACKUP_DIR/hop-data.tgz"

# Re-run the original command with:
# ghcr.io/oslo254804746/hop-rs:v0.1.4
```

For Docker Compose, keep the same sequence: pull the pinned image, stop the Hop
service, back up the bind mount or named volume, change the pinned tag, then
run `docker compose up -d`. Restore the pre-upgrade volume before selecting
v0.1.4 during rollback.

---

## Backup & Restore

Stop Hop before copying `hop.db`; this avoids a snapshot split across the
SQLite database, WAL, and shared-memory files. Back up the whole data directory
instead of selecting only the three known filenames so future migrations and
support files are included automatically.

```bash
# Binary deployment
BACKUP_DIR="$PWD/hop-backup-$(date +%Y%m%d%H%M%S)"
mkdir -m 0700 "$BACKUP_DIR"
sudo systemctl stop hop
sudo tar -C /var/lib -czf "$BACKUP_DIR/hop-data.tgz" hop
sudo tar -C /etc -czf "$BACKUP_DIR/hop-config.tgz" hop
sudo systemctl start hop

# Docker
BACKUP_DIR="$PWD/hop-backup-$(date +%Y%m%d%H%M%S)"
mkdir -m 0700 "$BACKUP_DIR"
docker stop hop
tar -C "$PWD" -czf "$BACKUP_DIR/hop-data.tgz" data
docker start hop
```

Restore in the reverse order: stop Hop, move the current data directory aside,
extract the backup into its original parent directory, restore config when
needed, and start Hop. Moving the failed directory aside preserves evidence and
avoids mixing a restored database with stale WAL files.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `Permission denied (publickey)` | User pubkey not in whitelist | `hop-server key add` then verify `key list` shows active |
| Target auth failure | Wrong credential bound to asset | Check `credential list`, verify username/password/key |
| Host key mismatch | Target host reinstalled | Admin Web → Known Hosts → delete stale target entry, then reconnect |
| Admin Web unreachable remotely | Loopback binding (by design) | Use SSH tunnel or adjust `admin_bind` with protection |
| Docker Desktop can't reach Admin | `--network host` doesn't work on Desktop | Use bridge, set `admin_bind = "0.0.0.0:8080"`, and publish `-p 127.0.0.1:8080:8080` |
| `DB locked` | Another process holds SQLite | Check for duplicate `hop-server` instances |

---

## systemd Unit Reference

```ini
[Service]
Type=simple
User=hop
WorkingDirectory=/var/lib/hop
ExecStart=/usr/local/bin/hop-server serve --config /etc/hop/config.toml
Restart=on-failure
Environment=RUST_LOG=info

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true
ReadWritePaths=/var/lib/hop
```

Full unit file: [`systemd/hop.service`](../systemd/hop.service)
