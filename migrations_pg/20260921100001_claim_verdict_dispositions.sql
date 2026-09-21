-- ABOUTME: Support disposition on a claim verdict — was the flag a true catch, a false positive, or unsure (PostgreSQL)
-- ABOUTME: Five nullable columns; the pipeline never writes them, only an admin's PUT through the triage drawer does
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the SQLite mirror for why the disposition lives on the row: the
-- false-positive rate by layer and by reason is a GROUP BY over these columns,
-- and a side table would give the rollup a join for nothing. `disposed_at` is
-- TIMESTAMPTZ to match `created_at`, which is what the day buckets are cut from.

ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS disposition VARCHAR(32) CHECK (disposition IN ('true_catch', 'false_positive', 'unsure'));
ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS disposition_reason VARCHAR(32) CHECK (disposition_reason IN ('missing_keyword', 'bound_too_tight', 'tolerance_too_tight', 'stale_evidence', 'extractor_misroute', 'judge_error', 'other'));
ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS disposition_note TEXT;
ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS disposed_by VARCHAR(255);
ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS disposed_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_claim_verdicts_tenant_disposition
    ON claim_verdicts(tenant_id, disposition, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_message
    ON claim_verdicts(message_id)
    WHERE message_id IS NOT NULL;
