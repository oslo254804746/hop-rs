# Hop MVP Task Breakdown

> Scope source: `docs/design.md`. This file tracks implementation work. MVP excludes TUI file browser, ZMODEM, fine-grained permissions, approval flow, session recording, TOTP, and SPA admin UI.

## Working Rules

- Keep MVP focused on one public SSH port plus a localhost/internal Admin API.
- Treat ProxyJump/ProxyCommand as pure TCP forwarding. It must never use stored target credentials.
- Treat TUI and `hop connect` as server-side managed connections. These may use stored target credentials.
- Developer CLI must use SSH exec/subsystem over port 2222, not Admin API.
- Prefer tests around protocol decisions, DB behavior, crypto envelope, and routing rules before adding UI polish.
- Commit after each milestone when the milestone builds and its tests pass.

## Milestone 0: Repository Scaffold

**Goal:** Create the Rust workspace shape without implementing behavior.

**Files:**
- Create `Cargo.toml`
- Create `config.example.toml`
- Create `migrations/001_init.sql`
- Create `crates/hop-core/Cargo.toml`
- Create `crates/hop-core/src/lib.rs`
- Create `crates/hop-server/Cargo.toml`
- Create `crates/hop-server/src/main.rs`
- Create `crates/hop-cli/Cargo.toml`
- Create `crates/hop-cli/src/main.rs`

**Tasks:**
- [x] Define workspace members: `hop-core`, `hop-server`, `hop-cli`.
- [x] Add release profile: `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`.
- [x] Add dependency baselines from `docs/design.md`; pin exact versions only after first successful build.
- [x] Configure SQLite bundled/static path; do not rely on system `libsqlite3.so`.
- [x] Add `config.example.toml` matching the design file.
- [x] Verify `cargo check --workspace` succeeds.

**Acceptance:**
- `cargo check --workspace` exits 0.
- `cargo metadata --no-deps` lists all three crates.

## Milestone 1: Core Models, DB, Config, Crypto

**Goal:** Make `hop-core` a tested library for persistent state and credential encryption.

**Files:**
- Create `crates/hop-core/src/models.rs`
- Create `crates/hop-core/src/db.rs`
- Create `crates/hop-core/src/config.rs`
- Create `crates/hop-core/src/crypto.rs`
- Create `crates/hop-core/src/errors.rs`
- Modify `migrations/001_init.sql`

**Tasks:**
- [x] Implement model structs for `AuthorizedKey`, `Asset`, `Credential`, `Session`, `KnownHost`, and settings.
- [x] Implement schema migration for the 6 MVP tables.
- [x] Enable SQLite WAL and `busy_timeout` at connection setup.
- [x] Implement config loading from TOML with defaults for SSH/Admin bind addresses.
- [x] Implement `hop.secret` creation with 32 random bytes and Linux `0600` permissions.
- [x] Implement credential envelope format: `v1:xchacha20poly1305:<nonce_b64>:<ciphertext_b64>`.
- [x] Implement HKDF-derived per-credential keys.
- [x] Add tests for encrypt/decrypt roundtrip, wrong-key failure, malformed envelope rejection, and nonce uniqueness.
- [x] Add tests for migration and basic CRUD using a temp SQLite database.

**Acceptance:**
- `cargo test -p hop-core` exits 0.
- Losing or changing the master key makes encrypted credentials undecryptable in tests.
- `migrations/001_init.sql` matches the 6-table design.

## Milestone 2: Minimal Management Entry

**Goal:** Provide a way to seed admin password, SSH public keys, assets, and credentials before the full Admin Web exists.

**Files:**
- Create `crates/hop-server/src/admin/mod.rs`
- Create `crates/hop-server/src/admin/bootstrap.rs`
- Create `crates/hop-server/src/admin/local_cli.rs`
- Modify `crates/hop-server/src/main.rs`

**Tasks:**
- [x] Implement first-run admin password generation, Argon2 hashing, and one-time console output.
- [x] Implement `hop-server reset-admin`.
- [x] Implement local management commands for adding/listing/deactivating public keys.
- [x] Implement local management commands for adding/listing assets.
- [x] Implement local management commands for adding/listing credentials without printing secrets.
- [x] Validate public key format and store SHA256 fingerprint.
- [x] Ensure secrets are never written to tracing logs.

**Acceptance:**
- `hop-server reset-admin` prints a new password once and stores only the hash.
- A local command can insert one key, one credential, and one asset.
- Listing commands display metadata but never print decrypted private keys, passwords, or passphrases.

## Milestone 3: SSH Server Authentication and Routing

**Goal:** Accept SSH connections, authenticate by public key whitelist, and route channel requests.

**Files:**
- Create `crates/hop-server/src/ssh/mod.rs`
- Create `crates/hop-server/src/ssh/server.rs`
- Create `crates/hop-server/src/ssh/routes.rs`
- Create `crates/hop-server/src/ssh/host_key.rs`
- Modify `crates/hop-server/src/main.rs`

**Tasks:**
- [x] Generate or load the hop SSH host key from `host_key_file`.
- [x] Implement russh public-key auth using `authorized_keys.fingerprint`.
- [x] Reject inactive or unknown keys.
- [x] Route shell/pty requests to a temporary minimal text response that confirms successful authentication.
- [x] Route exec requests for `hop-version`, `hop-list-assets`, and `hop-connect <asset>`.
- [x] Route `direct-tcpip` requests to an explicit rejection until Milestone 6.
- [x] Write session/auth logs with client IP, key fingerprint, key name, request mode, and status.

**Acceptance:**
- `ssh -p 2222 hop-host` succeeds only with a whitelisted active key.
- Unknown keys are rejected.
- `ssh -p 2222 hop-host hop-version` prints version/protocol metadata.

## Milestone 4: Outbound SSH and Raw Bridge

**Goal:** Prove the hardest path: hop connects to a target via stored credentials and bridges bytes both ways.

**Files:**
- Create `crates/hop-server/src/ssh/client.rs`
- Create `crates/hop-server/src/ssh/bridge.rs`
- Create `crates/hop-server/src/ssh/tofu.rs`
- Modify `crates/hop-server/src/ssh/routes.rs`

**Tasks:**
- [x] Implement outbound SSH client connection with password, key, and key+passphrase auth types.
- [x] Implement TOFU host key storage and mismatch rejection.
- [x] Implement raw bridge between inbound server channel and outbound target channel.
- [x] Forward PTY size changes to the target session.
- [x] Preserve Ctrl+C, EOF, and exit status semantics in raw bridge mode.
- [x] Record session start, success/failure, error message, and end time.
- [x] Add integration test fixture or manual test script with a local OpenSSH test target.

**Acceptance:**
- `ssh -p 2222 hop-host hop-connect <asset>` opens an interactive target shell.
- Exiting the target shell closes the managed session cleanly.
- A changed target host key is rejected and logged.

## Milestone 5: TUI over SSH

**Goal:** Replace the minimal shell response with the real ratatui asset picker and managed connection flow.

**Files:**
- Create `crates/hop-server/src/tui/mod.rs`
- Create `crates/hop-server/src/tui/app.rs`
- Create `crates/hop-server/src/tui/backend.rs`
- Create `crates/hop-server/src/tui/input.rs`
- Create `crates/hop-server/src/tui/views.rs`
- Modify `crates/hop-server/src/ssh/routes.rs`

**Tasks:**
- [x] Implement channel-backed ratatui backend for server-to-client output.
- [x] Implement termwiz `InputParser` adapter for client-to-server key events.
- [x] Render asset list with fuzzy search via `nucleo`.
- [x] Support `/` search, `Enter` connect, `q` quit, and basic help.
- [x] On connect, leave TUI foreground, run raw bridge, then restore TUI state after target disconnect.
- [x] Handle Ctrl+C in TUI as an app input, not as hop process termination.
- [x] Handle terminal resize via russh window-change events.

**Acceptance:**
- `ssh -p 2222 hop-host` opens TUI.
- Searching filters assets.
- `Enter` connects to target and returns to TUI after `exit`.
- Terminal state is usable after leaving target session and after quitting TUI.

## Milestone 6: ProxyJump and ProxyCommand

**Goal:** Support pure TCP forwarding with assets allowlist.

**Files:**
- Create `crates/hop-server/src/ssh/proxy.rs`
- Modify `crates/hop-server/src/ssh/routes.rs`
- Modify `crates/hop-core/src/db.rs`

**Tasks:**
- [x] Implement `direct-tcpip` request handling.
- [x] Match targets by `assets.hostname:assets.port`.
- [x] Match targets by `assets.name`.
- [x] Match `<asset>.hop` by stripping `.hop` and resolving `assets.name`.
- [x] Reject every unmatched target.
- [x] Bridge raw TCP bytes without credential lookup or SSH protocol inspection.
- [x] Record proxyjump session logs.

**Acceptance:**
- `ssh -J hop:2222 <allowed-target>` reaches the target when local target credentials are valid.
- `ssh -J hop:2222 <unlisted-target>` is rejected.
- Logs show `mode = proxyjump` and never reference decrypted credentials.

## Milestone 7: Admin Web

**Goal:** Add a localhost/internal admin UI for CRUD and read-only logs.

**Files:**
- Create `crates/hop-server/src/admin/routes.rs`
- Create `crates/hop-server/src/admin/auth.rs`
- Create `crates/hop-server/src/admin/html.rs`
- Modify `crates/hop-server/src/main.rs`

**Tasks:**
- [x] Implement Admin API binding to `127.0.0.1:8080` by default.
- [x] Implement login with Argon2 password verification and secure session cookie.
- [x] Implement asset CRUD.
- [x] Implement credential CRUD with secret redaction.
- [x] Implement authorized key CRUD and activation/deactivation.
- [x] Implement known host list and delete.
- [x] Implement sessions list.
- [x] Render pages server-side with `maud` or `askama`.

**Acceptance:**
- Admin Web is reachable via `http://127.0.0.1:8080`.
- Remote admin access works via `ssh -L 8080:127.0.0.1:8080 hop-server -p 2222`.
- No admin route is reachable without login.
- Secrets are never rendered back after save.

## Milestone 8: Local `hop` CLI

**Goal:** Provide a local wrapper that speaks SSH, not Admin API.

**Files:**
- Modify `crates/hop-cli/src/main.rs`
- Create `crates/hop-cli/src/config.rs`
- Create `crates/hop-cli/src/ssh_exec.rs`

**Tasks:**
- [x] Implement `hop` as a wrapper for `ssh -p <port> <host>`.
- [x] Implement `hop ls` using exec `hop-list-assets`.
- [x] Implement `hop connect <asset>` using exec `hop-connect <asset>`.
- [x] Implement `hop ssh-config` output for `Host hop` and `Host *.hop`.
- [x] Ensure CLI never contacts Admin API.

**Acceptance:**
- `hop ls` lists assets over SSH exec.
- `hop connect <asset>` opens managed target session.
- `hop ssh-config` prints usable OpenSSH config.

## Milestone 9: Polish, Packaging, and Docs

**Goal:** Make MVP shippable as a small single-binary Linux service.

**Files:**
- Modify `README.md`
- Modify `docs/design.md` if implementation decisions changed
- Create `Dockerfile`
- Create `systemd/hop.service`
- Modify `config.example.toml`

**Tasks:**
- [x] Add first-run guide.
- [x] Add backup warning for `hop.secret`, database, and host key.
- [x] Add ProxyJump and managed-connection examples.
- [x] Add Docker image build.
- [x] Add systemd service sample.
- [x] Add release build instructions for x86_64 and aarch64 Linux.
- [x] Add troubleshooting for host key mismatch, DB locked, unknown SSH key, and target auth failure.

**Acceptance:**
- A clean Linux machine can run hop from the binary plus config.
- README explains the difference between `hop connect` and `ssh -J`.
- Docs clearly say file browser is Post-MVP.

## Post-MVP Backlog

- [ ] Server-managed file download via target SFTP -> hop temp -> hop SFTP subsystem or local CLI pull.
- [ ] Server-managed file upload via local CLI -> hop temp -> target SFTP.
- [ ] Optional ZMODEM support after terminal compatibility is tested.
- [ ] Fine-grained asset grants through a relationship table.
- [ ] One-time admin login token with TTL.
- [ ] Optional TOTP for Admin Web.
- [ ] Import/export assets from SSH config.
- [ ] More complete audit metadata without session recording.

## End-to-End MVP Verification

- [ ] Start `hop-server` with an empty database.
- [ ] Capture one-time admin password from console output.
- [ ] Add one SSH public key, one credential, and two assets.
- [ ] Confirm `ssh -p 2222 hop-host` opens TUI.
- [ ] Confirm TUI search finds an asset.
- [ ] Confirm TUI `Enter` opens target shell using stored credential.
- [ ] Confirm target shell `exit` returns to TUI.
- [ ] Confirm `hop ls` lists assets through SSH exec.
- [ ] Confirm `hop connect <asset>` opens target shell through SSH exec.
- [ ] Confirm `ssh -J hop:2222 <allowed-target>` works with local target credentials.
- [ ] Confirm `ssh -J hop:2222 <unlisted-target>` is rejected.
- [ ] Confirm Admin Web is accessible only on the configured admin bind address.
- [ ] Confirm session logs contain TUI, exec-connect, and proxyjump entries.
