-- ABOUTME: One-time backfill: flip peer_data_sharing=true on auto-bound coaching groups
-- ABOUTME: Aligns existing channel-bound groups with the documented auto-bind default
--
-- See migrations_pg/20260510000001_backfill_auto_bound_peer_data_sharing.sql
-- for the rationale. SQLite stores BOOLEAN as INTEGER (0/1) and lacks NOW();
-- use strftime to match the iso8601 format the app uses elsewhere.
UPDATE coaching_groups
   SET peer_data_sharing = 1,
       updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE channel_type IS NOT NULL
   AND peer_data_sharing = 0;
