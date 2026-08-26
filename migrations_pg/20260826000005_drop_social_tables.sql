-- ABOUTME: Drops the five social tables retired by the Chat-First Cutover (Insights + Friends deleted).
-- ABOUTME: Dependents first, then the two trigger functions that only these tables ever fired.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The social graph and coach-mediated insight feed were retired on 2026-08-26
-- (dravr-vault "Chat-First Cutover"; ADR-020 superseded). No code reads or
-- writes these tables any more, so the rows go with the feature rather than
-- lingering as an unexplained cross-tenant dataset. CASCADE takes the indexes,
-- constraints and triggers; the trigger functions are separate objects and
-- are dropped explicitly.
DROP TABLE IF EXISTS adapted_insights CASCADE;
DROP TABLE IF EXISTS insight_reactions CASCADE;
DROP TABLE IF EXISTS shared_insights CASCADE;
DROP TABLE IF EXISTS friend_connections CASCADE;
DROP TABLE IF EXISTS user_social_settings CASCADE;
DROP FUNCTION IF EXISTS update_shared_insight_reaction_count();
DROP FUNCTION IF EXISTS update_shared_insight_adapt_count();
