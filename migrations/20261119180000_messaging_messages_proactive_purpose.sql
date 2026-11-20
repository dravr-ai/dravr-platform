-- ABOUTME: Marks an outbound messaging row a proactive notice wrote, with what the notice said (SQLite)
-- ABOUTME: A WhatsApp status webhook failing with 131047 finds the notice by its wamid and re-delivers it in the app (carnet#1672)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- proactive_purpose  what a notice the platform sent unprompted was saying
--                    (backfill_complete, provider_reauth, check_in, ...), the
--                    purpose messaging.proactive_held reports. Set on the
--                    outbound row of each message such a notice sent on a
--                    channel that fails an out-of-window send only afterwards
--                    (WhatsApp answers 200, then reports error 131047 in a
--                    status webhook). NULL for inbound rows and for replies.
-- Lookups go by (tenant_id, channel_message_id), which the unique
-- idx_messaging_messages_idempotency already serves. Additive and nullable;
-- no backfill: rows written before it were never re-deliverable.
ALTER TABLE messaging_messages ADD COLUMN proactive_purpose TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
