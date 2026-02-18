-- ABOUTME: Adds usage_quotas configuration category for quota enforcement parameters
-- ABOUTME: Seeds 10 quota parameters controlling message caps, tool limits, and token budgets
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

INSERT OR IGNORE INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('cat_usage_quotas', 'usage_quotas', 'Usage Quotas', 'Per-user and per-tenant usage limits for messages, tools, and tokens', 96, 'shield');
