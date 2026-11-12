PRAGMA foreign_keys = ON;

CREATE TABLE hop_schema (
    singleton_id  INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    schema_version TEXT NOT NULL CHECK (schema_version = 'hop/v0.2')
);

INSERT INTO hop_schema (singleton_id, schema_version)
VALUES (1, 'hop/v0.2');

CREATE TABLE catalog_meta (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    revision     INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
);

INSERT INTO catalog_meta (singleton_id, revision)
VALUES (1, 0);

CREATE TABLE credentials (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    username        TEXT NOT NULL,
    auth_type       TEXT NOT NULL CHECK (auth_type IN ('password', 'ssh_key')),
    password_enc    TEXT,
    private_key_enc TEXT,
    passphrase_enc  TEXT,
    secret_hmac     TEXT,
    created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE assets (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL UNIQUE,
    asset_type           TEXT NOT NULL CHECK (asset_type IN ('ssh', 'tcp')),
    host                 TEXT NOT NULL,
    port                 INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    display_name         TEXT,
    description          TEXT,
    tags_json            TEXT,
    credential_id        TEXT REFERENCES credentials(id) ON DELETE RESTRICT,
    via_asset_id         TEXT REFERENCES assets(id) ON DELETE RESTRICT,
    allow_tcp_forwarding INTEGER NOT NULL DEFAULT 1 CHECK (allow_tcp_forwarding IN (0, 1)),
    created_at           TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at           TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (asset_type = 'ssh' OR credential_id IS NULL)
);

CREATE INDEX idx_assets_host_port ON assets(host, port);
CREATE INDEX idx_assets_via_asset_id ON assets(via_asset_id);

CREATE TABLE access_keys (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    public_key  TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    access_mode TEXT NOT NULL DEFAULT 'all' CHECK (access_mode IN ('all', 'restricted')),
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE access_key_assets (
    key_id   TEXT NOT NULL REFERENCES access_keys(id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    PRIMARY KEY (key_id, asset_id)
);

CREATE INDEX idx_access_key_assets_asset_id ON access_key_assets(asset_id);

CREATE TABLE resource_ownership (
    resource_type     TEXT NOT NULL CHECK (resource_type IN ('credential', 'asset', 'access_key')),
    resource_id       TEXT NOT NULL,
    management_mode   TEXT NOT NULL CHECK (management_mode IN ('local', 'declarative')),
    source_id         TEXT,
    source_key        TEXT,
    source_generation INTEGER,
    last_applied_hash TEXT,
    last_applied_at   TEXT,
    orphaned_at       TEXT,
    PRIMARY KEY (resource_type, resource_id),
    UNIQUE (resource_type, source_id, source_key),
    CHECK (
        (management_mode = 'local' AND source_id IS NULL AND source_key IS NULL)
        OR
        (management_mode = 'declarative' AND source_id IS NOT NULL AND source_key IS NOT NULL)
    )
);

CREATE INDEX idx_resource_ownership_source
ON resource_ownership(source_id, resource_type, source_key);

CREATE TABLE config_sources (
    source_id             TEXT PRIMARY KEY,
    generation            INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
    last_success_at       TEXT,
    last_success_revision INTEGER,
    last_error_at         TEXT,
    last_error_code       TEXT,
    last_error_message    TEXT
);

CREATE TABLE known_hosts (
    hostname    TEXT NOT NULL,
    port        INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
    key_type    TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (hostname, port, key_type)
);

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    key_id      TEXT,
    key_finger  TEXT NOT NULL,
    key_name    TEXT,
    mode        TEXT NOT NULL,
    asset_id    TEXT,
    asset_name  TEXT,
    target_host TEXT,
    target_port INTEGER,
    client_ip   TEXT,
    status      TEXT NOT NULL DEFAULT 'started',
    error       TEXT,
    started_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ended_at    TEXT
);

CREATE INDEX idx_sessions_started_at ON sessions(started_at DESC);

CREATE TABLE asset_health (
    asset_id             TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'unknown' CHECK (status IN ('unknown', 'healthy', 'failed')),
    checked_at           TEXT,
    last_success_at      TEXT,
    latency_ms           INTEGER,
    error_code           TEXT,
    error_message        TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    alert_firing         INTEGER NOT NULL DEFAULT 0 CHECK (alert_firing IN (0, 1)),
    last_alerted_at      TEXT,
    last_recovered_at    TEXT
);

CREATE TRIGGER assets_create_health
AFTER INSERT ON assets
BEGIN
    INSERT OR IGNORE INTO asset_health (asset_id) VALUES (NEW.id);
END;

CREATE TABLE asset_probe_samples (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id   TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    checked_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    status     TEXT NOT NULL CHECK (status IN ('healthy', 'failed')),
    latency_ms INTEGER,
    error_code TEXT
);

CREATE INDEX idx_asset_probe_samples_asset_id
ON asset_probe_samples(asset_id, id DESC);

CREATE TABLE probe_settings (
    singleton_id       INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    enabled            INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    interval_seconds   INTEGER NOT NULL DEFAULT 300 CHECK (interval_seconds BETWEEN 30 AND 86400),
    timeout_ms         INTEGER NOT NULL DEFAULT 4000 CHECK (timeout_ms BETWEEN 500 AND 15000),
    failure_threshold  INTEGER NOT NULL DEFAULT 2 CHECK (failure_threshold BETWEEN 1 AND 10),
    notify_on_recovery INTEGER NOT NULL DEFAULT 1 CHECK (notify_on_recovery IN (0, 1)),
    updated_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO probe_settings (singleton_id) VALUES (1);

CREATE TABLE notification_channels (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL CHECK (kind IN ('telegram', 'bark', 'webhook')),
    enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    config_enc TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE audit_events (
    id           TEXT PRIMARY KEY,
    occurred_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    actor_label  TEXT NOT NULL,
    action       TEXT NOT NULL,
    target_type  TEXT NOT NULL,
    target_id    TEXT,
    target_label TEXT,
    result       TEXT NOT NULL,
    source_ip    TEXT,
    details_json TEXT
);

CREATE INDEX idx_audit_events_occurred_at ON audit_events(occurred_at DESC);
