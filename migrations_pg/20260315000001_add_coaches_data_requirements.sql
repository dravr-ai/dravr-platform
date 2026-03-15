-- ABOUTME: Adds data_requirements column to coaches table
-- ABOUTME: Structured JSON field for deterministic activity pre-fetching

ALTER TABLE coaches ADD COLUMN IF NOT EXISTS data_requirements TEXT;
