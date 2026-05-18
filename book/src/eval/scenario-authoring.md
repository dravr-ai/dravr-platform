# Authoring Chat Conversation Scenarios

The chat conversation eval framework turns a bug report (typically a
Telegram screenshot from a real user session) into a permanent
regression test. It lives under
`crates/pierre-server/tests/helpers/chat_scenario/` and ships two
file formats:

1. **YAML scenarios** in `tests/scenarios/*.yaml` — hand-authored,
   intent-driven, ideal for synthesizing a class of bug.
2. **Telegram trace JSON** in `tests/scenarios/telegram_traces/*.json`
   — captured turn sequences, ideal for replaying a real production
   bug verbatim.

Both share the same runner, assertion catalog, drift detector, and
locale matrix. Authors pick whichever format the bug origin makes
natural.

## When to write a scenario

A scenario should land alongside any commit that fixes a
chat-pipeline bug surfaced from a real user session. The bar:

> A future LLM regression that re-introduces the bug must cause this
> scenario to fail.

If the bug is about prompt steering, intent routing, freshness,
locale leakage, or the bot drifting across turns, this framework is
the right home. If the bug is about HTTP route shape, database
constraints, or pure pipeline structure, the existing unit /
integration tests are a better fit.

## YAML scenario format

```yaml
name: "Short headline — what regression this catches"
locales: ["fr", "en"]            # parameterizes the runner; defaults to ["en"]
notes: "Telegram screenshot 2026-05-18 (reference for triage)"
provider_state:
  providers:
    strava:
      initial_activities:
        - { name: "Trail 14k", sport: trail_run, distance_km: 14.48, date: "2026-05-16" }
      appears_after_sync:        # surfaces only when a sync fires
        - { name: "8k road", sport: run, distance_km: 8.0, date: "2026-05-17" }
turns:
  - user: "Suis encore trop fatigué pour faire 80 km"
    trigger_sync_before_turn: true
    assertions:
      - kind: no_substring
        values: ["Medical disclaimer"]
  - user: "Côté course, combien de km?"
    assertions:
      - kind: tool_called
        name: get_activities
      - kind: distance_mentioned
        value_km: 33.10
        tolerance_km: 0.5
```

### Field reference

| Field | Required | Purpose |
|---|---|---|
| `name` | yes | Human-readable headline, surfaced in test failure output |
| `locales` | no (default `["en"]`) | BCP-47 short codes; runner executes once per locale |
| `notes` | no | Free text; include a reference to the originating bug report |
| `provider_state.providers.<name>.initial_activities` | no | Activities visible to the cache at scenario start |
| `provider_state.providers.<name>.appears_after_sync` | no | Activities promoted into the provider view when a sync fires |
| `turns[].user` | yes | User-facing text for this turn |
| `turns[].trigger_sync_before_turn` | no (default `false`) | Promote `appears_after_sync` before running this turn |
| `turns[].assertions` | no | Property assertions; empty ⇒ smoke check only |

### Assertion catalog

| `kind` | Fields | Catches |
|---|---|---|
| `reply_contains` | `value` | Required substring missing from the reply (case-insensitive) |
| `no_substring` | `values: [...]` | Forbidden substring leaked into the reply (wrong-language disclaimer, refusal-text leak, …) |
| `distance_mentioned` | `value_km`, optional `tolerance_km` (default 0.5) | Numeric km claim mismatch (handles `33.10 km`, `33,10 km` FR decimal, `33 kilometers`) |
| `activity_count_mentioned` | `value`, optional `tolerance` (default 0) | Numeric count mismatch across `activités/sorties/runs/séances` |
| `tool_called` | `name`, optional `min_calls` (default 1) | LLM claimed a tool-derived answer without invoking the tool |
| `vocabulary_contract` | `coach_id` | Reply from coach `coach_id` honoured no terms from its declared vocabulary contract (see `vocabulary_contract.rs`) |
| `any_of` | `values: [...]` | Reply matched none of the OR-list (use for "expected phrasing varies by locale or LLM") |

Adding a new `kind` requires:

1. A variant on `AssertionSpec` in
   `helpers/chat_scenario/format.rs`.
2. A dispatch arm in `helpers/chat_scenario/asserters.rs`.
3. A unit test for the asserter in the same file.

Forgetting either compile-fails, which is intentional — the
asserter dispatch is `match`-exhaustive so a new variant cannot
silently no-op.

## Telegram trace format

Best for "replay the actual conversation that broke." Same provider
state + turn structure as YAML; adds `captured_at` for provenance
and an optional `assistant_reply_seen` per turn (operator reference;
runner ignores it — we deliberately do NOT snapshot LLM output as a
golden).

```json
{
  "name": "2026-05-18 Telegram: freshness pushback",
  "locale": "fr",
  "notes": "ChefFamille Telegram screenshots 3+4",
  "captured_at": "2026-05-18T16:09:00-04:00",
  "provider_state": { "providers": { "strava": { ... } } },
  "turns": [
    {
      "user": "...",
      "trigger_sync_before_turn": true,
      "assistant_reply_seen": "the captured reply for triage reference",
      "assertions": [ { "kind": "no_substring", "values": ["Medical disclaimer"] } ]
    }
  ]
}
```

Save under `tests/scenarios/telegram_traces/YYYY-MM-DD-name-locale.json`.

## Execution modes

The same test file runs in two modes:

- **`cargo test --test chat_scenario_test`** (default) — structural
  pass: every scenario parses, the runner exercises the mock driver,
  the framework's own asserter tests run. Fast (< 1 s after the
  build cache is warm). This is the per-push CI gate.
- **`CHAT_SCENARIO_LIVE=1 cargo test --test chat_scenario_test -- --include-ignored`**
  — live-LLM pass: scenarios are driven through the real chat
  pipeline against the configured LLM. Cost-bounded by the
  scenario count × the per-scenario token budget. This is the
  nightly drift detector (see `.github/workflows/chat-conversation-eval.yml`).

The live driver itself lands in P3 of the gap-analysis plan; the
ignored test is the placeholder it slots into.

## Drift detection

The runner walks every turn's reply for numeric aggregate claims
(running distance, activity count) and records them in a
[`ClaimTimeline`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-server/tests/helpers/chat_scenario/drift.rs). At scenario end the drift
asserter flags any category where the most recent value differs
from an earlier value by more than the tolerance — catching the
"bot acknowledges new data but never updates the totals" bug class
without requiring an explicit assertion per scenario.

## Vocabulary contracts

Each coach personality declares a list of domain-vocabulary terms
the coach commits to use in every reply, regardless of question
topic. Both:

- The coach prompt in `dravr-contremaitre` instructs the LLM to use
  the terms.
- The `vocabulary_contract` asserter loads the same list and
  asserts the reply uses ≥1 term.

This is the industry-standard fix for "LLM gave generic recovery
advice when asked a strength question" — the prompt and the test
share one source of truth.

Compile-time defaults ship in
`helpers/chat_scenario/vocabulary_contract.rs::with_defaults`. When
the contremaitre manifest grows a `vocabulary_contract` field per
coach (P4 follow-up), the registry will load from there instead.

## Adding a scenario from a Telegram bug report

1. Reproduce the bug in your local Pierre fixture if possible. If
   not, take ChefFamille's screenshots verbatim.
2. Pick a name: `<bug-class>-<short-tag>-<locale>.yaml` for YAML,
   or `YYYY-MM-DD-<short-tag>-<locale>.json` for a trace.
3. Translate each user turn into a `user:` line. For each bot reply
   you'd want to assert on, pick the smallest set of `assertions`
   that would have caught the bug — favour `no_substring` for
   leaks, `tool_called` for missing tool invocations,
   `distance_mentioned`/`activity_count_mentioned` for numeric
   claims, and `vocabulary_contract` for coach-steering regressions.
4. Run `cargo test --test chat_scenario_test`. If a scenario file
   doesn't parse, the structural test surfaces the error with a
   path-and-message.
5. Open a PR; reference the originating bug in the scenario's
   `notes`.
