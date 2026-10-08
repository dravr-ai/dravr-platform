-- ABOUTME: One row per (tenant, athlete, use-case starter): how often a welcome showed it, and when it was last tapped
-- ABOUTME: The starter ranker drops a once-only starter after its tap and any starter shown three times untapped (carnet#828)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A welcome that offers a starter counts it shown; a tap resets the count and
-- stamps tapped_at_ms. use_case_id is the catalogue id from dravr-contremaitre's
-- use_cases/catalogue.yaml; a row for an id the catalogue no longer lists is
-- simply never read. Instants are epoch milliseconds, identical on both engines.
CREATE TABLE IF NOT EXISTS use_case_exposures (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    use_case_id TEXT NOT NULL,
    shown_count BIGINT NOT NULL DEFAULT 0,
    last_shown_at_ms BIGINT,
    tapped_at_ms BIGINT,
    PRIMARY KEY (tenant_id, user_id, use_case_id)
);
