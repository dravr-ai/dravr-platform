# Dravr

**Multi-tenant fitness intelligence API** exposing fitness data via MCP/A2A/REST — OAuth provider integrations (Strava, Fitbit, Garmin, Whoop, Terra), LLM-powered analytics (training load, recovery, patterns), coach marketplace + admin tools, and a multi-transport MCP server (stdio, HTTP/SSE, A2A) with auth-gated tool discovery. See [README.md](README.md) for architecture.

## Project map

- `crates/` — Cargo workspace, 14 crates. Leaf crates are independent reusable modules; **none** depend on `pierre_mcp_server`. Tool extensibility lives in `pierre-server`'s `tools::ToolRegistry` (implement `McpTool`, register in `register_builtin_tools` — see [book/src/tool-development.md](book/src/tool-development.md)).
- `crates/pierre-server/tests/` — all integration tests (325 files). Doc tests compile per-crate. No `#[cfg(test)]` in `src/` — tests are external only.
- `frontend/` — web SPA (Vite, React, TailwindCSS, port 3000).
- `frontend-mobile/` — React Native / Expo app (NativeWind, port 8082).
- `packages/api-client/`, `packages/shared-types/` — cross-platform TS shared between web and mobile.
- `.build/` — git submodule (canonical hooks + validation) from https://github.com/dravr-ai/dravr-build-config.

This repo is often the entry point for cross-project work. `../dravr-vault` is the shared team knowledge base (JF + Phil) — read it for prior decisions and context, and write durable outputs there (see the docs-routing block below).

**Package manager: `bun` ONLY.** Never `npm`/`yarn`/`pnpm` for project deps (corrupts the project via conflicting lockfiles; `package.json` `preinstall` rejects them; `.gitignore` blocks the foreign lockfiles). `npm install -g` is allowed for global CLI tools only.

<important if="you are running a new session and about to touch code">

Run the startup checklist in order before any code work:

```bash
git submodule update --init --recursive          # pull .build/ (hooks, validation)
git config core.hooksPath .build/hooks            # canonical hooks path — NEVER .githooks
git log --oneline -10                             # recent context
gh run list --branch main --limit 10 --json workflowName,conclusion   # CI health on main
git status                                        # uncommitted work
```

If any workflow on main has been red for 2+ runs, STOP and ask the user "Should I investigate CI before doing X?" before starting the requested task.
</important>

<important if="you need to run, build, test, lint, or manage the server / database / tokens">

**Server & DB** (Pierre MCP Server — port 8081, RESERVED, never start anything else on it):

| Command | What it does |
|---|---|
| `./bin/start-server.sh` | Start server (loads `.envrc`, background, health check) |
| `./bin/stop-server.sh` | Graceful shutdown w/ force-kill fallback |
| `curl http://localhost:8081/health` | Health check |
| `./bin/reset-dev-db.sh` | Reset dev DB (fixes migration checksum mismatch; SQLite-only safety check; backs up to `data/backups/`, recreates, runs all seeders) |

After reset, default login: `admin@example.com` / `AdminPassword123`.

**Admin users & tokens** (`pierre-cli`):

| Command | What it does |
|---|---|
| `cargo run --bin pierre-cli -- user create --email <e> --password <p>` | Create admin user for frontend login |
| `cargo run --bin pierre-cli -- token generate --service <s> --expires-days 30` | Generate API token |
| `cargo run --bin pierre-cli -- token generate --service admin_console --super-admin` | Super-admin token (no expiry, all perms) |
| `cargo run --bin pierre-cli -- token list --detailed` | List admin tokens |
| `cargo run --bin pierre-cli -- token revoke <token_id>` | Revoke a token |

**Backend tests** — full suite is ~13 min across 325 binaries. ALWAYS target a file:

| Command | What it does |
|---|---|
| `cargo test --test <file> <name> -- --nocapture` | Run a targeted test (compiles only that file) |
| `cargo test --test <file> -- --list` | List tests in a file |
| `cargo clippy -p <pkg> --all-targets --all-features -- -D warnings` | Per-crate clippy (the crate you're in) |

Find a test's file: `rg "test_name" tests/ --files-with-matches`. NEVER `cargo test <name>` without `--test` (compiles all 325 files); NEVER `cargo test --lib` (runs ~0 tests).

**Pre-push validation** — `./scripts/ci/pre-push-validate.sh` is the ONLY local gate you run (see the CI block for why).
</important>

<important if="you have just run a test command">

Verify tests actually ran — exit code 0 is NOT sufficient (`cargo test` exits 0 when 0 tests run). Confirm `running N tests` with N > 0 AND `N passed` in the summary. Red flags to STOP on: `running 0 tests`, `0 passed; 0 failed`, `filtered out` with 0 passed (usually a wrong `--test` target or a typo'd test name). Never claim "tests pass" if 0 ran.
</important>

<important if="you are committing, branching, merging, or cleaning up git branches">

- **NEVER use `--no-verify`.** **NEVER create or suggest a Pull Request** (`gh pr create`) for platform self-merges — merges happen locally via squash merge. (Carve-outs: cross-repo dependency-notification PRs on sibling repos, and the explicit one-off the user authorizes.)
- **Bug fixes** go directly to `main`: commit and `git push origin main`.
- **Features** use a branch → push → local squash merge:
  ```bash
  git checkout main && git fetch origin
  git merge --squash origin/feature/my-feature && git commit && git push
  ```
- **MANDATORY cleanup in the same session** once the squash lands on main (squash makes a new SHA, so the feature branch is dead immediately — leaving it accumulates dead refs):
  ```bash
  git branch -D feature/my-feature
  git push origin --delete feature/my-feature
  git worktree remove <worktree-path>   # if a worktree was used
  ```
- Do not reference AI assistance in git commit messages.
</important>

<important if="you are about to push, or have just pushed, to a remote branch">

`./scripts/ci/pre-push-validate.sh` writes a `.git/validation-passed` marker (valid 15 min) that the pre-push hook checks against the current commit. Heavy compilation lives in CI by design.

- Do NOT run `cargo fmt`/`cargo check`/`cargo clippy --all-targets --all-features` ad-hoc as a pre-push gate — CI's `clippy` job runs the full workspace on every push. Per-crate clippy on the crate you're editing is fine during development.
- Never fake/create the `.git/validation-passed` marker.
- Locally, only the tiers whose files changed run: Tier 0 `cargo fmt --all -- --check`, Tier 1 `scripts/ci/architectural-validation.sh`, Tier 1b `check-vendor-contremaitre-readonly.sh`, Tier 5 frontend, Tier 6 SDK, Tier 7 mobile.
- CI fires parallel jobs on push (`ci-backend.yml`: `fast-gate` ~30s, `preflight-clippy` ~3–5min, `clippy` ~10–12min gating `release-binary`, `deadlock-analysis`, `security-audit`, `doc-tests`, `release-binary`; plus `ci-postgres.yml`, `integration-tests.yml`, `frontend-tests.yml`, `sdk-tests.yml`, `mobile-unit-tests.yml`, `mcp-compliance.yml`, each path-scoped). `cancel-in-progress: true` is set, so a new push cancels older runs on the same ref.

**Push is the start of validation, not the end.** After every push, watch CI for the pushed commit until all relevant workflows reach a terminal status. If any fails, fix the underlying issue and re-push in the same session — work is not "done" until CI is green on the head commit (cancelled runs for older commits don't count).

CI monitoring — use the first that works, NEVER ask the user for a GitHub token:
1. WebFetch `https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A<branch>` (no PAT quota).
2. `gh run list --branch <branch>` / single `gh run view <id>` (costs shared 5000/hr quota — use sparingly).
3. `mcp__github__*` for non-list ops (e.g. commenting on a failure).

Forbidden: `gh run watch`, background poll loops, any cadence < 60s. For long waits, use `ScheduleWakeup` to re-check after a fixed delay.
</important>

<important if="you are writing error handling in production code (src/)">

- **`anyhow!` / `anyhow::anyhow!` / `anyhow::Error::msg` are FORBIDDEN in every position (return, `map_err`, `ok_or_else`) — CI fails on detection, zero tolerance.** Use structured enums (`AppError`, `DatabaseError`, `ProviderError`); add a new variant if none fits. Convert via `.into()`/`?`/`.context()` so the base error stays a structured type. See `crates/pierre-server/src` error modules for the canonical patterns.
- `?` for propagation, `Result<T,E>` for fallible ops, `Option<T>` for maybe-absent values.
- `unwrap()` only in tests, compile-time-valid static data, or binary `main()`. `expect()` only to document invariants that cannot fail (static data, `main()` env setup) — never for runtime-possible errors. `panic!()` only in test assertions or unrecoverable binary errors.
</important>

<important if="you are writing auth, multi-tenant data access, OAuth, logging, or any security-sensitive code">

- **Authorization ≠ authentication.** Every admin/coach/write endpoint checks role/permission, not just a valid session. Super-admin minting requires existing super-admin credentials. API-key create/revoke/list verify ownership via `tenant_id`.
- **Multi-tenant isolation.** Every DB query includes `tenant_id` in the WHERE clause — no exceptions. OAuth tokens, API keys, LLM credentials are per-tenant (never global/shared). Cache keys include `tenant_id`. Config write/delete and admin tools modifying coach/user data verify tenant membership first.
- **OAuth/protocol.** `state` is cryptographically random and validated on callback. PKCE enforced for public clients. Grant types restricted per-client. Token endpoints validate `redirect_uri` matches authorization. `.well-known/` returns spec-compliant metadata.
- **Logging hygiene.** NEVER log access/refresh tokens, API keys, passwords, client secrets. PII (email, IP, UA) is DEBUG-level or redacted at INFO+. Auth failures → WARN, breaches → ERROR.
- **Canonical redaction (do not hand-roll).** URL credentials → `pierre_core::redaction::redact_url` (the only allowed redactor). HTTP request/response PII → the `middleware::redaction` layer (installed; don't bypass). Email for operator logs → `middleware::redaction::mask_email`. "Is this secret loaded?" → mirror `OAuthProviderConfig::secret_fingerprint` (SHA256 first 8 hex + length); never log the raw value.
- **Forbidden logging patterns** (enforced by `scripts/ci/architectural-validation.sh`): log/print macros referencing `Database URL`/`database_url`/`connection_string` without `redact_url` on the same line; interpolating vars named `password`/`client_secret`/`jwt_secret`/`encryption_key`/`access_token`/`refresh_token`; `{:?}`/`{:#?}` of `ServerConfig`/`DatabaseConfig`/`DatabaseUrl`/`OAuthProviderConfig`/`FirebaseConfig`/`WeatherServiceConfig`/`OAuth2ServerConfig` (they derive `Debug` but hold secrets — log only the fields you need).
- **Template/query safety.** Never `format!()` SQL — use parameterized queries (`$1`,`$2`). Escape server-rendered HTML with `html_escape::encode_text`. Percent-encode URL params with `urlencoding::encode`. User-facing errors carry no stack traces or internal details.
</important>

<important if="you are validating numeric input, pagination, or doing division">

- Any divisor is checked for zero first (`.max(1)` or explicit guard) before dividing.
- Pagination params have min/max bounds (e.g. limit clamped to `1..=100`).
- User-supplied numeric inputs are validated against domain-specific ranges. Do not hard-code magic values.
</important>

<important if="you are writing or modifying Rust code">

Default to idiomatic Rust; the project-enforced specifics:
- **No absolute paths** — `clippy::absolute_paths = "deny"` in `Cargo.toml`. Use `use` imports at the top; avoid inline `crate::...`/`std::...` paths except for name collisions. Flat module hierarchies.
- **No `#[allow(clippy::...)]`** except validated type-conversion casts (`cast_possible_truncation`, `cast_sign_loss`, `cast_precision_loss`). Fix the underlying issue instead of silencing.
- Clone the Arc, never its contents (`arc.clone()`); Arc/Rc clones need no comment, but justify non-obvious value clones. Document every `Arc<T>` with its sharing reason (async Tokio usually needs Arc over Rc); prefer `&T`/`Cow<T>` when lifetimes allow. Prefer `Arc<RwLock<T>>` for read-heavy state.
- `std::sync::LazyLock` for lazy statics, `OnceLock` for one-time init. Always handle `JoinHandle` results (don't swallow panics).
- Newtype pattern for domain ids; `enum` over boolean state flags. Small focused functions; `std` over external crates when sufficient.
- **Binary size: keep `pierre_mcp_server` < 80MB** (CI's `release-binary` job checks this). Watch large deps; use feature flags to drop unused code; document any justified exception.
</important>

<important if="you are writing comments, naming things, or scoping the size of a change">

- Every code file opens with a 2-line comment, each line prefixed `ABOUTME: ` (greppable).
- Comments are evergreen — describe code as it is, not how it changed. Never delete a comment unless provably false.
- Never name things `improved`/`new`/`enhanced`; no placeholder/`dead_code`/mock/`_`-prefixed vars; no "in future versions"/"implement later"/"fall back" stubs — implement the real thing.
- Make the smallest reasonable change. Prefer simple/maintainable over clever. Match surrounding style.
- **Never rewrite an existing implementation from scratch to fix a bug/error — STOP and get explicit permission first.**
</important>

<important if="you are writing or running tests, or tempted to skip/ignore one">

- Tests must cover the functionality. Cover error/log output too if errors are expected.
- **No skipping or ignoring, ever:** Rust `#[ignore]`; JS/TS `.skip()`/`xit()`/`xdescribe()`/`test.skip()`; CI `continue-on-error: true` on test jobs; commenting tests out. If a test fails, fix the code (or the test if the test is wrong), or ask for help — never skip.
- Every project needs unit, integration, AND e2e tests. Don't mark a test type "not applicable" — only the human saying exactly "I AUTHORIZE YOU TO SKIP WRITING TESTS THIS TIME" waives this.
- **Mocks only in test code**, documented with reasoning, realistic, and backed by an integration test with the real implementation. Never mock production features.

Frontend/mobile/SDK validation tiers (run from each subdir):

| Area | Commands |
|---|---|
| `frontend/` | `bun run type-check` → `bun run lint` → `bun run test -- --run` → `../scripts/ci/pre-push-frontend-tests.sh`; e2e: `bun run test:e2e` |
| `frontend-mobile/` | `bun run typecheck` → `bun run lint` → `bun run test` → `../scripts/ci/pre-push-mobile-tests.sh`; e2e: `bun run e2e:build && bun run e2e:test` |
</important>

<important if="you are working in frontend/ or packages/api-client (web API methods)">

- **No local API duplication.** Cross-platform API methods live in `packages/api-client/src/domains/`; web-only endpoints (admin, a2a, dashboard, keys, usage) stay in `frontend/src/services/api/`. Components import domain APIs from the `'../services/api'` barrel (`index.ts`), never individual domain files. New shared endpoint → add to `@pierre/api-client` first, then consume via the barrel.
- Shared web/mobile types come from `@pierre/shared-types`, never inline interfaces.
- State: React Query for server state, React Context for app state. Styling: TailwindCSS.
</important>

<important if="you are working in frontend-mobile/ or running Expo/Metro">

- **Port 8082 only for Expo** — `bun start` is configured for it. NEVER `expo start` without a port (defaults to 8081, which is the reserved Pierre port). If "Port 8081 in use" appears, the Pierre server is running correctly — use 8082.
- Styling: NativeWind classes via `className` (no inline styles). State: React Query + Context. Navigation: drawer/stack patterns in `src/navigation/`. Reusable UI in `src/components/ui/`. Props need explicit types; prefer `unknown` + type guards over `any`.
- **Physical-device testing via Cloudflare tunnel:** `bun run tunnel` (URL only), `bun run start:tunnel` (tunnel + Expo), `bun run tunnel:stop`. The tunnel points at localhost:8081 and rewrites `BASE_URL` in `.envrc` + `EXPO_PUBLIC_API_URL` in `frontend-mobile/.env`. After starting: `direnv allow`, then restart Pierre. When `BASE_URL` is set, OAuth redirect URIs use it instead of `http://localhost:8081`.
</important>

<important if="you need an API key, token, or credential for a service">

1. Check `.envrc` — all secrets live here with explanatory comments (`.gitignore`d). Keys include `GITHUB_PERSONAL_ACCESS_TOKEN`, `EXPO_TOKEN`, `STRAVA_CLIENT_ID`/`STRAVA_CLIENT_SECRET`, `PIERRE_JWT_TOKEN`, `OPENAI_API_KEY`.
2. Check `.mcp.json` for which env vars are required (committed; uses `${VAR}` placeholders, never real secrets).
3. If in neither, ask the project owner — never guess or fabricate.

**MCP-first:** when a service is configured in `.mcp.json`, use its MCP tools before CLI/web alternatives (e.g. GitHub → `mcp__github__*`, not `gh`, unless MCP lacks the operation).
</important>

<important if="you are saving a doc, plan, ADR, runbook, audit, or report — or need prior decisions/context">

`../dravr-vault` is the shared team knowledge base (JF + Phil). **Read it first** for prior decisions, designs, and context before starting structured work, and **write durable outputs there** — never leave structured docs only in chat (chat isn't durable).

Routing (use the `obsidian-writer` skill, which writes to the live vault):

| Doc type | Destination |
|---|---|
| ADR / decision | dravr-vault `Architecture/ADRs/` |
| Plan / phased build | dravr-vault `Claude Plans/` |
| Runbook / oncall procedure | dravr-vault `Development/Runbooks/` |
| Audit / design analysis / session handoff / report | dravr-vault `Claude Outputs/` |
| Reference docs that ship | repo `book/src/` (mdBook) |
| Directory-scoped specs | repo `<dir>/README.md` |

- **Local Claude Code (this CLI):** prefer the vault via `obsidian-writer`. Avoid `gh gist create` for the doc types above — gists aren't vault-searchable or wikilinkable.
- **Claude Code for Web (containerized, no vault checkout):** `gh gist create` is the only durable output — acceptable there as a fallback; drop the gist link in chat so a later local session can backfill it into the vault.
- Gists are also fine for pasteable snippets, cross-project material, and ephemeral share-with-stranger artifacts.
- Writing markdown via the Write tool is limited to the `claude_docs/` folder under the repo.
</important>

<important if="you are adding an abstraction, a dependency, or refactoring an existing system">

This is a pre-1.0 project with zero external API consumers — **no backward compatibility, no deprecation window.** Every rename/move/replacement is a single-commit cutover. Complete deletion, not deprecation: never mark code `// DEPRECATED` or `// TODO remove later` — delete it (file an issue and link it if deletion is blocked).

- **Single source of truth / when adding, remove.** Before adding an abstraction, grep for an existing one with similar purpose; if it exists, use it or delete it in the *same* commit that replaces it. Never leave two systems doing the same job "for compat."
- **Use the dependency you add (no phantom integrations).** A crate in `Cargo.toml` must have its real API called — implement *its* traits (`Store`/`Provider`/`Repository`), use *its* domain types (not bare primitives mirroring them), and don't re-export types with zero consumers while hand-rolling a parallel implementation. Don't add a direct dep + version pin for a crate that already arrives transitively and you don't call. Test: if I deleted this dependency line, what breaks? "Only a re-export no one reads" = phantom; finish or remove it. (Canonical failure: the `dravr-riviere` case — added + re-exported, but storage hand-rolled `TimeSeriesPointRepository` instead of implementing riviere's `TimeSeriesStore`.)
- **Forbidden "junk disguised as discipline"** — delete on sight, never add: `KNOWN_OFFENDERS`/`PENDING_*`/`EXEMPT_*` exception arrays in tests; adapter/wrapper types bridging an old trait to a new one (port the body, delete the old); parallel accessors bypassing a canonical config struct; invariant tests policing drift between two systems (delete one system — tests of a *single* system's internal consistency are fine); fallback dispatch paths (`if not found in new, try legacy`); feature flags creating "old mode vs new mode." Test: am I making a pre-existing parallel system *acceptable* rather than replacing it? If so, stop.
</important>

<important if="you are bumping or releasing a crate that sibling repos consume (e.g. dravr-tronc)">

`dravr-tronc` backs every satellite's `-server`/`-mcp` crate plus the platform's `pierre-server`/`-services`/`-logging`/`-contremaitre`. When you bump/release it, **open a notification PR on each consumer repo** bumping its dependency (this is the sanctioned cross-repo carve-out to the no-PR rule — it governs platform self-merges only). A satellite that *publishes* to crates.io must republish member crates in dependency order (root lib → `-mcp` → `-server`) so the graph resolves a single version — a local `cargo check` won't catch the skew, only `cargo publish --dry-run` does. Full procedure: dravr-vault `Development/Runbooks/Releasing dravr-tronc — Notify Consumers`.
</important>

<important if="you encounter duplication, stale state, red CI, version drift, or a request that conflicts with existing architecture">

STOP and ask the user before proceeding when you find: (1) two systems doing similar things; (2) stale `TODO`/`FIXME`/`for compat`/`temporary`/`v2` in code you're touching; (3) red CI on main; (4) two versions of a dep in `Cargo.lock`; (5) a request to add X when X already exists differently (surface the existing thing); (6) a half-finished migration with both paths live; (7) an adapter/wrapper added without deleting what it wraps; (8) an invariant test with an exception list; (9) a phantom dependency integration. Completing the requested task is the default — these triggers override it.
</important>

<important if="you are managing OAuth provider tokens at runtime">

Strava tokens expire after 6 hours. The server auto-refreshes expired tokens using the stored `refresh_token`, transparently to tool execution. If refresh fails, the user must re-authenticate via the OAuth flow.
</important>

<important if="you are about to run a shell command that deletes, overwrites, or modifies files or system state">

All read-only and analysis commands run freely without asking. Ask permission first for: deleting/overwriting files (`rm`, `mv` overwrite), system-state changes (`chmod`/`chown`/`sudo`), `--force` flags, writing to files via `>`/`>>`, and in-place edits (`sed -i`).
</important>
