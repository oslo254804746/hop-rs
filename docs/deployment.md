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

Default network:

```toml
[server]
ssh_bind   = "0.0.0.0:2222"
admin_bind = "127.0.0.1:8080"   # Loopback only — see "Admin Web Access"
```

---

## Binary + systemd

### 1. Build

```bash
cargo build --release -p hop-server
```

### 2. Install

```bash
sudo useradd --system --home-dir /var/lib/hop --shell /usr/sbin/nologin hop
sudo install -d -o hop -g hop -m 0750 /var/lib/hop
sudo install -d -m 0755 /etc/hop

sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
sudo install -m 0644 config.example.toml /etc/hop/config.toml
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

### 4. Verify SSH

```bash
ssh -p 2222 hop-host          # TUI (after adding your pubkey)
ssh -p 2222 web-01@hop-host   # Direct connect
ssh -J hop-host:2222 web-01.hop  # ProxyJump
```

---

## Docker

### Linux: host network (recommended)

Host network lets the container bind `127.0.0.1:8080` directly on the host — no exposure gymnastics needed.

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  --network host \
  -v "$PWD/data:/data" \
  -e RUST_LOG=info \
  ghcr.io/oslo254804746/hop-rs:latest
```

### Docker Desktop (macOS/Windows): bridge network

`--network host` on Docker Desktop doesn't give real host access. Use bridge with explicit loopback mapping:

1. First run to generate config:
   ```bash
   mkdir -p data
   docker run --rm -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:latest hop-server --help >/dev/null
   ```

2. Edit `data/config.toml`:
   ```toml
   [server]
   admin_bind = "0.0.0.0:8080"
   ```

3. Start with loopback-only publish:
   ```bash
   docker run -d \
     --name hop \
     --restart unless-stopped \
     -p 2222:2222 \
     -p 127.0.0.1:8080:8080 \
     -v "$PWD/data:/data" \
     -e RUST_LOG=info \
     ghcr.io/oslo254804746/hop-rs:latest
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

---

## Admin Web Access

Admin Web binds to `127.0.0.1:8080` by default. Remote access options:

```bash
# SSH tunnel (recommended)
ssh -N -L 8080:127.0.0.1:8080 root@hop-host
# Then open http://127.0.0.1:8080 locally
```

If you change `admin_bind` to `0.0.0.0:8080`, the server logs a warning. Protect with firewall / VPN / trusted management network.

---

## Initial Data Setup

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

---

## Upgrade

### Binary

```bash
sudo systemctl stop hop
sudo tar -C /var/lib -czf "hop-backup-$(date +%Y%m%d%H%M%S).tgz" hop
sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
sudo systemctl start hop
sudo journalctl -u hop -n 50 --no-pager
```

### Docker

```bash
tar -czf "hop-data-$(date +%Y%m%d%H%M%S).tgz" data
docker stop hop && docker rm hop
# Re-run docker run with new image tag
```

---

## Backup & Restore

```bash
# Binary deployment
sudo systemctl stop hop
sudo tar -C /var/lib -czf "hop-backup-$(date +%Y%m%d%H%M%S).tgz" hop
sudo systemctl start hop

# Docker
docker stop hop
tar -czf "hop-data-$(date +%Y%m%d%H%M%S).tgz" data
docker start hop
```

Restore: stop service → extract backup to data dir → start service.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `Permission denied (publickey)` | User pubkey not in whitelist | `hop-server key add` then verify `key list` shows active |
| Target auth failure | Wrong credential bound to asset | Check `credential list`, verify username/password/key |
| Host key mismatch | Target host reinstalled | Clear stale entry from Hop's known hosts |
| Admin Web unreachable remotely | Loopback binding (by design) | Use SSH tunnel or adjust `admin_bind` with protection |
| Docker Desktop can't reach Admin | `--network host` doesn't work on Desktop | Use bridge + `-p 127.0.0.1:8080:8080` |
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
