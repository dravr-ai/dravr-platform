-- ABOUTME: Adds agent_translations.sample_prompts, the per-locale Example Inputs an agent's <locale>.md declares
-- ABOUTME: NULL leaves the English samples visible; a JSON array replaces them for that locale
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An agent bound into a thread opens it with a few starter questions taken
-- from its sample prompts (carnet#735). The overlay translated the title and
-- description but not the samples; the seeder stores each locale's Example
-- Inputs here.
ALTER TABLE agent_translations ADD COLUMN IF NOT EXISTS sample_prompts TEXT;
