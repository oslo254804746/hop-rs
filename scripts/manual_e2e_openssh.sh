#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${HOP_TEST_PUBKEY:-}" || -z "${HOP_TARGET_HOST:-}" || -z "${HOP_TARGET_USER:-}" || -z "${HOP_TARGET_PASSWORD:-}" ]]; then
  cat >&2 <<'USAGE'
Set these variables before running:

  HOP_TEST_PUBKEY       path to the developer public key allowed into Hop
  HOP_TARGET_HOST       reachable OpenSSH target host
  HOP_TARGET_USER       target SSH username
  HOP_TARGET_PASSWORD   target SSH password for managed connection

This script starts Hop on localhost ports 2222/8080, seeds one key,
one password credential, and one asset, then prints manual SSH checks.
USAGE
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_DIR="${TMPDIR:-/tmp}/hop-e2e"
mkdir -p "$RUN_DIR"

cat > "$RUN_DIR/config.toml" <<EOF
[server]
ssh_bind = "127.0.0.1:2222"
admin_bind = "127.0.0.1:8080"

[database]
path = "$RUN_DIR/hop.db"

[ssh]
host_key_file = "$RUN_DIR/hop_host_key"
host_key_type = "ed25519"
banner = "Welcome to Hop E2E"
keepalive_interval = 30
connect_timeout = 10
proxy_policy = "assets_only"

[security]
secret_key_file = "$RUN_DIR/hop.secret"

[runtime]
temp_dir = "$RUN_DIR/tmp"
EOF

cargo run --manifest-path "$ROOT/Cargo.toml" -p hop-server -- key add \
  --config "$RUN_DIR/config.toml" \
  --name "e2e developer" \
  --public-key-file "$HOP_TEST_PUBKEY"

cargo run --manifest-path "$ROOT/Cargo.toml" -p hop-server -- credential add \
  --config "$RUN_DIR/config.toml" \
  --name "e2e password" \
  --username "$HOP_TARGET_USER" \
  --auth-type password \
  --password "$HOP_TARGET_PASSWORD"

CRED_ID="$(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- credential list --config "$RUN_DIR/config.toml" | awk 'NR==1 {print $1}')"

cargo run --manifest-path "$ROOT/Cargo.toml" -p hop-server -- asset add \
  --config "$RUN_DIR/config.toml" \
  --name e2e-target \
  --hostname "$HOP_TARGET_HOST" \
  --port 22 \
  --tags e2e \
  --credential-id "$CRED_ID"

cat <<EOF
Start Hop:

  cargo run --manifest-path "$ROOT/Cargo.toml" -p hop-server -- serve --config "$RUN_DIR/config.toml"

Manual checks:

  ssh -p 2222 127.0.0.1
  ssh -tt -p 2222 e2e-target@127.0.0.1
  ssh -J 127.0.0.1:2222 e2e-target.hop

ProxyJump requires your local SSH client to have credentials for the target.
Managed TUI and direct asset login use the Hop-stored credential seeded above.
EOF
