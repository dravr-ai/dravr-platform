---
name: register-limitation
description: Register a known gap in the limitation register — file the issue in the private tracker, write the LIMITATION(registre#n) marker, or ledger a dark-launched feature. Use when the register gates fail, or when you are about to document why something is incomplete.
user-invocable: true
---

# Register a Limitation

## When this fires

Any of these, without exception:

- The register gates failed — "unregistered deferral/confession prose" or "malformed LIMITATION marker".
- You are about to write a comment explaining why something is incomplete, restricted, or deferred.
- You are shipping a feature **disarmed** (flag off, shadow mode, log-only phase).
- You found a gap while reading code and are not fixing it in this change.

The gates are the Apache-2.0 [llm-registre](https://github.com/dravr-ai/llm-registre) tool,
vendored at `.build/vendor/llm-registre/` and run by `scripts/ci/architectural-validation.sh`
(pre-push Tier 1 and CI fast-gate).

## Step 0 — try to not need this

The register exists so honest gaps become tracked obligations, **not** so gaps become easy to
ship. If you can implement the real thing now, do that instead. Registering is the fallback, and
it costs a permanent entry someone has to close later.

## Where issues go

| | |
|---|---|
| Tracker | **`dravr-ai/dravr-carnet`** — PRIVATE, shared by the whole `dravr-*` family |
| Labels | `limitation` + the repo name (e.g. `dravr-platform`, `dravr-canot`) |
| Title | `[platform] <short statement of the gap>` — always project-prefixed |

**Never file on `dravr-ai/dravr-platform` or any other code repo.** They are PUBLIC. A limitation
issue states precisely where a defence is incomplete, which is a roadmap when the code is open.
Registers are per project: other projects (atmosphere, doodgamev2, snag, mirroir-mcp) each have
their own — `registre.toml` at the repo root always names the right one.

Issue bodies may hold reasoning and residual risk; the code comment stays thin.

## Step 1 — file the issue

```bash
gh issue create -R dravr-ai/dravr-carnet \
  --title "[platform] Short statement of the gap" \
  --label limitation --label dravr-platform \
  --body "Where it is (file + symbol). What is incomplete. What the correct fix looks like."
```

## Step 2 — write the marker

On the comment line that names the limited item:

```rust
/// LIMITATION(registre#42): `ChannelDescriptor::max_message_length` is not threaded through
/// `PlatformCommandContext`, so this is the cross-channel floor, not the per-channel value.
const PLAN_TEXT_BUDGET: usize = 2_000;
```

Rules that make a marker valid rather than decorative:

- The literal is `registre#<number>` — the bare word, never the tracker repo name. The tracker is
  configuration (`registre.toml`); the marker never changes when it moves.
- **Name the limited item on the marker line** (the symbol, variant, or endpoint). A marker that
  says only "this is incomplete" is unsearchable.
- The marker exempts **its own line** from the prose ban, not the file. A second unmarked deferral
  sentence on the next line still fails.

## Step 3 — if the feature ships disarmed, ledger it too

Add to `feature-phases.yaml` (fixed shape — the review workflow parses it with `awk`):

```yaml
  - name: kebab-case-feature-name
    surface: crates/pierre-tool-runtime/src/guardian/policy.rs
    current: what ships today, i.e. the disarmed state
    advance_when: the criterion that arms the next phase
    review_by: 2026-09-30
```

A weekly workflow opens a `feature-phase` issue in the tracker once `review_by` passes, so phase 1
cannot silently become forever. Keep values free of `": "` and `" #"`.

## Step 4 — verify

```bash
.build/vendor/llm-registre/limitation-gates.sh crates frontend/src frontend-mobile/src packages
```

Expect all three gates green. `scripts/ci/architectural-validation.sh` runs the same thing plus the
platform-specific phantom-capability check.

## Closing an entry

Fix the gap, **delete the marker in the same change**, close the issue. A stale marker still
exempts prose from the gates, so exhausted markers are debt of their own.

## Consume what you declare

A capability predicate, enum variant, or trait method whose only callers are tests is a phantom
surface. Wire a production consumer in the same change, or register it here with a marker naming
the item. CI enforces this for the canot messaging surface (`supports_*` / `max_*` predicates,
`MessageContent` variants).

## What the register does not cover

These gates are per-change: they stop new debt at authoring time and cannot reach the standing
stock of defects that live between diffs — a handler nothing reaches, an override nothing reads,
two components each locally correct and jointly wrong. Those come out of periodic adversarial
cold-reads and get filed here like anything else. A green gate means no new unregistered debt, not
a clean codebase.
