-- ABOUTME: short_links table — opaque high-entropy codes that 302-redirect to a full URL (SQLite)
-- ABOUTME: Backs the channel-agnostic URL shortener for chat reconnect/connect links (dot-free, WhatsApp-clickable)

-- The hosted reconnect/connect links carry a signed link-token JWT in the query
-- string. The JWT's dots make WhatsApp truncate linkification mid-token, so the
-- user can only tap a fragment of the URL. This table maps a short, dot-free code
-- to the full destination so chat surfaces can hand out `<base>/r/<code>` instead.
--
-- `code` is a caller-supplied url-safe token (uuid simple, 32 hex chars → 122-bit
-- entropy) so enumeration is infeasible — the redirect is public (the recipient
-- clicks before any auth), and the JWT inside `target_url` remains the real gate.
-- `expires_at` is a Unix epoch (seconds) so the TTL comparison is integer-pure and
-- identical across SQLite and PostgreSQL (no datetime-format ambiguity).
-- tenant_id / user_id are audit columns only; resolution is by code + expiry.

CREATE TABLE IF NOT EXISTS short_links (
    code        TEXT PRIMARY KEY,
    target_url  TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    expires_at  INTEGER NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Resolution filters on `expires_at`; a background sweep deletes expired rows.
CREATE INDEX IF NOT EXISTS idx_short_links_expires_at ON short_links(expires_at);
