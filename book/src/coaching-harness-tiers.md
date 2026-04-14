# Coaching Harness — Tier-by-Tier

This page walks through each tier of the coaching harness in depth:
what it does, which files own it, the design decisions, and the
invariants that tests lock in. Start with the
[Overview](coaching-harness-overview.md) if you haven't already.

## Tier 0 — Memory foundations

**Purpose:** Give the harness a typed, tenant-scoped, multi-backend
persistence layer for everything the higher tiers need to store.

**Key files**

- `crates/pierre-memory/src/` — pure types crate (no persistence, no
  LLM dependencies). Exports `CompactionBlock`, `UserFact`,
  `UserFactMetrics`, `CoachNote`, `CoachFollowup`, `CoachSession`,
  `MemoryScope`, `FactKind`, `FollowupStatus`, `SessionStatus`,
  `ClaimVerdict`, `ClaimCategory`, `ClaimStatus`, `EvidenceStrength`,
  `VerdictLayer`.
- `crates/pierre-database/src/repositories.rs` — `HarnessMemoryRepository`
  + `ClaimVerdictRepository` trait definitions.
- `crates/pierre-database/src/database/memory.rs` — SQLite impl.
- `crates/pierre-database/src/plugins/postgres/memory.rs` — PostgreSQL
  impl, byte-for-byte mirror of the SQLite code path.
- `migrations/20260413000002_harness_memory_foundations.sql` +
  `migrations_pg/20260413000002_harness_memory_foundations.sql` —
  schema and indexes.

**Design decisions**

- **Dual-backend from day 1.** Every new table ships with both a
  SQLite migration and a PostgreSQL migration in the same commit.
  The `feedback_pg_awareness.md` memory file documents the rule:
  "never implement SQLite-only; always add the Postgres backend in
  the same PR".
- **Embeddings as BLOB/BYTEA.** Vector columns use raw little-endian
  `f32` sequences, not `pgvector`. Both backends decode via shared
  helpers. This avoids a hard dependency on pgvector while keeping
  the option open to upgrade the column type later without touching
  the repository trait.
- **Parameter structs over positional args.** Repository methods that
  would otherwise take 7+ arguments take a borrowed params struct
  (`UpsertUserFactParams`, `InsertCoachNoteParams`,
  `InsertCoachFollowupParams`, `InsertCompactionBlockParams`,
  `InsertClaimVerdictParams`). This is enforced by clippy's
  `too_many_arguments` lint and documented in
  `repositories.rs`.

**What tests lock in**

- `crates/pierre-database/src/database/memory.rs` unit tests:
  embedding roundtrip, invalid length error.
- `crates/pierre-server/tests/memory_worker_metrics_test.rs`:
  `count_user_facts_metrics` returns zero for empty tenants,
  aggregates by kind, is tenant-scoped.
- `crates/pierre-server/tests/coach_followups_admin_test.rs`: tenant-
  wide listing orders by `due_at ASC NULLS LAST`, cancel is idempotent.
- `crates/pierre-server/tests/coach_notes_audit_test.rs`: tenant-wide
  audit list is newest-first, clamps to limit, tenant-isolated.

## Tier 1 — Conversation compaction

**Purpose:** Keep long conversations under the LLM context window by
summarizing older turns when usage crosses a threshold.

**Key files**

- `crates/pierre-server/src/services/conversation_compaction.rs` —
  `ConversationCompactor`, `CompactionConfig`, sliding-window fallback.
- `crates/pierre-server/src/services/chat_orchestration.rs` at
  `apply_tier1_compaction` — the hook into the dispatch loop.
- `crates/pierre-server/src/routes/admin/harness_config.rs` —
  persistent config knobs exposed via `HarnessConfigTab`.

**Algorithm**

Two strategies layered:

1. **Summary strategy.** When token usage >= `warn_threshold` (default
   70%), summarize the oldest `summarize_oldest_n` turns into a single
   `CompactionBlock` and replace them in the prompt.
2. **Sliding-window fallback.** When usage >= `emergency_threshold`
   (default 95%), drop `sliding_drop_n` oldest turns regardless of
   summarization state.

**Tunables (persisted in `system_settings` under key `harness_config`)**

| Field | Default | Valid range |
|---|---|---|
| `window_tokens` | 128_000 | > 0 |
| `warn_threshold` | 0.70 | `(0.0, 1.0]` |
| `emergency_threshold` | 0.95 | `(0.0, 1.0]`, strictly > `warn_threshold` |
| `summarize_oldest_n` | 6 | > 0 |
| `sliding_drop_n` | 4 | > 0 |

`validate_document` in `harness_config.rs` enforces these invariants
and has 8 tests in `tests/harness_config_validation_test.rs`.

## Tier 2 — Semantic user memory

**Purpose:** Remember what the user has told the coach across turns
and sessions, without stuffing the whole history into every prompt.

**Key files**

- `crates/pierre-llm/src/embeddings.rs` — `EmbeddingProvider` trait +
  `GeminiEmbeddingProvider` (text-embedding-004, 768 dims).
- `crates/pierre-server/src/services/memory_extraction.rs` —
  fire-and-forget `tokio::spawn` worker that runs after each assistant
  turn. Asks an LLM to decompose the turn into structured facts and
  upserts them via `HarnessMemoryRepository::upsert_user_fact`.
- `crates/pierre-server/src/services/memory_recall.rs` —
  `build_user_memory_context` queries the most relevant user facts
  at prompt build time and renders them as a system block.
- `crates/pierre-server/src/services/chat_orchestration.rs` at
  `inject_memory_recall` — the hook that prepends recall context to
  the coach system prompt.

**Admin observability**

Sprint C6 added `MemoryExtractionMonitorTab` which reads per-tenant
aggregates from the `user_facts` table via a new
`HarnessMemoryRepository::count_user_facts_metrics` repository
method. The worker itself is fire-and-forget with no loop state, so
its health is derived from the rows it has actually produced:

- `total_facts` — lifetime count for the tenant
- `facts_last_24h` / `facts_last_7d` — recency signals
- `distinct_users` — coverage
- `facts_by_kind` — category breakdown
- `newest_updated_at` — "is the worker alive?" signal

A tenant with active conversations but `facts_last_24h == 0` is a
stalled-worker signal operators should investigate.

## Tier 3 — Coach-authored memory tools

**Purpose:** Let the coach persona write its own long-term memory as
part of its tool loop, rather than leaving extraction as a purely
LLM-external concern.

**Key files**

- `crates/pierre-server/src/tools/implementations/memory.rs` — four
  new MCP tools: `coach_note_add`, `coach_followup_schedule`,
  `remember_fact`, `recall_user_memory`. All are `McpTool` impls with
  zero trait edits (the trait is wide enough to absorb them cleanly).
- `crates/pierre-server/src/tools/registry.rs` — tool registration
  behind the `tools-memory` feature flag.
- `crates/pierre-server/src/services/chat_orchestration.rs` at
  `inject_pending_followups` — reads pending followups via
  `HarnessMemoryRepository::list_pending_followups` and renders them
  as a system block in the next turn's prompt, then
  `finalize_session_state` marks them delivered after the turn
  succeeds.

**Tool surface**

| Tool | What it persists | When the coach calls it |
|---|---|---|
| `coach_note_add` | `CoachNote` rows | To record a private observation about the user |
| `coach_followup_schedule` | `CoachFollowup` rows | To promise a future check-in ("ask about the achilles tomorrow") |
| `remember_fact` | `UserFact` rows | To capture a durable user preference or constraint |
| `recall_user_memory` | read-only fact lookup | To pull earlier facts into the current turn |

## Tier 4 — Cross-channel coach sessions

**Purpose:** Introduce a logical container above `chat_conversation`
so the coach can maintain a consistent relationship with a user
across messaging channels, HTTP chat, and the mobile client.

**Key files**

- `crates/pierre-server/src/services/chat_orchestration.rs`:
  - `ensure_coach_session_attached` (pre-history): idempotent lookup
    or create for `(tenant, user, coach)`.
  - `finalize_session_state` (post-response): `touch_coach_session`
    and mark surfaced followups delivered.
- `crates/pierre-server/src/routes/admin/coach_followups.rs` — admin
  triage for pending followups (Sprint C7).
- Migration: `20260413000001_reify_coach_id_on_conversations.sql`
  which reifies `coach_id` as a foreign key on `chat_conversations`
  and deletes the legacy `system_prompt` TEXT column. This was a
  load-bearing prerequisite: before the reification, the coach
  identity was smeared into a string at conversation-create time, so
  grouping conversations by coach was impossible.

**The session-hierarchy refactor (Sprint C15)**

Tier 4 ships the backend `coach_sessions` table and dispatch wiring
in commit `71b665eb`. Sprint C15 lifts that model into the frontend:
`ConversationsPanel` now groups conversations by `coach_id` so the
sidebar shows one expandable section per coach, plus a "Without a
coach" bucket for unattached conversations. Collapsed state persists
in `localStorage` under `dravr.conversations-panel.collapsed`.

## Tier 5 — Evaluation harness

**Purpose:** Catch regressions in coach behavior before they ship by
running golden-fixture dialogues through a three-layer scorer.

**Key files**

- `crates/pierre-evals/src/` — the whole crate.
  - `deterministic.rs`: structural checks + persona keyword presence
    + prompt-injection pattern detection.
  - `judge.rs`: LLM-as-judge invocation backed by
    `pierre_llm::judge::ask_for_json`. Shares the JSON parsing
    plumbing lifted from `insight_validation.rs` in Tier −1.
  - `multi_turn.rs`: sliding-window scoring over a full fixture case.
  - `rubrics.rs`: `Rubric` types used by the judge.
  - `fixtures.rs`: `GoldenFixture::load_jsonl` / `parse_jsonl`.
- `crates/pierre-evals/fixtures/injury_triage.jsonl` — the seed
  fixture set.

**Admin surface (Sprint C16)**

`crates/pierre-server/src/services/eval_harness.rs` walks the
fixtures directory (configurable via `PIERRE_EVALS_FIXTURES_DIR`,
default `crates/pierre-evals/fixtures`) and parses each `.jsonl` file
into a `FixtureBrowserResponse`. The admin route
`GET /admin/evals/fixtures` is gated on the `tools-verification`
feature + `AdminPermission::ViewConfiguration`. The `EvalHarnessTab`
frontend renders fixture + case + persona + assertion counters and
an expandable per-case drill-down.

Live evaluation runs against the fixtures are intentionally **not**
exposed from the admin tab — a full judge pass is expensive and
needs its own execution queue. The current scope is "read-only
browser so an operator can see which scenarios ship with the
release and what each case asserts".

## Tier 5.5 — Bullshit detector

**Purpose:** Verify the factual claims a coach emits post-LLM and
block or flag unsupported / contradicted / dangerous claims before
the user sees them.

**Key files**

- `crates/pierre-evals/src/claim_extractor.rs` — decomposes the reply
  into atomic propositions tagged by category.
- `crates/pierre-evals/src/rhetoric_detector.rs` — Layer 1 filter
  that drops figures of speech, questions, greetings.
- `crates/pierre-evals/src/deterministic_bounds.rs` — Layer 2
  per-category hard bounds (HRmax, VO2max, protein intake, etc.).
- `crates/pierre-evals/src/evidence_retriever.rs` — Layer 3 RAG over
  the curated sports-science corpus.
- `crates/pierre-evals/src/verdict_engine.rs` — synthesizes the
  layers into a `VerdictOutcome` with evidence strength.
- `crates/pierre-evals/src/verification_config.rs` — per-coach YAML
  frontmatter config (`enabled`, `categories`, `fallback_behavior`).
- `crates/pierre-server/src/services/claim_verification.rs` — the
  `apply_claim_verification` hook + `resolve_corpus` that prefers
  the runtime `EvidenceRegistry` and falls back to the compiled-in
  `EMBEDDED_PROPOSITIONS` when the registry is empty.
- `crates/pierre-server/src/routes/admin/claim_verdicts.rs` — admin
  triage surface (Sprint C1).
- `crates/pierre-server/src/services/chat_verdicts.rs` — user-facing
  wire shapes for in-chat Evidence Strength chips (Sprint C4).

**The six-layer pipeline**

1. **Claim extraction** — LLM decomposes the coach reply into
   atomic propositions tagged by category.
2. **Rhetoric filter** — pure-Rust keyword + punctuation heuristics
   drop figures of speech before any LLM cost is incurred.
3. **Deterministic bounds** — hard category-specific checks
   (`HR > 220 BPM` → false, `100g protein/kg bw` → false).
4. **Evidence retrieval** — RAG over the curated sports-science
   corpus, returns DOI/PMID-backed atomic propositions.
5. **Consistency check** — cross-check the claim against the
   coach's earlier turns in the same conversation.
6. **LLM-as-judge** — only invoked when layers 1–5 don't reach a
   confident verdict. This is the cost optimization: target <10% of
   turns reach layer 6.

Each verdict carries an **Evidence Strength badge** (Strong / Mixed /
Weak / None). The `ClaimVerdictRepository::insert_claim_verdict` call
persists the row; both admin and end users see it rendered as a chip
on the offending message via the Sprint C1 and C4 UI work.

## Tier 6 — Text guardrails

**Purpose:** Post-LLM safety net for length, blocked topics, and
disclaimer injection.

**Key files**

- `crates/pierre-server/src/services/text_guardrails.rs` — the
  `apply_text_guardrails` function, `GuardrailsConfig`, `safe_default`.
- `crates/pierre-server/src/routes/admin/harness_config.rs` — Tier 6
  tunables (the `guardrails` half of the harness config document).

**Tunables**

| Field | Default | Notes |
|---|---|---|
| `max_response_chars` | 5000 | 0 disables the cap |
| `blocked_topics` | `["medical diagnosis", "prescription drugs"]` | Case-insensitive substring match |
| `disclaimer_triggers` | `["injury", "pain", "medical"]` | When any match, prepend `disclaimer_text` |
| `disclaimer_text` | safe medical boilerplate | Must be non-empty when any triggers are set |

Enforced by `validate_document` in `harness_config.rs`.

## Phase C tiers — Prompt injection defense

Four layered defenses added via Phase C sprints, all pure-Rust and
observational in this release (no auto-blocking).

### C2 — Input sanitization

`crates/pierre-core/src/safety.rs` scans inbound user text for 5
injection signatures: `InstructionOverride`, `PersonaSwap`,
`RoleInjection`, `DangerousUrlScheme`, `MarkdownDataUriImage`.
Matching substrings are replaced with `[redacted: signature]` before
persistence. The original text is never logged.
`messaging_ingress::persist_single_message` wires this in via
`sanitize_for_dispatch` which logs a structured `warn` with the
hashed user id and the signatures that fired.

### C9 — System-prompt fingerprinting

`crates/pierre-core/src/prompt_fingerprint.rs` computes a `PromptFingerprint`
for the system prompt: a SHA-256 of the normalized (lower-case,
whitespace-collapsed) prompt plus a rolling-window shingle set of
40-byte substrings. `scan_response_for_leaks` counts how many
shingles from the prompt appear verbatim in the assistant reply;
3+ hits flags as `LeakVerdict::Leaked`.

### C10 — Post-LLM tool allowlist

`services::tool_execution::enforce_tool_allowlist` queries
`ToolSelectionService::is_tool_enabled` for every tool call the LLM
emits. If the tenant has disabled the tool, the call is short-
circuited with a structured "tool blocked" response so the LLM sees
the refusal in the next turn. Fails open on selection-service
outages.

### C11 — Canary tokens

`crates/pierre-core/src/prompt_fingerprint.rs` generates a per-turn
16-char canary via `generate_canary`, injects it into the system
prompt with a "do not repeat this" instruction
(`inject_canary_instruction`), and scans the reply for the verbatim
token (`detect_canary_in_response`). `services::prompt_leak` wraps
the three together into a `PromptGuard` that holds the hardened
prompt + canary + fingerprint for the duration of the turn.
Canary hits escalate to `error` level logs because they are
unmistakable exfiltration evidence (the canary is generated
per-turn and appears only in the hidden instruction block).

## Phase D tiers — Proactive workers

Both Phase D sprints are pure-read aggregations over the
`claim_verdicts` table; neither introduces new storage.

### C13 — Myth-busting summary

`services::myth_busting::compute_summary` scans up to 500 recent
verdicts for a tenant, filters to unsupported + contradicted, and
rolls them up into three top-10 patterns: recurring claim texts,
offending coaches, flagged categories. Admin route
`GET /admin/myth-busting/summary` exposes it to the
`MythBustingTab`.

### C14 — Coach content grading

`services::coach_grading::compute_coach_grades` scans up to 1000
recent verdicts and produces a per-coach 0–1 quality score plus an
A–F letter grade. Weighting: `supported +1`, `unsupported +0.25`,
`contradicted -1`. Coaches with fewer than 3 scored verdicts get a
`Provisional` grade. Sorted worst-first so admins can review the
bottom of the leaderboard. Admin route
`GET /admin/coach-grading/summary` exposes it to the
`CoachGradingTab`. The score is intended to feed store ranking in a
follow-up integration sprint.

## Phase D Sprint C17 — ClaimVerdict backfill

**Purpose:** Close the Sprint C14 cold-start gap — coaches in
tenants with existing chat history should not wait for new verdicts
to accumulate before the grading tab becomes useful. The backfill
walks historical assistant messages and runs the same verification
pipeline the live dispatch path uses, writing `claim_verdicts` rows
tagged with the source `message_id` so historical and live rows
are distinguishable in audits.

**Key files**

- `crates/pierre-server/src/services/claim_verdict_backfill.rs` —
  `run_backfill` + `BackfillParams` + `BackfillStats`. Tenant-scoped
  SQL walker over `chat_conversations INNER JOIN chat_messages`
  filtered to `role = 'assistant'`. Pure-Rust pipeline via
  `verify_reply_with_config_and_corpus` (no LLM calls).
- `crates/pierre-server/src/bin/pierre_cli/commands/harness.rs` —
  CLI subcommand `pierre-cli harness backfill-verdicts` with
  `--tenant-id`, `--limit`, `--since`, `--dry-run`, `--sleep-ms`,
  `--resume` flags. Both the service module and the CLI command are
  gated on the `tools-verification` feature.

**Cost model**

`verify_reply_with_config_and_corpus` is synchronous and pure-Rust
— it uses `extract_heuristic` (regex + keyword heuristics), the
rhetoric filter (pattern matching), deterministic bounds (hard-
coded per-category checks), and the compiled-in evidence corpus.
No LLM, no network. A 100_000-message tenant backfills in minutes
on a single connection with zero LLM credits spent.

**Resume semantics**

The backfill stores the last-processed `(created_at, message_id)`
tuple in `system_settings` under
`harness_claim_verdict_backfill_cursor:{tenant_id}`. Re-invoking
with `--resume` restarts from strictly after the cursor, so
operators can incrementally walk very large tenants across multiple
runs without re-scanning old messages.

**Dry-run mode**

`--dry-run` counts every verdict the pipeline would produce but
skips the `insert_claim_verdict` call. Use it to size the job
before committing to a real run — the `BackfillStats` JSON output
tells you exactly how many rows and of what categories would land.

## What's deferred

- **Live eval runs from the admin tab** — a full judge pass is
  expensive and needs its own execution queue. Sprint C16 ships the
  read-only fixture browser; live runs can be a follow-up sprint.
- **Store ranking integration for coach grades** — Sprint C14 ships
  the per-coach grading service and admin table. Wiring the score
  into `StoreListingsRepository::browse` sort order is intentionally
  decoupled from the grading computation.
- **PostgreSQL backfill path** — Sprint C17 ships the SQLite walker
  only. The service returns a clear error on `Database::PostgreSQL`.
  Production deployments on Postgres will need a mirror of the
  `fetch_assistant_messages` SQL for that backend.

## Next reads

- [Coaching Harness — Overview](coaching-harness-overview.md) —
  high-level mental model
- [Coaching Harness — Implementation Log](coaching-harness-sprints.md) —
  sprint-by-sprint history and commit references
- [Coaching Harness — Operator Guide](coaching-harness-ops.md) —
  day-2 ops, admin tabs, troubleshooting, tuning
