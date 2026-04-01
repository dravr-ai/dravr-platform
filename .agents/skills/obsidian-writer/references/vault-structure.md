# dravr-vault Structure Reference

## Directory Map

```
dravr-vault/
  Architecture/            System design, component diagrams
    ADRs/                  Architecture Decision Records (ADR-NNN format)
  APIs/                    API endpoint docs, SDK references
  Development/             Developer guides, setup docs
    Runbooks/              Operational runbooks and on-call procedures
  Methodology/             Processes, workflows, team practices
  Claude Outputs/          Claude Code session outputs (via claude_docs symlink)
  Claude Plans/            Planning documents from Claude sessions
  Templates/               Templater templates: ADR.md, Plan.md, Runbook.md
  Assets/                  Images, diagrams, attachments
```

## Frontmatter Field Reference

| Doc Type | Field | Required | Values / Notes |
|----------|-------|----------|----------------|
| All | `date` | Yes | `YYYY-MM-DD` ISO 8601 |
| All | `tags` | Yes | Array — always include `dravr` plus type tag |
| ADR | `status` | Yes | `proposed`, `accepted`, `deprecated`, `superseded` |
| ADR | `supersedes` | No | Wikilink to replaced ADR |
| Runbook | `service` | Yes | Service name (e.g., `pierre-server`, `copilot-headless`) |
| Runbook | `severity` | Yes | `P0`, `P1`, `P2`, `P3` |
| Plan | `status` | Yes | `draft`, `active`, `completed` |
| Plan | `milestone` | No | Linear milestone or sprint name |
| API doc | `service` | Yes | Service or component name |
| API doc | `version` | No | API version string |
| Guide | — | — | Only `date` and `tags` required |
| Session output | — | — | Only `date` and `tags` required |

## Type Tags

| Doc Type | Primary Tag | Secondary Tags |
|----------|-------------|----------------|
| ADR | `adr` | |
| Runbook | `runbook` | `sre` |
| Plan | `plan` | |
| API doc | `api` | |
| Guide / how-to | `guide` | `<domain>` (e.g., `backend`, `mobile`, `devops`) |
| Session output | `claude-output` | |
| Methodology | `methodology` | |

## Naming Conventions

| Doc Type | Pattern | Example |
|----------|---------|---------|
| ADR | `ADR-NNN Short Title.md` | `ADR-042 Adopt OpenTelemetry.md` |
| Runbook | `<Service> <Topic>.md` | `Pierre Server Database Migration.md` |
| Plan | `<Topic> Plan.md` | `Mobile Auth Refactor Plan.md` |
| API doc | `<Service> API.md` or `<Endpoint>.md` | `Pierre MCP API.md` |
| Guide | `<Topic> Guide.md` or `How to <Topic>.md` | `How to Set Up Dev Environment.md` |

## Templates

Templates live in `Templates/` and are applied by the Templater plugin.

- `Templates/ADR.md` — ADR with frontmatter, Context, Decision, Consequences sections
- `Templates/Plan.md` — Plan with frontmatter, Goals, Approach, Tasks, Risks sections
- `Templates/Runbook.md` — Runbook with frontmatter, Prerequisites, Steps, Verification, Rollback sections

To use a template via obsidian-cli, read the template first then compose the note content
based on its structure:

```
obsidian read file="ADR"
```

## Wikilink Patterns

Always use wikilinks for internal references — never relative or absolute file paths:

```markdown
See [[ADR-042 Adopt OpenTelemetry]] for the decision rationale.
Related: [[Pierre Server Database Migration]] runbook.
Supersedes: [[ADR-038 Custom Tracing Spans]].
```

Wikilinks display with the note title by default. Use `|` for custom display text:

```markdown
[[ADR-042 Adopt OpenTelemetry|ADR-042]]
```
