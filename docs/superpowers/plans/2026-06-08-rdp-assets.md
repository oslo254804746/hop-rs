# RDP Assets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add first-class Admin Web support for RDP/TCP assets while continuing to use standard SSH local port forwarding instead of a Hop client CLI.

**Architecture:** Store an explicit `protocol` on assets with `ssh`, `rdp`, and `tcp` values. Keep Hop's existing SSH `direct-tcpip` bridge as the transport for RDP/TCP, clear managed credentials for non-SSH assets, and render copyable tunnel commands in Admin Web.

**Tech Stack:** Rust, sqlx SQLite migrations, axum, maud, clap, cargo test.

---

### Task 1: Asset Protocol Model And Persistence

**Files:**
- Modify: `crates/hop-core/src/models.rs`
- Modify: `crates/hop-core/src/db.rs`
- Create: `migrations/002_asset_protocol.sql`

- [ ] **Step 1: Write failing model and database tests**

Add tests that expect `NewAsset::new` to default to `ssh`, reject unsupported protocols, and clear credentials from `rdp` assets after insert.

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test -p hop-core asset_protocol`

- [ ] **Step 3: Implement protocol constants, validation, migration, query fields, and credential clearing**

Add `protocol TEXT NOT NULL DEFAULT 'ssh'` in migration `002`, include `protocol` in `Asset`, `AssetRow`, `NewAsset`, all asset `SELECT` lists, and `INSERT`/`UPDATE` statements.

- [ ] **Step 4: Run focused tests to verify GREEN**

Run: `cargo test -p hop-core asset_protocol`

### Task 2: Admin Web RDP/TCP Experience

**Files:**
- Modify: `crates/hop-server/src/admin/routes.rs`
- Modify: `crates/hop-server/src/admin/html.rs`
- Modify: `crates/hop-server/src/admin/i18n.rs`
- Modify: `crates/hop-server/src/main.rs`

- [ ] **Step 1: Write failing Admin Web tests**

Add tests that expect protocol controls, `rdp` rendering, and the command `ssh -p 2222 -N -T -L 127.0.0.1:13389:win-rdp.hop:3389 hop-host`.

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test -p hop-server admin::html::tests::assets_page_renders_protocol_controls_and_rdp_tunnel_hint`

- [ ] **Step 3: Implement protocol form fields, tunnel command rendering, and AdminState ssh port**

Pass the SSH bind port into Admin Web state, accept `protocol` in asset forms, clear credential IDs for non-SSH assets, and render read-only tunnel commands for RDP/TCP assets.

- [ ] **Step 4: Run focused tests to verify GREEN**

Run: `cargo test -p hop-server admin::html::tests::assets_page_renders_protocol_controls_and_rdp_tunnel_hint`

### Task 3: Import, Export, CLI Compatibility, And Docs

**Files:**
- Modify: `crates/hop-server/src/admin/transfer.rs`
- Modify: `crates/hop-server/src/admin/local_cli.rs`
- Modify: `crates/hop-server/src/main.rs`
- Modify: `README.md`
- Modify: `README-EN.md`

- [ ] **Step 1: Write failing transfer and CLI tests**

Add tests for CSV/JSON protocol round-trips and `hop-server asset add --protocol rdp`.

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test -p hop-server protocol`

- [ ] **Step 3: Implement protocol round-trip and docs**

Append `protocol` to asset CSV, accept old six-column CSV rows as `ssh`, add a `--protocol` option to the server-side asset command, and document Admin Web plus standard SSH tunnel usage.

- [ ] **Step 4: Run final verification**

Run: `cargo fmt --check`

Run: `cargo test`
