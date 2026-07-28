ALTER TABLE admin_users
ADD COLUMN password_hash TEXT;

ALTER TABLE admin_users
ADD COLUMN access_profile TEXT NOT NULL DEFAULT 'owner'
CHECK(access_profile IN ('owner', 'operator', 'viewer'));

ALTER TABLE admin_users
ADD COLUMN must_change_password BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE admin_users
SET password_hash = (
    SELECT value
    FROM settings
    WHERE key = 'admin_password_hash'
)
WHERE id = 'local-admin'
  AND password_hash IS NULL;

CREATE INDEX IF NOT EXISTS idx_admin_users_active_profile
ON admin_users(is_active, access_profile);

CREATE UNIQUE INDEX IF NOT EXISTS idx_admin_users_username_nocase
ON admin_users(username COLLATE NOCASE);
