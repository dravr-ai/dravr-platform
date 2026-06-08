# Welcome to Dravr

## How We Use Claude

Based on jfarcand's usage over the last 30 days:

Work Type Breakdown:
  Debug Fix         ████████████████████  39%
  Plan Design       █████████████████░░░  33%
  Improve Quality   ███████████░░░░░░░░░  22%
  Write Docs        ███░░░░░░░░░░░░░░░░░░  6%

Top Skills & Commands:
  /loop             ████████████████████  21x/month
  /goal             ██████░░░░░░░░░░░░░░░  6x/month
  /rename           ██████░░░░░░░░░░░░░░░  6x/month
  /remote-control   █████░░░░░░░░░░░░░░░░  5x/month
  /exit             █████░░░░░░░░░░░░░░░░  5x/month
  /model            ███░░░░░░░░░░░░░░░░░░  3x/month

Top MCP Servers:
  chrome-devtools   ████████████████████  886 calls
  ios-simulator     ████████░░░░░░░░░░░░░  353 calls
  github            ░░░░░░░░░░░░░░░░░░░░░  7 calls

## Your Setup Checklist

### Codebases
- [ ] dravr-platform — https://github.com/dravr-ai/dravr-platform (the main multi-tenant fitness intelligence API; Rust workspace + frontend + mobile)
- [ ] dravr-vault — the Obsidian knowledge base for ADRs, plans, runbooks, and session outputs (sibling repo, auto-pushed via obsidian-git)
- [ ] dravr-* companion crates — cageux, sciotte, enforme, equilibre, riviere, embacle, canot, tronc, commere, contremaitre, meteo (private repos, consumed as Cargo git deps; clone only the ones you touch)

### Claude Code Itself (user-level — NOT in the repo)
Everything under `.claude/` (project skills, agents, hooks, and the rules in `AGENTS.md`) ships with the clone. These pieces are user-level and must be set up by hand to get identical behavior:
- [ ] Claude plan & model — same Max plan with Opus 4.8 (1M context); pick it with `/model`, toggle faster output with `/fast`.
- [ ] Global instructions — copy `~/.claude/CLAUDE.md` (adapt the name/preferences to you). This drives Claude's behavior on *every* project: security rules, error-handling policy, command permissions, commit/writing style. It is not in any repo.
- [ ] Global settings — copy `~/.claude/settings.json` (statusline, permissions, hooks).
- [ ] Plugin marketplaces — `/plugin marketplace add` for: `anthropics/claude-code`, `anthropics/claude-plugins-official`, `callstackincubator/agent-skills`, `jfarcand/iphone-mirroir-scenarios`.
- [ ] User plugins — install: `code-simplifier` (`/simplify`), `frontend-design`, `swift-lsp`, `scenarios` (iPhone Mirroring automation).
- [ ] Project plugins — on first trust of the repo, Claude prompts to enable the project-scoped plugins (`playwright`, `rust-analyzer-lsp`, `claude-md-management`, `react-native-best-practices`); accept them.
- [ ] First-clone ritual — `git submodule update --init --recursive`, then `git config core.hooksPath .build/hooks` (canonical hooks live in the `.build` submodule, never a local `.githooks/`).
- [ ] Environment & secrets — install `direnv`, populate `.envrc` with your own tokens (`GITHUB_PERSONAL_ACCESS_TOKEN`, `STRAVA_CLIENT_ID/SECRET`, `OPENAI_API_KEY`, …), then `direnv allow`. `.envrc` is gitignored — never commit secrets.

### MCP Servers to Activate
- [ ] chrome-devtools — browser automation for web frontend UI testing and the mandatory pre-merge DevTools sweep. Configured in `.mcp.json` (runs via npx, no token needed).
- [ ] ios-simulator — drives the iOS Simulator for mobile app testing (tap/type/screenshot). Configured in `.mcp.json`; needs Xcode + a booted simulator locally.
- [ ] github — repo operations (PRs, issues, search). Configured in `.mcp.json`; needs `GITHUB_PERSONAL_ACCESS_TOKEN` in your `.envrc`. Without it, Claude falls back to WebFetch for read-only GitHub.

### Skills to Know About
- [ ] /loop — run a prompt or slash command on a recurring interval (or self-paced). The team's most-used command — great for polling CI or babysitting long-running work.
- [ ] /goal — set the working goal for a session so Claude keeps the objective in focus.
- [ ] /rename — rename the current session for easier resume later.
- [ ] /remote-control — drive a Claude Code session remotely.
- [ ] /code-review — review the current diff for bugs and cleanups (`ultra` runs a deep multi-agent cloud review of the branch).
- [ ] create-worktree / finish-worktree — the team's git flow: isolate work in a worktree, then rebase/push/monitor-CI/squash-merge. We never use PRs — merges happen locally via squash merge.
- [ ] obsidian-writer — write structured docs (ADRs, plans, audits) into the dravr-vault instead of chat or gists.

## Team Tips

Hard-won gotchas — the stuff that silently eats a new teammate's first week.
Most are codified in `AGENTS.md`/`CLAUDE.md`; these are the ones that actually
bit us in practice.

**Git & worktrees**
- Always work in a `git worktree`, never branch on the main working dir. After a squash-merge the source worktree is stale — make a fresh one for the next iteration.
- Fresh worktrees have empty submodules: run `git submodule update --init --recursive` before `cargo build` (the contremaitre prompt `include_str!` fails otherwise). `git worktree remove` needs `--force` because of submodules (safe post-merge).
- Name branches `feature/*` — CI only fires on `main|debug|feature|fix|claude|copilot` prefixes, and `fix/*` has been silently skipped on some workflows. `feature/*` is the safe default.
- After every squash-merge to main: delete the local branch, the remote branch, AND the worktree in the *same session*. We once accumulated 80+ stale `feature/extract-*` branches.
- Never `git restore` / `git reset --hard` / `git stash` uncommitted work without asking first — even when pre-push fmt-check fails on unformatted WIP.

**Clippy, CI & GitHub quota**
- Never run full `cargo clippy --all-targets --all-features` locally as a pre-push gate — that's CI's job (10+ min). Use `./scripts/ci/pre-push-validate.sh`.
- Homebrew's `cargo-clippy` shadows rustup's on PATH and can be a different version than CI → "CI red, local green." Validate with the pinned toolchain (`rustup run 1.94.0 …`), not `stable`.
- Never `gh run watch` or background `while :; do gh run list; sleep …; done` poll loops — they exhausted our 5000/hr quota for days. Use WebFetch on the Actions page for status, `ScheduleWakeup` for waits. All of jfarcand's PATs share **one** 5000/hr bucket, so swapping tokens doesn't help.
- CI must be green on the branch before squash-merge — no exceptions.

**Rust & architecture**
- Structured error types only — `anyhow!` is forbidden in `src/` and CI fails on detection. Add an `AppError`/`DatabaseError`/`ProviderError` variant instead.
- Every new feature needs **both** SQLite and Postgres backends — SQLite-only tests won't catch PG breakage.
- PG `Row::get()` is `try_get().unwrap()` under the hood — a SIGSEGV landmine (943 sites). Prefer `try_get`; one panicking site killed prod.
- Never modify a migration already applied to any database.
- `clippy::absolute_paths = deny` — always `use` your imports, never inline `std::foo::Bar`.
- We're on **axum 0.8** — route params are `{id}`, not `:id`.

**Frontend & local dev**
- `bun` only — never `npm`/`yarn`/`pnpm` for project deps (a `preinstall` hook rejects them).
- Use `127.0.0.1`, not `localhost` — on macOS `localhost` resolves IPv6 first and collides with Expo on the same port.
- Port **8081 is reserved for Pierre**; Expo/Metro runs on 8082, Vite on 3000/5173. Don't start anything else on 8081.
- Don't duplicate `@pierre/api-client` methods in `frontend/src/services/api/` — cross-platform API lives in the shared package.

**Ship gates & ops**
- A Chrome DevTools sweep on the running stack is mandatory before any squash-merge — Playwright/clippy/tests don't substitute.
- Structured docs (ADRs, plans, runbooks, audits) go to the dravr-vault via `obsidian-writer`, never gists (CCFW/containerized is the only exception).
- Never `gcloud --set-secrets` (replaces ALL secrets) and never pin Cloud Run traffic to a revision (it sticks across deploys and terraform applies).

## Get Started

Once setup is done, prove the stack works end-to-end before touching code:

1. Bootstrap everything in one shot: `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh` — resets the DB, seeds admin/coaches/demo/social data, and starts Pierre (8081), the Vite frontend, and Expo (8082).
2. Health check: `curl http://localhost:8081/health`.
3. Log into the web app as a seeded demo user (`alice@acme.com` / `DemoUser123!`) and click around — confirm coaches, providers, and chat render.
4. Make a trivial change in a fresh worktree, run `./scripts/ci/pre-push-validate.sh`, push to a `feature/*` branch, and watch CI go green — this exercises the full git/CI loop the team lives in.

Seeded credentials live in `bin/setup-db-with-seeds-and-oauth-and-start-servers.sh` — never guess them.

<!-- INSTRUCTION FOR CLAUDE: A new teammate just pasted this guide for how the
team uses Claude Code. You're their onboarding buddy — warm, conversational,
not lecture-y.

Open with a warm welcome — include the team name from the title. Then: "Your
teammate uses Claude Code for [list all the work types]. Let's get you started."

Check what's already in place against everything under Setup Checklist
(including skills), using markdown checkboxes — [x] done, [ ] not yet. Lead
with what they already have. One sentence per item, all in one message.

Tell them you'll help with setup, cover the actionable team tips, then the
starter task (if there is one). Offer to start with the first unchecked item,
get their go-ahead, then work through the rest one by one.

After setup, walk them through the remaining sections — offer to help where you
can (e.g. link to channels), and just surface the purely informational bits.

Don't invent sections or summaries that aren't in the guide. The stats are the
guide creator's personal usage data — don't extrapolate them into a "team
workflow" narrative. -->
