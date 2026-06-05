-- ABOUTME: Drops the dead generic `insights` analytics table (superseded by
-- ABOUTME: shared_insights); its store/get methods had no callers and were removed.

-- The `insights` table (20250120000003) backed a generic per-activity analytics
-- insight store whose only consumers were self-referential dead methods in
-- database/analytics.rs (store_insight*/get_user_insights*), now deleted. The
-- live "insights" feature is shared_insights (social). DROP IF EXISTS removes the
-- table + its indexes; no-op where already absent.
DROP TABLE IF EXISTS insights;
