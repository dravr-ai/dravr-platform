-- ABOUTME: Mark messaging channel configs that serve the whole deployment rather than one tenant
-- ABOUTME: The env-seeded bot is platform scope for every tenant; links get a by-user index for the athlete read
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A config row is stored under one tenant, but the deployment's own bot (the
-- one seeded from TELEGRAM_BOT_TOKEN / META_WHATSAPP_* / ...) answers every
-- athlete, whatever tenant they live in. `platform_scope = 1` records that:
-- the row is readable by every tenant that has not configured its own bot for
-- the same channel. Rows a tenant writes through the API stay 0.
ALTER TABLE messaging_channel_configs ADD COLUMN platform_scope INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

CREATE INDEX IF NOT EXISTS idx_messaging_channel_configs_platform
    ON messaging_channel_configs(channel_type) WHERE platform_scope = 1;

-- A channel link names the person it belongs to, so the athlete's own
-- Settings reads their links by `user_id` across tenants: a link made through
-- the deployment bot lives under the bot's tenant, not the athlete's.
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_user_only
    ON messaging_channel_links(user_id);
