# Hop Admin Web

This directory is the standalone frontend workspace for future Admin Web pages.
The Rust admin server still owns authentication, cookies, CSRF, and API routes.

Build output goes to:

```text
web/admin/dist
```

`hop-server` serves that directory at:

```text
/admin-static/*
```

Current server-rendered Maud pages remain active. This workspace establishes the
frontend build boundary without adding a separate static server to deployment.

## Commands

```bash
npm install
npm run dev
npm run build
```

`npm run build` writes static assets into `dist/`, which can be packaged with
`hop-server`.
