CREATE TABLE IF NOT EXISTS admin_users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    auth_source   TEXT NOT NULL DEFAULT 'local',
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_login_at TIMESTAMP
);

INSERT INTO admin_users (id, username, display_name, auth_source, is_active)
VALUES ('local-admin', 'admin', 'Local admin', 'local', TRUE)
ON CONFLICT(id) DO NOTHING;

CREATE TABLE IF NOT EXISTS audit_events (
    id           TEXT PRIMARY KEY,
    occurred_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    actor_id     TEXT REFERENCES admin_users(id) ON DELETE SET NULL,
    actor_label  TEXT NOT NULL,
    action       TEXT NOT NULL,
    target_type  TEXT NOT NULL,
    target_id    TEXT,
    target_label TEXT,
    result       TEXT NOT NULL,
    source_ip    TEXT,
    details_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_events_occurred_at
ON audit_events(occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_events_action_result
ON audit_events(action, result);

CREATE INDEX IF NOT EXISTS idx_audit_events_target
ON audit_events(target_type, target_id);

CREATE TABLE IF NOT EXISTS asset_health (
    asset_id         TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    status           TEXT NOT NULL DEFAULT 'unknown'
                     CHECK(status IN ('unknown', 'healthy', 'failed')),
    checked_at       TIMESTAMP,
    last_success_at  TIMESTAMP,
    latency_ms       INTEGER,
    error_code       TEXT,
    error_message    TEXT
);

INSERT OR IGNORE INTO asset_health (asset_id)
SELECT id FROM assets;

CREATE TRIGGER IF NOT EXISTS trg_assets_create_health
AFTER INSERT ON assets
BEGIN
    INSERT OR IGNORE INTO asset_health (asset_id)
    VALUES (NEW.id);
END;

CREATE TRIGGER IF NOT EXISTS trg_assets_delete_health
AFTER DELETE ON assets
BEGIN
    DELETE FROM asset_health WHERE asset_id = OLD.id;
END;
