-- ABOUTME: Aligns oauth_apps.app_type with the SQLite schema: the OAuth 2.0 client type, 'public' or 'confidential'
-- ABOUTME: The original check allowed only platform labels (desktop/web/mobile/server), so every app insert failed on PostgreSQL

ALTER TABLE oauth_apps DROP CONSTRAINT IF EXISTS oauth_apps_app_type_check;
UPDATE oauth_apps SET app_type = 'confidential'
 WHERE app_type IS NULL OR app_type NOT IN ('public', 'confidential');
ALTER TABLE oauth_apps ALTER COLUMN app_type SET DEFAULT 'public';
ALTER TABLE oauth_apps ALTER COLUMN app_type SET NOT NULL;
ALTER TABLE oauth_apps ADD CONSTRAINT oauth_apps_app_type_check
    CHECK (app_type IN ('public', 'confidential'));
