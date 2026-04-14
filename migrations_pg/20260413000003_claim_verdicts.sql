-- ABOUTME: Tier 5.5 claim verification (PostgreSQL) — verdicts from the bullshit detector pipeline
-- ABOUTME: Stores post-LLM claim checks with category, evidence strength, layer provenance

CREATE TABLE IF NOT EXISTS claim_verdicts (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    coach_id VARCHAR(255) REFERENCES coaches(id) ON DELETE SET NULL,
    conversation_id VARCHAR(255) REFERENCES chat_conversations(id) ON DELETE CASCADE,
    message_id VARCHAR(255),
    claim_text TEXT NOT NULL,
    category VARCHAR(64) NOT NULL CHECK (category IN (
        'physiological',
        'training_prescription',
        'nutrition',
        'recovery',
        'supplement',
        'injury_rehab'
    )),
    status VARCHAR(32) NOT NULL CHECK (status IN (
        'supported',
        'unsupported',
        'contradicted',
        'rhetorical',
        'unverifiable'
    )),
    evidence_strength VARCHAR(16) NOT NULL CHECK (evidence_strength IN (
        'strong',
        'mixed',
        'weak',
        'none'
    )),
    confidence REAL NOT NULL DEFAULT 0.0,
    layer_fired VARCHAR(32) NOT NULL CHECK (layer_fired IN (
        'rhetoric',
        'deterministic',
        'evidence',
        'consistency',
        'judge'
    )),
    explanation TEXT,
    evidence_refs TEXT,
    created_at TIMESTAMPTZ NOT NULL
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
