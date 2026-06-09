ALTER TABLE assets ADD COLUMN preset TEXT;

UPDATE assets
SET protocol = 'tcp', preset = 'rdp'
WHERE protocol = 'rdp';
