-- ABOUTME: One row per persona-held notification a digest returned, keyed by that notification
-- ABOUTME: The server-side record of what each digest handed back, which no user action can delete
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An armed persona floor persists a push it withholds, and a digest on the
-- persona's cadence hands it back. This table records which digest handed
-- back which held row. It is the digest's own record rather than a field on
-- the digest notification, because an athlete can delete that notification
-- from their feed, and deleting it must not make its rows due again.
--
-- notification_id is the held row, so the primary key is what makes a row
-- returned exactly once: two digests racing for it (two sessions landing at
-- once, two instances ticking) cannot both claim it. The row goes with the
-- held notification. batch_id names one digest's claim, released when that
-- digest did not go out. returned_at_ms is epoch milliseconds, identical on
-- both engines; the newest one is when the recipient's last digest went out.
CREATE TABLE IF NOT EXISTS persona_digest_returns (
    notification_id UUID NOT NULL PRIMARY KEY REFERENCES notifications(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL,
    batch_id UUID NOT NULL,
    returned_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_persona_digest_returns_recipient
    ON persona_digest_returns(user_id, tenant_id, returned_at_ms);

CREATE INDEX IF NOT EXISTS idx_persona_digest_returns_batch
    ON persona_digest_returns(batch_id);
