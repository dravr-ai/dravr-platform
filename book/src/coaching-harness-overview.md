# Coaching Harness — Overview

**Status:** shipped (Phase A, B, C, D complete; Tiers 0–6 + 5.5 landed on `main`).

The coaching harness is the set of subsystems that turns Pierre's chat
dispatch pipeline into a production-grade AI coaching platform. It
adds long-term memory, conversation compaction, cross-channel
sessions, Tier 5 evaluation infrastructure, a Tier 5.5 "bullshit
detector" for factual claims, text guardrails, prompt-injection
defenses, per-coach content grading, and a full admin GUI for
operators.

This document gives you the mental model. For implementation details
see [Coaching Harness — Tier-by-Tier](coaching-harness-tiers.md) and
[Coaching Harness — Implementation Log](coaching-harness-sprints.md).
For day-2 operations see [Coaching Harness — Operator Guide](coaching-harness-ops.md).

## Why it exists

Pierre's pre-harness chat pipeline was single-turn: the LLM saw every
message in the current conversation up to the context window, with no
mechanism to remember the user across sessions, summarize old history,
verify the model's own factual claims, or let operators audit what the
coach was saying. Shipping coaches at scale required closing all of
those gaps at once.

The harness architecture is a **seven-tier stack** layered on top of
the existing `services::chat_orchestration` dispatch path. Each tier
is independently shippable and observable.

## The seven tiers

| Tier | Name | What it does | Where it hooks |
|---|---|---|---|
| 0 | Memory foundations | `pierre-memory` crate + DB schema for compaction blocks, user facts, coach notes, coach followups, coach sessions, claim verdicts | `pierre-database::HarnessMemoryRepository`, migrations under `migrations/20260413000002_harness_memory_foundations.sql` |
| 1 | Conversation compaction | Summarize older turns when the context window fills | `chat_orchestration::apply_tier1_compaction`, `services::conversation_compaction` |
| 2 | Semantic user memory | Background extraction of `UserFact` rows from turns; recall at prompt build time | `services::memory_extraction`, `services::memory_recall`, Gemini text-embedding-004 provider |
| 3 | Coach-authored memory tools | `coach_note_add`, `coach_followup_schedule`, `remember_fact`, `recall_user_memory` MCP tools the coach can call to write its own memory | `tools/implementations/memory.rs`, `tools-memory` feature flag |
| 4 | Cross-channel coach sessions | One logical session per (user, coach) pair, spanning conversations and messaging channels | `services::chat_orchestration::ensure_coach_session_attached`, `coach_sessions` table |
| 5 | Evaluation harness | `pierre-evals` crate with deterministic + LLM-judge + multi-turn evaluators, golden fixtures in JSONL | `crates/pierre-evals/`, `fixtures/injury_triage.jsonl` |
| 5.5 | Bullshit detector | Post-LLM claim verification pipeline: rhetoric → deterministic → evidence → consistency → judge. Writes `claim_verdicts` rows. | `services::claim_verification::apply_claim_verification`, `EvidenceRegistry`, `contremaitre` sync |
| 6 | Text guardrails | Post-LLM length/topic/disclaimer checks | `services::chat_orchestration::apply_text_guardrails` |

## Phase map (B / C / D)

On top of the tiers, the platform ships three cross-cutting phases:

- **Phase B — Frontend admin GUI.** Every tier that produces data
  gets an admin surface: [Claim Verdicts](coaching-harness-ops.md#claim-verdicts-tab-sprint-c1),
  [Harness Config](coaching-harness-ops.md#harness-config-tab-sprint-c3),
  [Memory Panel](coaching-harness-ops.md#memory-panel-sprint-c5),
  [Memory Worker](coaching-harness-ops.md#memory-worker-tab-sprint-c6),
  [Coach Followups](coaching-harness-ops.md#coach-followups-tab-sprint-c7),
  [Coach Notes Audit](coaching-harness-ops.md#coach-notes-audit-tab-sprint-c8),
  [Coach Grades](coaching-harness-ops.md#coach-grading-tab-sprint-c14),
  [Myth Busting](coaching-harness-ops.md#myth-busting-tab-sprint-c13),
  [Eval Harness](coaching-harness-ops.md#eval-harness-tab-sprint-c16),
  and the session-hierarchy refactor of the chat sidebar.
- **Phase C — Prompt injection defense.** Four layered defenses: input
  sanitization, system-prompt fingerprinting + shingle leak detection,
  post-LLM tool allowlist enforcement, canary tokens with layered leak
  detection.
- **Phase D — Proactive workers.** Myth-busting summary over the
  `claim_verdicts` table and per-coach content grading that feeds
  store ranking.

## Dispatch pipeline

Here is the full dispatch pipeline for a coach-driven conversation
turn, top to bottom, after the harness is wired in. Every numbered
step is an actual function call, not a conceptual phase.

```
           ┌─────────────────────────────────────────┐
           │  messaging_ingress::persist_single_msg  │
           │                                         │
           │  Sprint C2: input sanitization          │◄── prompt injection
           │  scans for the 5 InjectionSignatures    │    signatures: override,
           │                                         │    persona swap, role,
           │                                         │    data-uri, markdown
           └─────────────────────┬───────────────────┘
                                 │
                                 ▼
           ┌─────────────────────────────────────────┐
           │  chat_orchestration::dispatch_for_turn  │
           │                                         │
           │  1. resolve tenant + coach_id           │
           │  2. get_conversation_history()          │
           │  3. ensure_coach_session_attached()  ◄──┼── Tier 4
           │  4. resolve coach runtime context       │
           │  5. build base_prompt from coach prompt │
           │  6. inject_group_context()              │
           │  7. inject_refresh_context()            │
           │  8. inject_memory_recall()           ◄──┼── Tier 2
           │  9. inject_pending_followups()       ◄──┼── Tier 3 + 4
           │  10. append messaging context prompt    │
           │  11. harden_system_prompt()          ◄──┼── C9/C11 canary
           │  12. build_llm_messages()               │
           │  13. apply_tier1_compaction()        ◄──┼── Tier 1
           │  14. run_tool_loop()                    │
           │       ├─ enforce_tool_allowlist()    ◄──┼── C10
           │       └─ execute_mcp_tool()             │
           │  15. scan_assistant_reply()          ◄──┼── C9/C11 detect leak
           │  16. apply_text_guardrails()         ◄──┼── Tier 6
           │  17. apply_claim_verification()      ◄──┼── Tier 5.5
           │  18. persist_assistant_response()       │
           │  19. finalize_session_state()           │
           │  20. spawn_extract_for_turn()        ◄──┼── Tier 2 background
           └─────────────────────────────────────────┘
```

The critical reading for new engineers is
`crates/pierre-server/src/services/chat_orchestration.rs:dispatch_for_turn`.
Every major hook in the list above is a single function call in that
file, and the ordering matters — C9/C11 fingerprinting must happen
**before** tool calls so the prompt the LLM sees is the one we scan
against, and Tier 5.5 verification must happen **after** the guardrails
so the detector sees the final user-visible reply.

## The data stores

Seven new tables land with the harness. All are tenant-scoped.

- `compaction_blocks` — Tier 1 summaries replacing older conversation turns
- `user_facts` — Tier 2 extracted semantic memory, with embeddings
- `coach_notes` — Tier 3 coach-authored notes about users
- `coach_followups` — Tier 3 promised future check-ins
- `coach_sessions` — Tier 4 long-lived (user, coach) containers above conversations
- `claim_verdicts` — Tier 5.5 detector output (claim text, category, verdict, layer)
- `system_settings` — Sprint C3 harness config JSON (compaction + guardrails tunables)

Schemas are in `migrations/20260413000002_harness_memory_foundations.sql`
(SQLite) and `migrations_pg/20260413000002_harness_memory_foundations.sql`
(PostgreSQL). The trait surface lives in
`crates/pierre-database/src/repositories.rs`
(`HarnessMemoryRepository`, `ClaimVerdictRepository`). Implementations
are split between `crates/pierre-database/src/database/memory.rs`
(SQLite) and `crates/pierre-database/src/plugins/postgres/memory.rs`
(PostgreSQL) with matching test coverage in
`crates/pierre-server/tests/`.

## What sits outside the dispatch path

A handful of subsystems never run inside `dispatch_for_turn`; they run
as background workers, admin-triggered endpoints, or observability
queries.

- **Memory extraction worker** (Tier 2). `tokio::spawn`'d per turn;
  reads the assistant reply, asks an LLM extractor for structured user
  facts, upserts them.
- **Admin read endpoints** (Phase B). Thin axum handlers over the
  repository traits — `claim_verdicts`, `coach_followups`,
  `coach_notes`, `memory_worker`, `myth_busting`, `coach_grading`,
  `eval_harness`. None of these touch the dispatch path. See
  `crates/pierre-server/src/routes/admin/`.
- **Contremaitre sync** (Tier 5.5 Phase A). GitHub-backed registry
  that periodically pulls updated prompts, tool descriptions, and
  evidence propositions into the running server. The `EvidenceRegistry`
  is the runtime source of truth for the Tier 5.5 evidence retriever;
  the `EMBEDDED_PROPOSITIONS` constant is a fallback when the registry
  is empty.

## Next reads

- [Coaching Harness — Tier-by-Tier](coaching-harness-tiers.md) — what each tier does, the key files, and the design tradeoffs
- [Coaching Harness — Implementation Log](coaching-harness-sprints.md) — sprint-by-sprint history of how the harness was built, in order
- [Coaching Harness — Operator Guide](coaching-harness-ops.md) — day-2 ops, admin tabs, troubleshooting, tuning knobs
