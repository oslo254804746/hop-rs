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
│  ProxyJump/ProxyCmd   Allowlist-restricted TCP forward  │
│  Admin Web            Lightweight management UI         │
│  Import/Export        Bulk asset/credential transfer    │
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

## Usage

```bash
# Interactive TUI — fuzzy search your fleet
ssh -p 2222 hop-host

# Direct connect — asset name as SSH username
ssh -p 2222 web-prod-01@hop-host

# ProxyJump — Hop as a transparent TCP relay
ssh -J hop-host:2222 web-prod-01.hop
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
hop-server credential add|list|delete
hop-server asset add|list|delete
hop-server export --kind assets --format csv --output dump.csv
hop-server import --file dump.csv --on-conflict skip
```

## Docker

```bash
# Linux (recommended): host network preserves loopback binding
docker run -d --name hop --network host \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:latest

# Docker Desktop: bridge with loopback-only admin port
docker run -d --name hop \
  -p 2222:2222 -p 127.0.0.1:8080:8080 \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:latest
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
