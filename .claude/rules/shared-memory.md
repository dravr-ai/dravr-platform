# Shared platform memory

256 verified facts about this system live in the team vault at
`../dravr-vault/Claude Memory/platform/` — one file per fact, each with a `description:` line.

**They are NOT in your context.** They are shared across every developer and
session through git, so before concluding something is unknown — a past decision,
a cost figure, a provider quirk, why a thing is the way it is — search them:

```bash
rg -il "<keyword>" "../dravr-vault/Claude Memory/platform/"                 # find candidate notes
rg -h "^description:" "../dravr-vault/Claude Memory/platform/"              # skim every fact at once
```

What is covered, by topic:

- **provider capture & sciotte** (34)
- **messaging & chat surfaces** (21)
- **coaching, LLM & evals** (52)
- **MCP, A2A, SDK & CLI** (13)
- **dravr-* satellites** (14)
- **CI, clippy, build & release** (35)
- **infra, deploy & cost** (16)
- **observability & analytics** (8)
- **frontend, mobile & UX** (9)
- **data, storage & tenancy** (14)
- **security & secrets** (6)
- **product, market & planning** (13)
- **tooling, vault & process** (6)
- **other** (15)

Personal working preferences and corrections are deliberately NOT here — they
stay in each person's local auto memory, so one developer's rules never bind
another's session.
