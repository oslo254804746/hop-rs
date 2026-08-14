# Hop

Hop v0.2 is a lightweight SSH jump server for individual developers, homelabs, and small teams sharing one management trust boundary. It works with native OpenSSH clients and covers TUI discovery, direct asset login, remote commands, SCP/SFTP, ProxyJump, and generic TCP forwarding without requiring a browser, custom client, external database, or container runtime.

[中文](README.md)

## v0.2 boundaries

- One Rust binary and one SQLite Catalog.
- Multiple ingress Access Keys that can be enabled, disabled, or deleted independently.
- An omitted `assets` field grants a key all current and future assets; `assets: []` authenticates but discovers and reaches none; a non-empty list is a strict allowlist.
- YAML/TOML, the local CLI, and the optional Control API all write through the same Catalog.
- The HTTP Control API is disabled by default. Core contains no Admin Web, administrator accounts, cookies, CSRF, roles, or RBAC.
- OpenWrt is a first-class distribution target: a lightweight LuCI/procd package downloads the verified static core for the router architecture instead of compiling or embedding Rust in the IPK/APK.

v0.2 is a clean break and does not migrate or read v0.1 data. Hop detects an old database before opening it for writes, refuses startup with backup/delete/new-path guidance, and leaves its bytes and mtime unchanged.

## Quick start

```bash
cargo build --release --locked -p hop-server
cp config.example.toml config.toml

./target/release/hop-server --config config.toml key add \
  --name laptop --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'target-password' | \
  ./target/release/hop-server --config config.toml credential add \
  --name homelab-root --username root --auth-type password --password-stdin

credential_id=$(./target/release/hop-server --config config.toml credential list | awk 'NR == 1 { print $1 }')
./target/release/hop-server --config config.toml asset add \
  --name nas --hostname 192.168.1.20 --port 22 --credential-id "$credential_id"

./target/release/hop-server --config config.toml serve
```

SSH listens on `0.0.0.0:2222` by default. No HTTP listener is created.

## Native SSH workflows

```bash
ssh -p 2222 menu@hop-host
ssh -p 2222 nas@hop-host
ssh -p 2222 nas@hop-host 'uname -a'
scp -P 2222 file.txt nas@hop-host:/tmp/file.txt
sftp -P 2222 nas@hop-host
ssh -p 2222 -L 13389:desktop.hop:3389 menu@hop-host
```

ProxyJump example:

```sshconfig
Host hop
  HostName hop-host
  Port 2222
  IdentityFile ~/.ssh/id_ed25519

Host *.hop
  ProxyJump hop
```

TUI discovery, direct login, managed interactive sessions, exec, SCP/SFTP, ProxyJump, and TCP forwarding all use the same Key-to-Asset authorization queries. Crafted targets cannot bypass an allowlist. Ordinary allowlist changes affect new connections; the Control API exposes explicit active-session termination for emergencies.

## Declarative resources

Manifests are strict YAML or TOML. Unknown fields and duplicate resources are rejected. Secrets use `file` or `env` sources and are encrypted with the Master Key before entering SQLite.

```yaml
api_version: hop/v1alpha1

credentials:
  homelab:
    type: password
    username: root
    password: { file: /etc/hop/secrets/homelab.password }

assets:
  nas:
    type: ssh
    host: 192.168.1.20
    port: 22
    credential: homelab
  desktop:
    type: tcp
    host: 192.168.1.30
    port: 3389
    preset: rdp

access:
  laptop:
    public_key: { file: /etc/hop/keys/laptop.pub }
    assets: [nas, desktop]
```

```bash
hop-server config validate -f resources.yaml --offline --json
hop-server --config config.toml config validate -f resources.yaml --json
hop-server --config config.toml config diff -f resources.yaml --source home --json
hop-server --config config.toml apply -f resources.yaml \
  --source home --base-revision 0 --json
hop-server --config config.toml apply -f resources.yaml --source home --dry-run
hop-server --config config.toml apply -f resources.yaml --source home --prune
hop-server --config config.toml config status --json
```

Applying identical content does not increment the Catalog revision. Resources missing from a successful full-source scan become usable orphans by default; only explicit `state: absent` or prune deletes them. The startup watcher calls the same Apply engine after a stable scope scan, retains the last valid Catalog on errors, and does not prune unless its source explicitly opts in.

## Startup configuration and Control API

`config.example.toml` documents SSH, database, Host Key, Master Key, timeout, keepalive, banner, proxy policy, API, inventory watcher, and runtime settings. YAML accepts the same structure.

Catalog resources are dynamically applied. Listener/database/key-path settings require restart. Credential and allowlist changes affect new connections.

The API requires an explicit token file and is opt-in:

```toml
[api]
enabled = true
listen = "127.0.0.1:8083"
token_file = "/etc/hop/control-api.token"
cors_allowlist = []
```

Every `/api/v1` request uses `Authorization: Bearer <token>`. The API provides status/version, Catalog revision, resource reads and local CRUD, sessions and termination, source/status, validate, diff, apply, and reload. Credential responses expose only `configured`/`missing` states. A non-loopback listener requires an explicit CORS allowlist.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
./scripts/manual_e2e_key_asset_access.sh
./scripts/e2e_local_openssh.sh
```

The isolated OpenSSH E2E covers exec streams and exit status, PTY, SCP, SFTP, ProxyJump, initial Host Key recording and changed-key rejection, and encrypted credential storage.

## Documentation

- [Deployment, backup, rebuild, and recovery](docs/deployment.md)
- [Control API and local management](docs/admin-guide.md)
- [Declarative Apply specification](docs/product/declarative-apply-spec.md)
- [Access Key allowlists](docs/product/lightweight-access-control.md)
- [OpenWrt packaging and measurements](docs/openwrt.md)
- [v0.2 product direction](docs/product/product-direction-v0.2.md)
- [Documentation index](docs/README.md)

## License

MIT
