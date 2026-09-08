---
name: bilan
description: Measure whether this session actually finished — the 0-10 completion number, computed from git, the carnet ledger, LIMITATION markers and CI instead of narrated. Use before reporting a completion number, when asked "where are we", at the end of a piece of work, and to find what a session that died left behind.
argument-hint: "[sweep] [--cheap] [--json]"
user-invocable: true
---

# Bilan

**The completion number is this script's number, never yours.**

It used to be narrated: it came from my account of the session, so a session that *believed* it
was done reported 8 or 10 while it still held open carnet issues, had commits sitting unpushed,
or had left a dev stack running. Every one of those facts is machine-checkable, and
`bilan.sh` checks them.

```bash
.agents/skills/bilan/bilan.sh            # full: local facts + one CI query
.agents/skills/bilan/bilan.sh --cheap    # local only, no network
.agents/skills/bilan/bilan.sh sweep      # what a session that DIED left behind
```

Exit code: `0` = 10/10 · `1` = incomplete · `2` = the script itself failed.

## The rules

1. **Run it before you give a number.** Report the score it prints. If you believe a cap is
   wrong, say so in words *and still report the script's number* — arguing with the measurement
   is a conversation, overriding it silently is the failure this exists to stop.
2. **A cap is a fact, not an opinion.** Each one prints its own evidence and its own remedy.
   The score is `min()` over the caps, so one open carnet issue holds the whole session at 6 no
   matter how much else landed.
3. **Failures never deduct.** A red that is now green, a mistake found and fixed, a rough path —
   none of it lowers the number. The score measures *completion*, and only completion.
4. **Friction is printed, never scored.** Tool errors, interrupts and denials are counted from
   the transcript and shown for context. They do not cap anything.

## What caps the score

| Evidence | Caps at |
|---|---|
| carnet issue claimed by this session, neither closed nor released | **6** |
| `LIMITATION(registre#…)` marker added in source naming no live issue | **6** |
| tracked files modified and uncommitted | **7** |
| commits not pushed | **8** |
| CI red on the pushed head | **5** |
| CI still running, cancelled, or not yet registered | **9** |
| issues *filed* this session | **9** — name each as deliberate residue |
| untracked files, stash created this session, branch whose upstream is gone | **9** |
| `.git/validation-passed` missing, stale, or for another sha | **9** |
| dev stack from this checkout still up | **9** |

## Uncommitted files that are not this session's

Several sessions share the main checkout. In bilan's first hour, a peer's mid-edit files held
**three** other sessions at 7 — and those sessions were right to refuse to touch them, so they
could not reach 10 no matter what they did. That was a category error: the score measures *this
session's* completion, and a peer's in-flight file is not this session's incompleteness.

Two mechanisms fix it, and the first needs nothing from you.

**The baseline.** The SessionStart hook records which tracked files were already dirty when the
session opened. A path dirty before the session existed is definitionally not its work — that
much *is* machine-decidable. Those files are stated as a note and never scored.

**`ack`, for what goes dirty afterwards.** A peer editing during your session is not covered by
the baseline, so you say so once:

```bash
bilan.sh ack --why "a peer's embacle pin bump, written into this shared checkout at 10:16"
```

That **clears** the cap rather than softening it, and carries the reason into every later
report. It is keyed to the exact set of paths, so dirtying one more file brings the cap
straight back, and ownership is a property of the files rather than their contents, so a peer
changing those same files again stays covered.

**Why not attribute automatically?** It was tried and it does not work. Claude Code records the
paths a session touched under `file-history-snapshot.trackedFileBackups`, but only for the
Edit/Write tools. Sessions here work Bash-first, and that map came back **empty** for a session
that had just written nine files. Attributing on it would have called a session's own work a
peer's and stopped blocking — the worst direction to be wrong in.

Two other caps deserve a note. **CI absence is its own outcome** — `gh run list --commit` returns
zero rows on this org even when runs exist, so rows are matched by `headSha` out of a wide branch
window, and a sha with no row reads as *absent, not necessarily done*, never as green. And a
**peer's worktree is not yours**: cross-checkout state is reported by `sweep`, never as a cap on
your score.

## The three call sites

**`/bilan`** — on demand, the full measurement including CI.

**The Stop gate** (`hooks/stop-gate.sh`) — refuses the first stop while the state is dirty and
sends the session back to work with the list. Two things bound it. It blocks **only at 8 or
below**, so a stray untracked file or a CI run still in flight is reported without interrupting
anyone — what blocks is a held carnet issue, an unregistered marker, uncommitted tracked files,
unpushed commits, red CI. And it blocks **once per distinct state signature**: fix one thing and
it speaks again about what is left; fix nothing and it stays quiet, because after it has told
you the outstanding work is your accountability, not the hook's. The reason it prints lists
every cap, including the 9s, so the threshold decides when to interrupt and never what to hide.

**The startup sweep** (`hooks/session-start-sweep.sh`) — the only cover for a session that was
killed. A `kill -9`, a closed terminal or an exhausted context fires no exit hook, so the dead
session can never report on itself; the next session in the repo looks for it instead, across
every worktree and every ledger on the machine. It found `carnet#236`, held by a session that
died on 2026-09-03, five days after the fact.

## What it does not do

It cannot tell you whether the work is *good*, only whether it is *finished*. A green bilan on
a wrong implementation is still a wrong implementation — that is what `/code-review` is for.
