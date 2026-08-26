<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# Architecture

Pierre Fitness Platform is a multi-protocol fitness data platform that connects AI assistants to strava, garmin, fitbit, whoop, coros, and terra (150+ wearables). Single binary, single port (8081), multiple protocols.

## System Design

```
┌─────────────────┐
│   mcp clients   │ claude desktop, chatgpt, etc
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   pierre sdk    │ typescript bridge (stdio → http)
│   (npm package) │
└────────┬────────┘
         │ http + oauth2
         ▼
┌─────────────────────────────────────────┐
│   Pierre Fitness Platform (rust)        │
│   port 8081 (all protocols)             │
│                                          │
│   • mcp protocol (json-rpc 2.0)        │
│   • oauth2 server (rfc 7591)           │
│   • a2a protocol (agent-to-agent)      │
│   • rest api                            │
│   • sse (real-time notifications)      │
└────────┬────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────┐
│   fitness providers (1 to x)            │
│   • strava                              │
│   • garmin                              │
│   • fitbit                              │
│   • whoop                               │
│   • coros                               │
│   • synthetic (oauth-free dev/testing)  │
│   • custom providers (pluggable)        │
│                                          │
│   ProviderRegistry: runtime discovery   │
│   Environment config: PIERRE_*_*        │
└─────────────────────────────────────────┘
```

## Core Components

### Protocol-Agnostic Tool Layer (`crates/pierre-tool-runtime/src/protocol/`)
- protocol-agnostic business logic shared by the mcp and a2a protocols
- `protocol/executor.rs` runs a tool independent of transport; `protocols/converter.rs` adapts protocol-specific payloads
- dozens of fitness tools (activities, analysis, goals, sleep, recovery, nutrition, configuration)

### MCP Implementation (`crates/pierre-server/src/mcp/`)
- json-rpc 2.0 over http
- sse transport for streaming
- tool registry and execution

### OAuth2 Server (`crates/pierre-routes-identity/src/oauth2.rs`, `crates/pierre-auth/`)
- rfc 7591 dynamic client registration
- rfc 7636 pkce support
- jwt access tokens for mcp clients

### OAuth2 Client (`crates/pierre-routes-auth/src/oauth.rs`, `crates/pierre-services/`)
- pierre connects to fitness providers as oauth client
- pkce support for enhanced security
- automatic token refresh
- multi-tenant credential isolation

### Providers (`crates/pierre-providers/`)
- **pluggable provider architecture**: factory pattern with runtime registration
- **feature flags**: compile-time provider selection (`provider-strava`, `provider-garmin`, `provider-fitbit`, `provider-whoop`, `provider-coros`, `provider-terra`, `provider-synthetic`)
- **service provider interface (spi)**: `ProviderDescriptor` trait for external provider registration
- **bitflags capabilities**: efficient `ProviderCapabilities` with combinators (`full_health()`, `full_fitness()`)
- **1 to x providers simultaneously**: supports strava + garmin + custom providers at once
- **provider registry**: `ProviderRegistry` manages all providers with dynamic discovery
- **environment-based config**: cloud-native configuration via `PIERRE_<PROVIDER>_*` env vars:
  - `PIERRE_STRAVA_CLIENT_ID`, `PIERRE_STRAVA_CLIENT_SECRET` (also: legacy `STRAVA_CLIENT_ID`)
  - `PIERRE_<PROVIDER>_AUTH_URL`, `PIERRE_<PROVIDER>_TOKEN_URL`, `PIERRE_<PROVIDER>_SCOPES`
  - Falls back to hardcoded defaults if env vars not set
- **shared `FitnessProvider` trait**: uniform interface for all providers
- **built-in providers**: strava, garmin, fitbit, whoop, coros, terra (150+ wearables), synthetic (oauth-free dev/testing)
- **oauth parameters**: `OAuthParams` captures provider-specific oauth differences (scope separator, pkce)
- **dynamic discovery**: `supported_providers()` and `is_supported()` for runtime introspection
- **zero code changes**: add new providers without modifying tools or connection handlers
- **unified oauth token management**: per-provider credentials with automatic refresh

### Intelligence (`crates/pierre-intelligence/`, re-exporting `dravr-cageux`)
- activity analysis and insights
- performance trend detection
- training load calculation
- goal feasibility analysis

### Database (`crates/pierre-database/`)
- **repository pattern**: focused repositories following SOLID principles
- repositories constructed via `RepositoryImpl::new(db)` pattern
- pluggable backend (sqlite, postgresql) via `crates/pierre-database/src/backends/`
- encrypted token storage
- multi-tenant isolation

#### Repository Architecture

The database layer implements the repository pattern with focused, cohesive repositories:

Focused repository modules live under `crates/pierre-database/src/repositories/`
(27 modules), each owning one domain. Representative examples:

- **identity & access**: `users`, `oauth`, `api_keys`, `tenants`, `security`, `admin`
- **fitness data**: `health`, `workouts`, `mobility`, `data_source`, `weather`, `insights`
- **coaching**: `coaches`, `roster`, `messaging`, `recipes`, `harness_memory`
- **platform**: `usage`, `a2a`, `notifications`, `feature_flags`, `tool_selection`, `fitness_config`, `claim_verdicts`, `chat`, `seeder`

**repository construction pattern**:
```rust
use pierre_database::repositories::UserRepository;
use pierre_database::backends::factory::Database;

// Construct repository with database connection
let db: Database = /* ... */;
let user_repo = UserRepositoryImpl::new(db.clone());

// Use repository trait methods
let user = user_repo.get_by_id(user_id).await?;
let users = user_repo.list_by_status("active", Some(tenant_id)).await?;
```

**benefits**:
- **single responsibility**: each repository handles one domain
- **interface segregation**: consumers only depend on needed methods
- **testability**: mock individual repositories independently
- **maintainability**: changes isolated to specific repositories

### Authentication (`crates/pierre-auth/`)
- jwt token generation/validation
- api key management
- rate limiting per tenant

## Error Handling

Pierre Fitness Platform uses structured error types for precise error handling and propagation. The codebase **does not use anyhow** - all errors are structured types using `thiserror`.

### Error Type Hierarchy

```
AppError (crates/pierre-core/src/errors/mod.rs)
├── Database(DatabaseError)
├── Provider(ProviderError)
├── Authentication
├── Authorization
├── Validation
└── Internal
```

### Error Types

**DatabaseError** (`crates/pierre-core/src/errors/database.rs`):
- `NotFound`: entity not found (user, token, oauth client)
- `QueryFailed`: database query execution failure
- `ConstraintViolation`: unique constraint or foreign key violations
- `ConnectionFailed`: database connection issues
- `TransactionFailed`: transaction commit/rollback errors

**ProviderError** (`crates/pierre-core/src/errors/provider.rs`):
- `ApiError`: fitness provider api errors (status code + message)
- `AuthenticationFailed`: oauth token invalid or expired
- `RateLimitExceeded`: provider rate limit hit
- `NetworkError`: network connectivity issues
- `Unavailable`: provider temporarily unavailable

**AppError** (`crates/pierre-core/src/errors/mod.rs`):
- application-level errors with error codes
- http status code mapping
- structured error responses with context

### Error Propagation

All fallible operations return `Result<T, E>` types with **structured error types only**:
```rust
pub async fn get_user(db: &Database, user_id: &str) -> Result<User, DatabaseError>
pub async fn fetch_activities(provider: &Strava) -> Result<Vec<Activity>, ProviderError>
pub async fn process_request(req: Request) -> Result<Response, AppError>
```

**AppResult type alias** (`crates/pierre-core/src/errors/mod.rs`):
```rust
pub type AppResult<T> = Result<T, AppError>;
```

Errors propagate using `?` operator with automatic conversion via `From` trait implementations:
```rust
// DatabaseError converts to AppError via From<DatabaseError>
let user = user_repo.get_by_id(user_id).await?;

// ProviderError converts to AppError via From<ProviderError>
let activities = provider.fetch_activities().await?;
```

**no blanket anyhow conversions**: the codebase enforces zero-tolerance for `impl From<anyhow::Error>` via static analysis (`scripts/ci/lint-and-test.sh`) to prevent loss of type information.

### Error Responses

Structured json error responses:
```json
{
  "error": {
    "code": "database_not_found",
    "message": "User not found: user-123",
    "details": {
      "entity_type": "user",
      "entity_id": "user-123"
    }
  }
}
```

Http status mapping:
- `DatabaseError::NotFound` → 404
- `ProviderError::ApiError` → 502/503
- `AppError::Validation` → 400
- `AppError::Authentication` → 401
- `AppError::Authorization` → 403

Implementation: `crates/pierre-core/src/errors/` (`mod.rs` → `AppError`/`AppResult`, `database.rs` → `DatabaseError`, `provider.rs` → `ProviderError`)

## Request Flow

```
client request
    ↓
[security middleware] → cors, headers, csrf
    ↓
[authentication] → jwt or api key
    ↓
[tenant context] → load user/tenant data
    ↓
[rate limiting] → check quotas
    ↓
[protocol router]
    ├─ mcp → universal protocol → tools
    ├─ a2a → universal protocol → tools
    └─ rest → direct handlers
    ↓
[tool execution]
    ├─ providers (strava/garmin/fitbit/whoop/coros)
    ├─ intelligence (analysis)
    └─ configuration
    ↓
[database + cache]
    ↓
response
```

## Multi-Tenancy

Every request operates within tenant context:
- isolated data per tenant
- tenant-specific encryption keys
- custom rate limits
- feature flags

## Key Design Decisions

### Single Port Architecture
All protocols share port 8081. Simplified deployment, easier oauth2 callback handling, unified tls/security.

### Focused Context Dependency Injection

Replaces service locator anti-pattern with focused contexts providing type-safe DI with minimal coupling.

**context hierarchy** (`crates/pierre-server/src/context/`):
```
ServerContext
├── AuthContext         (auth_manager, auth_middleware, admin_jwt_secret, jwks_manager, firebase_auth)
├── DataContext         (database, repos, cache, provider_registry, activity_intelligence)
├── ConfigContext       (config, tenant_oauth_client, a2a_client_manager, admin_config)
├── NotificationContext (websocket_manager, sse_manager, oauth_notification_sender)
├── SecurityContext     (redaction_config, oauth2_rate_limiter, csrf_manager, csrf_middleware)
└── ExtensionContext    (sampling_peer, progress_notification_sender, cancellation_registry)
```

**usage pattern**:
```rust
// Access database from context, then construct repository
let db = ctx.data().database().clone();
let user_repo = UserRepositoryImpl::new(db);
let user = user_repo.get_by_id(id).await?;
let token = ctx.auth().auth_manager().validate_token(jwt)?;
```

**benefits**:
- **single responsibility**: each context handles one domain
- **interface segregation**: handlers depend only on needed contexts
- **testability**: mock individual contexts independently
- **type safety**: compile-time verification of dependencies

**migration**: `ServerContext::from(&ServerResources)` provides gradual migration path while remaining call sites are converted.

### Protocol Abstraction
Business logic in `protocols::universal` works for both mcp and a2a. Write once, use everywhere.

### Pluggable Architecture
- database: sqlite (dev) or postgresql (prod)
- cache: in-memory lru or redis (distributed caching)
- tools: `tools::ToolRegistry` (`McpTool` trait, registered in `register_builtin_tools`)

### Runtime SQL Queries

The codebase uses `sqlx::query()` (runtime validation) exclusively, not `sqlx::query!()` (compile-time validation).

**Why runtime queries:**
- **Multi-database support**: SQLite and PostgreSQL have different SQL dialects (`?1` vs `$1`). Compile-time macros lock to one database.
- **No build-time database**: `query!` macros require `DATABASE_URL` at compile time. Runtime queries allow building without a database.
- **CI simplicity**: No need for `sqlx prepare` or database containers during builds.
- **Backend abstraction**: `DatabaseProvider` trait enables runtime database selection.

**Trade-off:**
- No compile-time SQL validation - typos caught at runtime, not build time
- Mitigated by comprehensive integration tests against both databases

Implementation: `crates/pierre-database/src/backends/mod.rs` (trait), `crates/pierre-database/src/backends/postgres/`, `crates/pierre-database/src/database/`

### SDK Architecture

**TypeScript SDK** (`sdk/`): stdio→http bridge for MCP clients (Claude Desktop, ChatGPT).

```
MCP Client (Claude Desktop)
    ↓ stdio (json-rpc)
pierre-mcp-client (npm package)
    ↓ http (json-rpc)
Pierre MCP Server (rust)
```

**key features**:
- automatic oauth2 token management (browser-based auth flow)
- token refresh handling
- secure credential storage via system keychain
- npx deployment: `npx -y pierre-mcp-client@next --server http://localhost:8081`

Implementation: `sdk/src/bridge.ts`, `sdk/src/cli.ts`

### Type Mapping System

**rust→typescript type generation**: auto-generates TypeScript interfaces from server JSON schemas.

```
crates/pierre-mcp-schema/src/ (tool definitions)
    ↓ bun run generate-types  (scripts/sdk/generate-sdk-types.js)
@pierre/mcp-types  (generated parameter interfaces)
    ↑ re-exported by sdk/src/types.ts
```

**type-safe json schemas** (`crates/pierre-mcp-schema/src/json_schemas.rs`):
- replaces dynamic `serde_json::Value` with typed structs
- compile-time validation via serde
- fail-fast error handling with clear error messages
- backwards compatibility via field aliases (`#[serde(alias = "type")]`)

**generated types include**:
- `ToolParamsMap` - maps tool names to parameter types
- `ToolName` - union type of all tool names (`sdk/src/response-schemas.ts`, `keyof typeof ToolResponseSchemaMap`)
- common data types: `Activity`, `Athlete`, `Stats`, `FitnessConfig`

Usage: `bun run generate-types`

## Workspace Architecture

Pierre is a Rust workspace under `crates/*`. The main binary lives in `pierre-server`; library crates provide reusable building blocks.

| Crate | Path | Description |
|-------|------|-------------|
| `pierre_mcp_server` | `crates/pierre-server/` | Main binary (apex): wiring + startup that assembles MCP/REST/A2A/SSE from the library crates |
| `pierre-core` | `crates/pierre-core/` | Foundation: core types, errors, pagination, redaction, constants; entry point for the external `dravr-*` domain libraries |
| `pierre-database` | `crates/pierre-database/` | Database abstraction with repository traits and SQLite/PostgreSQL backends |
| `pierre-auth` | `crates/pierre-auth/` | Authentication, authorization, JWT, OAuth2 server, CSRF |
| `pierre-providers` | `crates/pierre-providers/` | Fitness provider integrations (Strava, Garmin, Fitbit, WHOOP, COROS, Terra) |
| `pierre-tool-runtime` | `crates/pierre-tool-runtime/` | MCP tool engine: the `McpTool` trait, `ToolRegistry`, and the bulk of built-in tool implementations |
| `pierre-services` | `crates/pierre-services/` | Business-logic services (auth, OAuth client flow, health sync, …) |
| `pierre-routes-*` | `crates/pierre-routes-*/` | Per-domain HTTP handlers (admin, auth, coaches, groups + notifications, dashboard, identity, billing, a2a, web-admin) |
| `pierre-llm` | `crates/pierre-llm/` | LLM provider abstraction (Gemini, Groq, OpenAI-compatible, Ollama) |
| `pierre-cache` | `crates/pierre-cache/` | Cache abstraction with tenant isolation (in-memory LRU + Redis) |
| `pierre-memory` | `crates/pierre-memory/` | Coaching harness memory (facts, compaction, sessions, notes, followups) |
| `pierre-evals` | `crates/pierre-evals/` | Coach evaluation harness (golden sets, LLM-as-judge, deterministic checks) |
| `pierre-groups` | `crates/pierre-groups/` | Group coaching business logic |
| `pierre-messaging` | `crates/pierre-messaging/` | Bridge re-exporting `dravr-canot` multi-channel messaging |
| `pierre-notifications` | `crates/pierre-notifications/` | Bridge re-exporting `dravr-commere` push notifications |
| `pierre-intelligence` | `crates/pierre-intelligence/` | Bridge re-exporting `dravr-cageux` fitness intelligence |
| `pierre-a2a` | `crates/pierre-a2a/` | A2A protocol types and agent card (feature-gated) |

This table lists the principal crates; the workspace currently has 47. The complete per-crate catalog (LOC, role, key dependencies) is in [Appendix A](#appendix-a-crate-catalog).

### Layering Rules

The dependency graph is a DAG with `pierre_mcp_server` (the binary) at the apex. The single hard invariant is **acyclicity** — a dependency cycle would simply fail to compile, so the graph that builds *is* the enforcement.

- **No library crate may depend on the binary** (`pierre_mcp_server`). The binary depends, directly or transitively, on every library crate.
- **Library crates depend on `pierre-core` and on each other freely**, as long as the graph stays acyclic. Post-decomposition this is the norm, not the exception: `pierre-tool-runtime` depends on ~21 sibling crates and `pierre-services` on ~22. (The earlier rule that "library crates cannot depend on each other" predates the workspace decomposition and is no longer true.)
- **`pierre-core` is the base layer** and the main entry point for the external `dravr-*` domain libraries (`dravr-cageux`, `dravr-canot`, `dravr-equilibre`, `dravr-riviere`, `dravr-stripe`, `embacle`).
- **External libraries also enter through a ring of higher-layer crates**, each pulling what it wraps: `pierre-intelligence`←`dravr-cageux`, `pierre-llm`←`embacle`, `pierre-providers`←`dravr-equilibre`/`dravr-sciotte`/`embacle`, plus the thin bridge crates `pierre-messaging`←`dravr-canot`, `pierre-notifications`←`dravr-commere`, `pierre-enforme`←`dravr-enforme`, `pierre-weather`←`dravr-meteo`, and `pierre-contremaitre`/`pierre-services`/`pierre-logging`←`dravr-tronc`.
- `pierre-a2a` and its routes are optional, gated behind the `protocol-a2a` feature flag.

### Where new code goes

| Adding… | Goes in | Mechanism |
|---------|---------|-----------|
| A new **MCP tool** | `pierre-tool-runtime` | Implement the `McpTool` trait under `crates/pierre-tool-runtime/src/implementations/<category>/` and register it in `register_builtin_tools`. (A handful of tools that reach into server-only internals — the endurance tools — live in `crates/pierre-server/src/tools/implementations/` and are wired by `pierre-server`'s `register_builtin_tools`.) |
| A new **provider** | `pierre-providers` | Implement the `FitnessProvider` trait and register it in the `ProviderRegistry`. |
| A new **repository** | `pierre-database` | Add the trait + SQLite/PostgreSQL backend implementations under `crates/pierre-database/src/`. |
| A new **HTTP route domain** | a `pierre-routes-*` crate | Add or extend the matching route crate, then mount it from `pierre-server`. |

## File Structure

The workspace has 47 crates under `crates/*` — the full catalog (LOC, role, key
deps) is in [Appendix A](#appendix-a-crate-catalog). Most subsystems that once
lived under `pierre-server/src/` have been extracted into their own crates;
`pierre-server` is now a thin orchestration shell that wires the libraries
together and hosts the in-binary routes. Each library crate owns a cohesive
subsystem — for example:

- protocol-agnostic tool layer → `pierre-tool-runtime/src/protocol/`
- OAuth2 server endpoints → `pierre-routes-identity/`; OAuth client flow → `pierre-routes-auth/` + `pierre-services/`
- fitness-intelligence engine → `pierre-intelligence/` (re-exporting `dravr-cageux`)
- structured errors → `pierre-core/src/errors/`
- HTTP/PII middleware → `pierre-middleware/`
- MCP tool schemas → `pierre-mcp-schema/`

What remains inside the binary crate:

```
crates/pierre-server/
└── src/
    ├── bin/                  # pierre-mcp-server.rs (the server entrypoint)
    ├── lib.rs                # crate root / public wiring
    ├── context/              # focused di contexts (auth, data, config, notification, security, extension)
    ├── mcp/                  # mcp protocol surface (request processor, protocol, resources, transport manager, tool handlers)
    ├── routes/               # in-binary http handlers (chat, mcp, endurance, health, usage, webhooks, …) + admin/, api_keys/, chat/, messaging/, tenants/
    ├── services/             # binary-local services (slack/discord gateways, chat verdicts, memory facts, training-history compute)
    ├── tools/                # built-in tool registration + the endurance tools that reach into server internals
    ├── config/               # runtime configuration assembly (mod.rs, admin/, routes/)
    ├── types/                # binary-local type glue
    ├── sse/                  # server-sent events wiring
    ├── cache.rs              # cache wiring
    ├── features.rs           # feature-flag plumbing
    ├── protocols_a2a.rs      # a2a protocol wiring (feature-gated)
    ├── test_utils.rs         # shared test helpers
    └── websocket.rs          # websocket transport
sdk/                          # typescript mcp client
├── src/bridge.ts             # stdio→http bridge
├── src/types.ts              # auto-generated types
└── test/                     # integration tests
```

## Security Layers

1. **transport**: https/tls
2. **authentication**: jwt tokens, api keys
3. **authorization**: tenant-based rbac
4. **encryption**: two-tier key management
   - master key: encrypts tenant keys
   - tenant keys: encrypt user tokens
5. **rate limiting**: token bucket per tenant
6. **atomic operations**: toctou prevention
   - refresh token consumption: atomic check-and-revoke
   - prevents race conditions in token exchange
   - database-level atomicity guarantees

## Scalability

### Horizontal Scaling
Stateless server design. Scale by adding instances behind load balancer. Shared postgresql and optional redis for distributed cache.

### Database Sharding
- tenant-based sharding
- time-based partitioning for historical data
- provider-specific tables

### Caching Strategy
- health checks: 30s ttl
- mcp sessions: lru cache (10k entries)
- weather data: configurable ttl
- distributed cache: redis support for multi-instance deployments
- in-memory fallback: lru cache with automatic eviction

## Tool Extensibility

Tool dispatch is driven by the `ToolRegistry` in `crates/pierre-tool-runtime/`.
Tools implement the `McpTool` trait (one impl per tool, executed by `McpTool::execute`)
and are registered through `register_builtin_tools`. Capability flags
on each impl drive admin/user filtering at list time, and feature flags
(`tools-data`, `tools-analytics`, `tools-coaches`, …) gate categories at compile time.

Implementation: `crates/pierre-tool-runtime/src/registry.rs` (registry),
`crates/pierre-tool-runtime/src/traits.rs` (`McpTool` + `ToolCapabilities`),
`crates/pierre-tool-runtime/src/implementations/` (per-category tool bodies).
The pierre-server-side wiring that registers the built-in set lives in
`crates/pierre-server/src/tools/registry_builtin.rs`.

## Algorithm Dependency Injection

Zero-overhead algorithm dispatch using rust enums instead of hardcoded formulas.

> The algorithm engine itself lives in the external `dravr-cageux` crate and is
> re-exported into the platform via `pierre-intelligence` (`pub use dravr_cageux::algorithms`).
> The platform configures it through environment variables (surfaced in the admin
> UI by `pierre-config`); the code paths below refer to `dravr-cageux`.

### Design Pattern

Fitness intelligence uses enum-based dependency injection for all calculation algorithms:

```rust
pub enum VdotAlgorithm {
    Daniels,                    // Jack Daniels' formula
    Riegel { exponent: f64 },   // Power-law model
    Hybrid,                     // Auto-select based on data
}

impl VdotAlgorithm {
    pub fn calculate_vdot(&self, distance: f64, time: f64) -> IntelligenceResult<f64> {
        match self {
            Self::Daniels => Self::calculate_daniels(distance, time),
            Self::Riegel { exponent } => Self::calculate_riegel(distance, time, *exponent),
            Self::Hybrid => Self::calculate_hybrid(distance, time),
        }
    }
}
```

### Benefits

**compile-time dispatch**: zero runtime overhead, inlined by llvm
**configuration flexibility**: runtime algorithm selection via environment variables
**defensive programming**: hybrid variants with automatic fallback
**testability**: each variant independently testable
**maintainability**: all algorithm logic in single enum file
**no magic strings**: type-safe algorithm selection

### Algorithm Types

Nine algorithm domains, one enum file each under `dravr-cageux` (`src/algorithms/`),
with multiple variants each. The `selection:` line notes how the variant is chosen —
environment variable, intelligence config file, or in code (see Configuration
Integration below for which are wired).

1. **max heart rate** (`maxhr.rs`)
   - fox, tanaka, nes, gulati
   - selection: env `PIERRE_MAXHR_ALGORITHM`

2. **training impulse (trimp)** (`trimp.rs`)
   - bannister male/female, edwards, lucia, hybrid
   - selection: in code

3. **training stress score (tss)** (`tss.rs`)
   - avg_power, normalized_power, hybrid
   - selection: env `PIERRE_TSS_ALGORITHM`

4. **vdot** (`vdot.rs`)
   - daniels, riegel, hybrid
   - selection: in code

5. **training load** (`training_load.rs`)
   - ema, sma, wma, kalman filter
   - selection: in code

6. **recovery aggregation** (`recovery_aggregation.rs`)
   - weighted, additive, multiplicative, minmax, neural
   - selection: in code

7. **functional threshold power (ftp)** (`ftp.rs`)
   - 20min_test, 8min_test, ramp_test, from_vo2max, hybrid
   - selection: config field `ftp`

8. **lactate threshold heart rate (lthr)** (`lthr.rs`)
   - from_maxhr, from_30min, from_race, lab_test, hybrid
   - selection: config field `lthr`

9. **vo2max estimation** (`vo2max.rs`)
   - from_vdot, cooper, rockport, astrand, bruce, hybrid
   - selection: config field `vo2max`

### Configuration Integration

Algorithms configured via `dravr-cageux` (`src/config/intelligence/algorithms.rs`); the platform exposes the env-var knobs through `pierre-config`'s admin definitions:

```rust
pub struct AlgorithmConfig {
    pub tss: String,     // TSS: avg_power | normalized_power | hybrid
    pub maxhr: String,   // Max HR: fox | tanaka | nes | gulati
    pub ftp: String,     // FTP: 20min_test | from_vo2max | ramp_test | ...
    pub lthr: String,    // LTHR: from_maxhr | from_30min | ...
    pub vo2max: String,  // VO2max: from_vdot | cooper_test | ...
    pub tss_fallback: TssFallbackConfig, // pace-based TSS fallback (no power/HR)
}
```

Defaults are optimized for balanced accuracy vs data requirements. Only `tss` and
`maxhr` are overridable at runtime via environment variable (`PIERRE_TSS_ALGORITHM`,
`PIERRE_MAXHR_ALGORITHM`); `ftp`/`lthr`/`vo2max` are set through the intelligence
config file. The remaining algorithm domains (vdot, trimp, training load, recovery
aggregation) expose the same enum-DI pattern but are selected in code rather than
from a config field.

### Enforcement

Automated validation ensures no hardcoded algorithms bypass the enum system.

Patterns defined: `scripts/ci/validation-patterns.toml` (parsed by `scripts/ci/parse-validation-patterns.py`); the algorithm engine and its own DI tests live in `dravr-cageux`.

Checks for:
- hardcoded formulas (e.g., `220 - age`)
- magic numbers (e.g., `0.182258` in non-algorithm files)
- algorithmic logic outside enum implementations

Exclusions documented in validation patterns (e.g., tests, algorithm enum files).

Ci pipeline fails on algorithm di violations (zero tolerance).

### Hybrid Algorithms

Special variant that provides defensive fallback logic:

```rust
pub enum TssAlgorithm {
    AvgPower,                // Simple, always works
    NormalizedPower { .. },  // Accurate, requires power stream
    Hybrid,                  // Try NP, fallback to avg_power
}

impl TssAlgorithm {
    fn calculate_hybrid(&self, activity: &Activity, ...) -> IntelligenceResult<f64> {
        Self::calculate_np_tss(activity, ...)
            .or_else(|_| Self::calculate_avg_power_tss(activity, ...))
    }
}
```

Hybrid algorithms maximize reliability while preferring accuracy when data available.

### Usage Pattern

All intelligence calculations use algorithm enums:

```rust
use pierre_intelligence::algorithms::vdot::VdotAlgorithm;

// Each domain enum implements FromStr for config-string selection; vdot is
// currently selected in code:
let algorithm = VdotAlgorithm::Daniels;
let vdot = algorithm.calculate_vdot(5000.0, 1200.0)?; // 5K in 20:00
```

No hardcoded formulas anywhere in intelligence layer.

Implementation: `dravr-cageux` (`src/algorithms/`, `src/config/intelligence/algorithms.rs`), re-exported via `pierre-intelligence`; enforcement patterns in `scripts/ci/validation-patterns.toml`

## PII Redaction

Middleware layer removes sensitive data from logs and responses.

Redacted fields:
- email addresses
- passwords
- tokens (jwt, oauth, api keys)
- user ids
- tenant ids

Redaction patterns:
- email: `***@***.***`
- token: `[REDACTED-<type>]`
- uuid: `[REDACTED-UUID]`

Enabled via `LOG_FORMAT=json` for structured logging.
Implementation: `crates/pierre-middleware/src/redaction.rs`

## Cursor Pagination

Keyset pagination using composite cursor (`created_at`, `id`) for consistent ordering.

Benefits:
- no duplicate results during data changes
- stable pagination across pages
- efficient for large datasets

Cursor format: base64-encoded json with timestamp (milliseconds) + id.

Example:
```
cursor: "eyJ0aW1lc3RhbXAiOjE3MDAwMDAwMDAsImlkIjoiYWJjMTIzIn0="
decoded: {"timestamp":1700000000,"id":"abc123"}
```

Endpoints using cursor pagination:
- `GET /admin/users/pending?cursor=<cursor>&limit=20`
- `GET /admin/users/active?cursor=<cursor>&limit=20`

Implementation: `crates/pierre-core/src/pagination.rs`, `crates/pierre-database/src/database/users.rs`, `crates/pierre-database/src/backends/postgres/`

## Monitoring

Health endpoint: `GET /health`
- database connectivity
- provider availability
- system uptime
- cache statistics

Logs: structured json via tracing + opentelemetry
Metrics: request latency, error rates, provider api usage

## Appendix A: Crate Catalog

The complete workspace as it stands today: 47 crates under `crates/*`, ordered
roughly from the foundation upward to the apex binary. "Key deps" lists the most
load-bearing internal (`pierre-*`) and external (`dravr-*`/`embacle`) dependencies,
not the exhaustive set — the large orchestration crates pull in 20+ siblings.
LOC counts are `src/**.rs` line totals and are approximate.

| Crate | LOC | Role | Key deps |
|-------|-----|------|----------|
| `pierre-jsonrpc` | 310 | JSON-RPC 2.0 envelope types | — |
| `pierre-formatters` | 350 | Tool-output formatting helpers | — |
| `pierre-tools-core` | 235 | Core tool primitives/IDs shared across the tool layers | `pierre-core` |
| `pierre-coach-parser` | 746 | Coach markdown front-matter + section parser | `pierre-core` |
| `pierre-email` | 394 | Transactional email sender | `pierre-core` |
| `pierre-enforme` | 16 | Bridge re-exporting `dravr-enforme` (provider sync) | `dravr-enforme` |
| `pierre-weather` | 16 | Bridge re-exporting `dravr-meteo` (weather) | `dravr-meteo` |
| `pierre-messaging` | 46 | Bridge re-exporting `dravr-canot` (multi-channel messaging) | `dravr-canot` |
| `pierre-notifications` | 50 | Bridge re-exporting `dravr-commere` (push notifications) | `dravr-commere`, `pierre-core` |
| `pierre-core` | 16,948 | Foundation: types, errors, pagination, redaction, constants; entry point for external domain libs | `dravr-cageux`, `dravr-canot`, `dravr-equilibre`, `dravr-riviere`, `dravr-stripe`, `embacle` |
| `pierre-cache` | 1,373 | Cache abstraction (in-memory LRU + Redis), tenant-scoped | `pierre-core` |
| `pierre-memory` | 935 | Coaching memory (facts, sessions, notes, followups) | `pierre-core` |
| `pierre-llm` | 9,006 | LLM provider abstraction (Gemini, Groq, OpenAI-compatible, Ollama) over `embacle` | `pierre-core`, `embacle` |
| `pierre-mcp-schema` | 1,417 | MCP tool schema types + `tools/list` shapes | `pierre-config`, `pierre-core`, `pierre-jsonrpc` |
| `pierre-external` | 494 | External (non-provider) service clients (e.g. USDA) | `pierre-cache`, `pierre-core` |
| `pierre-database` | 57,674 | Repository traits + SQLite/PostgreSQL backends | `pierre-core`, `pierre-memory` |
| `pierre-auth` | 12,013 | Auth, JWT, OAuth2 server, CSRF, rate limiting | `pierre-core`, `pierre-database`, `pierre-llm` |
| `pierre-intelligence` | 4,310 | Fitness intelligence over `dravr-cageux` (analysis, training load) | `dravr-cageux`, `pierre-core`, `pierre-database`, `pierre-llm`, `pierre-weather` |
| `pierre-providers` | 17,080 | Fitness provider integrations (Strava, Garmin, Fitbit, WHOOP, COROS, Terra, scraping) | `dravr-equilibre`, `dravr-sciotte`, `embacle`, `pierre-auth`, `pierre-cache`, `pierre-database` |
| `pierre-groups` | 1,937 | Group-coaching business logic | `pierre-core`, `pierre-database` |
| `pierre-health` | 892 | Health-data domain (sleep, recovery, snapshots, sources) | `pierre-config`, `pierre-core`, `pierre-database` |
| `pierre-evals` | 3,201 | Coach evaluation harness (golden sets, LLM-as-judge) | `pierre-core`, `pierre-llm`, `pierre-memory` |
| `pierre-config` | 7,436 | Runtime server configuration assembly | `pierre-auth`, `pierre-cache`, `pierre-intelligence`, `pierre-llm`, `pierre-middleware` |
| `pierre-contremaitre` | 6,821 | Prompt hot-reload + contremaitre integration | `dravr-tronc`, `pierre-database`, `pierre-evals`, `pierre-intelligence`, `pierre-llm`, `pierre-memory` |
| `pierre-middleware` | 2,835 | HTTP middleware (auth, redaction, CSRF, tenant, request-id) | `pierre-auth`, `pierre-cache`, `pierre-database`, `pierre-runtime-context` |
| `pierre-agui` | 1,365 | In-process progress vocabulary for messaging channels (AG-UI events) | `pierre-core` |
| `pierre-logging` | 1,368 | Tracing/log setup + error notification | `dravr-sciotte`, `dravr-tronc`, `pierre-contremaitre`, `pierre-core` |
| `pierre-runtime-context` | 648 | Focused DI runtime context wiring | `pierre-auth`, `pierre-contremaitre`, `pierre-database`, `pierre-groups`, `pierre-intelligence`, `pierre-providers` |
| `pierre-mcp-transport` | 1,340 | MCP transport plumbing (sessions, streaming) | `pierre-auth`, `pierre-database`, `pierre-mcp-schema`, `pierre-runtime-context` |
| `pierre-services` | 13,595 | Business-logic services layer (auth, OAuth flow, health sync, …) | ~22 siblings incl. `pierre-providers`, `pierre-intelligence`, `pierre-contremaitre`, `dravr-tronc` |
| `pierre-tool-runtime` | 28,331 | MCP tool engine: `McpTool` trait, `ToolRegistry`, bulk of tool implementations | ~21 siblings incl. `pierre-providers`, `pierre-intelligence`, `pierre-services`, `pierre-tools-core` |
| `pierre-commands` | 1,472 | Slash-command / command handlers | `pierre-contremaitre`, `pierre-groups`, `pierre-messaging`, `pierre-services` |
| `pierre-chat-pipeline` | 3,991 | Chat orchestration pipeline (LLM + tools + memory) | ~15 siblings incl. `pierre-tool-runtime`, `pierre-services`, `pierre-evals`, `pierre-memory` |
| `pierre-sse` | 1,168 | Server-sent events transport | `pierre-mcp-transport`, `pierre-middleware`, `pierre-services` |
| `pierre-a2a` | 3,080 | A2A protocol types + agent card (feature-gated) | `pierre-mcp-schema`, `pierre-mcp-transport`, `pierre-tool-runtime` |
| `pierre-seeders` | 5,177 | Dev/test data seeders | `pierre-coach-parser`, `pierre-database`, `pierre-intelligence` |
| `pierre-routes-a2a` | 1,053 | A2A protocol HTTP endpoints | `pierre-a2a`, `pierre-mcp-transport`, `pierre-tool-runtime` |
| `pierre-routes-admin` | 8,729 | Admin endpoints | `pierre-contremaitre`, `pierre-evals`, `pierre-services`, `pierre-tool-runtime` |
| `pierre-routes-auth` | 3,473 | Auth / OAuth-client + provider-connect endpoints | `dravr-sciotte`, `embacle`, `pierre-providers`, `pierre-routes-admin`, `pierre-services` |
| `pierre-routes-billing` | 428 | Billing endpoints | `pierre-database`, `pierre-middleware`, `pierre-runtime-context` |
| `pierre-routes-coaches` | 3,269 | Coach-marketplace endpoints | `pierre-coach-parser`, `pierre-notifications`, `pierre-services` |
| `pierre-routes-dashboard` | 1,052 | Dashboard endpoints | `pierre-auth`, `pierre-database`, `pierre-middleware` |
| `pierre-routes-identity` | 1,581 | Identity / profile endpoints | `pierre-auth`, `pierre-database`, `pierre-middleware` |
| `pierre-routes-groups` | 2,618 | Group-coaching + push-notification endpoints (renamed from `pierre-routes-social` on 2026-08-26, once the Chat-First Cutover had deleted the social routes) | `pierre-groups`, `pierre-notifications`, `pierre-services`, `pierre-tool-runtime` |
| `pierre-routes-web-admin` | 1,803 | Web admin-console endpoints | `pierre-config`, `pierre-llm`, `pierre-services`, `pierre-tool-runtime` |
| `pierre-cli` | 1,808 | **Binary** — `pierre-cli` (admin users, tokens, seeding) | `pierre-auth`, `pierre-evals`, `pierre-seeders`, `pierre-services` |
| `pierre_mcp_server` | 29,510 | **Binary (apex)** — assembles MCP/REST/A2A/SSE and starts the server | every library crate |
