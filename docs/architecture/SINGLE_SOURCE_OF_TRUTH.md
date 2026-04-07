# Single Source of Truth — Canonical Subsystem Registry

<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

This document establishes the **single canonical location** for each major
subsystem in the Pierre platform. Every subsystem must have exactly one
authoritative source; duplicates are treated as bugs.

> **Why this exists:** In February 2026 a "temporary" parallel tool registration
> system (`mcp/schema/`) was never deleted after introducing `ToolRegistry`.
> When auth-based tiering was added it only read from the canonical system,
> silently dropping five analytics tools. This document prevents recurrence.

---

## 1. Tool Registration

| | |
|---|---|
| **Canonical source** | `crates/pierre-server/src/tools/registry.rs` — `ToolRegistry` |
| **Tool trait** | `crates/pierre-server/src/tools/traits.rs` — `McpTool` |
| **Pattern** | Central registry holds `Arc<dyn McpTool>` instances. Feature-flag-gated registration at startup. Capability-based filtering via `ToolCapabilities` bitflags controls visibility per auth tier. |
| **Anti-pattern** | Schema sub-modules (`mcp/schema/*.rs`) defining tool JSON independently of `McpTool` impls. Deleted in `aaeae90`. |

**Invariant:** Every tool callable via `tools/call` MUST be listed by
`tools/list` for the same auth tier. No fallback dispatch paths.

---

## 2. MCP Request Processing

| | |
|---|---|
| **Canonical source** | `crates/pierre-server/src/mcp/mcp_request_processor.rs` — `McpRequestProcessor` |
| **Pattern** | Single processor with `handle_request()` entry point. All transports (HTTP, SSE, stdio, A2A) delegate here. Transport-specific code handles framing/encoding only. |
| **Anti-pattern** | Duplicated method dispatch logic in individual transport route handlers. |

**Invariant:** Adding a new MCP method requires exactly one code change
in the processor, not per-transport changes.

---

## 3. Database Access

| | |
|---|---|
| **Canonical source** | `crates/pierre-database/src/repository_registry.rs` — `RepositoryRegistry` |
| **Pattern** | Trait-object registry holding `Arc<dyn Repository>` for every domain. Built once at startup via `from_sqlite()` or `from_postgres()` factory. No runtime backend dispatch. |
| **Anti-pattern** | Enum-based dispatch (`match backend { Sqlite => ..., Postgres => ... }`) at query time. |

**Invariant:** Adding a new repository requires: define trait in
`pierre-database`, implement for SQLite + PostgreSQL, register in the factory.

---

## 4. Provider Registration

| | |
|---|---|
| **Canonical source** | `crates/pierre-server/src/providers/registry.rs` — `ProviderRegistry` |
| **Core trait** | `crates/pierre-providers/src/core.rs` — `FitnessProvider` |
| **Pattern** | `ProviderRegistry::new()` factory with `#[cfg(feature = "provider-*")]` conditional compilation. All providers implement `FitnessProvider`. |
| **Anti-pattern** | Multiple abstraction layers wrapping providers, or transitive dependency drift between provider crates. |

**Invariant:** Every provider is registered in exactly one place
(`ProviderRegistry::new`), gated by its feature flag.

---

## 5. Authentication

| | |
|---|---|
| **Canonical source** | `crates/pierre-server/src/middleware/auth.rs` — `McpAuthMiddleware` |
| **Pattern** | **Deliberate cascading strategy** serving three client types: |

| Step | Method | Client type |
|------|--------|-------------|
| 1 | JWT cookie | Browser (web frontend) |
| 2 | API key (`pk_live_` / `pk_trial_` prefix) | Service-to-service, headless |
| 3 | Bearer token (JWT) | Mobile app, SDK |

Each step falls through on failure to the next. All three converge to the
same `AuthenticatedUser` struct. This is the **intended final design**, not
a consolidation target.

**Anti-pattern:** Adding a fourth auth path without updating this document
and the middleware cascade.

**Invariant:** All authenticated requests produce `AuthenticatedUser` with
`user_id` + `tenant_id`. No auth bypass routes except health checks and
public endpoints.

---

## 6. Type Schemas

| | |
|---|---|
| **Canonical source** | `packages/shared-types/` — `@pierre/shared-types` |
| **Pattern** | Single TypeScript package exporting all cross-platform types. Frontend (`frontend/src/types/api.ts`), mobile (`frontend-mobile/src/types/index.ts`), and SDK re-export from this package. |
| **Backend types** | `crates/pierre-core/src/models/` — Rust structs are the backend authority. Shared-types mirrors them for TypeScript consumers. |
| **Anti-pattern** | Inline TypeScript interfaces in frontend/mobile components that duplicate shared-types definitions. |

**Invariant:** Adding a new API response type requires: define in
`pierre-core`, mirror in `shared-types`, consume via re-exports.

---

## 7. Configuration

| | |
|---|---|
| **Canonical source** | `crates/pierre-server/src/config/environment.rs` — `ServerConfig::from_env()` |
| **Pattern** | Single orchestrator loading all config domains (database, auth, OAuth, security, cache, MCP, etc.) from environment variables. Sub-modules (`config/database.rs`, `config/cache.rs`, etc.) each expose a `from_env()` method called by the orchestrator. |
| **Anti-pattern** | Route handlers calling `env::var()` directly instead of reading from `ServerConfig` fields. |

**Invariant:** All environment variables consumed by the server are
documented in `ServerConfig` or its sub-modules. Route handlers access
config through `resources.config.*`.

**Known exceptions:** `provider_link_webhook.rs` reads
`PROVIDER_LINK_WEBHOOK_URL` / `PROVIDER_LINK_WEBHOOK_SECRET` in a
fire-and-forget `spawn_emit()` that lacks `resources` access (standalone
background task). Acceptable because the webhook is opt-in infrastructure
config, not application behavior.

---

## Anti-Patterns — What NOT to Add

These patterns are **forbidden** because they create parallel systems:

| Pattern | Why it's harmful |
|---------|-----------------|
| `KNOWN_OFFENDERS` / `PENDING_*` const arrays in tests | Pins architectural debt instead of fixing it |
| Adapter types bridging old trait → new trait | Keeps the old trait alive; port the body directly |
| Parallel accessors bypassing `ServerConfig` | Standalone `base_url()` when `ServerConfig::base_url` exists |
| Fallback dispatch (`if not found in new, try legacy`) | Hides incomplete migration |
| Feature flags creating "old mode vs new mode" | Freezes both paths |

See `CLAUDE.md` § "Forbidden patterns" for the full list with examples.

---

## Enforcement

| Guard | Location | What it checks |
|-------|----------|----------------|
| Architectural validation | `scripts/ci/architectural-validation.sh` | TOML-based pattern scanning, binary size, legacy function detection |
| Orphan duplication detection | `scripts/ci/detect-orphan-duplicates.sh` | Parallel systems, stale re-exports, env::var bypass |
| CI health check | `.github/workflows/ci-backend.yml` | Runs architectural validation on every push |
| Pre-push validation | `scripts/ci/pre-push-validate.sh` | Tier 0–6 checks including clippy strict mode |

---

## Updating This Document

When you add, rename, or delete a subsystem:

1. Update the relevant section above with the new canonical path
2. Delete the old path in the same commit (no deprecation period)
3. Add or update the invariant test if the subsystem boundary changed
