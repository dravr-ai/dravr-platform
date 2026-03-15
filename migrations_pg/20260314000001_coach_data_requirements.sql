-- ABOUTME: Adds data_requirements column to coaches for structured pre-fetch configuration.
-- ABOUTME: Enables deterministic activity data fetching instead of LLM-interpreted natural language.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- Add data_requirements column for structured data pre-fetching
-- ============================================================================

-- JSON column specifying what data to deterministically pre-fetch when
-- starting a coach conversation. Separates data fetching (structured)
-- from analysis instructions (natural language startup_query).
-- Schema: { "activities": { "count": 30, "sport_types": ["Run"], ... }, "athlete_profile": true }
ALTER TABLE coaches ADD COLUMN IF NOT EXISTS data_requirements TEXT;
