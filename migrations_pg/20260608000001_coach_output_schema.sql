-- ABOUTME: Adds output_schema column to coaches for structured-output builder coaches.
-- ABOUTME: Identifies coaches whose replies are extracted, schema-validated, and rendered as cards.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- Add output_schema column for structured plan rendering
-- ============================================================================

-- Identifier of the structured-output schema the coach emits (e.g.
-- "structured-workout"). When set, the chat pipeline appends the
-- structured-output contract to the system prompt, extracts the JSON object
-- from the reply, validates it against the named schema, and renders a plan
-- card instead of leaking raw JSON. NULL for free-text coaches.
ALTER TABLE coaches ADD COLUMN IF NOT EXISTS output_schema TEXT;
