# Hop deployment

## Path 1: official Compose with the web panel (recommended)

### Prepare and start

Use Docker Engine 24+ with Compose v2. Only this repository's `compose.yaml` and `examples/` are required; no frontend checkout is needed.

```bash
cd hop-rs
cp examples/panel-first.yaml hop.yaml
python3 -c 'import secrets; print(secrets.token_urlsafe(32))'
```

Put the generated value in `api.token`, then run:

```bash
chmod 0600 hop.yaml
docker compose config
docker compose pull
docker compose up -d
```

Compose pulls both CI-published images and contains no source `build`. It publishes only `${HOP_SSH_PORT:-2222}` and `${HOP_PANEL_PORT:-8080}`. The Control API must listen on container port 8083, which is visible only inside the Compose network. To publish the panel on host port 8081, set `HOP_PANEL_PORT=8081`; do not change `api.listen`.

An optional GHCR mirror can be selected consistently for both services:

```bash
export HOP_REGISTRY=ghcr.nju.edu.cn
docker compose pull
docker compose up -d
```

Open `http://<hop-host>:8080` and enter the webpage management Token. No API URL is needed. The initial Catalog is empty; add an ingress key, target credential, and asset in the panel. The `change-me` placeholder triggers warnings and must be replaced.

### Persistence and upgrades

- The `hop-data` volume holds the database, encryption key, and SSH host key.
- The YAML file is mounted read-only.
- Hop and panel use the same `HOP_VERSION` image tag.

```bash
HOP_VERSION=v0.2.1 docker compose pull
HOP_VERSION=v0.2.1 docker compose up -d
```

Back up the YAML and data volume before upgrading. Roll back both images together.

## Path 2: one YAML without the panel

### Binary

```bash
cp examples/config-first.yaml hop.yaml
chmod 0600 hop.yaml
hop-server --config ./hop.yaml config validate
hop-server --config ./hop.yaml serve
```

When `api.token` is missing or empty, only the Control API is disabled; SSH still starts.

### Standalone container

```bash
docker run -d --name hop --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/hop.yaml:/etc/hop/hop.yaml:ro" \
  -v hop-data:/data \
  ghcr.io/oslo254804746/hop-rs:v0.2.1 \
  hop-server --config /etc/hop/hop.yaml serve
```

Publish 8083 only for an intentional, direct cross-origin Control API deployment. In that case, list complete `http://` or `https://` Origins in `api.cors_allowlist`. Bare hostnames, bare IPs, and URLs containing paths produce a field-specific configuration error. `["*"]` is supported but must be the only item.

## systemd

```ini
[Unit]
Description=Hop SSH jump server
After=network-online.target

[Service]
User=hop
Group=hop
ExecStart=/usr/local/bin/hop-server --config /etc/hop/hop.yaml serve
Restart=on-failure
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/hop

[Install]
WantedBy=multi-user.target
```

Set `data_dir: /var/lib/hop`, make the YAML `0600`, and own it by the `hop` user.

## Operational checks

```bash
docker compose ps
docker compose logs hop panel
ssh -p 2222 <asset-name>@<hop-host>
```

An unmatched ingress key must receive only `Permission denied (publickey)`. A browser refresh clears the management Token from memory by design; enter it again to reconnect.
