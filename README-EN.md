# Hop

Hop is a lightweight, self-hosted SSH jump server. Ingress accepts registered public keys only, target credentials are encrypted at rest, and an official web panel is included.

## Quick start: web panel (recommended)

Place `hop-rs` and `hop-rs-frontend` next to each other, then run from the backend repository:

```bash
cp hop.yaml hop.local.yaml
sed -i 's/token: change-me/token: replace-with-a-long-random-value/' hop.local.yaml
chmod 0600 hop.local.yaml
HOP_CONFIG_FILE=./hop.local.yaml docker compose up -d --build
```

Open:

- Panel: `http://localhost:8080`
- SSH: `localhost:2222`

On first load, enter only the webpage management Token from `hop.local.yaml`. The panel uses same-origin `/api/v1`; the backend management port is not published to the host. The initial Catalog is empty, so add an ingress public key, target credential, and asset in the panel.

The checked-in `compose.yaml` mounts `./hop.yaml` by default. Set `HOP_CONFIG_FILE` for the local copy shown above, or safely edit and protect `hop.yaml` itself.

## One YAML, no panel

Copy the complete example and fill in your public key and target:

```bash
cp config.example.yaml hop.local.yaml
chmod 0600 hop.local.yaml
cargo run --release -p hop-server -- --config ./hop.local.yaml config validate
cargo run --release -p hop-server -- --config ./hop.local.yaml serve
```

One `hop.yaml` contains listeners, data directory, webpage management Token, SSH runtime settings, target credentials, assets, and ingress public keys. At startup, Hop applies its declared resources through the internal atomic Apply engine. Any error prevents listeners from starting and never leaves a partial Catalog.

## Minimal configuration

```yaml
listen: 0.0.0.0:2222
data_dir: ./data

api:
  enabled: true
  listen: 127.0.0.1:8083
  token: change-me

credentials:
  nas-root:
    username: root
    password: replace-this-password

assets:
  nas:
    host: 192.168.1.20
    credential: nas-root

access_keys:
  laptop:
    public_key_file: ./laptop.pub
    assets: [nas]
```

`password`, `private_key`, and `passphrase` are direct YAML strings, so protect the file with `chmod 0600` and keep it out of version control. Each ingress key must set exactly one of `public_key` or `public_key_file`. Relative paths resolve from the configuration file directory.

If `api.token` is missing or empty, only the Control API is disabled; SSH continues to run. `change-me` is a first-run placeholder and triggers warnings in both backend and panel. Replace it for every real deployment.

## Resource ownership

- Resources created through the panel or local commands are `local` and remain editable.
- Resources declared in `hop.yaml` are `config`; the panel marks them read-only before rendering actions.
- Restart Hop after changing configuration-managed resources. Startup Apply updates them and removes only resources deleted from the same configuration, preserving local resources.

## Connect

```bash
ssh -p 2222 <asset-name>@<hop-host>
```

Hop ingress advertises `publickey` authentication only. An unregistered key receives `Permission denied (publickey)` without a password prompt.

## Documentation

- [Deployment](docs/deployment.md)
- [Configuration reference (Chinese)](docs/configuration.zh-CN.md)
- [Administrator guide](docs/admin-guide.md)
- [中文 README](README.md)

## Development gates

```bash
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
shellcheck docker-entrypoint.sh scripts/*.sh
docker compose config
```

License: MIT.
