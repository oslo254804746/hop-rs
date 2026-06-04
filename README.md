# Hop

Hop is a lightweight SSH jump server MVP.

MVP scope:

- SSH public-key whitelist for entering Hop.
- SSH-over-TUI asset picker with server-managed target credentials.
- `ProxyJump` / `ProxyCommand` as pure TCP forwarding with an assets allowlist.
- SQLite only, six tables, encrypted stored credentials.
- Local `hop` CLI that shells out to OpenSSH and uses SSH exec, never Admin API.

Post-MVP items are intentionally not included: TUI file browser, ZMODEM, fine-grained asset grants, TOTP, approval flow, session recording, and SPA frontend.

## Build

```bash
cargo build --release --workspace
```

For Linux releases:

```bash
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
cargo build --release --bin hop-server --target x86_64-unknown-linux-gnu
cargo build --release --bin hop-server --target aarch64-unknown-linux-gnu
```

## First Run

```bash
cp config.example.toml config.toml
hop-server serve --config config.toml
```

On first run, Hop creates:

- SQLite database at `database.path`.
- Host key at `ssh.host_key_file`.
- Credential master secret at `security.secret_key_file`.
- Random admin password, printed once to the terminal.

Admin Web binds to `127.0.0.1:8080` by default. For remote administration:

```bash
ssh -L 8080:127.0.0.1:8080 hop-host -p 2222
```

Then open `http://127.0.0.1:8080`.

## Local Management CLI

Before Admin Web is populated, seed data locally on the server:

```bash
hop-server reset-admin --config config.toml

hop-server key add --config config.toml \
  --name "alice laptop" \
  --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'secret' | hop-server credential add --config config.toml \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin

hop-server asset add --config config.toml \
  --name web-prod-01 \
  --hostname 10.0.1.10 \
  --port 22 \
  --tags prod,web \
  --credential-id <credential-id>
```

Listing credentials never prints decrypted passwords, private keys, or passphrases.
Use `--password-stdin` to read a password from standard input instead of exposing
it in the process list.

## Developer Usage

Enter Hop TUI:

```bash
ssh hop-host -p 2222
```

Use the local wrapper:

```bash
hop --host hop-host --port 2222
hop --host hop-host --port 2222 ls
hop --host hop-host --port 2222 connect web-prod-01
hop --host hop-host --port 2222 ssh-config
```

The `hop` CLI only invokes `ssh`. It does not call Admin API.

## Managed Connection vs ProxyJump

`hop connect <asset>` and TUI Enter are server-side managed connections. Hop decrypts the asset credential and opens an outbound SSH session to the target.

`ssh -J hop:2222 target` and `ProxyCommand -W` are pure TCP forwarding. Hop only checks that `target` matches the assets allowlist. It never decrypts or uses stored target credentials for ProxyJump.

Allowlist matching supports:

- `assets.hostname:assets.port`
- `assets.name`, which forwards to that asset's stored `hostname:port`
- `<asset>.hop`, which strips `.hop` and forwards to that asset's stored `hostname:port`

## Backups

Back up these files together:

- SQLite database, for assets, keys, sessions, known hosts, and encrypted credentials.
- `hop.secret`, required to decrypt stored credentials.
- Hop SSH host key, to avoid client host-key warnings.

If `hop.secret` is lost, stored credentials cannot be recovered.

## Troubleshooting

- Unknown SSH key: add the user's public key in Admin Web or `hop-server key add`.
- Target auth failure: verify the asset has a credential and the username/secret works on the target.
- Host key mismatch: inspect Admin Web known hosts, delete the stale entry only after verifying the target host.
- DB locked: Hop enables WAL and a 5s busy timeout; persistent locks usually mean another process is holding the SQLite file.
- Admin Web not reachable remotely: it binds to localhost by default; use an SSH tunnel.

## File Transfer

MVP does not implement TUI file browser, ZMODEM, or server-managed SFTP transfer.

For users with their own target credentials, use standard OpenSSH through ProxyJump:

```bash
scp -J hop-host:2222 ./file web-prod-01.hop:/tmp/
sftp -J hop-host:2222 web-prod-01.hop
```
