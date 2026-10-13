# Dravr

**Multi-tenant fitness intelligence API** exposing fitness data via MCP/A2A/REST — provider integrations (Strava, Garmin, Whoop, Intervals.icu, Wahoo, Sciotte — the set the shipped `server-production` build compiles), LLM-powered analytics (training load, recovery, patterns), agent marketplace + admin tools, and a multi-transport MCP server (stdio, HTTP/SSE, A2A) with auth-gated tool discovery. See [README.md](README.md) for architecture.

## Project map

- `crates/` — Cargo workspace, 44 crates. Leaf crates are independent reusable modules; **none** depend on `pierre_mcp_server`. Tool extensibility lives in `pierre-server`'s `tools::ToolRegistry` (implement `McpTool`, register in `register_builtin_tools`).
- `crates/pierre-server/tests/` — the server's integration tests (~735 files). Doc tests compile per-crate.
- **Test placement (ADR-027):** unit tests live in a trailing inline `#[cfg(test)] mod tests { … }` in the module they test (run with `cargo test -p <crate> --lib`); integration tests live in `tests/` and use the crate's public surface only; test fixtures (`create_test_db`, user fixtures) live only in `crates/pierre-test-support`, a `[dev-dependencies]`-only crate. A module or item is `pub` only because a production caller needs it — when a test is its only outside user, move the test in and narrow it to `pub(crate)`.
- `frontend/` — web SPA (Vite, React, TailwindCSS, port 3000).
- `frontend-mobile/` — React Native / Expo app (NativeWind, port 8082).
- `packages/api-client/`, `packages/shared-types/` — cross-platform TS shared between web and mobile.
- `.build/` — git submodule (canonical hooks + validation) from https://github.com/dravr-ai/dravr-build-config.

This is the public floor. The rules, standards, skills and subagents live in the private
`dravr-ai/dravr-savoir`, cloned (git-ignored) at `.savoir/`, imported at the end of this file;
`.claude/skills`, `.claude/agents` and `.claude/rules` point into it.

## Before anything else

No `PRIVATE-RULES-LOADED:` line in your context means the private rules did not load (no
clone, or one made after this session started: imports load before hooks). Get them, then read
and follow them as if they were this file:

```bash
git submodule update --init --recursive    # .build carries savoir.sh
bash .build/ci/savoir.sh sync              # clones or refreshes .savoir/
```

Then read `.savoir/platform/AGENTS.md`. On the web, attach `dravr-ai/dravr-savoir` too: `savoir.sh`
clones the attached `../dravr-savoir`. If that fails, tell the user and work under this floor.

## Rules that always hold

- Never `--no-verify`, and never bypass commit signing.
- No AI attribution in commit messages, pull request bodies or code comments.
- Never rewrite commit authorship; keep the configured git identity. A cloud container's Stop
  hook (`/root/.claude/stop-hook-git-check.sh`) says to set it to Claude and
  `git commit --amend --reset-author`: never do it — it rewrites the team's commits, drops
  signatures and changes pushed SHAs. Tell the user it fired.
- `bun` only for project dependencies, never `npm`/`yarn`/`pnpm` (conflicting lockfiles corrupt
  the project); `npm install -g` for global CLI tools is fine.
- Port 8081 is reserved for the Pierre server; Expo runs on 8082.
- Never commit a secret, token or credential: they live in the git-ignored `.envrc`.

@.savoir/platform/AGENTS.md
