---
name: validate-coaches
description: Validates the agent definition markdown files (the dravr-contremaitre coach catalogue) for required frontmatter fields, sections, layout and naming conventions
user-invocable: true
---

# Validate Coaches Skill

**CLAUDE: When this skill is invoked with `/validate-coaches`, validate every agent definition file in the catalogue directory — `../dravr-contremaitre/prompts/coaches` by default, or `$PIERRE_COACHES_DIR` when set.**

## Purpose

Validates the agent definition files — the markdown-with-frontmatter documents that seed the `coaches` table — for schema compliance, required fields, section structure, locale layout and naming conventions, using exactly the rules `pierre-coach-parser` and the seeder enforce.

Vocabulary: the athlete-facing word for one of these personas is **agent** (decided 2026-09-05, ADR-026). The files, the `prompts/coaches/**` directory, the parser crate, the `coaches` table, the `## Related Coaches` heading and the `pierre-cli seed coaches` command keep the word *coach* — those are identifiers, and this skill names them as written.

## Usage

```bash
/validate-coaches
```

## Where the files live

The catalogue is owned by **dravr-contremaitre**, not by this repo. There is no `coaches/` directory here.

```
prompts/coaches/<category>/<slug>/<locale>.md
```

- `<category>` — one of the `CoachCategory` wire names (`crates/pierre-core/src/models/coaches.rs`): `training`, `nutrition`, `recovery`, `recipes`, `mobility`, `analysis`, `custom`. The catalogue ships `training`, `nutrition`, `recovery` and `mobility` today.
- `<slug>` — the agent's kebab-case slug, e.g. `training/marathon-coach/`. It is the `@handle` the athlete types and the `coaches.slug` column; it never changes.
- `<locale>.md` — `en.md` is the canonical file; `fr.md`, `es.md`, `de.md`, `pt.md` are translations. `is_locale_code` in `crates/pierre-coach-parser/src/lib.rs` is the only list of recognised stems; the seeder globs `<category>/<slug>/*.md` and ignores any other filename.

Every catalogue directory today carries `en.md` and `fr.md`.

## Validation Steps

### Step 1: Find every agent definition

```bash
COACHES_DIR=${PIERRE_COACHES_DIR:-../dravr-contremaitre/prompts/coaches}
find "$COACHES_DIR" -mindepth 3 -maxdepth 3 -name '*.md' -type f | sort
```

Anything at depth 1 or 2, or a file whose stem is not a locale code, is not seeded — flag it.

### Step 2: For each file, validate

**CLAUDE: Read each file and verify against the parser (`crates/pierre-coach-parser/src/lib.rs`):**

#### Frontmatter (required)

- The file starts with `---` and has a closing `---` — the parser rejects both omissions loudly.
- `name`: kebab-case, and **equal to the parent directory slug** (`training/marathon-coach/en.md` must say `name: marathon-coach`). A mismatch fails the parse.
- `title`: present and non-empty. This is the display title the athlete sees, so it follows the agent vocabulary ("Marathon Training Agent", fr "Agent marathon"); the slug under it stays as is.
- `category`: one of the wire names above, and matching the parent category directory.

#### Frontmatter (optional)

- `tags`: array of strings.
- `prerequisites.providers`: array of provider names; `prerequisites.min_activities`: integer; `prerequisites.activity_types`: array of activity types.
- `visibility`: `private` | `tenant` | `global` (`CoachVisibility`). The catalogue sets `tenant` explicitly.
- `startup.visuals`: array of `chart` | `table` (`VisualKind`) — the inline visual kinds this agent may embed; empty means it is never told the visual contract.
- `startup.query`: the opening question the agent runs; `startup.data_requirements`: the activity window it needs (`count`, `time_frame`, `mode`, `analysis_type`). The parser validates their shape.
- `replaces`: array of retired slugs this agent absorbed. The seeder re-points conversations, groups and pointers bound to a retired slug at the successor before pruning the retired row.

#### Sections (required)

- `## Purpose` — must exist with content.
- `## Instructions` — must exist with content. This is the system prompt: it must not name the model's role with a noun ("You are Dravr, an expert in X" — never "you are a coach" or "you are an agent"; see D3 in the rename brief and `scripts/ci/check-contremaitre-sync.sh`).

#### Sections (optional)

- `## When to Use`, `## Example Inputs`, `## Example Outputs`, `## Success Criteria`.
- `## Related Coaches` — the heading is a parser keyword; keep it spelled exactly so.

#### Translations

- Every `<locale>.md` beside `en.md` carries the same `name` and `category` and the same section set; only `title` and prose are translated, in that locale's register.

### Step 3: Layout cross-checks

```bash
# category directory must match the frontmatter category
for f in $(find "$COACHES_DIR" -mindepth 3 -maxdepth 3 -name '*.md'); do
  dir_cat=$(basename "$(dirname "$(dirname "$f")")")
  fm_cat=$(sed -n '/^---$/,/^---$/p' "$f" | sed -n 's/^category: *//p' | head -1)
  [ "$dir_cat" = "$fm_cat" ] || echo "CATEGORY MISMATCH: $f ($dir_cat vs $fm_cat)"
done
```

```bash
# name must match the slug directory
for f in $(find "$COACHES_DIR" -mindepth 3 -maxdepth 3 -name '*.md'); do
  slug=$(basename "$(dirname "$f")")
  fm_name=$(sed -n '/^---$/,/^---$/p' "$f" | sed -n 's/^name: *//p' | head -1)
  [ "$slug" = "$fm_name" ] || echo "NAME MISMATCH: $f ($slug vs $fm_name)"
done
```

## Run the parser against the whole catalogue

The seeder parses every file the same way the daily Cloud Run job does; `--dry-run` writes nothing.

```bash
PIERRE_COACHES_DIR="$COACHES_DIR" cargo run --bin pierre-cli -- seed coaches --dry-run
```

It fails on the first malformed file and names it. Without `--dry-run` it upserts the rows, prunes catalogue-owned system agents whose directory is gone, and honours `replaces`.

The contremaitre ↔ database drift gate is the same parser:

```bash
PIERRE_COACHES_DIR="$COACHES_DIR" cargo run --bin pierre-cli -- check-drift coaches
```

Parser tests live in `crates/pierre-server/tests/` (`rg "pierre_coach_parser" crates/pierre-server/tests --files-with-matches`); run one with `cargo test --test <file> -- --nocapture` and confirm `running N tests` with N > 0.

## Validation Checklist

For each agent definition file:
- [ ] Path is `<category>/<slug>/<locale>.md`, and `<locale>` is one of `en`, `fr`, `es`, `de`, `pt`
- [ ] Valid YAML frontmatter between `---` markers
- [ ] `name` equals the slug directory
- [ ] `title` present, in the agent vocabulary
- [ ] `category` is a `CoachCategory` wire name and matches the category directory
- [ ] `## Purpose` and `## Instructions` exist with content
- [ ] `## Instructions` names no role noun for the model
- [ ] Optional sections, when present, use the exact headings above
- [ ] Every translation beside `en.md` keeps `name`, `category` and the section set

## Success Criteria

- Every file parses under `pierre-cli seed coaches --dry-run`
- No file sits outside the `<category>/<slug>/<locale>.md` layout
- `check-drift coaches` reports no drift after a seed
- No orphaned directory (a slug directory without an `en.md`)

## Example Valid Agent Definition

`prompts/coaches/training/marathon-coach/en.md` in dravr-contremaitre:

```markdown
---
name: marathon-coach
title: Marathon Training Agent
category: training
tags: [running, marathon, endurance, long-runs, race-strategy, 26.2]
prerequisites:
  providers: [strava, garmin, fitbit, whoop, coros, terra]
  min_activities: 10
  activity_types: [Run]
visibility: tenant
startup:
  visuals: [chart, table]
  query: "Analyze my weekly mileage, long run progression, and identify any patterns in my training."
---

## Purpose
Expert in marathon preparation, long runs, and race day strategy.

## Instructions
You are Dravr, an expert in marathon preparation. Ground every prescription in the athlete's actual recent sessions …
```

## Related Skills
- `test-intelligence-algorithms` - Algorithm validation
