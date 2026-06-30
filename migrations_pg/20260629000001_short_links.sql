-- ABOUTME: short_links table — opaque high-entropy codes that 302-redirect to a full URL (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native types (BIGINT epoch, TIMESTAMPTZ created_at)

-- See the matching SQLite migration for the full rationale. Summary: the hosted
-- reconnect/connect links carry a signed link-token JWT whose dots break WhatsApp
-- linkification, so chat surfaces hand out a short, dot-free `<base>/r/<code>` that
-- this table resolves back to the full destination URL.
--
-- `code` is a caller-supplied url-safe token (uuid simple, 32 hex chars → 122-bit
-- entropy); `expires_at` is a Unix epoch (seconds) so the TTL comparison stays
-- integer-pure and identical to SQLite. tenant_id / user_id are TEXT audit columns
-- (resolution is by code + expiry, never by tenant) — kept as TEXT to dodge the
-- VARCHAR-tenant-id-decoded-as-native-UUID bind trap and because they are
-- write-only here.

CREATE TABLE IF NOT EXISTS short_links (
    code        TEXT PRIMARY KEY,
    target_url  TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    expires_at  BIGINT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Resolution filters on `expires_at`; a background sweep deletes expired rows.
CREATE INDEX IF NOT EXISTS idx_short_links_expires_at ON short_links(expires_at);
