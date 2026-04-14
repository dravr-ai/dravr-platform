-- ABOUTME: Tier 5.5 claim verification — verdicts emitted by the bullshit detector pipeline
-- ABOUTME: Stores post-LLM claim checks with category, evidence strength, and explanation

CREATE TABLE IF NOT EXISTS claim_verdicts (
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
        'injury_rehab'
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
        'evidence',
        'consistency',
        'judge'
    )),
    explanation TEXT,
    evidence_refs TEXT,
    created_at TEXT NOT NULL
);

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
