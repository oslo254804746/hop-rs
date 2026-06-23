# Admin Web CI Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and validate the Admin Web frontend in CI, Docker images, and GitHub release artifacts.

**Architecture:** Treat `web/admin` as a standalone Node/Vite build boundary that produces static files under `web/admin/dist`. CI validates the frontend before Rust release builds, Docker builds the frontend in a dedicated Node stage before compiling Rust, and GitHub releases upload a static asset archive next to the server binary.

**Tech Stack:** GitHub Actions, Docker multi-stage builds, Node.js/npm, Vite, TypeScript, Rust/Cargo.

---

### Task 1: Frontend Dependency Lockfile

**Files:**
- Create: `web/admin/package-lock.json`

- [ ] **Step 1: Generate the lockfile**

Run: `npm install --package-lock-only` from `web/admin`.

Expected: `web/admin/package-lock.json` is created and pins the existing `typescript` and `vite` dependency graph.

- [ ] **Step 2: Verify reproducible install metadata**

Run: `npm ci` from `web/admin`.

Expected: npm installs from the lockfile without changing `package.json`.

### Task 2: CI Frontend Build

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add Node setup to the Rust job**

Add `actions/setup-node` with Node 24 and npm cache scoped to `web/admin/package-lock.json`.

- [ ] **Step 2: Add frontend install and build before Rust build artifacts**

Run `npm ci` and `npm run build` with `working-directory: web/admin` before the Rust release binary build.

- [ ] **Step 3: Verify CI syntax by local inspection**

Run: `git diff --check .github/workflows/ci.yml`

Expected: no whitespace or patch syntax issues.

### Task 3: Docker Frontend Assets

**Files:**
- Modify: `Dockerfile`

- [ ] **Step 1: Add a Node builder stage**

Copy `web/admin/package*.json`, run `npm ci`, copy the frontend source files, and run `npm run build`.

- [ ] **Step 2: Copy built assets into the Rust builder context**

Copy `/src/web/admin/dist` from the Node stage into `web/admin/dist` before `cargo build`.

- [ ] **Step 3: Verify Dockerfile syntax by building**

Run: `docker build .`

Expected: image build completes and includes the server binary.

### Task 4: Release Static Asset Archive

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Build Admin Web in the release job**

Install Node 24, run `npm ci`, and run `npm run build` before preparing release assets.

- [ ] **Step 2: Package static assets**

Create `dist/hop-admin-web-static.tar.gz` from `web/admin/dist` and write a checksum file.

- [ ] **Step 3: Upload archive and checksum**

Add both static asset files to `gh release upload`.

### Task 5: Verification

**Files:**
- Inspect: `.github/workflows/ci.yml`
- Inspect: `Dockerfile`
- Inspect: `web/admin/package-lock.json`

- [ ] **Step 1: Run frontend build**

Run: `npm run build` from `web/admin`.

Expected: TypeScript and Vite build complete successfully.

- [ ] **Step 2: Run Rust tests**

Run: `cargo test --workspace --locked`.

Expected: Rust test suite completes successfully.

- [ ] **Step 3: Run Rust clippy**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`.

Expected: Clippy completes without warnings.
