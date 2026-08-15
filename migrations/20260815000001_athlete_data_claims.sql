-- ABOUTME: Add the 'athlete_data' claim category and its matching verdict layer (SQLite)
-- ABOUTME: SQLite cannot ALTER a CHECK constraint, so rebuild the table preserving rows + indexes

-- The six existing categories are all sports-science propositions, checked
-- against a literature corpus. A claim about *this athlete* — "your longest run
-- last month was 21 km" — is a database question, not a literature one, and had
-- no category to land in: the extractor's filter_map dropped anything that did
-- not parse into one of the six, so a fabricated personal fact never entered
-- the pipeline at all.
--
-- 'athlete_data' is that seventh category, and 'athlete_data' as a layer_fired
-- value is the layer that adjudicates it against the athlete's own records.

CREATE TABLE IF NOT EXISTS claim_verdicts_new (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL,
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE CASCADE,
    message_id TEXT,
    claim_text TEXT NOT NULL,
    category TEXT NOT NULL CHECK (category IN (
        'physiological',
        'training_prescription',
        'nutrition',
        'recovery',
        'supplement',
        'injury_rehab',
        'athlete_data'
    )),
    status TEXT NOT NULL CHECK (status IN (
        'supported',
        'unsupported',
        'contradicted',
        'rhetorical',
        'unverifiable'
    )),
    evidence_strength TEXT NOT NULL CHECK (evidence_strength IN (
        'strong',
        'mixed',
        'weak',
        'none'
    )),
    confidence REAL NOT NULL DEFAULT 0.0,
    layer_fired TEXT NOT NULL CHECK (layer_fired IN (
        'rhetoric',
        'deterministic',
        'personalized',
        'athlete_data',
        'evidence',
        'consistency',
        'judge'
    )),
    explanation TEXT,
    evidence_refs TEXT,
    created_at TEXT NOT NULL
);

INSERT INTO claim_verdicts_new
    SELECT id, tenant_id, user_id, coach_id, conversation_id, message_id, claim_text,
           category, status, evidence_strength, confidence, layer_fired, explanation,
           evidence_refs, created_at
    FROM claim_verdicts;

DROP TABLE IF EXISTS claim_verdicts;
ALTER TABLE claim_verdicts_new RENAME TO claim_verdicts; -- idempotency-ok: the rebuild's final step; the source table is dropped immediately above, so a re-run finds no claim_verdicts_new to rename

CREATE INDEX IF NOT EXISTS idx_claim_verdicts_tenant_user_created
    ON claim_verdicts(tenant_id, user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_conversation
    ON claim_verdicts(conversation_id)
    WHERE conversation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_category_status
    ON claim_verdicts(tenant_id, category, status);
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_coach
    ON claim_verdicts(coach_id, created_at DESC)
    WHERE coach_id IS NOT NULL;
