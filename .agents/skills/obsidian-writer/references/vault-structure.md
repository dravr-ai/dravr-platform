# dravr-vault Structure Reference

Consulted from `SKILL.md` when the routing table there isn't enough — an unfamiliar folder,
a naming question, or a field the main table doesn't list.

## Directory Map

```
dravr-vault/
  Architecture/            Design references that don't ship with the binary
    ADRs/                  Architecture Decision Records (ADR-NNN format)
  APIs/                    API endpoint docs, SDK references
  Development/             Developer docs
    Guides/                Long-form dev/admin how-tos
    Runbooks/              Operational procedures (severity-tagged)
  End-User Guides/         "Connect Dravr to <channel>" user-facing guides
  Features/                Feature portfolio — a Base (see below)
    Features.base          The database
    Catalogue/<YYYY-MM>/   Feature notes, bucketed by `created:`
    Potential/             Upstream R&D not yet committed to
  Work Log/                Dated working documents — a Base (see below)
    Work Log.base          The database
    <YYYY-MM>/             Notes bucketed by `date:`
  Methodology/             Training science the product encodes
  Pillars/                 Health-pillars framework
  Market Research/         Market-landscape analysis
  Competitors/             Competitor teardowns
  Fundraising/             Seed deck, deck reviews, SOM model
  Templates/               Templater templates
  Assets/                  Images and attachments (the configured attachment folder)
```

`Work Log/` is the symlink target for `dravr-platform/claude_docs`. It replaced
`Claude Plans/` + `Claude Outputs/` on 2026-08-12; neither exists any more.

## The two Bases

Both select on a `type:` field and ignore location entirely. Month folders are for humans
reading the file tree; the Base is what anyone actually reads.

| | `Work Log.base` | `Features.base` |
|---|---|---|
| Selector | `type: worklog` | `type: feature` |
| Bucketed by | `date:` | `created:` |
| Axes | `kind` (what it is), `area` (what it's about), `status` | `stage` (lifecycle), `surface`, `health` |
| Progress | — | `phases_done` / `phases_total`, rendered as a bar |
| Views | Recent · By month · Open work · Plans in flight · By area · By kind · Needs triage · Board | Board · In flight · All features · Needs attention · Shipped · By surface · Potential/R&D · By month |

Both folders have a Templater **folder template** that auto-files a new note into the
current month's bucket (`Templates/Work Log.md`, `Templates/Feature.md`). That mapping
lives in `.obsidian/plugins/`, which is gitignored — it does not survive a vault re-clone.
Each folder's `README.md` documents how to restore it.

### Feature-specific fields

`Features/` carries a lifecycle the Work Log doesn't. Read `Features/README.md` in the vault
before writing one; the fields that have no Work Log analogue:

| Field | Values | Purpose |
|---|---|---|
| `stage` | `potential` → `concept` → `planned` → `building` → `alpha` → `beta` → `ga` → `archived` / `rejected` | Lifecycle position; drives the board columns |
| `health` | `on-track` · `at-risk` · `blocked` | Manual signal; anything else surfaces in **Needs attention** |
| `phases_done` / `phases_total` | integers | Delivery progress. Both `0` renders `—`, which is the honest reading for un-counted work — **do not invent a number** |
| `surface` | `chat` · `web` · `mobile` · `api` · `infra` · `coach-tools` · `meta` | Where the capability lands |
| `posthog_feature_key` | slug (same as the file stem) | Missing it at `alpha`+ raises ⚠️ in **Needs attention** |
| `plans` / `adrs` | `"[[wikilink]]"` lists | Pointers into `Work Log/` and `Architecture/ADRs/` |
| `shipped` | `YYYY-MM-DD` | Set when the feature reaches `ga` |
| `verdict` | free text | `potential`-stage only — the answer the R&D note reached |

## Frontmatter Field Reference

| Doc Type | Field | Required | Values / Notes |
|----------|-------|----------|----------------|
| Work Log | `type` | Yes | Literally `worklog` — the selector |
| Work Log | `kind` | Yes | `plan`, `audit`, `post-mortem`, `design`, `research`, `report`, `decision`, `handoff` |
| Work Log | `area` | Yes | `coaching`, `providers`, `messaging`, `llm`, `infra`, `web`, `chat`, `security`, `business`, `architecture`, `notifications`, `testing`, `meta` |
| Work Log | `status` | Yes | `draft`, `active`, `completed`, `superseded` |
| Work Log | `date` | Yes | `YYYY-MM-DD` — also picks the month bucket |
| Work Log | `updated` | Yes | `YYYY-MM-DD` — drives the staleness column |
| Work Log | `features` | No | `"[[wikilink]]"` list into `Features/` |
| Feature | `type` | Yes | Literally `feature` — the selector |
| Feature | `title` | On folder notes | Set it on any note named `README.md` / `PRD.md`, or the base displays "README" |
| ADR | `date`, `status` | Yes | `proposed`, `accepted`, `deprecated`, `superseded` |
| ADR | `supersedes` | No | Wikilink to the replaced ADR |
| Runbook | `service`, `severity` | Yes | Service name; `P0`–`P3` |
| API doc | `service` | Yes | Service or component name |
| API doc | `version` | No | API version string |
| Guide | `date`, `tags` | Yes | Nothing else required |
| All | `tags` | Yes | Always `dravr` plus a type tag. **Not a classification axis** — `kind`/`area`/`stage` do that |

## Naming Conventions

| Doc Type | Pattern | Example |
|----------|---------|---------|
| Work Log | Descriptive title, no date prefix | `Guardian Observability and Progressive Enforcement.md` |
| Feature | kebab-case slug, matching `posthog_feature_key` | `coach-selection-onboarding.md` |
| ADR | `ADR-NNN Short Title.md` (zero-padded) | `ADR-042 Adopt OpenTelemetry.md` |
| Runbook | `<Service> <Topic>.md` | `Pierre Server Database Migration.md` |
| API doc | `<Service> API.md` or `<Endpoint>.md` | `Pierre MCP API.md` |
| Guide | `<Topic> Guide.md` or `How to <Topic>.md` | `How to Set Up Dev Environment.md` |

Work Log filenames carry **no date prefix** — the month bucket and `date:` already say
when, and a prefix only makes the wikilink uglier.

## Wikilink Patterns

Always wikilink by bare basename — never a path. Basename links survive a note moving
between month buckets; path links break on the first move.

```markdown
See [[ADR-042 Adopt OpenTelemetry]] for the decision rationale.
Related: [[Pierre Server Database Migration]] runbook.
[[ADR-042 Adopt OpenTelemetry|ADR-042]]          ← custom display text
```

Two traps:

- **`#` in a filename** splits the link. `[[Risk #5 gate - experimental-coach opt-in]]`
  resolves to a note called "Risk". The one affected note is linked with a percent-encoded
  markdown link instead — don't "fix" it back to a wikilink.
- **`|` inside a markdown table** must be escaped as `\|`, including in a wikilink alias:
  `[[Features/README\|Features]]`.
