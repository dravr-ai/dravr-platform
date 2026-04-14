# Coaching Harness — Operator Guide

Day-2 operations reference for the coaching harness. Read the
[Overview](coaching-harness-overview.md) first. This page is for the
person who needs to configure the harness, triage flagged content,
interpret admin tabs, and troubleshoot when something is broken.

## Admin tabs reference

All harness admin tabs live under the **Configuration** section of
the dashboard sidebar. Each requires at least
`AdminPermission::ViewConfiguration` on the admin token; a few
require stronger permissions as noted.

### Claim Verdicts tab (Sprint C1)

**Route:** `GET /admin/claim-verdicts?tenant_id=...`
**Permission:** `ViewConfiguration`

Triage surface for Tier 5.5 verdicts. Lists the most recent rows in
`claim_verdicts` with status / category / coach / limit filters.

**Filter reference**

| Filter | Values |
|---|---|
| `status` | `supported`, `unsupported`, `contradicted`, `rhetorical`, `unverifiable` |
| `category` | `physiological`, `training_prescription`, `nutrition`, `recovery`, `supplement`, `injury_rehab` |
| `coach_id` | exact match |
| `limit` | 1..=200 |

Click any row to open a drawer with the full claim text, evidence
strength badge, explanation, and the layer that produced the
verdict.

**What to do when you see a flagged row**

1. Read the explanation. Is the layer classification obviously wrong?
   (e.g. rhetorical flagged as physiological.)
2. Cross-check against [Myth Busting tab](#myth-busting-tab-sprint-c13)
   — is this a recurring pattern across coaches or a one-off?
3. If the coach is shipping bad content at scale, check
   [Coach Grades tab](#coach-grading-tab-sprint-c14) to see if the
   letter grade justifies unpublishing.
4. If the layer itself is wrong, file an issue for
   `crates/pierre-evals/` tuning.

### Harness Config tab (Sprint C3)

**Route:** `GET /admin/settings/harness`, `PUT /admin/settings/harness`
**Permission:** `ViewConfiguration` (GET), `ManageConfiguration` (PUT)

Persists a `HarnessConfigDocument` under
`system_settings.value` with key `harness_config`.

**Compaction tunables (Tier 1)**

| Field | Default | Valid range |
|---|---|---|
| `window_tokens` | 128_000 | > 0 |
| `warn_threshold` | 0.70 | `(0.0, 1.0]`, strictly less than `emergency_threshold` |
| `emergency_threshold` | 0.95 | `(0.0, 1.0]` |
| `summarize_oldest_n` | 6 | > 0 |
| `sliding_drop_n` | 4 | > 0 |

**Guardrail tunables (Tier 6)**

| Field | Default | Notes |
|---|---|---|
| `max_response_chars` | 5000 | 0 disables the cap |
| `blocked_topics` | `["medical diagnosis", "prescription drugs"]` | Case-insensitive substring match, rejected outright |
| `disclaimer_triggers` | `["injury", "pain", "medical"]` | When any match, prepend `disclaimer_text` |
| `disclaimer_text` | safe medical boilerplate | Must be non-empty when any trigger is set |

Validation is enforced server-side via `validate_document` and has
8 tests in `crates/pierre-server/tests/harness_config_validation_test.rs`.
The GET handler returns a `source` field (`default` or `persisted`)
so the UI knows whether to show "you're editing the shipped default"
vs "you're editing operator overrides".

### Memory Panel (Sprint C5)

**Route:** `GET /api/memory/facts?kind=...`, `DELETE /api/memory/facts/{fact_id}`
**Permission:** none — this is user-facing

Users see what the coach has stored about them and can forget
individual facts. Forgetting is GDPR-grade: the row is removed and
the coach will stop referencing it on the next turn.

Accessible from the user settings page. Grouped by kind
(preference / physiology / injury / goal / schedule / equipment /
other). Limit 100 rows per kind.

### Memory Worker tab (Sprint C6)

**Route:** `GET /admin/memory/worker-metrics?tenant_id=...`
**Permission:** `ViewConfiguration`

Observability for the Tier 2 extraction worker. The worker itself is
fire-and-forget `tokio::spawn` with no loop state, so its health is
derived from the rows it has actually produced in `user_facts`.

**Health banner decision tree**

| Condition | Banner | Operator action |
|---|---|---|
| `total_facts == 0` | **Idle** | Normal for new tenants. Run a few conversations to seed. |
| `facts_last_24h > 0` | **Healthy** | Nothing to do. |
| `facts_last_7d > 0 && facts_last_24h == 0` | **Warming** | Expected during low-usage periods. |
| `total_facts > 0 && facts_last_7d == 0` | **Stalled** | **Investigate.** Worker is broken or LLM credentials are revoked. Check server logs for `memory_extraction` spans. |

The per-kind breakdown is useful for spotting extractor prompt
drift: a tenant with 90% `other` facts has a prompt problem.

### Coach Followups tab (Sprint C7)

**Route:** `GET /admin/coach-followups/pending?tenant_id=...`,
`POST /admin/coach-followups/{id}/cancel?tenant_id=...`
**Permissions:** `ViewConfiguration` (list), `ManageConfiguration` (cancel)

Tenant-wide pending followup queue. Each row is a promise the coach
made that has not yet been injected into a next-turn system prompt.

**Columns**

- **Content** — the promise the coach wrote
- **Due** — relative ("in 2h", "3d ago") + absolute timestamp; overdue rows in red
- **Coach** / **User** — FK identifiers
- **Action** — Cancel button (opens confirm dialog)

Cancel is idempotent: transitioning `pending → cancelled` returns
`{"cancelled": true}`; any other starting state returns `{"cancelled": false}`
with HTTP 200 (the post-condition "this followup will not be
injected into a coach prompt" holds either way).

### Coach Notes Audit tab (Sprint C8)

**Route:** `GET /admin/coach-notes/audit?tenant_id=...`
**Permission:** `ViewAuditLogs` (**stronger** than `ViewConfiguration`)

Flat tenant-wide feed of every note a coach persona wrote about a
user. This is the GDPR/compliance audit surface, not a triage tab —
you're not expected to act on individual rows, you're expected to
review the aggregate for drift.

The permission is intentionally escalated to `ViewAuditLogs` because
coach notes contain personal data the coach has derived from
conversations. They are **not** user-visible (unlike Memory Panel
facts, which are user-visible + forgettable).

**Filters:** substring search, coach id, user id, scope
(`conversation` / `user` / `tenant`).

### Myth Busting tab (Sprint C13)

**Route:** `GET /admin/myth-busting/summary?tenant_id=...&limit=200`
**Permission:** `ViewConfiguration`

Phase D pattern view. Scans up to 500 recent `claim_verdicts`,
filters to unsupported + contradicted, rolls up into three top-10
lists:

- **Top claims** — recurring claim texts ranked by occurrence count
- **Top coaches** — offending coaches ranked by unsupported total
- **Top categories** — flagged categories ranked by frequency

Use this when you see a handful of flagged verdicts in
[Claim Verdicts tab](#claim-verdicts-tab-sprint-c1) and want to know
if they're a pattern or noise.

### Coach Grades tab (Sprint C14)

**Route:** `GET /admin/coach-grading/summary?tenant_id=...&limit=500`
**Permission:** `ViewConfiguration`

Per-coach A–F letter grade derived from the verdict history.

**Scoring formula**

```
raw = unsupported.mul_add(0.25, supported) - contradicted
    (where scored_total = supported + unsupported + contradicted)

score = (raw / scored_total).clamp(0.0, 1.0)
```

- Supported claims: +1 each
- Unsupported claims: +0.25 (mild penalty but not zero)
- Contradicted claims: -1
- Rhetorical + unverifiable: excluded from the denominator (they
  don't inform quality either way)

**Letter grade cutoffs (only when `total_verdicts >= 3`)**

| Grade | Score |
|---|---|
| A | `>= 0.90` |
| B | `>= 0.75` |
| C | `>= 0.60` |
| D | `>= 0.40` |
| F | `< 0.40` |
| Provisional | `total_verdicts < 3` |

Table is sorted worst-first so operators see the bottom of the
leaderboard, and a "Failing (D/F)" counter card at the top shows
how many coaches need attention. Store ranking integration (so a
low grade pushes a coach down in search results) is a planned
follow-up.

### Eval Harness tab (Sprint C16)

**Route:** `GET /admin/evals/fixtures`
**Permission:** `ViewConfiguration`
**Feature flag:** `tools-verification` (the whole admin tab is
compiled away when the feature is off, because it depends on
`pierre-evals`)

Read-only browser over the golden fixture set used by the Tier 5
evaluation harness. Walks the fixtures directory (default
`crates/pierre-evals/fixtures`, override with the
`PIERRE_EVALS_FIXTURES_DIR` env) and parses each `.jsonl` file via
`GoldenFixture::load_jsonl`.

**What you can see**

- Per-fixture counters: case count, persona(s), total assertions
- Per-case counters: turn count, `must_contain` count, `must_not_contain` count
- Scanned directory path (so you know which fixture set the running binary is pointing at)

**What you cannot do yet**

Live eval runs against the fixtures are intentionally **not**
triggerable from this tab. A full judge pass is expensive and needs
its own execution queue; the current scope is "let an operator see
what's on disk without SSHing into the box". Live runs land in a
follow-up sprint.

## Configuration reference

### Environment variables

| Variable | Default | Used by |
|---|---|---|
| `PIERRE_EVALS_FIXTURES_DIR` | `crates/pierre-evals/fixtures` | Sprint C16 eval browser |
| `CONTREMAITRE_GITHUB_REPO` | unset | Tier 5.5 evidence registry sync |
| `CONTREMAITRE_GITHUB_TOKEN` | unset | Tier 5.5 evidence registry sync |

### Feature flags

| Flag | Default | Gates |
|---|---|---|
| `tools-memory` | on | Tier 3 coach memory MCP tools + `MemoryPanel` backend |
| `tools-verification` | on | Tier 5.5 verification pipeline + `EvalHarnessTab` |
| `tools-groups` | on | Group coaching context injection |

### Permission flags

The harness admin tabs use three distinct permissions to stratify
access:

| Permission | Grants |
|---|---|
| `ViewConfiguration` | Read-only admin tabs: claim verdicts, memory worker, followups list, myth busting, coach grades, harness config (GET), eval browser |
| `ManageConfiguration` | Write actions: harness config PUT, cancel followup |
| `ViewAuditLogs` | Coach notes audit log (personal data the coach derived) |

Generate admin tokens with specific permissions via
`pierre-cli token generate --service X --permissions ViewConfiguration,ManageConfiguration`.

## Troubleshooting

### "Memory Worker tab shows Stalled"

The extraction worker hasn't produced any `user_facts` rows in 7+
days despite having prior rows in the tenant. Most likely causes:

1. **LLM provider down.** Check `services::memory_extraction` logs
   for `reqwest::Error` spans. Rotate the LLM credential in
   LLM Settings if the provider revoked it.
2. **Embedding provider down.** Tier 2 extraction writes rows without
   embeddings via `UpsertUserFactParams { embedding: None, .. }`, so
   this should not block extraction — but it does block recall,
   which might surface as "coach suddenly lost memory of the user".
3. **Extractor prompt drift.** Check
   `crates/pierre-llm/src/prompts/memory_extraction.md` for recent
   changes. A malformed JSON schema in the prompt can silently drop
   all extractions.

### "Claim verdict log is empty"

- Verify the coach's `verification_config` has `enabled: true`.
- Verify the `tools-verification` feature flag is on at compile time.
- Check server logs for `claim_verification::apply_claim_verification`
  spans.

### "Coach grade is Provisional for a coach with many conversations"

The `Provisional` grade is computed on the **scored** verdict count
(supported + unsupported + contradicted), not the raw `total_verdicts`
count. If all the coach's verdicts are `rhetorical` or `unverifiable`
they don't count toward the threshold. This usually means the
rhetoric filter (Layer 1) is catching everything before the
deterministic layer fires — normal for coaches whose content is
mostly motivational rather than factual.

### "Harness Config PUT rejected with 400"

Validation rules (all enforced by `validate_document` in
`harness_config.rs`):

- `emergency_threshold > warn_threshold`
- Both thresholds in `(0.0, 1.0]`
- `window_tokens > 0`
- If `disclaimer_triggers` non-empty, `disclaimer_text` non-empty

The error body contains the specific invariant that failed.

### "ChatSidebar shows a coach title I don't recognize"

`ConversationsPanel` loads coach metadata via
`coachesApi.list()` with a 5-minute stale-time. If a coach was
renamed recently, force a refresh with the browser Refresh button or
wait out the cache. Conversations whose `coach_id` is not in the
user's `coaches.list()` response render under "Unknown coach" — this
happens when the coach was deleted but the conversation survived.

### "Canary leak detected in server logs — is it a real attack?"

The canary is 23 characters, generated per turn from SystemTime
nanoseconds + salt. Collision probability is effectively zero, so
any `canary_leak_confirmed` log line in a freshly-rotated tenant is
a real leak. Act:

1. Redact the offending conversation message via the DB if it's
   already persisted.
2. Rotate the coach's system prompt via the coach editor.
3. File an issue with the full conversation log so we can tune the
   coach's response template.

### "Backend Rust CI keeps failing clippy"

Per-crate clippy on `pierre-server` is explicitly skipped in
`pre-push-validate.sh` per CLAUDE.md ("the pierre_mcp_server crate
is large enough that per-crate clippy is as slow as the full
workspace run"). That means **CI is the only place clippy runs on
the server crate**, and errors surface after push. When fixing
clippy errors in server-crate code:

1. Run `cargo clippy --all-targets --all-features -- -D warnings`
   locally before pushing **if** you have the 45 minutes to spare.
2. Otherwise, fix and push, and use `gh api /repos/…/actions/jobs/<id>/logs`
   to fetch logs while the run is still in progress (the normal
   `gh run view --log-failed` requires the parent run to complete).

## Related documentation

- [Coaching Harness — Overview](coaching-harness-overview.md)
- [Coaching Harness — Tier-by-Tier](coaching-harness-tiers.md)
- [Coaching Harness — Implementation Log](coaching-harness-sprints.md)
- [Admin Tool Management](admin-tool-management.md) — general admin
  token and permission reference
- [Tool Development](tool-development.md) — how the Tier 3 MCP tools
  were added without trait edits
