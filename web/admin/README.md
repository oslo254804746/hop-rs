# Hop Admin Web

This directory is the companion static-asset workspace for Admin Web.

The production Admin pages are currently rendered by Rust/Maud in
`crates/hop-server/src/admin/html.rs`. The Rust server also owns routing,
authentication, cookies, CSRF, authorization, and database access. This Vite
workspace establishes a separately buildable frontend boundary without adding
another process or static server to deployment.

Build output goes to:

```text
web/admin/dist
```

`hop-server` serves that directory at:

```text
/admin-static/*
```

`hop-server` serves the generated files below `/admin-static/*`. The current
`src/main.ts` page is a pipeline smoke target, not the production Admin
Dashboard.

## Commands

```bash
npm ci
npm run dev
npm run build
npm run preview
```

`npm run build` writes static assets into `dist/`, which can be packaged with
`hop-server`. CI uses Node.js 24, runs this build before Rust checks, and
packages `dist/` as `hop-admin-web-static.tar.gz` for tagged releases.

## Where to make changes

- Server-rendered pages and inline behavior:
  `crates/hop-server/src/admin/html.rs`
- Admin routes, authorization, audit recording, and form handling:
  `crates/hop-server/src/admin/routes.rs`
- Translations: `crates/hop-server/src/admin/i18n.rs`
- Release-specific Admin styles:
  `crates/hop-server/src/admin/release_*.css`
- Static build-boundary assets: `web/admin/src/`

Run both validation paths after changing Admin UI code:

```bash
cd web/admin
npm ci
npm run build

cd ../..
cargo test --workspace --locked
```
