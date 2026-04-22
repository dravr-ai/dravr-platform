-- ABOUTME: Add temperature column to coaches table (PostgreSQL)
-- ABOUTME: Per-coach LLM temperature override; NULL means use provider/server default
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE coaches ADD COLUMN temperature DOUBLE PRECISION DEFAULT NULL;
