# Shared platform memory

343 verified facts about this system live in the team vault at
`../dravr-vault/Claude Memory/platform/` — one file per fact, each with a `description:` line.

**They are NOT in your context.** They are shared across every developer and
session through git, so before concluding something is unknown — a past decision,
a cost figure, a provider quirk, why a thing is the way it is — search them:

```bash
rg -il "<keyword>" "../dravr-vault/Claude Memory/platform/"                 # find candidate notes
rg -I "^description:" "../dravr-vault/Claude Memory/platform/"              # skim every fact at once
```

In a Claude Code for Web session the vault is a cloned snapshot that can be ~7 days
stale, so `git -C ../dravr-vault pull` before treating a fact as current.

What is covered, by topic:

- **provider capture & sciotte** (37)
- **messaging & chat surfaces** (23)
- **coaching, LLM & evals** (60)
- **MCP, A2A, SDK & CLI** (20)
- **dravr-* satellites** (15)
- **CI, clippy, build & release** (49)
- **infra, deploy & cost** (17)
- **observability & analytics** (9)
- **frontend, mobile & UX** (15)
- **data, storage & tenancy** (19)
- **security & secrets** (8)
- **product, market & planning** (15)
- **tooling, vault & process** (8)
- **other** (43)

Personal working preferences and corrections are deliberately NOT here — they
stay in each person's local auto memory, so one developer's rules never bind
another's session.
