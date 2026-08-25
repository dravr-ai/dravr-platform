---
name: potential-feature
description: Records a feature idea as an R&D note in the dravr-vault Features/Potential folder. Use when the user floats an idea to investigate, asks for a feasibility or build-vs-buy call, or names an external data source, API, or competitor capability worth evaluating before committing to build.
argument-hint: [the idea, in a sentence]
user-invocable: true
---

# Potential Feature

A `Potential/` note is **one question and one verdict**. Everything between them is the
evidence that the verdict is not **speculation** — which is why a *rejected* idea leaves
the same record as an accepted one, and why the folder exists at all instead of the
analysis dying in a chat log.

**Facts are your job. Decisions are the user's.** Never ask what you could look up; never
decide what only they can.

## 1 — Search before writing

Read `Features/README.md` and `Features/Potential/README.md` in the vault (they carry
rules this skill does not repeat), then hunt for the idea already existing:

```bash
cd ../dravr-vault && grep -ril "<keyword>" Features/ DRAVR-BACKLOG.md Architecture/ADRs/ | head
```

**Done when** you can name the closest existing note and say why this idea is not it. If it
*is* it, edit that note and stop — a duplicate is worse than an edit.

## 2 — Ground it in the code

Build the *What the platform holds today* table: one row per capability the idea needs,
marked ✅ / ◐ / ❌, each carrying the `path:line` that proves it.

**Done when** every row names a file or says "none" explicitly. A row with an empty
evidence column is speculation and does not ship. Grep the crates; never answer from memory
— the interesting rows are usually the ones you were sure about (a field that exists but has
no writer, a provider value parsed only in a test fixture).

## 3 — Survey who else does it

`Market Research/` already covers 80+ tools in the field — read it before the web. Then
research the specific vendor, API, or dataset named in the ask: its terms, its licence,
whether the data is obtainable at all.

**Done when** you can name who does it, or say "nobody" and name where you looked.

## 4 — Grill the user

Map what is still open as a **design tree** and work it in **rounds**. The **frontier** is
every decision whose prerequisites are settled. Ask the whole frontier in one round,
numbered, each with your recommended answer, then wait for the reply before the next round:

```
❓ **Q1** — **<question title>**: <body, with the choices if there are any>

➡️ <your recommended answer>

---

❓ **Q2** — **<question title>**: <body>

➡️ <your recommended answer>
```

Steps 2 and 3 exist to shrink this round: a question you could have answered by grepping is
a wasted one. What genuinely belongs here is appetite, scope, ordering, what is deliberately
out of scope, and anything about coaches, pricing, or the business that is not readable out
of the repo. A question that depends on another question still open belongs to a *later*
round.

**Done when** the frontier is empty and the user has confirmed the verdict — not when you
have reached one you like.

## 5 — Write the note

`Features/Potential/<Title Case With Spaces>.md` — Title Case, not the kebab-case
`Catalogue/` uses. Never a `#` in the filename: Obsidian splits `[[Note#Heading]]` at the
first `#`. Write the file directly; `Potential/` sits outside the Templater folder-template
mapping (which covers `Features/Catalogue` only), so nothing rewrites it on create and no
`<%* … %>` block belongs in it.

`type: feature` is what `Features.base` selects on and `stage: potential` is what puts the
note in the **Potential / R&D** view. Missing either, the note is **invisible** to the only
view anyone reads:

```yaml
type: feature
title: <same as the filename>
date: YYYY-MM-DD
verified: YYYY-MM-DD          # when the code table was last checked
tags: [dravr, feature, r-and-d, <domain>]
stage: potential
health: on-track
phases_done: 0
phases_total: 0
surface: chat | web | mobile | api | infra | coach-tools | meta
owner: phil
created: YYYY-MM-DD
updated: YYYY-MM-DD
verdict: "<the answer, in prose — this is the payload>"
status: analysis
alpha_users: []
related_pillars: []            # training_and_movement | fuelling | sleep_and_recovery | mental_resilience | community_and_connection | recovery_optimisation
plans: []
adrs: []
```

Sections, in order: **source of the ask** · **what the platform holds today** (the table) ·
**the gap, in build order** · **why it fits Dravr** · **who else does it** · **what it
costs** · **verdict** · **promotion path** · **related**.

**Done when** `verdict:` reads as an answer someone could disagree with — a decision plus
its first move or its blocker, never a summary of the options.

## 6 — Register it in the index

Add a row to the table in `Features/Potential/README.md` (note · question it answers ·
verdict) and bump that file's `updated:`. The base finds the note by frontmatter; the README
table is hand-maintained and is what a human actually reads.

**Done when** the row exists and the note carries `[[wikilinks]]` — bare basename, never a
path — to the features, ADRs and research notes it touches.

## 7 — Commit

```bash
cd ../dravr-vault && git add -A && git commit -m "features: potential note — <title>" && git push
```

obsidian-git auto-commits every 10 minutes under a generic `vault: auto-save`; an explicit
commit is attributable and revertible.
