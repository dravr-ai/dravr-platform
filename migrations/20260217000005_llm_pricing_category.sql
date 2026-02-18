-- ABOUTME: Admin config category for per-model LLM token pricing overrides
-- ABOUTME: Allows admins to adjust pricing rates without code changes
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

INSERT OR IGNORE INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('cat_llm_pricing', 'llm_pricing', 'LLM Pricing', 'Per-model token pricing for cost tracking and billing', 97, 'dollar-sign');
