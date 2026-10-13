---
name: retro
description: Turn the period's repeated review findings, recurring carnet themes and session friction into proposals — new gates for anything caught twice, standards additions or removals, AGENTS.md navigation hints, token-efficient tool use, instruction files or skills to cut. Use weekly, at the end of a slice, or when asked "what should the next session inherit?".
argument-hint: "[--days N | --since YYYY-MM-DD]  (default: the last 7 days of origin/main)"
user-invocable: true
---

# Retro

Every Tier 1x gate and most rules in `AGENTS.md` began as an incident somebody noticed twice:
carnet#399 became gate-on-exit-status, carnet#474 became Tier 1m, and carnet#169 became the sha
concurrency groups. When nobody notices, a later session writes the same review comment again.
`bilan` measures whether a session finished. This step decides what the next session inherits
(carnet#662).

It has two halves, kept apart on purpose:

- **`retro.sh` collects.** It is deterministic: the same inputs always print the same digest. It
  counts and quotes the evidence and never proposes anything.
- **You propose.** Each proposal is a judgement the digest supports. Every one quotes the digest
  line it rests on, so a reader can check it without re-running anything.

## 1. Collect

```bash
.agents/skills/obsidian-writer/vault-sync.sh pull          # the vault is an input; read it current
git fetch -q origin                                        # a shallow clone may need --shallow-since=<date>
.agents/skills/retro/retro.sh --days 7 --save-carnet "$SCRATCH/retro-carnet" > "$SCRATCH/retro.md"
```

Use a scratch directory for `$SCRATCH`, never the repo. `--save-carnet` keeps the tracker
snapshot. With `--carnet-from <dir>`, the same digest can be rebuilt later by whoever reviews the
proposals. If the clone is shallow and its history does not reach the window, git silently
returns fewer commits. Check that the digest's commit count looks like a week of main
(~20–35 a day).

What the digest reads (sections 1–7):

| Source | What it yields | Gap when it is missing |
|---|---|---|
| git log on `origin/main` | commit types, `Reviewed-Standards:` coverage (a `covers <sha>` trailer clears an earlier commit), `review:` fix-up commits, carnet issues referenced by 3+ commits, files touched by 3+ `fix` commits, gate files and pre-push tiers added, instruction files changed | none (required) |
| carnet REST (`gh api`, else curl through the agent proxy) | review-standards run records grouped by theme. Each run is one comment (the carnet#660 thread), so "caught twice" means two separate runs. Also issues created and closed, recurring title terms, CI-red issues and "Directive from JF" comments | sections 2 and 5 say why |
| vault `Work Log/` | notes dated in the window, by kind; the bullets under any lessons or follow-up heading | section 6 says so |
| `~/.claude*/projects/*<repo>*` transcripts | tool mix, errors, largest results, empty searches, files read 3+ times, skills invoked | **this machine only.** A cloud VM's transcripts are lost when it ends. `--transcripts <dir>` (repeatable) reads the named directories instead of that default |

The theme table that classifies review findings is `THEMES` in `retro.sh`. When the digest lists
an *unclassified* label that recurs, add it to the table rather than reading around it.

## 2. Propose

Write proposals in exactly these five categories, which are the ones carnet#662 names. Leave a
category empty when the evidence does not support anything, and say that it is empty.

1. **New gates, for anything caught twice.** Candidates are a theme marked *caught twice*, a file
   touched by 3+ `fix` commits, and a carnet issue fixed repeatedly. Before you propose a gate,
   check the digest's *existing gates* list and `scripts/ci/README.md`, because a recurrence the
   gate already covers is a gate bug and not a new gate. A gate needs a mechanical predicate (a
   grep, a parse, a diff of two files) and a self-test that proves it fires
   (`docs/coding-standards.md`, "CI gates and scripts"). A judgement such as "is this comment
   true?" is not a gate. Propose the narrower mechanical part instead, for example history words
   like "as before" or "no longer" in the lines a diff adds to a comment.
2. **Standards: additions and removals.** Propose an addition when a finding cites a rule that
   lives outside `docs/coding-standards.md`, because the reviewer reads only that file. Propose a
   removal or a move to a gate when a standards section found nothing across every run in the
   window, or when a gate now enforces it.
3. **Navigation hints for AGENTS.md.** Use the empty searches and the files read 3+ times. AGENTS.md
   is near its byte budget (section 4 prints the figure). A hint must name the removal that pays
   for it, or go into the skill that owns the area.
4. **Token-efficient tool use.** Use the largest results and the tool mix. Propose the narrower
   command, such as `--stat` before `-p`, `head`, `--jq`, or a targeted `--test`. Do not propose a
   general rule to be careful.
5. **Redundant instruction files or skills to cut.** Candidates are skills named nowhere else and
   never invoked, two skills or files that cover the same procedure, and instruction text that
   repeats a gate. "Named nowhere" is a candidate list, not a verdict, because a skill is also
   reached through its description. Before you propose a cut, read the skill and check it against
   the transcripts.

Every proposal gets a row: id, category, the evidence (quote the digest), the change, its cost,
and an empty **verdict** column for a human to fill in.

## 3. Record, then let a human decide

Write the run up as a vault report with the `obsidian-writer` skill: `Work Log/<YYYY-MM>/`,
`type: worklog`, `kind: report`, `area: meta` (it spans the platform), `status: active` until
every verdict is filled, plus `date` and `updated`. Put the digest's headline numbers in the
note, not the whole digest, and link the previous retro.

**A proposal is not a decision.** ChefFamille accepts or rejects each row. Only an accepted row
becomes work, as an issue filed through `carnet.sh create`. A retro session does not implement
its own proposals unless it is told to, because a gate built on an unaccepted proposal is exactly
the unreviewed instruction growth this step exists to remove.

## Trial

carnet#662 runs this for one week on real sessions. Then each proposal gets a verdict, and a
decision is made on whether to keep the skill and at what cadence. Until that decision, run it
weekly over the last 7 days of `origin/main`.

## Not covered

- **Matt Pocock's `retro` skill** is the starting point the issue names. It is not installed
  here, and its tracker-aware siblings default to `gh issue` against the current public repo
  (carnet#660, precondition B). This skill reads carnet over REST and writes to the vault
  instead.
- **Review runs that are never recorded are invisible.** Record each review-standards run in the
  carnet#660 thread, as that skill's Pilot section asks, or the retro never sees it.
