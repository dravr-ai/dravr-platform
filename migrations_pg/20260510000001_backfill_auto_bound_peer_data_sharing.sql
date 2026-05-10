-- ABOUTME: One-time backfill: flip peer_data_sharing=true on auto-bound coaching groups
-- ABOUTME: Aligns existing channel-bound groups with the documented auto-bind default
--
-- Context: messaging_group_bind::resolve_or_create_channel_group sets
-- peer_data_sharing=true on creation so that an individual member running
-- /group consent yes immediately surfaces their data to peers without an
-- extra owner action. Pre-fix REST POST /api/groups created groups with
-- peer_data_sharing=false; some auto-bound rows in production still sit
-- at false (created earlier, or modified). This backfill restores the
-- documented default for any group with a channel binding so that
-- already-consenting members start surfacing peer data immediately.
--
-- Scope: only rows with channel_type IS NOT NULL — REST-created groups
-- without a channel binding are intentionally excluded; their owner sets
-- peer_data_sharing through Group Settings deliberately and we don't
-- want to silently flip their kill switch.
UPDATE coaching_groups
   SET peer_data_sharing = TRUE,
       updated_at = NOW()
 WHERE channel_type IS NOT NULL
   AND peer_data_sharing = FALSE;
