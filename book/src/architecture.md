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

### Protocols Layer (`crates/pierre-server/src/protocols/`)
- `universal/` - protocol-agnostic business logic
- shared by mcp and a2a protocols
- dozens of fitness tools (activities, analysis, goals, sleep, recovery, nutrition, configuration)

### MCP Implementation (`crates/pierre-server/src/mcp/`)
- json-rpc 2.0 over http
- sse transport for streaming
- tool registry and execution

### OAuth2 Server (`crates/pierre-server/src/routes/oauth2.rs`, `crates/pierre-auth/`)
- rfc 7591 dynamic client registration
- rfc 7636 pkce support
- jwt access tokens for mcp clients

### OAuth2 Client (`crates/pierre-server/src/services/oauth_flow.rs`)
- pierre connects to fitness providers as oauth client
- pkce support for enhanced security
- automatic token refresh
- multi-tenant credential isolation

### Providers (`crates/pierre-server/src/providers/`, re-exporting `crates/pierre-providers/`)
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

### Intelligence (`crates/pierre-server/src/intelligence/`, `crates/pierre-intelligence/`)
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

**18 focused repositories** (`crates/pierre-database/src/repositories/`):
1. `UserRepository` - user account management
2. `OAuthTokenRepository` - oauth token storage (tenant-scoped)
3. `ApiKeyRepository` - api key management
4. `UsageRepository` - usage tracking and analytics
5. `A2ARepository` - agent-to-agent management
6. `ProfileRepository` - user profiles and goals
7. `InsightRepository` - ai-generated insights
8. `AdminRepository` - admin token management
9. `TenantRepository` - multi-tenant management
10. `OAuth2ServerRepository` - oauth 2.0 server functionality
11. `SecurityRepository` - key rotation and audit
12. `NotificationRepository` - oauth notifications
13. `FitnessConfigRepository` - fitness configuration management
14. `RecipeRepository` - recipe and nutrition management
15. `CoachesRepository` - custom AI coach personas
16. `ToolSelectionRepository` - per-tenant tool configuration
17. `MobilityRepository` - stretching and yoga routines
18. `SocialRepository` - friend connections and shared insights

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

### Authentication (`crates/pierre-auth/`, `crates/pierre-server/src/services/auth.rs`)
- jwt token generation/validation
- api key management
- rate limiting per tenant

## Error Handling

Pierre Fitness Platform uses structured error types for precise error handling and propagation. The codebase **does not use anyhow** - all errors are structured types using `thiserror`.

### Error Type Hierarchy

```
AppError (crates/pierre-server/src/errors.rs)
├── Database(DatabaseError)
├── Provider(ProviderError)
├── Authentication
├── Authorization
├── Validation
└── Internal
```

### Error Types

**DatabaseError** (`crates/pierre-database/src/errors.rs`):
- `NotFound`: entity not found (user, token, oauth client)
- `QueryFailed`: database query execution failure
- `ConstraintViolation`: unique constraint or foreign key violations
- `ConnectionFailed`: database connection issues
- `TransactionFailed`: transaction commit/rollback errors

**ProviderError** (`crates/pierre-providers/src/errors.rs`):
- `ApiError`: fitness provider api errors (status code + message)
- `AuthenticationFailed`: oauth token invalid or expired
- `RateLimitExceeded`: provider rate limit hit
- `NetworkError`: network connectivity issues
- `Unavailable`: provider temporarily unavailable

**AppError** (`crates/pierre-server/src/errors.rs`):
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

**AppResult type alias** (`crates/pierre-server/src/errors.rs`):
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

Implementation: `crates/pierre-server/src/errors.rs`, `crates/pierre-database/src/errors.rs`, `crates/pierre-providers/src/errors.rs`

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
crates/pierre-server/src/mcp/schema.rs (tool definitions)
    ↓ npm run generate-types
sdk/src/types.ts (47 parameter interfaces)
```

**type-safe json schemas** (`crates/pierre-server/src/types/json_schemas.rs`):
- replaces dynamic `serde_json::Value` with typed structs
- compile-time validation via serde
- fail-fast error handling with clear error messages
- backwards compatibility via field aliases (`#[serde(alias = "type")]`)

**generated types include**:
- `ToolParamsMap` - maps tool names to parameter types
- `ToolName` - union type of all 47 tool names
- common data types: `Activity`, `Athlete`, `Stats`, `FitnessConfig`

Usage: `npm run generate-types` (requires running server on port 8081)

## Workspace Architecture

Pierre is a Rust workspace under `crates/*`. The main binary lives in `pierre-server`; library crates provide reusable building blocks.

| Crate | Path | Description |
|-------|------|-------------|
| `pierre_mcp_server` | `crates/pierre-server/` | Main binary: routes, MCP protocol, tools, transports, orchestration |
| `pierre-core` | `crates/pierre-core/` | Core types, errors, pagination, redaction, constants |
| `pierre-database` | `crates/pierre-database/` | Database abstraction with repository traits and SQLite/PostgreSQL backends |
| `pierre-auth` | `crates/pierre-auth/` | Authentication, authorization, JWT, OAuth2 server, CSRF |
| `pierre-providers` | `crates/pierre-providers/` | Fitness provider integrations (Strava, Garmin, Fitbit, WHOOP, COROS, Terra) |
| `pierre-llm` | `crates/pierre-llm/` | LLM provider abstraction (Gemini, Groq, OpenAI-compatible, Ollama) |
| `pierre-cache` | `crates/pierre-cache/` | Cache abstraction with tenant isolation (in-memory LRU + Redis) |
| `pierre-memory` | `crates/pierre-memory/` | Coaching harness memory (facts, compaction, sessions, notes, followups) |
| `pierre-evals` | `crates/pierre-evals/` | Coach evaluation harness (golden sets, LLM-as-judge, deterministic checks) |
| `pierre-groups` | `crates/pierre-groups/` | Group coaching business logic |
| `pierre-messaging` | `crates/pierre-messaging/` | Bridge re-exporting `dravr-canot` multi-channel messaging |
| `pierre-notifications` | `crates/pierre-notifications/` | Bridge re-exporting `dravr-commere` push notifications |
| `pierre-intelligence` | `crates/pierre-intelligence/` | Bridge re-exporting `dravr-cageux` fitness intelligence |
| `pierre-a2a` | `crates/pierre-a2a/` | A2A protocol types and agent card (feature-gated) |

The main crate (`pierre_mcp_server`) depends on all library crates. Library crates can depend on `pierre-core` but not on each other or on the main crate. `pierre-a2a` is optional, gated behind the `protocol-a2a` feature flag.

Tool extensibility is provided by `pierre-server`'s `tools::ToolRegistry`: implement the `McpTool` trait under `crates/pierre-server/src/tools/implementations/<category>/` and register the impl in `ToolRegistry::register_builtin_tools`.

## File Structure

```
crates/
├── pierre-core/              # foundation: errors, models, config, constants, redaction
├── pierre-database/          # repository traits + sqlite/postgres backends
├── pierre-auth/              # auth, oauth2 server, jwt, csrf
├── pierre-providers/         # fitness data providers (Strava, Garmin, etc.)
├── pierre-intelligence/      # bridge re-exporting dravr-cageux
├── pierre-llm/               # LLM providers (Gemini, Groq, OpenAI-compatible, Ollama)
│   └── src/prompts/          # system prompts and prompt templates
├── pierre-cache/             # cache backends (memory LRU, Redis)
├── pierre-memory/            # coaching memory facts/sessions/notes
├── pierre-evals/             # coaching eval harness
├── pierre-groups/            # group coaching business logic
├── pierre-messaging/         # bridge re-exporting dravr-canot
├── pierre-notifications/     # bridge re-exporting dravr-commere
├── pierre-a2a/               # A2A protocol types (feature-gated: protocol-a2a)
└── pierre-server/            # main binary + orchestration
    └── src/
        ├── bin/                  # binaries (pierre-mcp-server, pierre-cli, seeders)
        ├── lib.rs                # public api
        ├── context/              # focused di contexts (auth, data, config, notification, security, extension)
        ├── mcp/                  # mcp protocol (json-rpc 2.0, sse transport, ServerResources)
        ├── jsonrpc/              # json-rpc plumbing
        ├── protocols/            # protocol-agnostic universal layer
        ├── routes/               # http handlers (rest + protocol endpoints)
        │   ├── oauth2.rs         # oauth2 server endpoints
        │   ├── social/           # social feature endpoints
        │   ├── admin/            # admin endpoints
        │   └── ...
        ├── services/             # business-logic services
        │   ├── auth.rs           # authentication service
        │   ├── oauth_flow.rs     # oauth2 client flow (pierre → providers)
        │   ├── health_sync.rs    # provider health sync
        │   └── ...
        ├── providers/            # local provider plumbing (re-exports pierre-providers)
        ├── intelligence/         # intelligence layer
        ├── tools/                # tool execution engine (ToolRegistry, McpTool impls)
        ├── middleware/           # http middleware (auth, redaction, csrf)
        ├── permissions/          # rbac
        ├── models/               # request/response models
        ├── types/                # type-safe json schemas
        ├── formatters/           # response formatters
        ├── config/               # runtime configuration
        ├── constants/            # server-only constants (re-exports pierre-core where shared)
        ├── llm/                  # llm integration (re-exports pierre-llm)
        ├── cache/                # caching layer (re-exports pierre-cache)
        ├── coaches/              # coach personas
        ├── commands/             # command handlers
        ├── contremaitre/         # contremaitre integration
        ├── email/                # transactional email
        ├── external/             # external service clients
        ├── insight_samples/      # canonical insight samples
        ├── logging.rs, logging/  # tracing setup
        ├── seeders/              # data seeders
        ├── sse/                  # server-sent events
        ├── agui/                 # admin/agent UI plumbing
        ├── a2a/                  # a2a protocol (re-exports pierre-a2a)
        ├── admin/                # admin auth + ops
        ├── errors.rs             # AppError + AppResult
        ├── features.rs           # feature-flag plumbing
        ├── health.rs             # health endpoint internals
        ├── pagination.rs         # pagination glue
        ├── test_utils.rs         # shared test helpers
        ├── utils/                # misc utilities
        └── websocket.rs          # websocket transport
sdk/                              # typescript mcp client
├── src/bridge.ts                 # stdio→http bridge
├── src/types.ts                  # auto-generated types
└── test/                         # integration tests
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

Tool dispatch is driven by `tools::ToolRegistry` in `crates/pierre-server/src/tools/`.
Tools implement the `McpTool` trait (one impl per tool, executed by `McpTool::execute`)
and are registered through `ToolRegistry::register_builtin_tools`. Capability flags
on each impl drive admin/user filtering at list time, and feature flags
(`tools-data`, `tools-analytics`, `tools-coaches`, …) gate categories at compile time.

Implementation: `crates/pierre-server/src/tools/registry.rs` (registry),
`crates/pierre-server/src/tools/traits.rs` (`McpTool` + `ToolCapabilities`),
`crates/pierre-server/src/tools/implementations/` (per-category tool bodies).

## Algorithm Dependency Injection

Zero-overhead algorithm dispatch using rust enums instead of hardcoded formulas.

### Design Pattern

Fitness intelligence uses enum-based dependency injection for all calculation algorithms:

```rust
pub enum VdotAlgorithm {
    Daniels,                    // Jack Daniels' formula
    Riegel { exponent: f64 },   // Power-law model
    Hybrid,                     // Auto-select based on data
}

impl VdotAlgorithm {
    pub fn calculate_vdot(&self, distance: f64, time: f64) -> Result<f64, AppError> {
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

Nine algorithm categories with multiple variants each. Source paths are under
`crates/pierre-server/src/intelligence/algorithms/` unless noted otherwise.

1. **max heart rate** (`max_heart_rate.rs`)
   - fox, tanaka, nes, gulati
   - environment: `PIERRE_MAXHR_ALGORITHM`

2. **training impulse (trimp)** (`trimp.rs`)
   - bannister male/female, edwards, lucia, hybrid
   - environment: `PIERRE_TRIMP_ALGORITHM`

3. **training stress score (tss)** (`tss.rs`)
   - avg_power, normalized_power, hybrid
   - environment: `PIERRE_TSS_ALGORITHM`

4. **vdot** (`vdot.rs`)
   - daniels, riegel, hybrid
   - environment: `PIERRE_VDOT_ALGORITHM`

5. **training load** (`training_load.rs`)
   - ema, sma, wma, kalman filter
   - environment: `PIERRE_TRAINING_LOAD_ALGORITHM`

6. **recovery aggregation** (`recovery_aggregation.rs`)
   - weighted, additive, multiplicative, minmax, neural
   - environment: `PIERRE_RECOVERY_ALGORITHM`

7. **functional threshold power (ftp)** (`ftp.rs`)
   - 20min_test, 8min_test, ramp_test, from_vo2max, hybrid
   - environment: `PIERRE_FTP_ALGORITHM`

8. **lactate threshold heart rate (lthr)** (`lthr.rs`)
   - from_maxhr, from_30min, from_race, lab_test, hybrid
   - environment: `PIERRE_LTHR_ALGORITHM`

9. **vo2max estimation** (`vo2max_estimation.rs`)
   - from_vdot, cooper, rockport, astrand, bruce, hybrid
   - environment: `PIERRE_VO2MAX_ALGORITHM`

### Configuration Integration

Algorithms configured via `crates/pierre-server/src/config/intelligence/algorithms.rs`:

```rust
pub struct AlgorithmConfig {
    pub max_heart_rate: String,     // PIERRE_MAXHR_ALGORITHM
    pub trimp: String,               // PIERRE_TRIMP_ALGORITHM
    pub tss: String,                 // PIERRE_TSS_ALGORITHM
    pub vdot: String,                // PIERRE_VDOT_ALGORITHM
    pub training_load: String,       // PIERRE_TRAINING_LOAD_ALGORITHM
    pub recovery_aggregation: String, // PIERRE_RECOVERY_ALGORITHM
    pub ftp: String,                 // PIERRE_FTP_ALGORITHM
    pub lthr: String,                // PIERRE_LTHR_ALGORITHM
    pub vo2max: String,              // PIERRE_VO2MAX_ALGORITHM
}
```

Defaults optimized for balanced accuracy vs data requirements.

### Enforcement

Automated validation ensures no hardcoded algorithms bypass the enum system.

Validation script: `scripts/validate-algorithm-di.sh`
Patterns defined: `scripts/ci/validation-patterns.toml`

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
    fn calculate_hybrid(&self, activity: &Activity, ...) -> Result<f64, AppError> {
        Self::calculate_np_tss(activity, ...)
            .or_else(|_| Self::calculate_avg_power_tss(activity, ...))
    }
}
```

Hybrid algorithms maximize reliability while preferring accuracy when data available.

### Usage Pattern

All intelligence calculations use algorithm enums:

```rust
use crate::intelligence::algorithms::vdot::VdotAlgorithm;
use crate::config::intelligence_config::get_config;

let config = get_config();
let algorithm = VdotAlgorithm::from_str(&config.algorithms.vdot)?;
let vdot = algorithm.calculate_vdot(5000.0, 1200.0)?; // 5K in 20:00
```

No hardcoded formulas anywhere in intelligence layer.

Implementation: `crates/pierre-server/src/intelligence/algorithms/`, `crates/pierre-server/src/config/intelligence/algorithms.rs`, `scripts/validate-algorithm-di.sh`

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
Implementation: `crates/pierre-server/src/middleware/redaction.rs`

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
