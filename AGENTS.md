# Dravr

**Multi-tenant fitness intelligence API** exposing fitness data via MCP/A2A/REST protocols. Core capabilities:
- OAuth provider integrations (Strava, Fitbit, Garmin, Whoop, Terra, etc.)
- LLM-powered analytics (training load, fitness scoring, recovery, patterns)
- Coach marketplace + admin tools, push notifications, WebSocket messaging
- Multi-transport MCP server (stdio, HTTP/SSE, A2A) with auth-gated tool discovery

See [README.md](README.md) for architecture details.

---

## Package Manager: bun ONLY

**CRITICAL: This project uses `bun` exclusively for project dependencies. NEVER use `npm`, `yarn`, or `pnpm` for project packages.**

Using npm/yarn for project dependencies will corrupt the project by creating conflicting lock files and inconsistent `node_modules/`.

**Exception**: `npm install -g` is allowed for installing global CLI tools (e.g., `npm install -g @github/copilot`) that are not project dependencies.

### Enforcement
- All `package.json` files have a `preinstall` script that rejects npm/yarn
- `.gitignore` blocks `package-lock.json`, `yarn.lock`, and `pnpm-lock.yaml`
- CI workflows use `bun install --frozen-lockfile`

## Git Workflow: NO Pull Requests

**CRITICAL: NEVER create Pull Requests. All merges happen locally via squash merge.**

### Rules
- **NEVER use `gh pr create`** or any PR creation command
- **NEVER suggest creating a PR**
- Feature branches are merged via **local squash merge**

### Workflow for Features
1. Create feature branch: `git checkout -b feature/my-feature`
2. Make commits, push to remote: `git push -u origin feature/my-feature`
3. When ready, squash merge locally (from main worktree):
   ```bash
   git checkout main
   git fetch origin
   git merge --squash origin/feature/my-feature
   git commit
   git push
   ```
4. **MANDATORY cleanup after squash-merge lands on main**:
   ```bash
   git branch -D feature/my-feature            # delete local branch
   git push origin --delete feature/my-feature # delete remote branch
   git worktree remove <worktree-path>         # if work was done in a worktree
   ```
   Step 4 is non-negotiable. Squash-merge produces a new SHA on main, so the
   feature branch's commits are gone — the branch is dead the moment the
   squash commit lands. Leaving it on origin accumulates dead refs (we hit
   80+ stale `feature/extract-*` branches once, all already merged). The
   agent MUST run step 4 in the same session as step 3, not "later."

### Bug Fixes
- Bug fixes go directly to `main` branch (no feature branch needed)
- Commit and push directly: `git push origin main`

## Development Quick Start

### Server Management Scripts
Use these shell scripts to manage the Pierre MCP Server:

```bash
# Start the server (loads .envrc, runs in background, shows health check)
./bin/start-server.sh

# Stop the server (graceful shutdown with fallback to force kill)
./bin/stop-server.sh

# Check server health
curl http://localhost:8081/health

# Reset development database (fixes migration checksum mismatches)
./bin/reset-dev-db.sh
```

### Database Reset (Development Only)
If you encounter migration checksum mismatch errors like:
```
migration 20250120000009 was previously applied but has been modified
```

Use the reset script to fix this:
```bash
./bin/reset-dev-db.sh
```

This script:
1. **Safety check**: Refuses to run against non-SQLite databases
2. **Backs up** the current database to `data/backups/`
3. **Deletes and recreates** the database with fresh migrations
4. **Runs all seeders** (admin user, coaches, demo data, social, mobility)

Default credentials after reset:
- Email: `admin@example.com`
- Password: `AdminPassword123`

### Admin User and Token Management
The `pierre-cli` binary manages admin users and API tokens:

```bash
# Create admin user for frontend login
RUST_LOG=info cargo run --bin pierre-cli -- user create --email admin@example.com --password SecurePassword123

# Generate API token for a service
RUST_LOG=info cargo run --bin pierre-cli -- token generate --service my_service --expires-days 30

# Generate super admin token (no expiry, all permissions)
RUST_LOG=info cargo run --bin pierre-cli -- token generate --service admin_console --super-admin

# List all admin tokens
RUST_LOG=warn cargo run --bin pierre-cli -- token list --detailed

# Revoke a token
cargo run --bin pierre-cli -- token revoke <token_id>
```

### OAuth Token Lifecycle
- Strava tokens expire after 6 hours
- The server automatically refreshes expired tokens using stored refresh_token
- Token refresh is transparent to tool execution
- If refresh fails, user must re-authenticate via OAuth flow

## API Keys and Credentials Lookup

When you need an API key or credential for any service:

1. **Check `.envrc`** — all API keys and tokens live here with comments explaining each one. This file is `.gitignore`d and holds the actual secret values.
2. **Check `.mcp.json`** to see which env vars are required — it references them as `${VAR_NAME}` placeholders (this file is committed to git, never contains actual secrets)
3. **If not found in either**, ask the project owner — never guess or fabricate credentials

### MCP-First Rule

When a service is configured in `.mcp.json`, **always use its MCP tools first** instead of CLI alternatives or web APIs. For example:
- **GitHub** — use `mcp__github__*` tools, not `gh` CLI (unless MCP lacks the needed operation)

### Key credentials in `.envrc`:
- `GITHUB_PERSONAL_ACCESS_TOKEN` — GitHub PAT for MCP and API access
- `EXPO_TOKEN` — Expo MCP server token
- `STRAVA_CLIENT_ID` / `STRAVA_CLIENT_SECRET` — Strava OAuth credentials
- `PIERRE_JWT_TOKEN` — JWT token for Pierre MCP server auth
- `OPENAI_API_KEY` — OpenAI API key for LLM features

## Rust Workspace Architecture

The backend is a Cargo workspace with 14 crates under `crates/`. Leaf crates are independent, reusable modules — none depend on `pierre_mcp_server`. Tool extensibility lives in `pierre-server`'s `tools::ToolRegistry` (implement `McpTool` and register in `register_builtin_tools`).

### Test Location
All integration tests live in `crates/pierre-server/tests/` (325 files). Doc tests compile per-crate. No `#[cfg(test)]` in `src/` — tests are external only.

## Development Guides

| Guide | Description |
|-------|-------------|
| [Tool Development](book/src/tool-development.md) | How to create new MCP tools using the pluggable architecture |

## Port Allocation (CRITICAL)

**Port 8081 is RESERVED for the Pierre MCP Server. NEVER start other services on this port.**

| Service | Port | Notes |
|---------|------|-------|
| Pierre MCP Server | 8081 | Backend API, health checks, OAuth callbacks |
| Expo/Metro Bundler | 8082 | Mobile dev server (configured in metro.config.js) |
| Web Frontend | 3000 | Vite dev server |

### Mobile Development Warning
When working on `frontend-mobile/`:
- **NEVER run `expo start` without specifying port** - it defaults to 8081
- **ALWAYS use `bun start`** which is configured for port 8082
- The `metro.config.js` and `package.json` are configured to use port 8082

If you see "Port 8081 is already in use", the Pierre server is running correctly. Use port 8082 for Expo:
```bash
# Correct way to start mobile dev server
cd frontend-mobile && bun start

# If you must use expo directly, specify port
npx expo start --port 8082
```

### Mobile Testing with Cloudflare Tunnels

To test the mobile app on a physical device, use Cloudflare tunnels to expose the local Pierre server:

```bash
# From frontend-mobile directory:
bun run tunnel           # Start tunnel only (outputs URL)
bun run start:tunnel     # Start tunnel AND Expo together
bun run tunnel:stop      # Stop the tunnel
```

**How it works:**
1. The tunnel script starts a Cloudflare tunnel pointing to localhost:8081
2. It updates `BASE_URL` in `.envrc` with the tunnel URL
3. It updates `EXPO_PUBLIC_API_URL` in `frontend-mobile/.env`
4. OAuth callbacks use `BASE_URL` instead of hardcoded localhost

**After starting the tunnel:**
1. Run `direnv allow` in the backend directory
2. Restart the Pierre server: `./bin/stop-server.sh && ./bin/start-server.sh`
3. The mobile app will connect via the tunnel URL

**Environment Variable:**
- `BASE_URL` - When set, OAuth redirect URIs use this instead of `http://localhost:8081`

## Mobile Development (frontend-mobile/)

### Mobile Validation Commands
When working on `frontend-mobile/`, run these validations:

```bash
cd frontend-mobile

# Tier 0: TypeScript (fastest feedback)
bun run typecheck

# Tier 1: ESLint
bun run lint

# Tier 2: Unit tests (~3s, 135 tests)
bun run test

# All tiers at once (what pre-push runs)
../scripts/ci/pre-push-mobile-tests.sh

# E2E tests (requires iOS Simulator, run before PR)
bun run e2e:build && bun run e2e:test
```

### React Native Patterns
- **Styling**: Use NativeWind (Tailwind) classes via `className`, not inline styles
- **State**: React Query for server state, Context API for app state
- **Navigation**: Follow drawer/stack patterns in `src/navigation/`
- **Components**: Reusable UI in `src/components/ui/` (Button, Card, Input)

### TypeScript Requirements
- All files must pass `bun run typecheck` with zero errors
- Use explicit types for component props (no implicit `any`)
- Prefer `unknown` with type guards over `any`

## Web Frontend Development (frontend/)

### API Client Architecture: No Local Duplication

**CRITICAL: Never create local API modules in `frontend/src/services/api/` for functionality that belongs in `@pierre/api-client`.**

- All cross-platform API methods MUST live in `packages/api-client/src/domains/`
- Web-only endpoints (admin, a2a, dashboard, keys, usage) stay local in `frontend/src/services/api/`
- Components import domain APIs from `'../services/api'` barrel (`index.ts`), never directly from individual domain files
- If a new endpoint is needed for both web and mobile: add it to `@pierre/api-client` first, then consume via the barrel
- Types shared between web and mobile MUST come from `@pierre/shared-types`, not inline interfaces in local files

### Frontend Validation Commands
When working on `frontend/`, run these validations:

```bash
cd frontend

# Tier 0: TypeScript (fastest feedback)
bun run type-check

# Tier 1: ESLint
bun run lint

# Tier 2: Unit tests (~4s)
bun run test -- --run

# All tiers at once (what pre-push runs)
../scripts/ci/pre-push-frontend-tests.sh

# E2E tests (requires browser, run before PR)
bun run test:e2e
```

### Frontend Patterns
- **Styling**: TailwindCSS classes
- **State**: React Query for server state, React Context for app state
- **Components**: Follow existing patterns in `src/components/`

# Writing code

- CRITICAL: NEVER USE --no-verify WHEN COMMITTING CODE
- We prefer simple, clean, maintainable solutions over clever or complex ones, even if the latter are more concise or performant. Readability and maintainability are primary concerns.
- Make the smallest reasonable changes to get to the desired outcome. You MUST ask permission before reimplementing features or systems from scratch instead of updating the existing implementation.
- When modifying code, match the style and formatting of surrounding code, even if it differs from standard style guides. Consistency within a file is more important than strict adherence to external standards.
- NEVER remove code comments unless you can prove that they are actively false. Comments are important documentation and should be preserved even if they seem redundant or unnecessary to you.
- All code files should start with a brief 2 line comment explaining what the file does. Each line of the comment should start with the string "ABOUTME: " to make it easy to grep for.
- When writing comments, avoid referring to temporal context about refactors or recent changes. Comments should be evergreen and describe the code as it is, not how it evolved or was recently changed.
- When you are trying to fix a bug or compilation error or any other issue, YOU MUST NEVER throw away the old implementation and rewrite without explicit permission from the user. If you are going to do this, YOU MUST STOP and get explicit permission from the user.
- NEVER name things as 'improved' or 'new' or 'enhanced', etc. Code naming should be evergreen. What is new today will be "old" someday.
- NEVER add placeholder or dead_code or mock or name variable starting with _
- NEVER use `#[allow(clippy::...)]` attributes EXCEPT for type conversion casts (`cast_possible_truncation`, `cast_sign_loss`, `cast_precision_loss`) when properly validated - Fix the underlying issue instead of silencing warnings
- Be RUST idiomatic
- Do not hard code magic value
- Do not leave implementation with "In future versions" or "Implement the code" or "Fall back". Always implement the real thing.
- Do not reference AI assistance in git commits.
- avoid #[cfg(test)] in the src code. Only in tests

## Security Engineering Rules

### Authorization Boundaries
- Authentication (who you are) is NOT authorization (what you can do)
- Every admin/coach/write endpoint MUST check role/permission, not just valid session
- Super-admin token minting MUST require existing super-admin credentials
- API key operations (create/revoke/list) MUST verify ownership via tenant_id

### Multi-Tenant Isolation
- Every database query MUST include `tenant_id` in WHERE clause (no exceptions)
- OAuth tokens, API keys, and LLM credentials are per-tenant — NEVER use global/shared storage
- Cache keys MUST include tenant_id to prevent cross-tenant cache poisoning
- Config write/delete operations MUST verify tenant membership before executing
- Admin tools that modify coach/user data MUST verify target belongs to caller's tenant

### Input Domain Validation
- Any value used as a divisor MUST be checked for zero before division
- Pagination parameters MUST have min/max bounds (e.g., limit clamped to 1..=100)
- Numeric inputs from users MUST be validated against domain-specific ranges
- Use `.max(1)` or equivalent guard before any division operation

### OAuth & Protocol Compliance
- OAuth state parameter MUST be cryptographically random and validated on callback
- PKCE (code_challenge/code_verifier) MUST be enforced for public clients
- Grant types MUST be restricted per-client (reject unregistered grant types)
- Token endpoints MUST validate redirect_uri matches the one used in authorization
- Discovery endpoints (`.well-known/`) MUST return spec-compliant metadata

### Logging Hygiene
- NEVER log: access tokens, refresh tokens, API keys, passwords, client secrets
- Redact or hash sensitive fields before logging (use redaction middleware)
- PII (email, IP, user agent) in logs MUST be at DEBUG level or redacted at INFO+
- Log levels for security events: WARN for auth failures, ERROR for breaches

### Canonical Redaction Helpers
- URL credentials: use `pierre_core::redaction::redact_url` — the only allowed redactor. Do NOT write a new one.
- HTTP request/response PII: use the `middleware::redaction` layer — already installed, do not bypass.
- Email masking: use `middleware::redaction::mask_email` for operator-visible logs that must include an email.
- "Is this secret loaded?" diagnostics: mirror `OAuthProviderConfig::secret_fingerprint` (SHA256 first 8 hex chars + length). Never log the raw value to "confirm it's set".

### Forbidden Logging Patterns (enforced by `scripts/ci/architectural-validation.sh`)
- `info!`/`warn!`/`error!`/`debug!`/`trace!`/`println!`/`eprintln!` lines referencing `Database URL`, `database_url`, or `connection_string` without also calling `redact_url` on the same line.
- Direct interpolation of variables named `password`, `client_secret`, `jwt_secret`, `encryption_key`, `access_token`, or `refresh_token` in log macros (e.g. `info!("...{password}...")`).
- `{:?}` / `{:#?}` inline-capture debug formatting of `ServerConfig`, `DatabaseConfig`, `DatabaseUrl`, `OAuthProviderConfig`, `FirebaseConfig`, `WeatherServiceConfig`, or `OAuth2ServerConfig`. These structs derive `Debug` for developer ergonomics but contain secrets — log only the specific fields you need.

### Template & Query Safety
- NEVER use `format!()` to build SQL queries — always use parameterized queries (`$1`, `$2`)
- HTML rendered server-side MUST escape all user-supplied values (use `html_escape::encode_text`)
- URL parameters MUST be percent-encoded with `urlencoding::encode()`
- Error messages returned to users MUST NOT contain stack traces or internal details

## Command Permissions

I can run any command WITHOUT permission EXCEPT:
- Commands that delete or overwrite files (rm, mv with overwrite, etc.)
- Commands that modify system state (chmod, chown, sudo)
- Commands with --force flags
- Commands that write to files using > or >>
- In-place file modifications (sed -i, etc.)

Everything else, including all read-only operations and analysis tools, can be run freely.

### Write Permissions
- Writing markdown files is limited to the `claude_docs/` folder under the repo

## Documentation Targets

Structured documents (ADR, runbook, plan, design analysis, audit, session artifact,
API reference) MUST land in the dravr-vault via the `obsidian-writer` skill, NOT in
GitHub gists.

Decision rule:
- ADR / decision               → dravr-vault `Architecture/ADRs/`
- Plan / phased build          → dravr-vault `Claude Plans/`
- Runbook / oncall procedure   → dravr-vault `Development/Runbooks/`
- Audit / design analysis      → dravr-vault `Claude Outputs/`
- Session handoff / report     → dravr-vault `Claude Outputs/`
- Reference docs that ship     → repo `book/src/` (mdBook)
- Directory-scoped specs       → repo `<dir>/README.md`

**Local Claude Code (this CLI on a developer machine):** prefer the vault. Use
`obsidian-writer`; if `obsidian-cli` is unwired, write to `claude_docs/` (symlinked
into the vault, `obsidian-git` auto-pushes within 10 min). Avoid `gh gist create`
for the doc types above — gists aren't searchable from the vault, can't be
wikilinked, and require gh-cli auth to read.

**Claude Code for Web (CCFW, containerized):** the container has no Obsidian app,
no `obsidian-cli`, and no vault checkout — `gh gist create` is the only durable
output for structured docs in that environment, so gists are acceptable there as a
fallback. Later, a local session backfills the gist into the vault (see the gist
backlog triage workflow). CCFW prompts that produce ADRs/plans/audits should still
explicitly drop a gist link in chat so the local follow-up can find it.

Gists are also fine for: pasteable code snippets, cross-project material that
doesn't belong in any single repo's vault, ephemeral share-with-stranger artifacts.

NEVER write structured docs only to chat — chat history is not durable.

## Pre-Push Validation & CI Monitoring

`./scripts/ci/pre-push-validate.sh` is the ONLY local validation command you need. It creates a `.git/validation-passed` marker (valid 15 minutes) that the pre-push hook checks against the current commit. All heavy compilation lives in CI.

Do NOT run `cargo fmt`, `cargo check`, or `cargo clippy` ad-hoc as a pre-push gate. The script runs the lightweight checks with the correct flags and scopes; the heavy gates are CI's job by design.

### What runs locally (non-compiling, tier-scoped)

Only tiers whose files actually changed on the branch run:

1. **Tier 0** — `cargo fmt --all -- --check`
2. **Tier 1** — `scripts/ci/architectural-validation.sh`
3. **Tier 1b** — `scripts/ci/check-vendor-contremaitre-readonly.sh`
4. **Tier 5** — frontend sub-script (only when `frontend/` changed)
5. **Tier 6** — SDK sub-script (only when `sdk/` changed)
6. **Tier 7** — mobile sub-script (only when `frontend-mobile/` changed)

### What runs in CI (heavy, compiling) — parallel jobs, fire immediately on push

In `ci-backend.yml`:
- **`fast-gate`** — fmt + architectural + secret patterns (~30s)
- **`preflight-clippy`** — per-crate clippy on changed leaf crates (~3–5 min, first failure for most regressions)
- **`clippy`** — full-workspace clippy (~10–12 min, gates `release-binary`)
- **`deadlock-analysis`** — lockbud static analysis (~10 min)
- **`security-audit`** — cargo deny + dravr-* duplicate check (~3 min)
- **`doc-tests`** — `cargo test --doc` (~5 min)
- **`backend-tests`** — SQLite shards (cron / workflow_dispatch only; per-push DB coverage comes from `ci-postgres.yml`)
- **`release-binary`** — release build + size check + smoke test (after `clippy`)

Separately on every push: `ci-postgres.yml`, `integration-tests.yml`, `frontend-tests.yml`, `sdk-tests.yml`, `mobile-unit-tests.yml`, `mcp-compliance.yml` — each scoped to its own paths filter.

### NEVER

- Run `cargo clippy --all-targets --all-features` (or full `cargo fmt`/`cargo check`) locally as a pre-push gate — CI's `clippy` job already runs it on every push.
- Run `cargo test` without `--test <file>` targeting — see "Test Targeting Patterns" below.
- Manually create or fake the `.git/validation-passed` marker — CI will catch the regression and main will break.
- Bypass with `git push --no-verify` unless explicitly asked.

### Workflow

1. **During development**: write code, run targeted tests (`cargo test --test <test_file>`), run per-crate clippy on the crate you're in (`cargo clippy -p <pkg> --all-targets --all-features -- -D warnings`).
2. **Before pushing**: `git add` your changes, commit, then run `./scripts/ci/pre-push-validate.sh`. `cancel-in-progress: true` is set on all workflows, so your push automatically cancels older runs on the same ref.
3. **Push**: `git push` (the pre-push hook verifies the validation marker).
4. **After pushing**: monitor CI until green — see "CI Monitoring" below.

### After Pushing — MANDATORY CI MONITORING

**The Agent MUST treat "push" as the start of validation, not the end.** Local pre-push only catches fmt/architecture/secret regressions. Clippy, deadlock, schema, and the test suite live in CI.

- After every push, the Agent MUST watch CI for the pushed commit until *all* relevant workflows reach a terminal status (success, failure, cancelled).
- If any workflow fails, fix the underlying issue and re-push in the same session. Do not move on to the next task.
- The Agent does NOT consider work "done" until CI is green on the head commit (`cancelled` workflows for older commits don't count).

### CI Monitoring

Use the first available method. **NEVER ask the user for a GitHub token** — fall back instead.

| Priority | Method | When to use |
|----------|--------|-------------|
| 1 | **WebFetch** on `https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A<branch>` | Default — does not consume any GitHub PAT rate-limit budget. |
| 2 | `gh run list --branch <branch>` / single targeted `gh run view <id>` calls | Only when WebFetch can't surface the information you need. Each call costs one quota slot from the shared 5000/hr `core` bucket. |
| 3 | GitHub MCP tools (`mcp__github__*`) | When the operation isn't a list/status check (e.g., commenting on a failure). |

**Forbidden during monitoring:**
- `gh run watch` — polls every few seconds and burns quota fast.
- Background `while :; do gh run list; sleep 60; done` loops — caused multi-day quota exhaustion in past sessions.
- Any polling cadence under 60s.

For waiting on long-running workflows, prefer `ScheduleWakeup` to re-check after a fixed delay. The session startup hook outputs `CI_MONITORING=gh` or `CI_MONITORING=fallback` to tell you which path is available.

### Test Targeting Patterns

Full test suite is ~13 min across 325 test binaries. **Always use `--test <file>`** to compile only the targeted test file:

```bash
# ❌ SLOW — compiles ALL 325 test files
cargo test test_browse_store_with_cursor_pagination

# ✅ FAST — only compiles the specific test file
cargo test --test store_routes_test test_browse_store_with_cursor_pagination -- --nocapture

# Run all tests in a specific file
cargo test --test intelligence_test -- --nocapture

# List tests in a specific test file
cargo test --test <test_file> -- --list
```

Find which file contains a test: `rg "test_name" tests/ --files-with-matches`.

### Test Output Verification — MANDATORY

After running ANY test command, you MUST verify tests actually ran. Exit code alone is NOT sufficient — `cargo test` exits 0 even when 0 tests run.

Red flags — STOP and investigate:
- `running 0 tests` — wrong target or flag used
- `0 passed; 0 failed` — no tests executed
- `filtered out` with 0 passed — filter pattern too restrictive

Verify: `running N tests` where N > 0, AND `N passed` in the summary. If 0 tests ran, the validation FAILED — do not proceed.

Common mistakes that run 0 tests:
```bash
# ❌ --lib only runs doc tests in src/, usually 0
cargo test --lib

# ❌ Typo in test name matches nothing
cargo test --test store_test test_brwose
```

Never claim "tests pass" if 0 tests ran.

## Error Handling Requirements

### Acceptable Error Handling
- `?` operator for error propagation
- `Result<T, E>` for all fallible operations
- `Option<T>` for values that may not exist
- Custom error types implementing `std::error::Error`

### Prohibited Error Handling
- `unwrap()` except in:
  - Test code with clear failure expectations
  - Static data known to be valid at compile time
  - Binary main() functions where failure should crash the program
- `expect()` - Acceptable ONLY for documenting invariants that should never fail:
  - Static/compile-time data: `"127.0.0.1".parse().expect("valid IP literal")`
  - Environment setup in main(): `env::var("DATABASE_URL").expect("DATABASE_URL must be set")`
  - NEVER use expect() for runtime errors that could legitimately occur
- `panic!()` - Only in test assertions or unrecoverable binary errors
- **Any form of `anyhow!` / `anyhow::anyhow!` / `anyhow::Error::msg`** — ABSOLUTELY FORBIDDEN in all production code (src/). ZERO TOLERANCE — CI fails on detection. Use structured error types instead (see below).

### Structured Error Type Requirements

When creating errors, you MUST:
1. **Use project-specific error enums** (e.g., `AppError`, `DatabaseError`, `ProviderError`)
2. **Use `.into()` or `?` for conversion** - let trait implementations handle the conversion
3. **Add context with `.context()`** when needed - but the base error MUST be a structured type
4. **Define new error variants** if no appropriate variant exists in the error enums

#### Correct Error Patterns
```rust
// GOOD: Using structured error types
return Err(AppError::not_found(format!("User {user_id}")));
return Err(DatabaseError::ConnectionFailed { source: e.to_string() }.into());
return Err(ProviderError::RateLimitExceeded {
    provider: "Strava".to_string(),
    retry_after_secs: 3600,
    limit_type: "Daily quota".to_string(),
});

// GOOD: Converting with context
database_operation().context("Failed to fetch user profile")?;
let user = get_user(id).await?; // Let ? operator handle conversion

// GOOD: Mapping external errors to structured types
external_lib_call().map_err(|e| AppError::internal(format!("External API failed: {e}")))?;
```

#### Prohibited Error Anti-Patterns
```rust
// FORBIDDEN in EVERY position (return, map_err, ok_or_else) — CI fails on detection:
anyhow::anyhow!("...")   anyhow!("...")   anyhow::Error::msg("...")
```

If no existing error variant fits your use case, add a new variant to the appropriate error enum (`AppError`, `DatabaseError`, `ProviderError`) with proper conversion traits.

## Mock Policy

### Real Implementation Preference
- PREFER real implementations over mocks in all production code
- NEVER implement mock modes for production features

### Acceptable Mock Usage (Test Code Only)
Mocks are permitted ONLY in test code for:
- Testing error conditions that are difficult to reproduce consistently
- Simulating network failures or timeout scenarios
- Testing against external APIs with rate limits during CI/CD
- Simulating hardware failures or edge cases

### Mock Requirements
- All mocks MUST be clearly documented with reasoning
- Mock usage MUST be isolated to test modules only
- Mock implementations MUST be realistic and representative of real behavior
- Tests using mocks MUST also have integration tests with real implementations

## Performance Standards

### Binary Size Constraints
- Target: <80MB for pierre_mcp_server
- Review large dependencies that significantly impact binary size
- Consider feature flags to minimize unused code inclusion
- Document any legitimate exceptions with business justification

### Clone Usage
- Document why each `clone()` is necessary
- Prefer `&T`, `Cow<T>`, or `Arc<T>` over `clone()`
- Justify each clone with ownership requirements analysis

### Arc Usage
- Only use when actual shared ownership required across threads
- Document the sharing requirement in comments
- Consider `Rc<T>` for single-threaded shared ownership
- Prefer `&T` references when data lifetime allows
- **Current count: ~107 Arc usages** - appropriate for multi-tenant async architecture

# Testing

- Tests MUST cover the functionality being implemented.
- NEVER ignore the output of the system or the tests - Logs and messages often contain CRITICAL information.
- If the logs are supposed to contain errors, capture and test it.
- NO EXCEPTIONS POLICY: Under no circumstances should you mark any test type as "not applicable". Every project, regardless of size or complexity, MUST have unit tests, integration tests, AND end-to-end tests. If you believe a test type doesn't apply, you need the human to say exactly "I AUTHORIZE YOU TO SKIP WRITING TESTS THIS TIME"

## Test Integrity: No Skipping, No Ignoring

**CRITICAL: All tests must run and pass. No exceptions.**

### Forbidden Patterns
- **Rust**: NEVER use `#[ignore]` attribute on tests
- **JavaScript/TypeScript**: NEVER use `.skip()`, `xit()`, `xdescribe()`, or `test.skip()`
- **CI Workflows**: NEVER use `continue-on-error: true` on test jobs
- **Any language**: NEVER comment out tests to make CI pass

### If a Test Fails
1. **Fix the code** - not the test
2. **Fix the test** - only if the test itself is wrong
3. **Ask for help** - if you're stuck, don't skip

### Rationale
Skipped/ignored tests become forgotten tech debt. A red CI that gets ignored is worse than no CI at all.

# RUST IDIOMATIC CODE GENERATION

Default to idiomatic Rust. The points below are the non-obvious or project-enforced ones.

## Ownership & Collections
- PREFER borrowing (`&T`, `&str`, `&[T]`) over owned params unless ownership is needed; `Cow<T>` for conditionally owned data; `AsRef<T>`/`Into<T>` for flexible APIs.
- Clone the Arc, never its contents: `arc.clone()`, not `(*arc).clone()`. Arc/Rc clones need no comment; JUSTIFY non-obvious value clones.
- PREFER iterator chains, `filter_map()` over `filter().map()`, `and_then()` over nested match. Pre-size with `with_capacity()` when the size is known.
- PREFER format args `format!("{name}")` over concatenation; `&'static str` for string constants.

## Control Flow, Types & API Design
- PREFER early returns with `?` over nested matches; `if let` for single patterns, `match` for complex logic. Exhaustive match when every variant needs distinct handling; catch-all `_` for evolving enums.
- Newtype pattern for domain ids (`struct UserId(i64)`); `enum` over boolean flags for state; `const fn`/associated consts for type-level values; associated types when the relationship is 1:1.
- `impl Trait` in argument position for flexibility, concrete return types when callers must name them. DESIGN APIs to be hard to misuse (parse, don't validate); builder pattern for many-optional-field structs.
- PREFER small focused functions (~50 lines), composition over inheritance, minimal dependencies, `std` over external crates when sufficient.

## Async, Concurrency & Performance
- PREFER `async fn` over `impl Future`; `tokio::spawn` for concurrent tasks, `.await` for sequential; structured concurrency via `join!`/`select!`; always handle `JoinHandle` results (don't ignore panics).
- `Arc<RwLock<T>>` over `Arc<Mutex<T>>` for read-heavy; channels over shared mutable state; atomics for simple counters. DOCUMENT every `Arc<T>` with its sharing justification; `Rc<T>` for single-threaded (async Tokio usually needs Arc).
- `std::sync::LazyLock` for lazy statics (Rust 1.80+, replaces lazy_static!), `OnceLock` for one-time runtime init. AVOID premature `#[inline]`; `#[cold]` for error paths; `const fn` for compile-time eval; `Box<T>` for recursive types.

## Modules & Imports (Enforced by `clippy::absolute_paths = "deny"` in Cargo.toml)
- USE `use` imports at the top of the file; AVOID inline qualified paths like `crate::models::User` or `std::collections::HashMap`. Qualified paths only for name collisions or single-use clarity.
- PREFER flat module hierarchies. This is a binary crate: `pub(crate)` documents intent but has no visibility effect; use explicit module paths for clarity.


## Mandatory Session Startup Checklist

Before touching any code in a new session, run in this order:

```bash
# 1. Pull shared build config (provides .build/hooks, .build/validation, etc.)
git submodule update --init --recursive

# 2. Set canonical git hooks path — ALWAYS .build/hooks, NEVER .githooks
git config core.hooksPath .build/hooks

# 3. Scan recent history for context
git log --oneline -10

# 4. Check CI health on main
gh run list --branch main --limit 10 --json workflowName,conclusion

# 5. See uncommitted work
git status
```

**If any workflow on main has been red for 2+ runs, STOP and surface it to the user** before starting the requested task. Ask: "Should I investigate CI before doing X?"

The canonical hooks/validation live in the `.build/` git submodule from
https://github.com/dravr-ai/dravr-build-config — never use a local `.githooks/`.

## Architectural Discipline

### No Backward Compatibility, No Legacy
Pre-1.0 project, zero external API consumers, no deprecation window.
Every rename, move, or replacement is a single-commit cutover. If you
want to keep "the old path around for now," STOP and ask — the answer
is almost always "finish the migration in this branch."

### Single Source of Truth
Before adding a new abstraction:
1. Grep for existing abstractions with similar purposes
2. If one exists, USE IT or DELETE it in the same commit that replaces it
3. Never leave two systems doing the same job "for compat"

### When Adding, Remove
Every commit that adds a new abstraction must identify what it replaces
and delete that in the same commit.

### Forbidden patterns (junk disguised as discipline)
These freeze architectural debt by making it *testable* instead of *fixed*.
Delete them when you find them; do not add them:

- **`KNOWN_OFFENDERS` / `PENDING_*` / `EXEMPT_*` const arrays** in tests
  enumerating files that violate an invariant. Fix offenders in the same
  branch, or change the invariant — don't list exceptions.
- **Adapter/wrapper types** bridging an old trait to a new trait
  (`impl NewTrait for X { fn m() { call_old(...) } }`). Port the body
  directly, delete the old function and its types.
- **Parallel accessors** bypassing a canonical config struct
  (standalone `base_url()` when `ServerConfig::base_url` exists).
- **Invariant tests policing drift between two systems** ("legacy map X
  must stay in sync with registry Y"). Delete X. Tests policing a
  *single* canonical system's internal consistency are fine.
- **Fallback dispatch paths** (step-3 fallbacks, `ToolId::from_name`
  parallel to a registry, `if not found in new, try legacy`).
- **Feature flags creating "old mode vs new mode"**.

Test: am I making a pre-existing parallel system *acceptable*, or
replacing it? If "acceptable," stop — that's junk.

### Complete Deletion, Not Deprecation
Don't mark code `// DEPRECATED` or `// TODO remove later`. Delete it.
If deletion is blocked, file an issue and link it from the code.

## Pushback Triggers — When to Stop and Ask

STOP and ask the user before proceeding when you find:

1. **Duplication** — two systems/modules doing similar things
2. **Stale state** — `TODO`, `FIXME`, `for compat`, `temporary`, `v2`
   comments in code you're touching
3. **Red CI** — workflows failing on main
4. **Version drift** — two versions of the same dep in Cargo.lock
5. **Request conflicts with architecture** — user asks you to add X but
   X exists differently → surface the existing thing
6. **Half-finished migrations** — both old and new paths still live
7. **Adapter/wrapper added without matching deletion** — `impl NewTrait
   for X { fn m() { call_old(...) } }` — why does `call_old` still exist?
8. **Invariant test with an exception list** — you're pinning debt.

Default behavior is to complete the requested task. These triggers override that.
