-- ABOUTME: coach_artefacts — the flavour, skeleton and workout files a coach package ships beside its prompt (PostgreSQL)
-- ABOUTME: One row per (coach, kind, slug) holding the source text the kernel validated; the resolver re-parses it

-- The second authorship tier of the training catalogue. contremaitre's
-- training/ tree is the first (shared by every coach, hot-reloaded into the
-- registry); an athlete's own workout_templates row is the third. A coach
-- package sits between: files laid beside the coach's <locale>.md, seeded
-- with the coach, and lent to a plan only while the coach's store listing
-- is published. Resolution is by slug, package over catalogue over
-- compiled-in.
--
-- The text is stored, not a decomposed shape, so dravr-cageux stays the one
-- parser: the seeder validated exactly what the resolver re-parses.

CREATE TABLE IF NOT EXISTS coach_artefacts (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('flavour', 'skeleton', 'workout')),
    slug TEXT NOT NULL,
    content TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (coach_id, kind, slug)
);

CREATE INDEX IF NOT EXISTS idx_coach_artefacts_coach ON coach_artefacts(tenant_id, coach_id);
