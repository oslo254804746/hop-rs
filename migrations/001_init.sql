CREATE TABLE IF NOT EXISTS authorized_keys (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    public_key     TEXT NOT NULL UNIQUE,
    fingerprint    TEXT NOT NULL,
    is_active      BOOLEAN NOT NULL DEFAULT TRUE,
    created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_authorized_keys_fingerprint ON authorized_keys(fingerprint);

CREATE TABLE IF NOT EXISTS assets (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    hostname      TEXT NOT NULL,
    port          INTEGER NOT NULL DEFAULT 22,
    description   TEXT,
    tags          TEXT,
    credential_id TEXT REFERENCES credentials(id),
    created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_assets_hostname_port ON assets(hostname, port);

CREATE TABLE IF NOT EXISTS credentials (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    username        TEXT NOT NULL,
    auth_type       TEXT NOT NULL,
    password_enc    TEXT,
    private_key_enc TEXT,
    passphrase_enc  TEXT,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    key_finger  TEXT NOT NULL,
    key_name    TEXT,
    mode        TEXT NOT NULL,
    asset_name  TEXT,
    target_host TEXT,
    target_port INTEGER,
    client_ip   TEXT,
    status      TEXT NOT NULL DEFAULT 'started',
    error       TEXT,
    started_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    ended_at    TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at);

CREATE TABLE IF NOT EXISTS known_hosts (
    hostname    TEXT NOT NULL,
    port        INTEGER NOT NULL DEFAULT 22,
    key_type    TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (hostname, port, key_type)
);

CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);
