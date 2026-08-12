---
name: obsidian-writer
description: Write notes into the dravr-vault Obsidian knowledge base with the frontmatter
  its Bases require. Use when creating or updating an ADR, a runbook, a feature note, an
  API doc, or any dated working document — plan, audit, post-mortem, design analysis,
  research, report, session handoff — that should outlive the chat, even when the user
  never says "Obsidian". Also use when another skill needs to file its run report in the
  vault.
user-invocable: true
metadata:
  version: "2.0.0"
  domain: documentation
  triggers: obsidian, vault, note, document, adr, runbook, plan, audit, post-mortem, design analysis, research, report, handoff, feature, api-doc, guide, knowledge-base, architecture decision, session output
  role: specialist
  scope: implementation
  output-format: document
  related-skills: obsidian-vault-setup, obsidian-cli, obsidian-bases
---

# Obsidian Writer

Knowledge-base writer for `../dravr-vault`. Routes a note to the right folder and gives it
the frontmatter that makes it **visible** — because two of the vault's folders are
databases, and a note that misses their selector is **invisible** to them.

## Invisible is the failure mode

`Work Log/` and `Features/` are not folders you file into — they are
[Bases](https://help.obsidian.md/bases) that select on a frontmatter field:

| Folder | Base | Selector | A note without it |
|---|---|---|---|
| `Work Log/` | `Work Log.base` | `type: worklog` | never appears in any view — no matter its tags, folder, or filename |
| `Features/` | `Features.base` | `type: feature` | same |

Location is *not* the selector. The bases ignore folders entirely, which is why moving a
note between month buckets is safe — and why writing one into the right folder without
`type:` still loses it. Get the selector right and the filing is forgiving; get it wrong
and the note is gone from the only view anyone reads.

## Route the note

| Doc type | Destination | Selector + required fields |
|---|---|---|
| Plan, audit, post-mortem, design analysis, research, report, decision, handoff | `Work Log/<YYYY-MM>/` | `type: worklog` + `kind`, `area`, `status`, `date`, `updated` |
| Feature / capability entry | `Features/Catalogue/<YYYY-MM>/` | `type: feature` + `stage`, `surface`, `created`, `updated` |
| ADR | `Architecture/ADRs/ADR-NNN Short Title.md` | `date`, `status`, `tags: [dravr, adr]` |
| Runbook | `Development/Runbooks/` | `date`, `service`, `severity`, `tags: [dravr, runbook, sre]` |
| API doc | `APIs/` | `date`, `service`, `tags: [dravr, api]` |
| Guide / how-to | `Development/Guides/` | `date`, `tags: [dravr, guide, <domain>]` |
| Methodology | `Methodology/` | `date`, `tags: [dravr, methodology]` |

`Work Log/` replaced `Claude Plans/` + `Claude Outputs/` on 2026-08-12. Both are gone; so
are their 13 themed subfolders. What those folders encoded is now `kind:` and `area:`.

### Work Log fields

| Field | Values |
|---|---|
| `kind` | `plan` · `audit` · `post-mortem` · `design` · `research` · `report` · `decision` · `handoff` |
| `area` | `coaching` · `providers` · `messaging` · `llm` · `infra` · `web` · `chat` · `security` · `business` · `architecture` · `notifications` · `testing` · `meta` |
| `status` | `draft` · `active` · `completed` · `superseded` |
| `date` | `YYYY-MM-DD` — when the work happened. **Picks the month bucket.** |
| `updated` | `YYYY-MM-DD` — drives the staleness column |
| `features` | `"[[wikilink]]"` list back into `Features/` |

`area: meta` is the escape hatch and lands the note in the **Needs triage** view. Pick a
real area unless the note genuinely spans the whole platform.

For `Features/`, read `Features/README.md` in the vault before writing — the stage
lifecycle and the PostHog pairing have rules this table doesn't carry.

## Workflow

**1 — Search before creating.** A duplicate is worse than an edit.

```
obsidian search query="<keywords from the topic>" limit=5
obsidian read file="<name>"      # read any match before deciding to add a new note
```

**2 — Compose.** Frontmatter first, then an `#` heading matching the filename. Start from
the vault template for the type (`Templates/Work Log.md`, `Feature.md`, `ADR.md`,
`Runbook.md`) — read it with `obsidian read file="Work Log"` and fill the skeleton, or
strip the `<%* … %>` Templater block if writing the file directly.

**3 — Write to the live vault.** Obsidian must be running with dravr-vault focused.

```
obsidian create name="<Note Title>" path="Work Log/2026-08/<Note Title>.md"
obsidian append file="<Note Title>" content="## New Section\n<content>"
```

Creating the note at the `Work Log/` *root* lets Templater file it into the current month
automatically. Naming the bucket explicitly also works and is what to do when `date:` is
not this month.

**4 — Verify and link.** Read the note back, then add a `[[wikilink]]` to it from whatever
it relates to — a feature note's `plans:`, an ADR, the prior post-mortem. An unlinked note
is findable only by search.

## Constraints

- **Never fabricate a field to fill the schema.** An invented `status` or a guessed `area`
  is worse than `meta` plus a note saying it needs triage — the bases have a **Needs
  triage** view for exactly this.
- **Wikilink by bare basename** (`[[Guardian Observability and Progressive Enforcement]]`),
  never by path. Basename links survive the month-bucket reorganisations; path links break
  on the first move.
- **Never put `#` in a filename you intend to wikilink.** Obsidian splits `[[Note#Heading]]`
  at the first `#`, so `[[Risk #5 gate …]]` silently resolves to a note called "Risk".
- **`date` is ISO 8601** (`YYYY-MM-DD`), and for Work Log notes it decides the folder.
- **ADR filenames carry the zero-padded number** so they sort. Check the highest existing
  number first — the vault already has accidental duplicates (two ADR-011, two ADR-018,
  two ADR-020).
- The `obsidian` command MUST be the first-party app CLI
  (`/Applications/Obsidian.app/Contents/MacOS/obsidian`). It needs **no API key**. If it
  errors with "An API key must be provided via OBSIDIAN_API_KEY", a stray global npm
  package (`obsidian-cli`, the unrelated ObsidianQA tool) is shadowing it on PATH — fix
  with `npm uninstall -g obsidian-cli`. Verify with `command -v obsidian`.
- **`claude_docs/` is the fallback, not the default.** It is a gitignored symlink to the
  vault's `Work Log/`, so a Write-tool note must include the `type: worklog` frontmatter
  itself — nothing downstream adds it.
- **Commit explicitly** after writing. obsidian-git auto-commits every 10 minutes under a
  generic `vault: auto-save` message; an explicit commit is attributable and revertible.
- See `references/vault-structure.md` for the full directory map and naming conventions.
