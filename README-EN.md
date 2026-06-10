<div align="center">

[中文](README.md) | **English**

# 🦀 Hop

**Minimal SSH bastion, maximum control.**

A single Rust binary that replaces your bloated jump server with pubkey auth, a TUI asset picker, managed credentials, and proxy-aware forwarding — all backed by SQLite.

[![CI](https://github.com/oslo254804746/hop-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/oslo254804746/hop-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

---

## Why Hop?

Most bastion/jump-server solutions are bloated Java/Python stacks with databases, caches, message queues, and admin panels that take a week to deploy. Hop is the opposite:

- **Single binary** — `hop-server` does everything
- **Zero external deps** — SQLite bundled, no Redis/Postgres/RabbitMQ
- **Secure by default** — Admin Web on loopback, credentials encrypted with ChaCha20-Poly1305
- **SSH-native** — your users just `ssh`, no proprietary client needed

## Features

```text
┌─────────────────────────────────────────────────────────┐
│  Pubkey Whitelist     Only trusted keys enter Hop       │
│  TUI Asset Picker     Fuzzy search, connect in seconds  │
│  Managed Credentials  Server-side auth to targets       │
│  Generic TCP Forward  RDP/VNC/database SSH tunnels      │
│  SSH/SFTP             Managed access to SSH targets     │
│  Admin Web            Lightweight management UI         │
│  Import/Export        Asset and credential metadata     │
│  TOFU Host Keys       Auto-trust on first connect       │
│  i18n Admin           Multi-language admin interface    │
└─────────────────────────────────────────────────────────┘
```

## Quick Start

```bash
# Build
cargo build --release -p hop-server

# Run
cp config.example.toml config.toml
./target/release/hop-server serve --config config.toml
```

First boot auto-generates:
- SQLite database
- Ed25519 host key
- ChaCha20-Poly1305 master key (`hop.secret`)
- One-time admin password (printed to stdout)

Default ports: **SSH `0.0.0.0:2222`** | **Admin Web `127.0.0.1:8080`**

## First Setup

In another terminal, add your SSH public key to the Hop whitelist, then create a managed credential and asset:

```bash
./target/release/hop-server --config config.toml key add \
  --name "alice laptop" \
  --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'target-password' | ./target/release/hop-server --config config.toml credential add \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin

./target/release/hop-server --config config.toml asset add \
  --name web-prod-01 \
  --hostname 10.0.1.10 \
  --port 22 \
  --tags prod,web \
  --credential-id <credential-id>
```

`credential add` prints the credential ID for `--credential-id`. Assets without managed credentials can still be used for ProxyJump forwarding.

## Usage

```bash
# Interactive TUI — fuzzy search your fleet
ssh -p 2222 hop-host

# Direct connect — asset name as SSH username
ssh -p 2222 web-prod-01@hop-host

# SFTP — reuse the SSH asset and its managed credential
sftp -P 2222 web-prod-01@hop-host
scp -P 2222 ./file web-prod-01@hop-host:/tmp/file

# ProxyJump — Hop as a transparent TCP relay
ssh -J hop-host:2222 web-prod-01.hop

# RDP — create a protocol=RDP, port=3389 asset in Admin Web, then copy the tunnel command
ssh -p 2222 -N -T -L 127.0.0.1:13389:win-prod-rdp.hop:3389 hop-host
mstsc /v:127.0.0.1:13389

# VNC and MySQL use the same generic TCP forwarding path
ssh -p 2222 -N -T -L 127.0.0.1:15900:vnc-prod.hop:5900 hop-host
ssh -p 2222 -N -T -L 127.0.0.1:13306:mysql-prod.hop:3306 hop-host
```

Interactive TUI, direct connect, and SFTP use Hop-managed credentials to reach SSH targets. ProxyJump and local forwarding are transparent, asset-allowlisted TCP relays. RDP, VNC, MySQL, PostgreSQL, and Redis are presets for ports and client guidance; the core does not parse those application protocols. Generic forwarding supports TCP only, not UDP or dynamic multi-port protocols.

Each active Hop SSH key has its own asset access mode:

- `all`: access every current and future asset. New keys and keys migrated from earlier releases default to this mode, so upgrades do not revoke existing access.
- `restricted`: access only explicitly assigned assets. An empty assignment allows authentication to Hop but exposes and reaches no assets.

The entry key controls which assets may be reached; the credential attached to an asset controls how Hop authenticates to that target. TUI, direct SSH, SFTP, ProxyJump, and local TCP forwarding all enforce the same policy by stable key and asset IDs. Assign assets from the Admin Web key edit page or use the CLI:

```bash
hop-server key access show <key-id>
hop-server key access set <key-id> --mode restricted --asset-id <asset-id>
hop-server key access set <key-id> --mode restricted  # Clear access
hop-server key access set <key-id> --mode all         # Include future assets
```

## Architecture

```text
crates/
├── hop-core/       Config, models, SQLite, credential encryption
└── hop-server/     SSH server, TUI, Admin Web, local CLI
migrations/         SQLite schema migrations
systemd/            Production service unit
```

**Stack:** `russh` · `ratatui` · `axum` · `sqlx` · `chacha20poly1305` · `maud`

## CLI Reference

```bash
hop-server serve                    # Start the server (default)
hop-server reset-admin              # Reset admin password
hop-server key add|list|activate|deactivate
hop-server key access show|set
hop-server credential add|list|delete
hop-server asset add|list|delete       # add supports ssh|tcp and common TCP presets
hop-server export --kind assets --format csv --output dump.csv
hop-server import --file dump.csv --on-conflict skip
```

Credential import/export transfers metadata only, such as `name`, `username`, and `auth_type`; passwords and private keys are never exported.
Per-key assignments are stored in the `hop.db` relation table and are intentionally excluded from asset and credential transfer formats. Backing up `hop.db` includes these authorization settings.
Change the admin password from Admin Web Settings; use `hop-server reset-admin` for recovery if you forget it.

## Docker

```bash
# Linux (recommended): host network preserves loopback binding
docker run -d --name hop --network host \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:vX.Y.Z

# Docker Desktop: first set data/config.toml admin_bind to "0.0.0.0:8080"
docker run -d --name hop \
  -p 2222:2222 -p 127.0.0.1:8080:8080 \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:vX.Y.Z
```

Initial admin password: `docker logs hop`

## Deployment

Full deployment guide (binary, systemd, Docker, upgrades, backup, troubleshooting):

**→ [docs/deployment.md](docs/deployment.md)**

## Security Model

| Layer | Mechanism |
|-------|-----------|
| Hop entry auth | SSH public key whitelist only |
| Credential storage | ChaCha20-Poly1305 + HKDF-SHA256 |
| Admin Web auth | Argon2 password hash |
| ProxyJump targets | Asset allowlist enforcement |
| Admin Web exposure | Loopback-only by default |

> **`hop.secret` is your crown jewel.** Lose it and all stored credentials become unrecoverable. Back it up.

## Backup

Three files, one atomic snapshot:

```bash
hop.db          # Everything: assets, keys, sessions, encrypted creds
hop.secret      # Master key — unrecoverable if lost
hop_host_key    # SSH host identity
```

## License

MIT
