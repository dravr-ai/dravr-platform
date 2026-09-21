-- ABOUTME: Support disposition on a claim verdict — was the flag a true catch, a false positive, or unsure (SQLite)
-- ABOUTME: Five nullable columns; the pipeline never writes them, only an admin's PUT through the triage drawer does
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The pipeline's verdict is immutable once written. What support adds on top
-- is a judgement about the verdict itself: an athlete disputed a reply, someone
-- read the flagged claim, and decided whether the detector was right. That
-- judgement, its reason and who made it are what the aggregate health numbers
-- (false-positive rate by layer, by reason) are computed from, so they live on
-- the row rather than in a side table nothing joins to.
--
-- `disposition_reason` is the knob engineering moves: a missing keyword is a
-- corpus edit, a bound too tight is the deterministic table, a tolerance too
-- tight is the personalized margin, and so on. The CHECK lists are the same
-- vocabulary `VerdictDisposition` and `DispositionReason` parse in pierre-memory.

ALTER TABLE claim_verdicts ADD COLUMN disposition TEXT CHECK (disposition IN ('true_catch', 'false_positive', 'unsure'));  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE claim_verdicts ADD COLUMN disposition_reason TEXT CHECK (disposition_reason IN ('missing_keyword', 'bound_too_tight', 'tolerance_too_tight', 'stale_evidence', 'extractor_misroute', 'judge_error', 'other'));  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE claim_verdicts ADD COLUMN disposition_note TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE claim_verdicts ADD COLUMN disposed_by TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE claim_verdicts ADD COLUMN disposed_at TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The health rollup groups flagged rows by disposition inside a window, and
-- the triage list filters on it; both start from the tenant.
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_tenant_disposition
    ON claim_verdicts(tenant_id, disposition, created_at DESC);
-- Support's entry point is the message the athlete disputed.
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_message
    ON claim_verdicts(message_id)
    WHERE message_id IS NOT NULL;
