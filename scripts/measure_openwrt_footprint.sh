#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary=${1:-$repo_dir/target/release/hop-server}

if [[ ! -x "$binary" ]]; then
	echo "release binary not found: $binary" >&2
	exit 1
fi

run_dir=$(mktemp -d)
server_pid=
cleanup() {
	if [[ -n "$server_pid" ]]; then
		kill "$server_pid" 2>/dev/null || true
		wait "$server_pid" 2>/dev/null || true
	fi
	rm -rf "$run_dir"
}
trap cleanup EXIT

printf '%s\n' \
	'[server]' \
	'ssh_listen = "127.0.0.1:0"' \
	'' \
	'[database]' \
	"path = \"$run_dir/hop.db\"" \
	'' \
	'[api]' \
	'enabled = false' \
	'' \
	'[ssh]' \
	"host_key_file = \"$run_dir/host_key\"" \
	'banner = ""' \
	'' \
	'[security]' \
	"master_key_file = \"$run_dir/master.key\"" \
	>"$run_dir/config.toml"

"$binary" --config "$run_dir/config.toml" serve >"$run_dir/server.log" 2>&1 &
server_pid=$!

for _ in $(seq 1 100); do
	if [[ -f "$run_dir/hop.db" ]]; then
		break
	fi
	if ! kill -0 "$server_pid" 2>/dev/null; then
		cat "$run_dir/server.log" >&2
		exit 1
	fi
	sleep 0.05
done

sleep 1
rss_kib=$(awk '/^VmRSS:/ { print $2 }' "/proc/$server_pid/status")
binary_bytes=$(stat -c %s "$binary")
cp "$binary" "$run_dir/hop-server.stripped"
strip "$run_dir/hop-server.stripped"
stripped_binary_bytes=$(stat -c %s "$run_dir/hop-server.stripped")
database_bytes=$(stat -c %s "$run_dir/hop.db")
wal_path="$run_dir/hop.db-wal"
wal_before=0
[[ -f "$wal_path" ]] && wal_before=$(stat -c %s "$wal_path")

sleep 2
database_after=$(stat -c %s "$run_dir/hop.db")
wal_after=0
[[ -f "$wal_path" ]] && wal_after=$(stat -c %s "$wal_path")

printf '{"binary_bytes":%s,"stripped_binary_bytes":%s,"idle_rss_kib":%s,"database_bytes":%s,"idle_database_growth_bytes":%s,"wal_bytes":%s,"idle_wal_growth_bytes":%s}\n' \
	"$binary_bytes" \
	"$stripped_binary_bytes" \
	"$rss_kib" \
	"$database_bytes" \
	"$((database_after - database_bytes))" \
	"$wal_after" \
	"$((wal_after - wal_before))"
