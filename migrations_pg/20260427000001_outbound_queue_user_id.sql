-- ABOUTME: Phase 4 — messaging_outbound_queue gains user_id so dead-letter analytics carry a real distinct_id
-- ABOUTME: Replaces the entry_id-as-distinct_id placeholder used by the previous PostHog dead-letter writer
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE messaging_outbound_queue
    ADD COLUMN user_id UUID;

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_user
    ON messaging_outbound_queue(user_id);
