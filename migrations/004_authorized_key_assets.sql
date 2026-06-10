ALTER TABLE authorized_keys
ADD COLUMN asset_access_mode TEXT NOT NULL DEFAULT 'all'
CHECK (asset_access_mode IN ('all', 'restricted'));

CREATE TABLE authorized_key_assets (
    key_id   TEXT NOT NULL REFERENCES authorized_keys(id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    PRIMARY KEY (key_id, asset_id)
);

CREATE INDEX idx_authorized_key_assets_asset_id
ON authorized_key_assets(asset_id);
