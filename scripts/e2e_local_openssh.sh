#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/hop-openssh-e2e.XXXXXX")
hop_pid=
target_pid=

cleanup() {
	for pid in "$hop_pid" "$target_pid"; do
		if [[ -n "$pid" ]]; then
			kill "$pid" 2>/dev/null || true
			wait "$pid" 2>/dev/null || true
		fi
	done
	rm -rf "$run_dir"
}

report_error() {
	local status=$?
	printf 'OpenSSH E2E failed at line %s: %s\n' "${BASH_LINENO[0]}" "$BASH_COMMAND" >&2
	for log in "$run_dir/hop.log" "$run_dir/target.log" "$run_dir/rotated-target.log"; do
		if [[ -f "$log" ]]; then
			printf '\n===== %s =====\n' "$(basename "$log")" >&2
			cat "$log" >&2
		fi
	done
	exit "$status"
}

trap cleanup EXIT
trap report_error ERR

for command in cargo ssh scp sftp sshd ssh-keygen python3 timeout; do
	command -v "$command" >/dev/null || {
		echo "missing required command: $command" >&2
		exit 2
	}
done

free_port() {
	python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

target_port=$(free_port)
hop_port=$(free_port)
current_user=$(id -un)
bin="$repo_dir/target/debug/hop-server"
sshd_bin=$(command -v sshd)

cargo build --quiet --locked --manifest-path "$repo_dir/Cargo.toml" -p hop-server
ssh-keygen -q -t ed25519 -N '' -f "$run_dir/target_host_key"
ssh-keygen -q -t ed25519 -N '' -f "$run_dir/target_login_key"
ssh-keygen -q -t ed25519 -N '' -f "$run_dir/hop_ingress_key"
ssh-keygen -q -t ed25519 -N '' -f "$run_dir/wrong_ingress_key"
cp "$run_dir/target_login_key.pub" "$run_dir/authorized_keys"
chmod 600 "$run_dir/authorized_keys" "$run_dir/target_login_key" "$run_dir/hop_ingress_key"

cat >"$run_dir/sshd_config" <<EOF
Port $target_port
ListenAddress 127.0.0.1
HostKey $run_dir/target_host_key
PidFile $run_dir/sshd.pid
AuthorizedKeysFile $run_dir/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
AllowUsers $current_user
LogLevel ERROR
Subsystem sftp internal-sftp
EOF

"$sshd_bin" -D -e -f "$run_dir/sshd_config" >"$run_dir/target.log" 2>&1 &
target_pid=$!

cat >"$run_dir/hop.yaml" <<EOF
listen: 127.0.0.1:$hop_port
data_dir: $run_dir
ssh:
  banner: ""
  keepalive_interval: 30
  connect_timeout: 5
credentials:
  e2e-target:
    username: $current_user
    private_key: |
$(sed 's/^/      /' "$run_dir/target_login_key")
assets:
  managed-target:
    host: 127.0.0.1
    port: $target_port
    credential: e2e-target
access_keys:
  e2e-ingress:
    public_key_file: ./hop_ingress_key.pub
    assets: [managed-target]
EOF

"$bin" --config "$run_dir/hop.yaml" config validate

"$bin" --config "$run_dir/hop.yaml" serve >"$run_dir/hop.log" 2>&1 &
hop_pid=$!
for _ in $(seq 1 100); do
	if (exec 3<>/dev/tcp/127.0.0.1/"$hop_port") 2>/dev/null; then
		exec 3>&-
		break
	fi
	if ! kill -0 "$hop_pid" 2>/dev/null || ! kill -0 "$target_pid" 2>/dev/null; then
		cat "$run_dir/hop.log" "$run_dir/target.log" >&2
		exit 1
	fi
	sleep 0.05
done

# Exercise a deterministic early EOF instead of relying on the readiness probe's
# close timing, which can be observed as either a clean disconnect or an EOF.
python3 - "$hop_port" <<'PY'
import socket
import sys

with socket.create_connection(("127.0.0.1", int(sys.argv[1]))) as connection:
    connection.sendall(b"SSH-2.0-hop-e2e")
PY

common_client_options=(
	-i "$run_dir/hop_ingress_key"
	-o IdentitiesOnly=yes
	-o StrictHostKeyChecking=no
	-o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR
)
ssh_options=(-p "$hop_port" "${common_client_options[@]}")
scp_options=(-P "$hop_port" "${common_client_options[@]}")
sftp_options=(-P "$hop_port" "${common_client_options[@]}")

if wrong_key_output=$(timeout 5 ssh -p "$hop_port" \
	-i "$run_dir/wrong_ingress_key" \
	-o IdentitiesOnly=yes \
	-o PreferredAuthentications=publickey,password,keyboard-interactive \
	-o StrictHostKeyChecking=no \
	-o UserKnownHostsFile=/dev/null \
	managed-target@127.0.0.1 true 2>&1); then
	wrong_key_status=0
else
	wrong_key_status=$?
fi
if [[ $wrong_key_status -eq 0 ]] || ! grep -q 'Permission denied (publickey)' <<<"$wrong_key_output"; then
	echo "unmatched key did not fail with publickey-only authentication" >&2
	printf '%s\n' "$wrong_key_output" >&2
	exit 1
fi
if grep -Eqi 'password:|keyboard-interactive' <<<"$wrong_key_output"; then
	echo "unmatched key was offered an ingress password method" >&2
	exit 1
fi

test "$(ssh "${ssh_options[@]}" managed-target@127.0.0.1 'printf managed-ok')" = managed-ok
test "$(printf 'stdin-roundtrip' | ssh "${ssh_options[@]}" managed-target@127.0.0.1 cat)" = stdin-roundtrip
test "$(ssh "${ssh_options[@]}" managed-target@127.0.0.1 'printf stderr-ok >&2' 2>&1)" = stderr-ok
if ssh "${ssh_options[@]}" managed-target@127.0.0.1 'exit 42'; then
	exit_status=0
else
	exit_status=$?
fi
test "$exit_status" -eq 42
timeout 10 ssh -tt "${ssh_options[@]}" managed-target@127.0.0.1 tty 2>&1 | grep -q '/dev/pts/'

printf 'scp-through-hop\n' >"$run_dir/scp-source.txt"
scp -q "${scp_options[@]}" "$run_dir/scp-source.txt" \
	"managed-target@127.0.0.1:$run_dir/scp-remote.txt"
scp -q "${scp_options[@]}" "managed-target@127.0.0.1:$run_dir/scp-remote.txt" \
	"$run_dir/scp-download.txt"
cmp "$run_dir/scp-source.txt" "$run_dir/scp-download.txt"

printf 'sftp-through-hop\n' >"$run_dir/sftp-source.txt"
cat >"$run_dir/sftp.batch" <<EOF
put $run_dir/sftp-source.txt $run_dir/sftp-remote.txt
get $run_dir/sftp-remote.txt $run_dir/sftp-download.txt
rm $run_dir/sftp-remote.txt
EOF
sftp -q -b "$run_dir/sftp.batch" "${sftp_options[@]}" managed-target@127.0.0.1
cmp "$run_dir/sftp-source.txt" "$run_dir/sftp-download.txt"

cat >"$run_dir/ssh_config" <<EOF
Host hop-e2e-gateway
  HostName 127.0.0.1
  Port $hop_port
  User proxy
  IdentityFile $run_dir/hop_ingress_key
  IdentitiesOnly yes
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  LogLevel ERROR

Host managed-target.hop
  User $current_user
  Port $target_port
  IdentityFile $run_dir/target_login_key
  IdentitiesOnly yes
  ProxyJump hop-e2e-gateway
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  LogLevel ERROR
EOF
test "$(ssh -F "$run_dir/ssh_config" managed-target.hop 'printf proxy-ok')" = proxy-ok

original_host_fingerprint=$(python3 - "$run_dir/hop.db" "$run_dir/target_login_key" <<'PY'
import pathlib
import sqlite3
import sys

database, private_key_path = sys.argv[1:]
connection = sqlite3.connect(database)
assert connection.execute("SELECT COUNT(*) FROM known_hosts").fetchone()[0] == 1
assert connection.execute("SELECT COUNT(*) FROM sessions WHERE status = 'ok'").fetchone()[0] >= 7
raw_database = pathlib.Path(database).read_bytes()
private_key = pathlib.Path(private_key_path).read_bytes()
assert private_key not in raw_database
print(connection.execute("SELECT fingerprint FROM known_hosts").fetchone()[0])
PY
)

kill "$target_pid"
wait "$target_pid"
target_pid=
ssh-keygen -q -t ed25519 -N '' -f "$run_dir/rotated_target_host_key"
sed "s#HostKey $run_dir/target_host_key#HostKey $run_dir/rotated_target_host_key#" \
	"$run_dir/sshd_config" >"$run_dir/rotated_sshd_config"
"$sshd_bin" -D -e -f "$run_dir/rotated_sshd_config" >"$run_dir/rotated-target.log" 2>&1 &
target_pid=$!
for _ in $(seq 1 100); do
	if (exec 3<>/dev/tcp/127.0.0.1/"$target_port") 2>/dev/null; then
		exec 3>&-
		break
	fi
	if ! kill -0 "$target_pid" 2>/dev/null; then
		cat "$run_dir/rotated-target.log" >&2
		exit 1
	fi
	sleep 0.05
done

if rotated_output=$(timeout 10 ssh "${ssh_options[@]}" managed-target@127.0.0.1 \
	'printf should-not-connect' 2>&1); then
	rotated_status=0
else
	rotated_status=$?
fi
if [[ $rotated_status -eq 0 ]] || grep -q 'should-not-connect' <<<"$rotated_output"; then
	echo "managed connection accepted a changed target Host Key" >&2
	exit 1
fi
stored_host_fingerprint=$(python3 - "$run_dir/hop.db" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
print(connection.execute("SELECT fingerprint FROM known_hosts").fetchone()[0])
PY
)
test "$stored_host_fingerprint" = "$original_host_fingerprint"

disconnect_log=
for _ in $(seq 1 100); do
	disconnect_log=$(grep -m1 'ssh client disconnected' "$run_dir/hop.log" || true)
	if [[ -n "$disconnect_log" ]]; then
		break
	fi
	sleep 0.05
done
if [[ -z "$disconnect_log" ]]; then
	echo "missing SSH disconnect log" >&2
	cat "$run_dir/hop.log" >&2
	exit 1
fi
grep -q 'client_ip=' <<<"$disconnect_log"
if grep 'ssh session error' "$run_dir/hop.log" | grep -q 'early eof'; then
	echo "benign early EOF was logged as an SSH session error" >&2
	exit 1
fi
authenticated_log=$(grep -m1 'ssh ingress authenticated' "$run_dir/hop.log" || true)
if [[ -z "$authenticated_log" ]]; then
	echo "missing SSH authentication log" >&2
	cat "$run_dir/hop.log" >&2
	exit 1
fi
grep -q 'client_ip=' <<<"$authenticated_log"

rm -f "$run_dir/scp-remote.txt"
echo "Single-YAML OpenSSH publickey/exec/PTY/SCP/SFTP/ProxyJump/Host-Key end-to-end test passed"
