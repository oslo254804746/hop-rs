# Hop v0.2 Control API and local management

Hop v0.2 has no built-in Admin Web or administrator-account protocol. Management belongs to one trust domain and uses either the local CLI, declarative Apply, or one equal-privilege Control API token.

## Enable the API

The API is disabled by default and creates no HTTP listener. Create a protected high-entropy token file, configure the loopback listener, and restart:

```toml
[api]
enabled = true
listen = "127.0.0.1:8083"
token_file = "/var/lib/hop/control-api.token"
cors_allowlist = []
```

Every request requires:

```http
Authorization: Bearer <token>
```

A non-loopback listener is rejected unless `cors_allowlist` is explicit. Put remote access behind TLS and network controls; CORS alone is not a security boundary.

## Read and operate

Versioned endpoints:

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/status` | health, version, Catalog revision |
| GET | `/api/v1/catalog/revision` | optimistic-concurrency revision |
| GET/POST | `/api/v1/assets` | list/create local assets |
| PUT/DELETE | `/api/v1/assets/{id}` | update/delete one local asset |
| GET/POST | `/api/v1/credentials` | list secret status/create local credentials |
| PUT/DELETE | `/api/v1/credentials/{id}` | update/delete one local credential |
| GET/POST | `/api/v1/access-keys` | list/create local Access Keys |
| DELETE | `/api/v1/access-keys/{id}` | revoke and remove a local key |
| PUT | `/api/v1/access-keys/{id}/enabled` | enable/disable a key |
| PUT | `/api/v1/access-keys/{id}/access` | replace all/restricted asset scope |
| GET | `/api/v1/sessions` | recent sessions |
| POST | `/api/v1/sessions/{id}/terminate` | explicitly terminate an active registered session |
| GET | `/api/v1/config/sources` | source success/failure generations |
| GET | `/api/v1/config/status` | sources, orphans, schema and revision |
| POST | `/api/v1/config/validate` | validate manifest content |
| POST | `/api/v1/config/diff` | read-only manifest diff |
| POST | `/api/v1/config/apply` | atomic manifest apply with required base revision |
| POST | `/api/v1/config/reload` | reload configured inventory sources through the same Apply engine |

Credential responses never contain plaintext, encrypted envelopes, private keys, or passwords. Each secret field is `configured` or `missing`. Access Key responses omit the public-key body and expose only name, fingerprint, state, mode, and assigned asset IDs.

## Local CRUD examples

```json
POST /api/v1/credentials
{
  "name": "root",
  "username": "root",
  "auth_type": "password",
  "password": "request-only-secret"
}
```

Supported `auth_type` values are `password`, `key`, and `key_passphrase`. On update, omitting a secret preserves the existing compatible secret; switching authentication type requires the new type's material.

```json
POST /api/v1/assets
{
  "name": "server",
  "protocol": "ssh",
  "hostname": "192.0.2.10",
  "port": 22,
  "credential_id": "<credential-id>",
  "tags": ["home"]
}
```

TCP assets use `protocol: "tcp"` without an SSH credential. RDP, VNC, MySQL, PostgreSQL, and Redis all use the same TCP asset type.

```json
POST /api/v1/access-keys
{
  "name": "laptop",
  "public_key": "ssh-ed25519 AAAA...",
  "assets": []
}
```

For key create/access updates, omitting `assets` means all assets, `[]` means none, and a non-empty list is a strict list of internal asset IDs.

Local CRUD can modify only resources whose ownership is `local`. A declarative resource returns HTTP 409 with code `managed_by_source`; change its owning manifest and apply it instead. Unique-name/reference conflicts are also reported without exposing database or secret details.

## Validate, diff, and apply

The API accepts manifest content rather than arbitrary server-side paths:

```json
POST /api/v1/config/diff
{
  "content": "api_version: hop/v1alpha1\nassets: {}\n",
  "format": "yaml",
  "source_id": "panel",
  "prune": false
}
```

Apply additionally requires the revision returned by `/api/v1/catalog/revision`:

```json
POST /api/v1/config/apply
{
  "content": "api_version: hop/v1alpha1\nassets: {}\n",
  "format": "yaml",
  "source_id": "panel",
  "base_revision": 12,
  "prune": false,
  "dry_run": false
}
```

An outdated base returns `409 revision_conflict`. Validation and apply errors use stable codes and resource paths. Failed writes record a non-sensitive source/audit summary but do not partially modify resources. Dry-run does not write Catalog state.

## Session behavior

Disabling a key, narrowing an allowlist, changing a credential, or deleting an asset affects new connections. Existing SSH streams are not implicitly killed. Use the explicit termination endpoint when an active session must be interrupted immediately.

## Panel availability and boundary

v0.2.0 does not currently ship an official graphical interface for Catalog resources. `hop-rs` removed the old Admin Web, while the current `luci-app-hop` page covers only the OpenWrt service shell, core download, and logging; it does not call the Control API. Use the local CLI, manifests, or `/api/v1` to manage assets, credentials, Access Keys, and allowlists today.

The planned standalone panel and LuCI resource views must both consume `/api/v1`. They must not read/write SQLite directly or duplicate assets, credentials, and Access Keys in UCI. The core does not serve panel assets and does not depend on a frontend repository. See the [v0.2 management-panel delivery contract](product/management-panel-v0.2.md) for the agreed scope and security boundary.
