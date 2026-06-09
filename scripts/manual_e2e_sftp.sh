#!/usr/bin/env bash
set -euo pipefail

required=(HOP_TEST_KEY HOP_TARGET_HOST HOP_TARGET_USER HOP_TARGET_PASSWORD HOP_TARGET_DIR)
for name in "${required[@]}"; do
  if [[ -z "${!name:-}" ]]; then
    printf 'missing required environment variable: %s\n' "$name" >&2
    exit 2
  fi
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hop-sftp-e2e.XXXXXX")"
HOP_PID=""
cleanup() {
  if [[ -n "$HOP_PID" ]]; then
    kill "$HOP_PID" 2>/dev/null || true
    wait "$HOP_PID" 2>/dev/null || true
  fi
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cat > "$RUN_DIR/config.toml" <<EOF
[server]
ssh_bind = "127.0.0.1:2222"
admin_bind = "127.0.0.1:8080"

[database]
path = "$RUN_DIR/hop.db"

[ssh]
host_key_file = "$RUN_DIR/hop_host_key"
host_key_type = "ed25519"
banner = ""
keepalive_interval = 30
connect_timeout = 10
proxy_policy = "assets_only"

[security]
secret_key_file = "$RUN_DIR/hop.secret"

[runtime]
temp_dir = "$RUN_DIR/tmp"
EOF

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- key add \
  --config "$RUN_DIR/config.toml" --name "sftp e2e" --public-key-file "$HOP_TEST_KEY.pub"
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- credential add \
  --config "$RUN_DIR/config.toml" --name "sftp target" --username "$HOP_TARGET_USER" \
  --auth-type password --password "$HOP_TARGET_PASSWORD"
CRED_ID="$(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- credential list \
  --config "$RUN_DIR/config.toml" | awk 'NR==1 {print $1}')"
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- asset add \
  --config "$RUN_DIR/config.toml" --name sftp-target --hostname "$HOP_TARGET_HOST" \
  --port 22 --credential-id "$CRED_ID"

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server -- serve \
  --config "$RUN_DIR/config.toml" >"$RUN_DIR/hop.log" 2>&1 &
HOP_PID=$!
for _ in $(seq 1 50); do
  if (exec 3<>/dev/tcp/127.0.0.1/2222) 2>/dev/null; then
    exec 3>&-
    break
  fi
  sleep 0.1
done
kill -0 "$HOP_PID"

printf 'hop managed sftp e2e\n' > "$RUN_DIR/upload.txt"
cat > "$RUN_DIR/sftp.batch" <<EOF
put $RUN_DIR/upload.txt $HOP_TARGET_DIR/hop-sftp-upload.txt
rename $HOP_TARGET_DIR/hop-sftp-upload.txt $HOP_TARGET_DIR/hop-sftp-renamed.txt
get $HOP_TARGET_DIR/hop-sftp-renamed.txt $RUN_DIR/download.txt
rm $HOP_TARGET_DIR/hop-sftp-renamed.txt
EOF

sftp -q -b "$RUN_DIR/sftp.batch" -i "$HOP_TEST_KEY" \
  -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -P 2222 sftp-target@127.0.0.1
cmp "$RUN_DIR/upload.txt" "$RUN_DIR/download.txt"
printf 'SFTP end-to-end test passed\n'
