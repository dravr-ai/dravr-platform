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
| `./bin/stop-server.sh` | Stop the whole dev stack — server, dev fixture, Vite, Expo, tunnel |
| `./bin/stop-server.sh --server-only` | Stop only the backend, leaving the frontend up (simulates an outage) |
| `curl http://localhost:8081/health` | Health check |

To reset the dev DB, re-run `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh` — it
recreates the database from scratch and runs every seeder.

**The admin login is environment-dependent**: the script resolves
`${ADMIN_EMAIL:-admin@example.com}`, so whatever `.envrc` sets wins (locally that is
`admin@pierre.mcp`). Resolve it — `set -a; source .envrc; set +a; echo "$ADMIN_EMAIL"` —
rather than assuming the default, which returns `invalid_grant` and looks like a broken
server. Seeded user accounts are constants in `crates/pierre-seeders/src/demo_data.rs`.
Both kinds are pinned by `frontend/e2e-real/seeded-credentials.real.spec.ts`.

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

<important if="you need disk space back from Cargo builds, or you are setting up a git worktree">

**Every worktree and clone has its own plain `target/`** — cargo's default, nothing shared. Builds in two worktrees of one repo are fully independent: no shared build lock, no cross-worktree fingerprint collisions. Disk comes back on a schedule instead.

- **`scripts/setup/cargo-sweep-nightly.sh` is the only disk tool.** `status` prints per-repo sizes, the fleet total and headroom (read-only); `sweep` runs the 30-day `cargo-sweep` age pass then enforces the cap; `purge` is the escape hatch — drops incremental caches, then wipes repos idle 7+ days wholesale (`--idle-days`, keeping the `--keep` most-recently-built), and pays a cold next build for it. Every destructive path skips a repo whose cargo lock is held (`--force` overrides); `--dry-run` prints the plan. It exits 127 rather than no-op when `cargo-sweep` is missing (`cargo install cargo-sweep`).
- **The ceiling is 400 GiB across every target dir under `~/workspace`** (`--cap`). The nightly run enforces it rather than warning: when the age pass leaves the fleet over, it reclaims from the least-recently-built repos until it is under, so the repo you are working in keeps its warm cache longest. The output names every repo the cap forced.
- **`sweep` runs nightly at 02:00 Eastern** via the `ai.dravr.cargo-sweep` LaunchAgent (log: `~/Library/Logs/cargo-sweep.log`). Install or remove it with `./scripts/setup/cargo-sweep-nightly.sh install|uninstall` — never hand-edit the plist, and don't add a second scheduler for this.
- Never "fix" a `target` in `git status` by committing it. `.gitignore` line 2 is `target/` — a trailing-slash rule, which matches the real directory, so a build tree is already ignored. An untracked `target` therefore means it is not a plain directory; investigate it, don't commit it.
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
- Locally, only the tiers whose files changed run: Tier 0 `cargo fmt --all -- --check`, Tier 1 `scripts/ci/architectural-validation.sh`, Tier 1b `scripts/ci/check-contremaitre-sync.sh` (compile-free locale + notify-event + MCP-tool-list drift check vs the pinned dravr-contremaitre catalogues), Tier 1c `scripts/ci/check-phantom-surfaces.sh` (compile-free; fails when the diff adds a trait with no implementor, an api-client method with no production caller, or an api-client method only one client calls), Tier 1d `scripts/ci/check-turn-envelope.sh` (compile-free; fails on a reintroduced `is_messaging`/`ChannelProfile`, a reply block only one client renders, a hand-rolled `fetch()` at `/api/chat`, or a stale `surface-capabilities.generated.ts`), Tier 1e changed server-test clippy (`cargo clippy -p pierre_mcp_server --test <name>` on exactly the top-level `crates/pierre-server/tests/` files the push touched — zero cost when it touched none; `common.rs`/`helpers/` stay CI-covered), Tier 1e-move `scripts/ci/check-moved-symbols.sh` (compile-free; fails when the diff removes or moves a `pub` item while any file still imports its old module path — the symbol-move blind spot of carnet#197), Tier 1f `--no-default-features` probe (`cargo check -p <crate> --no-default-features -- -D warnings` on changed probe crates — catches an item whose sole caller is feature-gated before CI's feature-profiles job does; probe set and exclusions are documented in the script), Tier 5 frontend, Tier 6 SDK, Tier 7 mobile.
- **A new chat surface, reply block, or notification screen is generated, not written.** `GET /api/surfaces/capabilities` serves the `SurfaceProfile::resolve` table; `cd packages/shared-constants && bun run generate` rewrites `src/surface-capabilities.generated.ts` from a running server, exactly as `packages/mcp-types` regenerates from the tool registry. Both clients read that file — the registry's per-surface `blocks` column, the notification screen vocabulary — so Tier 1d fails the push while it is behind the Rust source.
- **Branch-lane coverage contract (decided 2026-08-31, carnet#154):** `ci-postgres.yml` runs the full server test suite (~500 files, ~21-24 min) on `main`, `schedule`, `workflow_dispatch`, and `feature/*` refs only. A push on `fix/*`/`debug/*`/`claude/*`/`copilot/*` runs the 8-file `*_postgresql_test.rs` smoke — a green `CI: Backend (PostgreSQL)` there means the smoke ran, NOT the suite. Per-push coverage on those refs = that smoke + `changed-server-tests` (SQLite, exactly this push's server test files) + `leaf-crate-tests` (every non-server crate). For a full verdict on any ref, dispatch ci-postgres. Coverage (~1h45m) is a SQLite run that fires weekly (Sunday 03:17 UTC), on `workflow_dispatch`, and on PRs — never on push, so it holds no slot during a main burst — and it is not a PG gate.
- CI fires parallel jobs on push (`ci-backend.yml`: `fast-gate` ~30s, `preflight-clippy` ~3–5min, `clippy` ~10–12min gating `release-binary`, `deadlock-analysis`, `security-audit`, `doc-tests`, `contremaitre-sync` ~5min, `release-binary`; plus `ci-postgres.yml`, `integration-tests.yml`, `frontend-tests.yml`, `sdk-tests.yml`, `mobile-unit-tests.yml`, `mcp-compliance.yml`, each path-scoped). `cancel-in-progress` behaviour differs per workflow and it matters when diagnosing a missing run. **Protected on main:** every test-verdict push lane (`ci-backend.yml`, `ci-postgres.yml`, `frontend-tests.yml`, `mobile-unit-tests.yml`, `ci-redis.yml`, `coverage.yml`, `integration-tests.yml`, `sdk-tests.yml`, `mcp-compliance.yml`, … — `grep -l 'github.sha ||' .github/workflows/*.yml` is the live list) appends `github.sha` to the concurrency group when the ref is `main` (`...${{ github.ref == 'refs/heads/main' && github.sha || 'shared' }}`), so every commit gets its own group and nothing cancels or evicts another's verdict. `cancel-in-progress: false` alone is NOT protection: it keeps a running run alive, but a group holds one waiting run and the next push replaces it with zero jobs started — under a ref-only group `ci-postgres.yml` lost 9 of 84 main verdicts that way (carnet#169; runs 33450894413, 33552222435, 33555424548). **Still evictable on main, by design:** `chat-conversation-eval.yml` groups by ref alone with `cancel-in-progress: false`, so its push run can be replaced while it waits behind a live-llm run; `terraform.yml` (`terraform-state`) and `website.yml` (`pages`) also group without the sha, because a newer plan or Pages deploy superseding a waiting one is the intended behaviour, not a lost verdict. So a missing `ci-backend`/`ci-postgres` run on main is NOT a cancellation and needs another explanation (usually the commit was not the tip of its push — see below), while a missing `chat-conversation-eval` run may well be. Read the workflow's `group:` expression before blaming cancellation.

**A push's CI run belongs to the TIP, not to each commit.** GitHub fires one run per push, so a push carrying three commits produces one run, on the third. That run tests the tip's TREE — which contains all three — so a break that survives to the tip IS caught normally. Main's health is covered.

What a burst does NOT give you is per-commit validation, and that costs you in two places: `git bisect` can land on a tree nothing ever ran, and reverting one commit out of a burst produces a combination no run has seen. A commit broken and then fixed inside the same burst is invisible, which is harmless.

**Do NOT push commits one at a time to get per-commit runs.** `8897e9312` exists because burst pushes queued one deploy per push — 60 commits on 2026-08-26 became 44 image builds, each a full cargo-chef build pulling the multi-GB registry buildcache back as billed egress. Batching is the cheap direction and the fix now collapses bursts deliberately.

**The real exposure is the window, not the batching.** On 2026-08-28 a commit adding a 13th system prompt without updating the counts that pin it went up in a burst; the tip's run reported `failure` correctly — 31 minutes later. Nothing was red in between because the run was still in flight, and nobody was watching it. An unrelated feature branch hit the break first. So: after pushing to main, watch the run to terminal (below). That is what would have caught it, not a different push shape.

**When a run is genuinely ABSENT** — no row for that SHA at all — do not reach for "it was cancelled" without checking; on `ci-backend`/`frontend-tests` a main commit cannot be cancelled, so the explanation is that the SHA was never a push tip:

```bash
git log --oneline <base>..origin/main        # every sha that landed
gh run list --branch main --limit 15 --json headSha,conclusion,status
```

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
- Make the smallest reasonable change. Prefer simple/maintainable over clever — but correctness wins ties: smallest means least code, never the flimsier algorithm. Match surrounding style.
- **Never rewrite an existing implementation from scratch to fix a bug/error — STOP and get explicit permission first.**
</important>

<important if="you are implementing a new function, handler, trait method, provider capability, or API endpoint">

A stub that compiles is worse than an honest error — it passes `.is_ok()` tests and hides for months (the 2026-07 audit found ~11: in-memory OAuth store, fabricated JWT expiry, all-zero WHOOP stats, a "disconnect" that revoked nothing, a webhook that broadcast to everyone). Do not ship one:

- **Never return empty/default/fabricated data as a placeholder.** `Ok(vec![])`, `Ok(None)`, an all-zero struct, `""` arguments, a hardcoded id, or `_`-prefixed params that silently ignore the input you were handed — each is a stub *unless it is the genuinely correct result of a real, documented limitation*, and then the comment says so factually **and registers it**: `LIMITATION(registre#issue):` naming the limited item on the marker line, backed by an issue in the **private** `dravr-ai/dravr-carnet` tracker (labels `limitation` + the repo name, title prefixed `[platform]`) — registers are **per project**, so `dravr-*` shares dravr-carnet while every other project names its own; the gates are the Apache-2.0 [llm-registre](https://github.com/dravr-ai/llm-registre) tool, pointed at the tracker by `registre.toml`. **Run the `register-limitation` skill — it walks the whole procedure.** **This repo is PUBLIC — limitation and phase-review issues never go on dravr-platform itself, and issue bodies naming security residuals must never be public.** An honest gap without a marker is invisible debt — the 2026-08 audit found a text-budget floor and a whole capability surface that hid for months behind factually-worded comments.
- **Confession comments are banned and CI-gated** (`scripts/ci/architectural-validation.sh` "Functional Stub / Confession Comment Detection"): "for now", "not yet implemented", "in a real implementation", "would be … in production", "return empty … for now", "implement later", "trigger … for all". Deferral prose is gated the same way: "is the follow-up", "in a follow-up commit/change/PR", "not yet wired", "not (yet) threaded through" — registered `LIMITATION(registre#issue):` lines are the only exemption. If you're about to write one, you're stubbing — implement the real thing or STOP and surface the gap.
- **Every advertised capability needs a real backing impl in the *same* change** — a new MCP tool, agent-card flag, API endpoint, or trait method must do real work in every backend. No advertised-but-empty surfaces.
- **Consume what you declare (the dual rule).** A capability predicate, enum variant, or trait method whose only callers are tests is a phantom surface — wire a production consumer in the same change, or register it with a `LIMITATION(registre#issue):` marker line naming the item. CI's "Phantom Capability Surface Detection" enforces this for the canot messaging surface (`supports_*`/`max_*` predicates, `MessageContent` variants). Pre-push **Tier 1c** (`scripts/ci/check-phantom-surfaces.sh`) extends it to three more cases, compile-free: a Rust trait with zero implementors anywhere, a `@pierre/api-client` domain method with zero production callers, and a domain method reached from one in-app client but not the other. The caller pools are split per surface for that last one — pooled into a single list, a method called from web alone read as consumed, which is how every client parity gap in the 2026-08 survey passed green. It gates the *diff* — a newly added phantom, or a live file dropping its last caller of a method the other client still uses, fails the push — and reports the standing stock without blessing it.
- **Dark launches are ledgered.** A feature that ships disarmed (flag off, shadow/observe mode, Log-only phase) gets an entry in `feature-phases.yaml` (surface, current state, arming criterion, `review_by` date) in the same change; the weekly "Monitor: Feature Phase Review" workflow opens an issue when the date passes, so Phase 1 cannot silently become forever.
- **Test for content, not success.** New functionality needs a test asserting a concrete non-trivial result (`assert_eq!(x.len(), N)`, real field values) that a returns-empty stub would fail — not just `assert!(res.is_ok())`. Weakening an assertion to accommodate a stub is itself a violation.
- If you genuinely cannot finish it now, STOP and tell the user. Never leave a silent placeholder behind.
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
| Plan / phased build | dravr-vault `Work Log/` (`kind: plan`) |
| Runbook / oncall procedure | dravr-vault `Development/Runbooks/` |
| Guide / how-to | dravr-vault `Development/Guides/` |
| Audit / design analysis / session handoff / report | dravr-vault `Work Log/` (`kind:` audit / design / handoff / report) |
| Training science: a formula, threshold, or framing rule | dravr-vault `Methodology/` (see the standing-folders block below) |
| Feature R&D / feasibility analysis, not yet committed to | dravr-vault `Features/Potential/` (`stage: potential` + `verdict:`) |
| Reference docs that ship | repo `book/src/` (mdBook) |
| Directory-scoped specs | repo `<dir>/README.md` |

- **Local Claude Code (this CLI):** prefer the vault via `obsidian-writer`. Avoid `gh gist create` for the doc types above — gists aren't vault-searchable or wikilinkable.
- **Claude Code for Web (containerized, no vault checkout):** `gh gist create` is the only durable output — acceptable there as a fallback; drop the gist link in chat so a later local session can backfill it into the vault.
- Gists are also fine for pasteable snippets, cross-project material, and ephemeral share-with-stranger artifacts.
- Writing markdown via the Write tool is limited to the `claude_docs/` folder under the repo — a per-dev, gitignored symlink into the vault's `Work Log/` (create it if missing; without the symlink, output stays local and never reaches the vault). Notes there need `type: worklog` plus `kind:`/`area:`/`status:`/`date:` or they stay invisible to `Work Log.base` — `obsidian-writer` applies that contract for you.
</important>

<important if="you changed an algorithm, threshold, config default, or athlete-facing framing — or you shipped, planned, or investigated a feature">

Three vault folders are **standing** documents: they describe the product as it is
*now*. `Work Log/` is dated and append-only, so it ages honestly; these do not —
they go stale silently and are read as current, by humans and by you.

| Folder | Source of truth for | **Read** it when | **Update** it when |
|---|---|---|---|
| `Methodology/` | the science the product encodes — formulas, bands, config defaults, citations, and the athlete-facing **framing rules** | you touch `dravr-cageux`, `pierre-fitness-compute`, analytics / recovery / nutrition / mobility tools, or coach prompts — or you need the evidence behind a number | you change a formula, band, threshold, or config **default** that reaches an athlete; you move a module it quotes; you change how a metric is *framed*; you register a `LIMITATION` against a documented algorithm |
| `Features/` | the portfolio — what exists, its `stage:`, who is in the alpha, distilled feedback | **before proposing any feature** (it usually already exists), or you need a feature's history and decisions | a feature changes stage, ships, gets blocked, or a phase lands — bump `phases_done` and `updated:` |
| `Pillars/` | the six-pillar framework and its evidence base | you touch `Pillar`, the pillars walk, coverage, or onboarding topics | the pillar set, its definitions, or its assessment approach changes |

- **Update in the same session as the change**, not "later" — a note whose `updated:` trails its source is how a folder quietly stops being maintained.
- **The code wins every disagreement.** Most `Methodology/` notes mirror `book/src/*-methodology.md` and name it in `source:`, but the repo doc drifts from the code too: the 2026-08-21 review found published recovery weights of `TSB 40 / Sleep 35 / HRV 25` against shipped defaults of `40 / 40 / 20`. A note faithful to a stale mirror is still wrong. Read thresholds from the source, never from memory — and when the repo doc is the one at fault, fix it there too.
- **Some framing rules in `Methodology/` are CI-enforced, not advisory.** ACWR and load ratios ship as descriptive magnitudes, never injury risk (`scripts/ci/check-contremaitre-sync.sh` Check 4, all five locales); form is banded as a share of the athlete's own CTL, never absolute TSB. Breaking either fails a push — read the note before writing a prompt, tool description, or locale string.
- `Methodology/README.md` carries the sync contract and a runnable drift check; `Features/README.md` carries the frontmatter contract that `Features.base` selects on. These are enforced by Bases views and human review, not by CI — which is exactly why they need you to follow them.
- R&D that is not yet committed to goes in `Features/Potential/` with `stage: potential` and a `verdict:`, plus a row in that folder's README index — not in `Work Log/`.
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

<important if="you are adding a notify event, a messaging/locale string, or an McpTool (a platform change that must mirror into dravr-contremaitre)">

The platform is coupled to **dravr-contremaitre** catalogues. The tests policing that coupling (`contremaitre_test`, `notify_catalogue_test`, `messaging_locale_test`, `configuration_mcp_integration_test`) now run on **every push** via the `contremaitre-sync` job in `ci-backend.yml` — they used to be full-suite-only, which meant they first ran *after* the squash landed on main and red main post-merge. Two gates guard this now, but both only help if you mirror the change at authoring time:

- **New `info!(target: "notify", event = "x", …)`** → add `x` (with its `tier` + required fields) to `notify-events.yaml` in `dravr-contremaitre`, **or reuse an already-catalogued event** (cheapest). Else `notify_catalogue_test` fails. The yaml lives in the *pinned* contremaitre rev (resolve via `cargo metadata`), not in-repo.
- **New messaging/UI string** → ship **all 5 locales** (fr/en/es/de/pt) in `crates/pierre-contremaitre/src/messaging_strings.rs`; the invariant is `entries == keys × 5`. A fr+en-only key reds the locale test (recurrence: `d73eec36f`, `a60209307`).
- **New `McpTool`** → update `EXPECTED_TOOLS` in `contremaitre_test.rs` (kept sorted) + the count in `configuration_mcp_integration_test`, give operator-only tools `ADMIN_ONLY` so both discovery surfaces withhold them from non-admins, and regen the TS SDK types from a running server (`cd packages/mcp-types && bun run generate` — admin-gated, so it needs `PIERRE_ADMIN_TOKEN`, `ADMIN_EMAIL`+`ADMIN_PASSWORD`, or `logs/admin-token.txt`) — a changed `input_schema`/description alone reds `CI: TypeScript SDK`.
- **Editing `dravr-contremaitre` itself** → your change only reaches the platform once `contremaitre-bump.yml` advances the pinned rev (auto-discovers all consumers; runs on `repository_dispatch`/hourly cron). A rev-bump-only commit does **not** auto-deploy — the running binary's compiled-in schema lags until the next deploy.

`scripts/ci/check-contremaitre-sync.sh` (pre-push **Tier 1b**) catches all three drifts *before* you push — compile-free, seconds. Tool names are greppable: every tool declares itself as a literal in `tool_definition("<name>", …)`, so the check enumerates the set from src and diffs it against `EXPECTED_TOOLS`, the count assertion, and the generated `packages/mcp-types/src/tools.ts`. If it ever reports "Tool scan incomplete", a tool was registered with a computed name and the scan can no longer see everything — fix the name or extend the check, never ignore it.
</important>

<important if="you encounter duplication, stale state, red CI, version drift, or a request that conflicts with existing architecture">

STOP and ask the user before proceeding when you find: (1) two systems doing similar things; (2) stale `TODO`/`FIXME`/`for compat`/`temporary`/`v2` in code you're touching; (3) red CI on main; (4) two versions of a dep in `Cargo.lock`; (5) a request to add X when X already exists differently (surface the existing thing); (6) a half-finished migration with both paths live; (7) an adapter/wrapper added without deleting what it wraps; (8) an invariant test with an exception list; (9) a phantom dependency integration. Completing the requested task is the default — these triggers override it.
</important>

<important if="you are managing OAuth provider tokens at runtime">

Strava tokens expire after 6 hours. The server auto-refreshes expired tokens using the stored `refresh_token`, transparently to tool execution. If refresh fails, the user must re-authenticate via the OAuth flow.
</important>

<important if="you are about to run a shell command that deletes, overwrites, or modifies files or system state">

All read-only and analysis commands run freely without asking. Ask permission first for: deleting/overwriting files (`rm`, `mv` overwrite), system-state changes (`chmod`/`chown`/`sudo`), `--force` flags, and clobbering an existing file via `>`. Appending with `>>` and in-place edits (`sed -i`) on files inside the repo/worktree are equivalent to normal Edit-tool writes and need no extra permission; outside the repo they still require asking.
</important>
