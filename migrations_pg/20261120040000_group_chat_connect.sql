-- ABOUTME: Connect an existing coaching group to a messaging group chat from the web app (carnet#1683) (PostgreSQL version)
-- ABOUTME: The bot tenant that serves the chat, the chat's title, a single-use connect code, the one-time non-member prompt
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A group bound to a chat used to be one the messaging ingress created on the
-- chat's first message, under the bot's own tenant, so `tenant_id` named both
-- the group's owner tenant and the tenant whose bot serves the chat. A group
-- made in the web app lives under its creator's tenant, while the chat is
-- served by the bot of another one, so the binding now records the serving
-- tenant itself: `channel_tenant_id`. NULL keeps the old meaning (the group's
-- own `tenant_id` serves the chat), so every auto-created group reads exactly
-- as before and a rollback deploy still finds them.
--
-- `channel_connected_at` marks a binding someone made deliberately from the
-- web app. Such a chat is the group's room for its members only: a sender who
-- is not a member is never enrolled by writing in it (carnet#1167), and is
-- told once how to join (`group_chat_invite_prompts`). An auto-created group
-- leaves it NULL and keeps enrolling every linked sender, as it always did.

ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS channel_tenant_id TEXT;
ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS channel_chat_title TEXT;
ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS channel_connected_at TIMESTAMPTZ;

-- One chat of one channel, served by one bot tenant, belongs to at most one
-- group. The serving tenant is the stored one, or the group's own when none
-- is stored, which is what the ingress looks the chat up by.
CREATE UNIQUE INDEX IF NOT EXISTS idx_coaching_groups_served_chat
    ON coaching_groups(COALESCE(channel_tenant_id, tenant_id), channel_type, channel_chat_id)
    WHERE channel_type IS NOT NULL AND channel_chat_id IS NOT NULL;

-- A short-lived, single-use code a group's owner or coach mints in the web
-- app and the bot redeems in the chat to connect (`/start <code>` from the
-- add-to-group link, or `/group link <code>`). `channel_tenant_id` is the
-- tenant whose bot will see the code, so only that bot can redeem it.
CREATE TABLE IF NOT EXISTS group_chat_connect_codes (
    id                TEXT NOT NULL PRIMARY KEY,
    code              TEXT NOT NULL UNIQUE,
    group_id          UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    channel_type      TEXT NOT NULL,
    channel_tenant_id TEXT NOT NULL,
    created_by        UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at        TIMESTAMPTZ NOT NULL,
    used_at           TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_group_chat_connect_codes_group
    ON group_chat_connect_codes(group_id);

-- A sender in a connected chat who is not a member of its group, told once
-- how to join. Keyed by the platform's sender id because such a sender may
-- have no Dravr account at all; nothing else about them is stored, and the
-- row goes with the group.
CREATE TABLE IF NOT EXISTS group_chat_invite_prompts (
    group_id        UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    channel_type    TEXT NOT NULL,
    channel_user_id TEXT NOT NULL,
    prompted_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (group_id, channel_type, channel_user_id)
);

-- A chat whose connected group was disconnected from the web app. Without it
-- the next message there would auto-create a fresh group for the chat, which
-- is exactly what disconnecting must not do: the chat stays ungrouped until
-- someone connects it again, which deletes the row.
CREATE TABLE IF NOT EXISTS group_chat_releases (
    channel_tenant_id TEXT NOT NULL,
    channel_type      TEXT NOT NULL,
    channel_chat_id   TEXT NOT NULL,
    released_at       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (channel_tenant_id, channel_type, channel_chat_id)
);
