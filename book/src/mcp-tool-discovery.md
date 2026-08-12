<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# MCP Tool Discovery & Visibility

Pierre gates which tools a caller can discover on that caller's authenticated identity. Discovery is not a preview surface: a tool schema names the capability, spells out every argument, and describes what the tool does, so an ungated list tells an anonymous caller exactly which admin operations exist and how to shape a call against them.

Two surfaces expose the catalog, and both run the same gate:

| surface | shape | who serves it |
|---|---|---|
| `POST /mcp` + `tools/list` | MCP JSON-RPC (what MCP clients speak) | the `dravr-tronc` engine, through the platform's host seams |
| `GET /mcp/tools` | plain REST, `{"tools": [...]}` | `routes/mcp.rs`, calling those same host seams directly |

`GET /mcp/tools` exists so SDK type generation can pull the catalog over plain HTTP without speaking JSON-RPC. It is a convenience twin, not a second policy: it authenticates through `PierreAuthHook` and lists through `PierreToolDispatcher`, so for any given bearer it returns exactly what `tools/list` would.

## Visibility Tiers

```
tools/list  ·  GET /mcp/tools
      │
      ▼
┌──────────────┐
│ Valid bearer? │──── no ──── 401 + WWW-Authenticate (RFC 9728). No tools.
└──────┬───────┘
       │ yes
       ▼
┌──────────────┐
│ Tenant        │──── no ──── 403 Forbidden. No tools.
│ membership?   │
└──────┬───────┘
       │ yes
       ▼
┌──────────────┐
│ Global admin  │──── yes ─── all_schemas (every registered tool)
│ (User.is_admin)│
└──────┬───────┘
       │ no
       ▼
tenant_filtered_schemas
(ToolSelectionService + uncatalogued feature-flag tools, minus ADMIN_ONLY)
```

| auth state | what discovery returns | source |
|---|---|---|
| no bearer | `401` + `WWW-Authenticate` challenge | `PierreAuthHook::authenticate` |
| invalid/expired bearer | `401` + challenge carrying `error="invalid_token"` | same — no downgrade to a public subset |
| valid bearer, no tenant membership | `403` | `extract_tenant_context_internal` |
| authenticated tenant member | tenant-filtered tools minus `ADMIN_ONLY` | `ToolSelectionService` + uncatalogued tools |
| authenticated global admin | every registered tool | `ToolRegistry::all_schemas()` |

There is no unauthenticated tier. An invalid token is a rejection, not a downgrade — per the MCP authorization spec and RFC 6750, a present-but-bad credential must fail rather than silently resolve to a lesser identity.

Admin status is the **global** `User.is_admin` flag, resolved from the database at request time. A tenant owner is admin *of their tenant*; that does not grant system-wide admin powers, so owners do not see `ADMIN_ONLY` tools.

## Token Extraction

The bearer comes from the HTTP `Authorization` header. The tronc HTTP transport strips the `Bearer ` prefix and populates `JsonRpcRequest::auth_token`; `routes/mcp.rs` mirrors that extraction for `GET /mcp/tools` so both surfaces accept the same credential forms. `PierreAuthHook` then reconstructs the header the auth middleware expects — `Bearer <jwt>` for JWTs, a bare `pk_live_<key>` for API keys.

Implementation: `crates/pierre-server/src/mcp/host_seams.rs` (`PierreAuthHook`), `crates/pierre-server/src/routes/mcp.rs` (`bearer_token`).

## Tenant-Filtered Tools

For authenticated non-admin users, the tool list comes from two sources combined:

### 1. ToolSelectionService (catalog-based)

`ToolSelectionService` (`crates/pierre-server/src/mcp/tool_selection.rs`) computes the effective tool list for a tenant by applying rules in precedence order:

1. **Global Disabled** -- `PIERRE_DISABLED_TOOLS` environment variable disables tools for all tenants
2. **Plan Restriction** -- tools require a minimum plan level (starter, professional, enterprise)
3. **Tenant Override** -- admin-configured per-tenant enable/disable with optional reason
4. **Catalog Default** -- default enablement from the `tool_catalog` database table

Only tools where `is_enabled` is true after this cascade are included. When a `user_id` is present, per-user overrides are overlaid on top of the tenant computation, so a tool disabled for one user is hidden from that user's discovery.

### 2. Uncatalogued feature-flag tools

Tools registered via feature flags (`tools-coaches`, `tools-mobility`) exist in the `ToolRegistry` but may not have entries in `tool_catalog`. `ToolRegistry::uncatalogued_user_schemas()` returns these so they are not lost when filtering through the catalog.

`ADMIN_ONLY` tools are excluded from both paths for non-admin users.

### Fallback behavior

If `ToolSelectionService` fails (e.g., database error), discovery falls back to `user_visible_schemas()` -- all non-admin tools from the registry, without tenant filtering. The fallback never widens visibility past the non-admin tier.

## Generating SDK Types

`packages/mcp-types` is generated from the live catalog, so generation authenticates as a global admin. `scripts/sdk/generate-sdk-types.js` resolves a bearer in this order:

1. `PIERRE_ADMIN_TOKEN` — used by CI, which mints one with `pierre-cli user create` plus a password-grant login.
2. `ADMIN_EMAIL` + `ADMIN_PASSWORD` — a fresh `POST /oauth/token` password-grant login. `.envrc` exports both on a dev machine.
3. `logs/admin-token.txt` — written by `bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`.

Run it with `cd packages/mcp-types && bun run generate` against a running server.

## Implementation References

- Auth seam: `crates/pierre-server/src/mcp/host_seams.rs` (`PierreAuthHook`)
- Tool-listing seam: `crates/pierre-server/src/mcp/host_seams.rs` (`PierreToolDispatcher::list_tools`, `tenant_filtered_schemas`)
- REST discovery endpoint: `crates/pierre-server/src/routes/mcp.rs` (`McpRoutes::handle_tools`)
- Tool selection service: `crates/pierre-server/src/mcp/tool_selection.rs` (`ToolSelectionService`)
- Registry methods: `crates/pierre-tool-runtime/src/registry.rs` (`list_schemas_by_name_set`, `uncatalogued_user_schemas`, `user_visible_schemas`, `admin_tool_schemas`, `all_schemas`)
- Tenant context resolution: `crates/pierre-mcp-transport/src/tenant_isolation.rs` (`extract_tenant_context_internal`)

## Tests

- `tests/mcp_tools_list_count_e2e_test.rs` (real HTTP server):
  - `test_tools_list_unauthenticated_returns_401` / `test_tools_list_invalid_token_returns_401` -- no bearer and a bad bearer both reject, and disclose no tools
  - `test_discovery_endpoint_unauthenticated_returns_401` -- the same posture for `GET /mcp/tools`
  - `test_tools_list_admin_matches_registry_discovery_endpoint` -- the two surfaces return an identical set for one admin bearer
  - `test_tools_list_authenticated_owner_exceeds_floor` / `test_tools_list_tenant_member_non_admin_path_no_collapse` -- count floors that catch silent tool loss
- `tests/routes_mcp_http_test.rs`:
  - `test_mcp_tools_requires_auth` -- `GET /mcp/tools` 401s with the RFC 9728 challenge and returns no `tools` key
  - `test_mcp_tools_withholds_admin_tools_from_non_admin` -- an authenticated member's set contains none of `admin_tool_schemas()`
