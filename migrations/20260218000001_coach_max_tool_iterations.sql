-- ABOUTME: Add max_tool_iterations column to coaches table
-- ABOUTME: Allows per-coach override of the global MAX_TOOL_ITERATIONS limit
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE coaches ADD COLUMN max_tool_iterations INTEGER DEFAULT NULL;
