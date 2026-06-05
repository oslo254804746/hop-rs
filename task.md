# Remove Hop CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the developer-side `hop` binary and make `hop-server` the only product deliverable.

**Architecture:** Hop is a standard SSH server. Trusted SSH public keys are the only user credentials for entering Hop; interactive SSH opens the TUI, asset-name usernames trigger direct managed connections, and ProxyJump remains pure TCP forwarding. There is no local client binary, no generated `~/.ssh/config` product path, and no user-facing SSH exec command surface such as `hop-list-assets`, `hop-connect`, `hop-version`, or `ssh ... ls`.

**Tech Stack:** Rust workspace, Cargo, russh, OpenSSH, Docker, Markdown docs.

---

## Current Decision

- `hop-server` is the only binary users deploy or run.
- Users connect with standard OpenSSH:
  - `ssh -p 2222 hop-host` opens the Hop TUI.
  - `ssh -p 2222 asset-name@hop-host` directly enters a managed asset.
  - `ssh -J hop-host:2222 asset-name.hop` uses ProxyJump and local target credentials.
- Do not preserve `hop` as an unpublished crate.
- Do not add `ssh -p 2222 hop-host ls`.
- Do not keep `~/.ssh/config` examples in the main product path.
- Do not treat `hop-list-assets`, `hop-connect`, or `hop-version` as hidden compatibility APIs.

## Worktree Notes

- The worktree is already dirty. Do not revert unrelated user changes.
- Existing dirty files observed before this plan:
  - Modified: `crates/hop-server/src/admin/html.rs`
  - Deleted: `docs/task.md`
- This plan intentionally creates root-level `task.md` and does not restore `docs/task.md`.

## File Map

- Delete: `crates/hop-cli/Cargo.toml`
- Delete: `crates/hop-cli/src/main.rs`
- Delete: `crates/hop-cli/src/config.rs`
- Delete: `crates/hop-cli/src/ssh_exec.rs`
- Modify: `Cargo.toml`
  - Remove `crates/hop-cli` from workspace members.
- Modify: `Cargo.lock`
  - Regenerate after removing the workspace package.
- Modify: `Dockerfile`
  - Build and copy only `hop-server`.
- Modify: `crates/hop-server/src/ssh/mod.rs`
  - Remove the `routes` module.
- Delete: `crates/hop-server/src/ssh/routes.rs`
- Modify: `crates/hop-server/src/ssh/server.rs`
  - Remove `ExecCommand` parsing and handling.
  - Reject all SSH remote command exec requests with one clear unsupported-command message.
- Modify: `crates/hop-server/src/ssh/bridge.rs`
  - Remove `ManagedSessionMode::Exec`.
  - Remove audit mode `exec-connect`.
  - Change non-TUI managed-connection failure text away from `hop-connect`.
- Modify: `README.md`
  - Remove local wrapper references and document OpenSSH-only developer usage.
- Modify: `docs/deployment.md`
  - Remove `hop` build, install, Docker, upgrade, and verification instructions.
- Modify: `docs/design.md`
  - Remove local CLI design, SSH exec protocol, and `exec-connect` session mode.
- Modify: `scripts/manual_e2e_openssh.sh`
  - Remove manual checks for `hop-version`, `hop-list-assets`, and `hop-connect`.
  - Keep checks for TUI, direct managed login by asset username, and ProxyJump.

## Task 1: Remove the Workspace Client Package

**Files:**
- Delete: `crates/hop-cli/Cargo.toml`
- Delete: `crates/hop-cli/src/main.rs`
- Delete: `crates/hop-cli/src/config.rs`
- Delete: `crates/hop-cli/src/ssh_exec.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Remove the workspace member**

  In `Cargo.toml`, remove `"crates/hop-cli"` from `workspace.members`.

- [ ] **Step 2: Delete the client crate files**

  Delete the entire `crates/hop-cli` directory.

- [ ] **Step 3: Regenerate the lockfile**

  Run:

  ```bash
  cargo generate-lockfile
  ```

  Expected: command succeeds and `Cargo.lock` no longer needs the `hop` package as a workspace package.

- [ ] **Step 4: Verify workspace package list**

  Run:

  ```bash
  cargo metadata --format-version 1 --no-deps
  ```

  Expected: packages include `hop-core` and `hop-server`, but not a package named `hop`.

## Task 2: Remove SSH Exec Product Surface

**Files:**
- Delete: `crates/hop-server/src/ssh/routes.rs`
- Modify: `crates/hop-server/src/ssh/mod.rs`
- Modify: `crates/hop-server/src/ssh/server.rs`
- Modify: `crates/hop-server/src/ssh/bridge.rs`

- [ ] **Step 1: Update bridge mode tests first**

  In `crates/hop-server/src/ssh/bridge.rs`, change the existing test to expect only:

  ```rust
  assert_eq!(managed_session_mode(ManagedSessionMode::Tui), "tui-connect");
  assert_eq!(managed_session_mode(ManagedSessionMode::Direct), "direct");
  ```

  Remove any assertion for `ManagedSessionMode::Exec`.

- [ ] **Step 2: Run the bridge test and confirm it fails**

  Run:

  ```bash
  cargo test -p hop-server ssh::bridge::tests::managed_session_mode_distinguishes_tui_connects
  ```

  Expected before implementation: compile failure or test failure because `ManagedSessionMode::Exec` still exists in the implementation.

- [ ] **Step 3: Remove the exec bridge mode**

  In `crates/hop-server/src/ssh/bridge.rs`:

  - Delete `ManagedSessionMode::Exec`.
  - Delete the `managed_session_mode` match arm returning `exec-connect`.
  - Rename the test if desired, for example `managed_session_mode_distinguishes_user_visible_connects`.
  - Change the non-TUI failure message from `hop-connect failed` to a neutral direct-connection message such as `direct connection failed`.

- [ ] **Step 4: Remove route parsing**

  In `crates/hop-server/src/ssh/mod.rs`, remove:

  ```rust
  pub mod routes;
  ```

  Delete `crates/hop-server/src/ssh/routes.rs`.

- [ ] **Step 5: Replace exec handling with a single rejection path**

  In `crates/hop-server/src/ssh/server.rs`:

  - Remove the import of `parse_exec_command` and `ExecCommand`.
  - Replace the current `exec_request` command parsing and match with a simple rejection.
  - Keep the message short and product-oriented, for example:

  ```text
  Hop does not support SSH remote commands. Open an interactive TUI session or connect directly with <asset>@host.
  ```

  Expected behavior: any `ssh -p 2222 hop-host <command>` exits non-zero and does not list assets, connect assets, or return version JSON.

- [ ] **Step 6: Remove stale server references**

  Search and remove code references to:

  ```text
  hop-list-assets
  hop-connect
  hop-version
  exec-connect
  ManagedSessionMode::Exec
  ExecCommand
  parse_exec_command
  ```

- [ ] **Step 7: Run focused server tests**

  Run:

  ```bash
  cargo test -p hop-server
  ```

  Expected: all `hop-server` tests pass.

## Task 3: Update Build and Container Packaging

**Files:**
- Modify: `Dockerfile`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Update Docker build command**

  In `Dockerfile`, change:

  ```dockerfile
  RUN cargo build --release --bin hop-server --bin hop
  ```

  to:

  ```dockerfile
  RUN cargo build --release --bin hop-server
  ```

- [ ] **Step 2: Remove client binary copy**

  Delete the Dockerfile line that copies `/src/target/release/hop` into `/usr/local/bin/hop`.

- [ ] **Step 3: Verify release build command**

  Run:

  ```bash
  cargo build --release --bin hop-server
  ```

  Expected: build succeeds and no `hop` binary is required.

## Task 4: Update User-Facing Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/deployment.md`
- Modify: `docs/design.md`

- [ ] **Step 1: Update README product boundary**

  In `README.md`:

  - Remove `hop` local wrapper from MVP capabilities.
  - Change release build examples to `cargo build --release --bin hop-server`.
  - Remove the developer wrapper section.
  - Remove all `hop --host ...`, `hop connect`, `hop ls`, and `hop ssh-config` examples.
  - Keep OpenSSH usage as the only developer path:

  ```bash
  ssh -p 2222 hop-host
  ssh -p 2222 asset-name@hop-host
  ssh -J hop-host:2222 asset-name.hop
  ```

  - Explain that user identity is the trusted SSH public key.
  - Explain that target credentials stored in Hop are only for server-managed asset connections, not for logging into Hop.
  - Remove `crates/hop-cli` from the project structure.

- [ ] **Step 2: Update deployment docs**

  In `docs/deployment.md`:

  - Build only `hop-server`.
  - List only `target/release/hop-server` as a release artifact.
  - Install only `/usr/local/bin/hop-server`.
  - Remove developer local `hop` installation and usage.
  - Remove Docker text saying the image copies `hop`.
  - Remove upgrade steps that install `target/release/hop`.
  - Keep direct OpenSSH validation:

  ```bash
  ssh -p 2222 hop-host
  ssh -p 2222 web-prod-01@hop-host
  ssh -J hop-host:2222 web-prod-01.hop
  ```

- [ ] **Step 3: Update design docs**

  In `docs/design.md`:

  - Remove the local CLI section.
  - Remove the SSH exec protocol section.
  - Remove implementation stage item for CLI.
  - Remove verification item for local `hop ls`, `hop connect`, and `hop ssh-config`.
  - Update session mode comments to remove `exec-connect`.
  - Update SSH server flow so exec requests are rejected as unsupported remote commands.
  - Preserve TUI, direct asset username, and ProxyJump as the three user-facing SSH paths.

- [ ] **Step 4: Verify docs contain no old product path**

  Run:

  ```bash
  rg -n "hop --host|hop ls|hop connect|hop ssh-config|--bin hop|target/release/hop|crates/hop-cli|hop-list-assets|hop-version|exec-connect" README.md docs Dockerfile Cargo.toml crates scripts
  ```

  Expected: no matches, except any matches in this `task.md` if the search includes it.

## Task 5: Update Manual E2E Script

**Files:**
- Modify: `scripts/manual_e2e_openssh.sh`

- [ ] **Step 1: Remove old exec checks**

  Delete these manual check lines:

  ```bash
  ssh -p 2222 127.0.0.1 hop-version
  ssh -p 2222 127.0.0.1 hop-list-assets
  ssh -tt -p 2222 127.0.0.1 'hop-connect e2e-target'
  ```

- [ ] **Step 2: Add direct managed login check**

  Add a manual check that uses the asset name as the SSH username:

  ```bash
  ssh -tt -p 2222 e2e-target@127.0.0.1
  ```

- [ ] **Step 3: Keep TUI and ProxyJump checks**

  Keep:

  ```bash
  ssh -p 2222 127.0.0.1
  ssh -J 127.0.0.1:2222 e2e-target.hop
  ```

  Update explanatory text so it says managed TUI and direct asset login use Hop-stored credentials.

## Task 6: Final Verification

**Files:**
- All modified and deleted files above.

- [ ] **Step 1: Format Rust code**

  Run:

  ```bash
  cargo fmt --all
  ```

  Expected: succeeds.

- [ ] **Step 2: Run all tests**

  Run:

  ```bash
  cargo test --workspace
  ```

  Expected: all tests pass.

- [ ] **Step 3: Run release build**

  Run:

  ```bash
  cargo build --release --bin hop-server
  ```

  Expected: build succeeds.

- [ ] **Step 4: Confirm old binary is gone from workspace metadata**

  Run:

  ```bash
  cargo metadata --format-version 1 --no-deps
  ```

  Expected: no package named `hop`.

- [ ] **Step 5: Confirm old CLI and exec surface are gone**

  Run:

  ```bash
  rg -n "crates/hop-cli|--bin hop|hop ssh-config|hop-list-assets|hop-version|exec-connect|ManagedSessionMode::Exec|ExecCommand|parse_exec_command" Cargo.toml Dockerfile README.md docs crates scripts
  ```

  Expected: no matches.

- [ ] **Step 6: Optional Docker verification**

  If Docker is available, run:

  ```bash
  docker build -t hop:cli-removed .
  ```

  Expected: image builds and contains `hop-server` only.

## Task 7: Commit

**Files:**
- Stage only files changed for this task.
- Do not stage unrelated existing user changes unless explicitly requested.

- [ ] **Step 1: Review diff**

  Run:

  ```bash
  git diff --stat
  git diff -- Cargo.toml Cargo.lock Dockerfile README.md docs/deployment.md docs/design.md scripts/manual_e2e_openssh.sh crates/hop-server/src/ssh
  ```

- [ ] **Step 2: Stage scoped changes**

  Run:

  ```bash
  git add Cargo.toml Cargo.lock Dockerfile README.md docs/deployment.md docs/design.md scripts/manual_e2e_openssh.sh crates/hop-server/src/ssh task.md
  git add -u crates/hop-cli
  ```

- [ ] **Step 3: Commit**

  Run:

  ```bash
  git commit -m "Remove local hop CLI"
  ```

  Expected: commit includes only CLI removal, SSH exec removal, packaging updates, docs updates, script updates, and `task.md`.
