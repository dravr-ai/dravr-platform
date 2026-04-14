# Coaching Harness — Implementation Log

Chronological history of how the coaching harness was built. Each
sprint is independently shippable, lands in a single commit, and
passes `./scripts/ci/pre-push-validate.sh` green. Sprint numbers
match the commit history on `feature/harness-prep` (Tiers −1 through
D) and `feature/harness-finish` (Sprints C15/C16 + docs).

## Tier −1 — Architectural prep

Before any harness code could land, the existing codebase had to
stop breaking the "single source of truth" rule in two places. Both
were called out as blockers in the architecture analysis.

| Commit | Subject |
|---|---|
| `aa2408f6` | Consolidate `estimate_tokens` → `pierre-core::tokens` (killed 2 duplicates hard-coded with `CHARS_PER_TOKEN = 4`) |
| `3a1af842` | Lift LLM-as-judge plumbing into `pierre-llm::judge::ask_for_json` + `extract_json` (previously inline in `insight_validation.rs`) |
| `ec5d3576` | `cargo fmt` import ordering |
| `05879ff9` | Judge tests refactor: `AppResult<()>` + `?` instead of `.unwrap()` |
| `27b597a5` | Reify `coach_id` on `chat_conversations` — migration + backfill + delete the legacy `system_prompt` TEXT column |
| `106711a3` | CI clippy + PostgreSQL parity test fallout from the coach_id reification |

The coach_id reification in `27b597a5` is load-bearing: it's what
lets Tier 4 group conversations by `(user, coach)`. Before this
commit, the coach identity was smeared into a string at
conversation-create time and there was nothing to join on.

## Tier 0 — Memory foundations

| Commit | Subject |
|---|---|
| `32af30ad` | `pierre-memory` crate + repositories (SQLite + PG) + migrations for `compaction_blocks`, `user_facts`, `coach_notes`, `coach_followups`, `coach_sessions` |

This commit is the largest single landing of the entire harness
series. It defines the data model, both repository implementations,
and the migration in one atomic change so there's no partial state
on either database backend.

## Tier 1 — Conversation compaction

| Commit | Subject |
|---|---|
| `0c81ec19` | `ConversationCompactor` wired into `chat_orchestration` with sliding-window + summary strategies, 128k token window, 70%/95% thresholds |

## Tier 2 — Semantic user memory

| Commit | Subject |
|---|---|
| `56d1b898` | Gemini `text-embedding-004` provider (768 dims), extraction worker via `tokio::spawn`, `memory_extraction.md` prompt, recall injection |

## Tier 3 — Coach-authored memory tools

| Commit | Subject |
|---|---|
| `ffe16571` | `coach_note_add`, `coach_followup_schedule`, `remember_fact`, `recall_user_memory` registered via the existing `McpTool` pattern with zero trait edits |

## Tier 4 — Cross-channel coach sessions

| Commit | Subject |
|---|---|
| `71b665eb` | `coach_sessions` table + `session_id` FK + cross-channel resolver in dispatch (`ensure_coach_session_attached`, `inject_pending_followups`, `finalize_session_state`) |

## Tier 5 — Evals

| Commit | Subject |
|---|---|
| `bcea1197` | `pierre-evals` crate — deterministic + LLM-judge + multi-turn evaluators, `fixtures/injury_triage.jsonl`, `tests/eval_pipeline_test.rs` |

## Tier 6 — Text guardrails

| Commit | Subject |
|---|---|
| `a88f7de3` | `TextGuardrails` (safe_default with medical keywords, 5000 char cap), wired into messaging dispatch as `apply_text_guardrails` post-LLM check |

## Tier 5.5 — Bullshit detector (Phase A)

| Commit | Subject |
|---|---|
| `e8a9dfb9` | Tier 5.5 bullshit detector backend (Phase A) |
| `c1f56363` | Tier 5.5 CI red + Phase B per-coach verification wiring |
| `1f0b2c0f` | Tier 5.5 corpus to markdown with YAML frontmatter |
| `1af8b0b0` | Tool count drift guard for Tier 3 + Tier 5.5 tools |
| `1303ee2e` | Tier 5.5 `EvidenceRegistry` + contremaitre sync |

The `1303ee2e` commit is where the evidence corpus becomes hot-
reloadable via the dravr-contremaitre GitHub-backed registry. It
adds `EvidenceRegistry` as a sibling to `PromptRegistry` and
`ToolDescriptionRegistry`, and extends `full_sync` + `selective_sync`
to fetch markdown propositions from `evidence/sports_science/`.
`claim_verification::resolve_corpus` prefers the runtime registry
and falls back to the compiled-in `EMBEDDED_PROPOSITIONS` when the
registry is empty.

## Phase B — Frontend admin GUI

Eight sprints delivering the admin tabs. Each lands as a single
commit on `feature/harness-prep`.

### Sprint C1 — ClaimVerdictsTab

| Commit | Subject |
|---|---|
| `592d1dfe` | Phase B Sprint C1 — admin `ClaimVerdictsTab` for Tier 5.5 triage |

Backend: `crates/pierre-server/src/routes/admin/claim_verdicts.rs`
with two handlers (list recent, list by conversation). Frontend:
`ClaimVerdictsTab` + `ClaimVerdictDrawer` with status/category/coach
filters and a drill-down drawer.

### Sprint C3 — HarnessConfigTab + GuardrailsTab merged

| Commit | Subject |
|---|---|
| `f142a66b` | Phase B Sprint C3 — admin `HarnessConfigTab` + persistence |

Backend: `routes/admin/harness_config.rs` with GET + PUT handlers,
`HarnessConfigDocument` persisted under `system_settings.value` with
`HARNESS_CONFIG_SETTING_KEY`. `validate_document` is `pub` so the
8 validation tests in `tests/harness_config_validation_test.rs` can
exercise it directly. `database::plugins::factory::get_system_setting`
+ `set_system_setting` wrappers were added to dispatch to SQLite and
PostgreSQL backend impls. The Tier 6 guardrails tunables are merged
into the same config document.

### Sprint C4 — In-chat Evidence chips + drawer

| Commit | Subject |
|---|---|
| `66c6c056` | Phase B Sprint C4 — in-chat Tier 5.5 verdict chips + drawer |

Backend: `services::chat_verdicts::get_verdicts_handler` (extracted
from `routes/chat.rs` to keep the file under the 1750-line route
size limit). The service owns `ChatVerdictRow` +
`ChatVerdictListResponse`. Route:
`GET /api/chat/conversations/{conversation_id}/verdicts`. Frontend:
`MessageItem` adds a verdict chip + summary line for the worst
verdict, and a "Ask me about this claim" CTA that opens a
`ChatVerdictDrawer`.

### Sprint C5 — User MemoryPanel

| Commit | Subject |
|---|---|
| `5e0b51ce` | Phase B Sprint C5 — user-facing `MemoryPanel` ("what the coach remembers") |

Backend: `services::memory_facts` with `get_facts_handler` +
`forget_fact_handler`, tenant-scoped via `resolve_tenant_id`. Route:
`GET /api/memory/facts` + `DELETE /api/memory/facts/{fact_id}`.
Frontend: `MemoryPanel` groups facts by kind, renders a `ConfirmDialog`
before forgetting, and surfaces a filter dropdown.

### Sprint C6 — MemoryExtractionMonitorTab

| Commit | Subject |
|---|---|
| `d70072dd` | Phase B Sprint C6 — `MemoryExtractionMonitorTab` |

Backend: new `HarnessMemoryRepository::count_user_facts_metrics`
method (SQLite + PostgreSQL) aggregates `user_facts` into a
`UserFactMetrics` snapshot. Admin route
`GET /admin/memory/worker-metrics`. Frontend tab with health banner
(Healthy / Warming / Stalled / Idle), 4 counter cards, and a per-kind
breakdown. 3 SQLite integration tests + 4 frontend tests.

### Sprint C7 — CoachFollowupsTab

| Commit | Subject |
|---|---|
| `07662c11` | Phase B Sprint C7 — `CoachFollowupsTab` admin triage queue |

Backend: new repository methods
`list_pending_followups_for_tenant` + `cancel_followup`. Admin
routes `GET /admin/coach-followups/pending` (ViewConfiguration) and
`POST /admin/coach-followups/{id}/cancel` (ManageConfiguration).
Frontend: overdue counter, coach/user filters, `ConfirmDialog`-
guarded cancel.

### Sprint C8 — CoachNotesAuditTab

| Commit | Subject |
|---|---|
| `aad987a5` | Phase B Sprint C8 — `CoachNotesAuditTab` compliance audit log |

Backend: `list_coach_notes_for_tenant` + admin route
`GET /admin/coach-notes/audit` gated on
`AdminPermission::ViewAuditLogs` (stronger than `ViewConfiguration`
because coach notes contain personal data the coach derived).
Frontend: content search, scope/coach/user filters, per-scope
counters. 3 SQLite tests + 4 frontend tests.

### Sprint C12 — Mobile MemoryScreen (stretch)

| Commit | Subject |
|---|---|
| `3588d94d` | Phase B Sprint C12 — mobile `MemoryScreen` |

React Native port of the web `MemoryPanel`. Uses
`userApi.listMemoryFacts` + `forgetMemoryFact` from the shared
`@pierre/api-client` (mobile index re-exported the types).
expo-router route at `app/(app)/memory.tsx`. 3 Jest tests.

### Sprint C15 — Session-hierarchy refactor

| Commit | Subject |
|---|---|
| `e519295c` | Phase B Sprints C15 + C16 — session hierarchy + eval browser |

Frontend-only refactor of `ConversationsPanel` to group conversations
by `coach_id`, with collapsible per-coach sections and a "Without a
coach" bucket for unattached conversations. State persists in
`localStorage` under `dravr.conversations-panel.collapsed`. 4
frontend tests.

## Phase C — Prompt injection defense

Four sprints delivering layered defenses. All pure-Rust, all
observational (no auto-blocking) in this release.

### Sprint C2 — Input sanitization

| Commit | Subject |
|---|---|
| `6766fcd6` | Phase C Sprint C2 — input sanitization for inbound messages |

`pierre-core::safety` module with 5 `InjectionSignature` variants
and a `scan(text) -> SanitizationOutcome` helper. Wired into
`messaging_ingress::persist_single_message` via
`sanitize_for_dispatch` which logs a structured `warn` with hashed
`user_id` + signatures when matches fire. 12 unit tests.

### Sprint C9 — System-prompt fingerprinting

| Commit | Subject |
|---|---|
| `785e7273` | Phase C Sprint C9 — system-prompt fingerprinting + leak detection |

`pierre-core::prompt_fingerprint` module with `fingerprint_prompt`,
`scan_response_for_leaks`, 40-byte shingle window, and
`DEFAULT_LEAK_THRESHOLD` of 3. `services::prompt_leak` service
wraps the fingerprint + scan in the dispatch path's logging policy.
Consolidated `build_llm_messages` helper between
`routes/chat.rs` and `services/chat_orchestration.rs` into a single
`pub fn` to free line budget for the new wiring.

### Sprint C10 — Tool allowlist post-LLM

| Commit | Subject |
|---|---|
| `f2bd58fe` | Phase C Sprint C10 — post-LLM tool allowlist enforcement |

`services::tool_execution::enforce_tool_allowlist` queries
`ToolSelectionService::is_tool_enabled` before every
`execute_mcp_tool` call. On block, returns a structured
`UniversalResponse` the LLM sees in the next turn. Fails open on
selection-service outages.

### Sprint C11 — Canary tokens

| Commit | Subject |
|---|---|
| `3279cf22` | Phase C Sprint C11 — canary tokens + layered leak detection |

`pierre-core::prompt_fingerprint` gets `generate_canary`,
`inject_canary_instruction`, `detect_canary_in_response`.
`services::prompt_leak::harden_system_prompt` bundles the canary
generation + injection + fingerprinting into a `PromptGuard` the
dispatch path holds for the whole turn. `scan_assistant_reply`
returns a `ReplyLeakReport` with both the shingle verdict and the
canary hit flag. Canary hits escalate to `error` level — unmistakable
exfiltration evidence.

## Phase D — Proactive workers

| Commit | Subject |
|---|---|
| `57cd322f` | Phase D Sprint C13 — myth-busting summary over Tier 5.5 verdicts |
| `9e68f61d` | Phase D Sprint C14 — coach content grading from Tier 5.5 verdicts |

Both ship as pure-read aggregations over `claim_verdicts`. Neither
adds new storage or runs as a background worker — "proactive worker"
in the gist is the analysis surface, not a periodic batch job.

## CI shakeout (Phase D → merge)

After C14 landed, the full pedantic + nursery clippy suite surfaced
10+ errors that local per-crate clippy had missed (pierre-server is
explicitly skipped from per-crate clippy because the full run is as
slow as the full workspace run). Four commits of clippy fallout
followed:

| Commit | Subject |
|---|---|
| `b2a0d8e3` | clippy pedantic/nursery fallout from C3/C5/C6/C7/C8/C9/C11 (5 errors) |
| `4f98d5bb` | CI fallout: test PartialEq, ES2020 `replaceAll`, cognitive complexity (4 errors) |
| `e6a141fd` | pedantic fallout from C13/C14 sprint code (4 errors: doc_markdown, missing_errors_doc, suboptimal_flops) |
| `68bd7ce6` | `clippy::suboptimal_flops` inner expression on coach_grading (1 error) |
| `8d0426eb` | clippy absolute_paths + suboptimal_flops fallout (2 errors) |
| `7ee7d8c1` | clippy absolute_paths in test files (3 errors, pre-existing pattern in `claim_verdicts_repository_test` + `coach_notes_audit_test`) |
| `824dbbf6` | clippy doc_markdown `coach_notes` in test doc (1 error) |
| `47ac957f` | clippy `unnecessary_get_then_check` in `memory_worker_metrics_test` (2 errors) |

`47ac957f` was the commit where Backend (Rust) + all other critical
CI gates finally came green together, allowing the squash-merge to
main.

**Lesson for future harness work:** don't skip full-workspace clippy
as a local gate when pushing changes that span `pierre-server/` and
its test tree. The 45-minute full workspace run is still faster
than chasing the errors across three or four round-trip pushes.

## Squash merge to main

| Commit | Subject |
|---|---|
| `035488c0` | feat(harness): Phase B/C/D harness sprints — tiers 0–5 + C1–C14 |

Squash-merged 195 files changed, +20,390 / −645. All tiers 0–5.5 + 6,
all Phase B/C/D sprints except the three gist items deferred:
ChatSidebar session hierarchy, EvalHarnessTab, ClaimVerdict backfill.

## feature/harness-finish branch

Started after the `035488c0` merge to close the three remaining
items.

### Sprint C15 — ChatSidebar session hierarchy

See [Sprint C15 entry above](#sprint-c15--session-hierarchy-refactor).

### Sprint C16 — EvalHarnessTab

Backend: `services::eval_harness` walks the fixtures directory (via
`PIERRE_EVALS_FIXTURES_DIR` env or the canonical workspace path),
parses `.jsonl` files via `GoldenFixture::load_jsonl`, and returns a
`FixtureBrowserResponse`. Admin route `GET /admin/evals/fixtures`
gated on `tools-verification` feature + `AdminPermission::ViewConfiguration`.
Frontend: `EvalHarnessTab` shows 4 counter cards + expandable per-
fixture case rows with turn count, `must_contain`, `must_not_contain`.
4 backend integration tests (with temp-dir fixture seeding) + 4
frontend component tests.

### Sprint C17 — ClaimVerdict backfill

Backend: `services::claim_verdict_backfill::run_backfill` walks every
`role='assistant'` message for a tenant in `(created_at ASC, id ASC)`
order and runs the pure-Rust heuristic pipeline
(`verify_reply_with_config_and_corpus`) over each. Persists verdicts
via `ClaimVerdictRepository::insert_claim_verdict` with `message_id`
set so historical rows are distinguishable from live-stream verdicts.
No LLM calls — cost is O(messages) wall time only, which was the
critical discovery that moved this sprint from "deferred, expensive"
to "ship it now, it's free".

Resume support: the cursor is stored in `system_settings` under
`harness_claim_verdict_backfill_cursor:{tenant_id}`; `--resume`
picks up where the previous run left off, making incremental runs
safe across long-running tenants.

CLI: `pierre-cli harness backfill-verdicts` with `--tenant-id`,
`--limit` (clamped `1..=100_000`), `--since`, `--dry-run`,
`--sleep-ms`, `--resume`. Gated on the `tools-verification`
feature. Pretty-prints `BackfillStats` as JSON to stdout for piping
into log files and log aggregation pipelines.

4 integration tests cover: persistence path writes verdicts,
`--dry-run` doesn't persist, limit clamping to the `1..=MAX` range,
resume cursor skips previously-processed messages.

Sprint C17 unblocks Sprint C14 (coach grading) for tenants with
existing history — without backfill, every coach starts as
`Provisional` until the live verdict stream accumulates 3+ scored
verdicts per coach.

### Sprint C18 — Documentation

This file, plus
[Coaching Harness — Overview](coaching-harness-overview.md),
[Coaching Harness — Tier-by-Tier](coaching-harness-tiers.md),
and [Coaching Harness — Operator Guide](coaching-harness-ops.md).
