---
name: single-source-review
description: Sweep a range of main for the one defect this codebase produces — a new single source lands and the old one stays alive. Use on a schedule (fortnightly), before a release, or when asked to review recent commits for duplication or architecture problems.
---

# Single-source review

The platform takes ~32 commits a day, almost all agent-authored. One defect
dominates: **a new single source of truth lands and the old one stays alive**,
in the same commit or the next. Thirteen pre-push tiers do not catch it, because
each gate asks its own originating incident's question and this defect fails
none of them.

The 2026-09-02 pass found **41 verified findings in 30 commits** (15 hours of
main). One session, start to swept. That yield is why this runs on a schedule.

## When

- **Fortnightly**, or every ~450 commits on main, whichever comes first.
- Before a release.
- When asked to "review the last N commits" for architecture or duplication.

## 0 — Claim it before you fix anything

The review itself needs no issue. **The fix branch does.** Self-found work
claims nothing, so no peer can see it coming:

```bash
.agents/skills/carnet/carnet.sh create \
  --title "Single-source review of <base>..<tip>" \
  --body-file /tmp/scope.md --claim
```

On 2026-09-02 this step was skipped. Another session rebuilt the same settings
surface in parallel; the squash conflicted in 14 files and three hours of
finished, green work was discarded. One minute against three hours.

## 1 — Pick the range and read it

```bash
BASE=$(git log --oneline -n 200 main | sed -n '200p' | cut -d' ' -f1)  # or the last sweep's tip
git log --oneline $BASE..origin/main | cat
git diff --stat $BASE origin/main | tail -1
```

Separate the **substantive** commits from chore/ci/docs/bump. The last pass had
12 substantive out of 30; only those can carry this defect.

**Read the commit messages first, and treat these as the primary signal:**

> "one list" · "single source" · "eliminates drift" · "shared" · "unified" ·
> "no longer duplicated" · "both clients now"

A message making that claim is a hypothesis to falsify. Four of the twelve
substantive commits made it and were wrong in the same tree.

## 2 — The five patterns

Work each one over the range. Every candidate needs a `file:line` on **both**
sides — the new source and the old one still alive.

### A. New source landed, old one left alive

For each commit that adds a shared helper, constant table or module:

```bash
git show <sha> --stat | grep -E '^ create'          # what it added
git show <sha> | grep -E '^\+.*export (const|function)|^\+pub (fn|const|struct)'
```

Then grep the tree for anything still doing that job. The old copy usually
still compiles, still has callers, and still has a comment describing itself as
the source of truth.

### B. Two doors to one room

Two routes, two entry points or two components reaching one surface.

```bash
rg "window.location.hash = " frontend/src            # raw navigation
rg "id: '" packages/shared-constants/src/surfaces.ts # the registry's own answer
```

A surface reachable two ways will drift: which one a user lands on depends on
where they came from.

### C. Declared, never consumed — **check BOTH directions**

This is the pattern the gates structurally cannot see. Tier 1c asks
"api-client method with no production caller" — it starts at the client, so a
server capability with no client is invisible to it.

```bash
# client -> server (Tier 1c covers this)
./scripts/ci/check-phantom-surfaces.sh main

# server -> client (NOTHING covers this; do it by hand until the gate exists)
rg -o '"/api/[a-z0-9/_-]+"' crates/*/src/routes | sort -u > /tmp/routes.txt
# for each: does packages/api-client mention it at all?
```

`GET /api/personas` had a route, an axum shim, a renderer and a 302-line
endpoint test, and zero clients, for weeks.

### D. Written twice

```bash
rg -n "new Intl.DateTimeFormat" frontend/src frontend-mobile/src packages
rg -n "tokio::time::interval" crates/*/src
```

Look for the same rule spelled in more than one place: date formatters, tick
loops, locale resolution, label maps, filter predicates. Twelve `formatDate`
copies and eight hand-rolled tick loops were found this way.

### E. Dead code kept warm, registries by hand

```bash
# a constructor nobody calls
rg -n "pub fn new" crates/<crate>/src | while read -r l; do :; done  # then rg each symbol
rg -n "const [A-Z_]+_TOOLS|KNOWN_|EXEMPT_|PENDING_" crates
```

`CoachesManager` had 2,269 lines, zero constructors called anywhere, and was
still receiving security fixes. A hand-curated list sitting beside a registry
(`TASK_CAPABLE_TOOLS`) is the same defect in registry form.

## 3 — Verify every candidate before you report it

**Non-negotiable, and the step that costs the most credibility when skipped.**
The last pass had ~3 wrong candidates in ~44 (7%), and one was backwards: it
claimed `.agents/skills/` was eight copies of `.claude/skills/*` when
`.claude/skills` holds the symlinks and `.agents/` the real directories —
deleting it would have broken the assets.

For each candidate, run a check that **can disconfirm it**:

- "Nothing reads X" → `rg -n "X" --glob '!*test*'` across the whole tree, then
  check the file that *does* match is itself reachable.
- "These two are identical" → diff them. Three admin `formatDate` copies turned
  out to be three different shapes, nine of them deliberately English-only.
- "This is a copy" → check which way the symlink points before calling either
  side the duplicate.

A finding you could not disconfirm is not verified; it is a hunch.

## 4 — Report, then split the work

Write the findings up as a single artifact with `file:line` evidence on both
sides of each — the reader must be able to check any one of them in one grep.

Then split:

| Kind | Where it goes |
|---|---|
| Mechanical (delete the dead copy, fold the duplicates) | fix it |
| A product call (which of two behaviours is right) | carnet issue, do NOT decide it |
| Blocked on another repo or worktree | carnet issue naming the blocker |

Three of the 41 were product calls and were registered rather than resolved:
carnet#231, #232, #233. **Never decide scope for ChefFamille** — surface the
choice with its cost.

## 5 — Land in slices

The last pass landed as one 34-commit branch and paid for it. Main moved four
times underneath it in fourteen hours — once during the final pre-push gate —
and a colliding refactor landed mid-flight.

- Land each slice as it goes green. Do not accumulate.
- `git fetch origin main` immediately before every push; on this repo that is
  the expected path, not a precaution.
- Read what the other side added before resolving a conflict in your own
  favour. Main's new `record_tool_call` audit had its **only** call site inside
  a dead dispatcher being deleted; landing the deletion unchanged would have
  silently killed their feature.

## Not covered by this skill

- **The reverse-direction gate** belongs in `scripts/ci/` as a pre-push tier,
  not here. A skill is a procedure a session follows; a gate is a check that
  runs whether anyone remembers it. Section 2C is the manual stand-in until
  that script exists.
- **The commit-template line** ("what did this replace, and where did it go?")
  is a template change, not a procedure.

## Related

- `carnet` — claiming, and filing the product calls
- `register-limitation` — when the right answer is an honest documented gap
- `obsidian-writer` — the audit note goes to dravr-vault `Work Log/`
