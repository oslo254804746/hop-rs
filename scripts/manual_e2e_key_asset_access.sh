#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hop-key-access-e2e.XXXXXX")"
SSH_PORT=32222
ADMIN_PORT=38080
ASSET_ONE_PORT=39001
ASSET_TWO_PORT=39002
HOP_PID=""
HTTP_ONE_PID=""
HTTP_TWO_PID=""

cleanup() {
  for pid in "$HOP_PID" "$HTTP_ONE_PID" "$HTTP_TWO_PID"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

for command in cargo ssh ssh-keygen timeout python3; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 2
  }
done

cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server ssh::
cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p hop-server
BIN="$ROOT/target/debug/hop-server"

ssh-keygen -q -t ed25519 -N '' -f "$RUN_DIR/key-all"
ssh-keygen -q -t ed25519 -N '' -f "$RUN_DIR/key-restricted"

cat > "$RUN_DIR/config.toml" <<EOF
[server]
ssh_bind = "127.0.0.1:$SSH_PORT"
admin_bind = "127.0.0.1:$ADMIN_PORT"

[database]
path = "$RUN_DIR/hop.db"

[ssh]
host_key_file = "$RUN_DIR/hop_host_key"
host_key_type = "ed25519"
banner = ""
keepalive_interval = 30
connect_timeout = 5
proxy_policy = "assets_only"

[security]
secret_key_file = "$RUN_DIR/hop.secret"

[runtime]
temp_dir = "$RUN_DIR/tmp"
EOF

"$BIN" --config "$RUN_DIR/config.toml" key add \
  --name key-all --public-key-file "$RUN_DIR/key-all.pub"
"$BIN" --config "$RUN_DIR/config.toml" key add \
  --name key-restricted --public-key-file "$RUN_DIR/key-restricted.pub"
"$BIN" --config "$RUN_DIR/config.toml" asset add \
  --name asset-one --protocol tcp --hostname 127.0.0.1 --port "$ASSET_ONE_PORT"
"$BIN" --config "$RUN_DIR/config.toml" asset add \
  --name asset-two --protocol tcp --hostname 127.0.0.1 --port "$ASSET_TWO_PORT"

KEY_RESTRICTED_ID="$("$BIN" --config "$RUN_DIR/config.toml" key list | awk '$4 == "key-restricted" {print $1}')"
ASSET_ONE_ID="$("$BIN" --config "$RUN_DIR/config.toml" asset list | awk '$2 == "asset-one" {print $1}')"
test -n "$KEY_RESTRICTED_ID"
test -n "$ASSET_ONE_ID"

"$BIN" --config "$RUN_DIR/config.toml" key access set "$KEY_RESTRICTED_ID" \
  --mode restricted --asset-id "$ASSET_ONE_ID"

python3 -m http.server "$ASSET_ONE_PORT" --bind 127.0.0.1 \
  --directory "$RUN_DIR" >"$RUN_DIR/http-one.log" 2>&1 &
HTTP_ONE_PID=$!
python3 -m http.server "$ASSET_TWO_PORT" --bind 127.0.0.1 \
  --directory "$RUN_DIR" >"$RUN_DIR/http-two.log" 2>&1 &
HTTP_TWO_PID=$!
"$BIN" --config "$RUN_DIR/config.toml" serve >"$RUN_DIR/hop.log" 2>&1 &
HOP_PID=$!

for _ in $(seq 1 100); do
  if (exec 3<>/dev/tcp/127.0.0.1/"$SSH_PORT") 2>/dev/null; then
    exec 3>&-
    break
  fi
  sleep 0.1
done
kill -0 "$HOP_PID"

ssh_options=(
  -p "$SSH_PORT"
  -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o LogLevel=ERROR
)

capture_tui() {
  local key=$1
  { sleep 0.4; printf 'q'; } | timeout 5 ssh -tt "${ssh_options[@]}" -i "$key" 127.0.0.1 2>&1 || true
}

ALL_TUI="$(capture_tui "$RUN_DIR/key-all")"
RESTRICTED_TUI="$(capture_tui "$RUN_DIR/key-restricted")"
grep -q 'asset-one' <<<"$ALL_TUI"
grep -q 'asset-two' <<<"$ALL_TUI"
grep -q 'asset-one' <<<"$RESTRICTED_TUI"
if grep -q 'asset-two' <<<"$RESTRICTED_TUI"; then
  printf 'restricted key discovered unassigned asset-two in TUI\n' >&2
  exit 1
fi

http_through_hop() {
  local key=$1
  local target=$2
  local output
  local status
  set +e
  output="$({ printf 'GET / HTTP/1.0\r\nHost: localhost\r\n\r\n'; sleep 0.5; } | \
    timeout 2 ssh "${ssh_options[@]}" -i "$key" -W "$target" 127.0.0.1 2>&1)"
  status=$?
  set -e
  printf '%s' "$output"
  [[ $status -eq 0 || $status -eq 124 ]]
}

http_through_hop "$RUN_DIR/key-all" "asset-one.hop:$ASSET_ONE_PORT" | grep -q '200 OK'
http_through_hop "$RUN_DIR/key-all" "asset-two.hop:$ASSET_TWO_PORT" | grep -q '200 OK'
http_through_hop "$RUN_DIR/key-restricted" "asset-one.hop:$ASSET_ONE_PORT" | grep -q '200 OK'

for denied_target in \
  "asset-two:$ASSET_TWO_PORT" \
  "asset-two.hop:$ASSET_TWO_PORT" \
  "127.0.0.1:$ASSET_TWO_PORT"; do
  if http_through_hop "$RUN_DIR/key-restricted" "$denied_target" >/dev/null 2>&1; then
    printf 'restricted key unexpectedly reached %s\n' "$denied_target" >&2
    exit 1
  fi
done

"$BIN" --config "$RUN_DIR/config.toml" key access set "$KEY_RESTRICTED_ID" \
  --mode restricted
if http_through_hop "$RUN_DIR/key-restricted" "asset-one.hop:$ASSET_ONE_PORT" \
  >/dev/null 2>&1; then
  printf 'revoked asset remained reachable without restart\n' >&2
  exit 1
fi

python3 - "$RUN_DIR/hop.db" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
rows = connection.execute(
    "SELECT key_name, error FROM sessions WHERE status = 'failed' AND mode = 'tcp-forward'"
).fetchall()
assert rows, "expected denied forwarding audit records"
assert any(name == "key-restricted" for name, _ in rows)
assert all(error == "target not authorized or not found" for _, error in rows)
PY

printf 'Per-key asset authorization end-to-end test passed\n'
