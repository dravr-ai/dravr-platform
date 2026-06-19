-- ABOUTME: Durable cross-instance idempotency log so the backfill-completion push fires exactly once
-- ABOUTME: The PK (tenant_id, user_id, provider, after_ts) is claimed once; a second claim no-ops
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per completed historical backfill window that was pushed to a channel.
-- The in-process IN_FLIGHT_BACKFILLS set only de-dups the FETCH within a single
-- replica; this table de-dups the PUSH across every replica so a user never gets
-- two "your history is ready" notices for the same (user, provider, window).
-- No FK: it is a pure idempotency log, deliberately decoupled from the data it
-- guards so a churned user/provider row never blocks or cascades the claim.
CREATE TABLE IF NOT EXISTS backfill_push_log (
    tenant_id   TEXT    NOT NULL,
    user_id     TEXT    NOT NULL,
    provider    TEXT    NOT NULL,
    after_ts    BIGINT  NOT NULL,  -- historical-window `after` (unix seconds); 0 when None
    pushed_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (tenant_id, user_id, provider, after_ts)
);
