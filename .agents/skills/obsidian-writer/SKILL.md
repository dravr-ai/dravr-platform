---
name: obsidian-writer
description: Write well-formatted notes to the dravr-vault Obsidian knowledge base.
  Use this skill whenever creating or updating an ADR, runbook, plan, API doc, guide,
  session output, or any structured document that should land in the vault — even when
  the user doesn't say "Obsidian" explicitly. Delegates to obsidian:obsidian-cli to
  write to the live vault and applies Dravr frontmatter and formatting standards.
user-invocable: true
metadata:
  version: "1.0.0"
  domain: documentation
  triggers: obsidian, vault, note, document, adr, runbook, plan, api-doc, guide, knowledge-base, architecture decision, session output
  role: specialist
  scope: implementation
  output-format: document
  related-skills: obsidian-vault-setup
---

# Obsidian Writer

## Role

Knowledge-base writer for the dravr-vault shared Obsidian vault. Applies Dravr
frontmatter standards and directory conventions so every note lands in the right place
with the right metadata — consistent enough to be searched, linked, and understood later.

## When to Use

Invoke for any of these document types, whether the user names the type or not:

- **ADR** — Architecture Decision Record (`Architecture/ADRs/`)
- **Runbook** — Operational procedure or on-call guide (`Development/Runbooks/`)
- **Plan** — Feature or project planning document (`Claude Plans/`)
- **API doc** — Endpoint reference, SDK documentation (`APIs/`)
- **Guide / how-to** — Developer setup, walkthrough, tutorial (`Development/`)
- **Session output** — Claude Code session artifact (`Claude Outputs/`)
- **Methodology** — Process, workflow, team practice (`Methodology/`)

## Workflow

### Step 1 — Identify doc type and target directory

Use the quick-reference table below. When ambiguous, ask the user which type fits best.

### Step 2 — Search for existing notes

Before creating, check whether a relevant note already exists to avoid duplicates:

```
obsidian search query="<keywords from the topic>" limit=5
```

If a match is found, read it first (`obsidian read file="<name>"`) and decide whether
to update the existing note or create a new one.

### Step 3 — Compose content

Read `references/vault-structure.md` for the complete frontmatter field table.

Start every note with YAML frontmatter, then a level-1 heading matching the filename:

```markdown
---
date: YYYY-MM-DD
tags: [dravr, <type-tag>]
status: <value>        # only for ADR and Plan
service: <name>        # only for Runbook and API doc
severity: <P0–P3>      # only for Runbook
---

# Note Title
```

Use `[[wikilinks]]` to link related vault notes — never absolute file paths.

### Step 4 — Write to the live vault

Obsidian must be running and the dravr-vault must be the focused vault.

```
# Create a new note
obsidian create name="<Note Title>" path="<Directory/Filename.md>"

# Append a section to an existing note
obsidian append file="<Note Title>" content="## New Section\n<content>"
```

### Step 5 — Verify and link

After writing, read the note back to confirm content landed correctly:

```
obsidian read file="<Note Title>"
```

Then update any related notes with a `[[wikilink]]` to the new document.

## Quick Reference

| Doc Type | Target Directory | Template | Required Tags |
|----------|-----------------|----------|---------------|
| ADR | `Architecture/ADRs/` | `Templates/ADR.md` | `dravr, adr` |
| Runbook | `Development/Runbooks/` | `Templates/Runbook.md` | `dravr, runbook, sre` |
| Plan | `Claude Plans/` | `Templates/Plan.md` | `dravr, plan` |
| API doc | `APIs/` | — | `dravr, api` |
| Session output | `Claude Outputs/` | — | `dravr, claude-output` |
| Guide / how-to | `Development/` | — | `dravr, guide, <domain>` |
| Methodology | `Methodology/` | — | `dravr, methodology` |

## Frontmatter Standards

**ADR** — required: `date`, `status`, `tags: [dravr, adr]`
- Status values: `proposed` → `accepted` → `deprecated` / `superseded`
- Filename convention: `ADR-NNN Short Title.md` (zero-padded number)

**Runbook** — required: `date`, `severity`, `service`, `tags: [dravr, runbook, sre]`
- Severity values: `P0` (critical), `P1` (high), `P2` (medium), `P3` (low)

**Plan** — required: `date`, `status`, `tags: [dravr, plan]`
- Status values: `draft`, `active`, `completed`

**API doc** — required: `date`, `service`, `tags: [dravr, api]`

**Session output** — required: `date`, `tags: [dravr, claude-output]`

**Guide / how-to** — required: `date`, `tags: [dravr, guide, <domain>]`
- Replace `<domain>` with the relevant area (e.g., `backend`, `mobile`, `devops`)

## Key obsidian-cli Patterns

```
# Search before creating to avoid duplicates
obsidian search query="OpenTelemetry tracing" limit=5

# Read an existing note before editing
obsidian read file="ADR-042 Adopt OpenTelemetry"

# Create a new note (Obsidian must be open and vault focused)
obsidian create name="ADR-042 Adopt OpenTelemetry" \
  path="Architecture/ADRs/ADR-042 Adopt OpenTelemetry.md"

# Append a section to an existing note
obsidian append file="ADR-042 Adopt OpenTelemetry" \
  content="## Update 2026-03-14\nApproved in team review."
```

## Constraints

- Obsidian must be running and the dravr-vault must be the active vault before using
  `obsidian create` or `obsidian append` — the CLI communicates with the open app.
- Always search first to avoid duplicate notes on the same topic.
- Always use `[[wikilinks]]` for internal vault references, not relative or absolute paths.
- Frontmatter `date` must be ISO 8601 format (`YYYY-MM-DD`).
- ADR filenames must include the zero-padded number prefix so they sort chronologically.
- After writing Claude-generated content, commit explicitly rather than waiting for the
  obsidian-git auto-commit (auto-commit runs every 10 minutes, explicit commits are
  easier to attribute and revert).
- See `references/vault-structure.md` for the full directory map and field reference.
