-- ABOUTME: Drops the five social tables retired by the Chat-First Cutover (Insights + Friends deleted).
-- ABOUTME: Dependents first — reactions and adaptations reference shared_insights; SQLite drops their indexes and triggers with them.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The social graph and coach-mediated insight feed were retired on 2026-08-26
-- (dravr-vault "Chat-First Cutover"; ADR-020 superseded). No code reads or
-- writes these tables any more, so the rows go with the feature rather than
-- lingering as an unexplained cross-tenant dataset.
DROP TABLE IF EXISTS adapted_insights;
DROP TABLE IF EXISTS insight_reactions;
DROP TABLE IF EXISTS shared_insights;
DROP TABLE IF EXISTS friend_connections;
DROP TABLE IF EXISTS user_social_settings;
