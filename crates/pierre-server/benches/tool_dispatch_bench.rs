// ABOUTME: Criterion benchmarks for the MCP dispatch hot path — tools/list visibility rungs, tools/call gating, auth hook.
// ABOUTME: Answers "what does a tool request cost before the tool itself does any work?" with numbers, not vibes.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every MCP request pays the seam path before any tool logic runs:
//! [`PierreAuthHook::authenticate`] (JWT + tenant resolution), then either
//! `tools/list` (tenant/plan/user-override visibility filtering) or
//! `tools/call` ([`ToolHandlers::dispatch_tool_call`]: tenant re-resolution,
//! quota checks, per-user enablement, Guardian gate, usage recording). These
//! benches pin that overhead so regressions surface as CI numbers instead of
//! production tail latency.
//!
//! Scenario notes:
//! - `tools_list` measures hot-server steady state: the 5-minute tenant-rung
//!   cache is warmed in setup. The per-user override overlay is uncached by
//!   design, so `member_10_overrides` vs `member_no_overrides` isolates its
//!   marginal cost.
//! - `tools_call/get_configuration_catalog` is the representative dispatch:
//!   registry-routed through the full chokepoint (quota → enablement →
//!   Guardian-gated executor → usage recording) with a pure-static tool body.
//! - `tools_call/get_connection_status` takes the early-return carve-out in
//!   `route_tool_call` that bypasses the executor/Guardian — kept as an
//!   explicit contrast between the two dispatch shapes.
//! - No tracing subscriber is installed (matches the other benches), so
//!   numbers slightly undercount production tracing overhead.
//!
//! Run: `cargo bench -p pierre_mcp_server --bench tool_dispatch_bench`
//! Flamegraphs: append `-- --profile-time 10`, SVGs land under
//! `target/criterion/**/profile/`.

#![allow(
    clippy::missing_docs_in_private_items,
    clippy::unwrap_used,
    missing_docs
)]

use std::collections::HashMap;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use dravr_tronc::mcp::auth::AuthHook;
use dravr_tronc::mcp::host::ToolDispatcher;
use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::models::{TenantId, User};
use pierre_mcp_server::mcp::host_seams::{PierreAuthHook, PierreToolDispatcher};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::context::AuthMethod;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
#[cfg(unix)]
use pprof::criterion::{Output, PProfProfiler};
use serde_json::json;
use tokio::runtime::Runtime;

#[path = "common/server_fixture.rs"]
mod server_fixture;

/// Catalogued starter-plan tools disabled for the override scenario — all
/// visible on a starter tenant, so each override genuinely changes the
/// effective tool set (validated against the catalog at seed time).
const OVERRIDDEN_TOOLS: [&str; 10] = [
    "get_activities",
    "get_athlete",
    "get_stats",
    "analyze_activity",
    "get_activity_intelligence",
    "set_goal",
    "suggest_goals",
    "track_progress",
    "calculate_metrics",
    "compare_activities",
];

/// Everything the async groups share, built once outside the timed loops.
struct DispatchFixture {
    rt: Runtime,
    resources: Arc<ServerContext>,
    state: Arc<dyn ToolRuntime>,
    dispatcher: PierreToolDispatcher,
    member: User,
    member_tenant: TenantId,
    overridden: User,
    overridden_tenant: TenantId,
}

impl DispatchFixture {
    fn new() -> Self {
        let rt = Runtime::new().unwrap();
        let resources = rt.block_on(server_fixture::build_server_context());
        let state: Arc<dyn ToolRuntime> = resources.clone();
        let dispatcher = PierreToolDispatcher::new(resources.clone());

        let (member, member_tenant) = rt.block_on(server_fixture::seed_user(
            &resources,
            "bench-member@example.com",
        ));
        let (overridden, overridden_tenant) = rt.block_on(server_fixture::seed_user(
            &resources,
            "bench-overridden@example.com",
        ));
        rt.block_on(server_fixture::seed_user_overrides(
            &resources,
            overridden.id,
            &OVERRIDDEN_TOOLS,
        ));

        Self {
            rt,
            resources,
            state,
            dispatcher,
            member,
            member_tenant,
            overridden,
            overridden_tenant,
        }
    }
}

/// Per-call context as [`PierreAuthHook`] would resolve it for a JWT bearer.
fn ctx_for(user: &User, tenant: TenantId, is_admin: bool) -> ToolContext {
    ToolContext::new()
        .with_user(user.id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method(AuthMethod::JwtBearer.as_str())
        .as_admin(is_admin)
}

/// Pure in-memory registry operations — the sync floor under every dispatch.
fn bench_registry(c: &mut Criterion) {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let all_names: Vec<String> = registry
        .tool_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert!(!all_names.is_empty(), "builtin registry must not be empty");

    let mut group = c.benchmark_group("registry");
    group.bench_function("all_schemas", |b| {
        b.iter(|| black_box(registry.all_schemas()));
    });
    group.bench_function("list_schemas_by_name_set", |b| {
        b.iter(|| black_box(registry.list_schemas_by_name_set(black_box(&all_names))));
    });
    group.bench_function("get", |b| {
        b.iter(|| black_box(registry.get(black_box("get_configuration_catalog"))));
    });
    group.finish();
}

/// `tools/list` through the public dispatcher seam: admin (registry-only),
/// member (tenant rungs, warm cache), member with 10 user overrides (adds the
/// uncached per-user overlay).
fn bench_tools_list(c: &mut Criterion) {
    let f = DispatchFixture::new();

    let admin_ctx = ctx_for(&f.member, f.member_tenant, true);
    let member_ctx = ctx_for(&f.member, f.member_tenant, false);
    let overridden_ctx = ctx_for(&f.overridden, f.overridden_tenant, false);

    // Warm the 5-minute tenant-rung cache so the timed loops measure
    // hot-server steady state, and sanity-check the scenarios diverge.
    let member_tools =
        f.rt.block_on(f.dispatcher.list_tools(&f.state, &member_ctx));
    let overridden_tools =
        f.rt.block_on(f.dispatcher.list_tools(&f.state, &overridden_ctx));
    assert!(!member_tools.is_empty(), "member must see tools");
    assert!(
        overridden_tools.len() < member_tools.len(),
        "user overrides must hide tools from discovery"
    );

    let mut group = c.benchmark_group("tools_list");
    group.bench_function("admin", |b| {
        b.iter(|| {
            f.rt.block_on(f.dispatcher.list_tools(&f.state, black_box(&admin_ctx)))
        });
    });
    group.bench_function("member_no_overrides", |b| {
        b.iter(|| {
            f.rt.block_on(f.dispatcher.list_tools(&f.state, black_box(&member_ctx)))
        });
    });
    group.bench_function("member_10_overrides", |b| {
        b.iter(|| {
            f.rt.block_on(
                f.dispatcher
                    .list_tools(&f.state, black_box(&overridden_ctx)),
            )
        });
    });
    group.finish();
}

/// `tools/call` end-to-end dispatch overhead with near-zero tool bodies.
fn bench_tools_call(c: &mut Criterion) {
    let f = DispatchFixture::new();
    let ctx = ctx_for(&f.member, f.member_tenant, false);

    // Sanity: the representative tool must succeed, otherwise the loop
    // benchmarks an error path and the numbers lie.
    let warmup = f.rt.block_on(ToolHandlers::dispatch_tool_call(
        &f.resources,
        &f.state,
        &ctx,
        f.member.id,
        f.member_tenant,
        "get_configuration_catalog",
        json!({}),
    ));
    assert!(
        !warmup.is_error,
        "get_configuration_catalog must succeed in the bench fixture: {warmup:?}"
    );

    let mut group = c.benchmark_group("tools_call");
    group.bench_function("get_configuration_catalog", |b| {
        b.iter(|| {
            f.rt.block_on(ToolHandlers::dispatch_tool_call(
                &f.resources,
                &f.state,
                &ctx,
                f.member.id,
                f.member_tenant,
                black_box("get_configuration_catalog"),
                json!({}),
            ))
        });
    });
    group.bench_function("get_connection_status_carveout", |b| {
        b.iter(|| {
            f.rt.block_on(ToolHandlers::dispatch_tool_call(
                &f.resources,
                &f.state,
                &ctx,
                f.member.id,
                f.member_tenant,
                black_box("get_connection_status"),
                json!({"provider": "strava"}),
            ))
        });
    });
    group.finish();
}

/// [`PierreAuthHook::authenticate`] with a real JWT — the pre-dispatch cost
/// (bearer validation, tenant resolution, global-admin lookup) every MCP
/// request pays before reaching the dispatcher.
fn bench_auth_hook(c: &mut Criterion) {
    let f = DispatchFixture::new();

    let token = f
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &f.member,
            &f.resources.auth.jwks_manager,
            Some(f.member_tenant.to_string()),
        )
        .unwrap();
    let hook = PierreAuthHook {
        resources: f.resources.clone(),
    };
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_owned(),
        method: "tools/list".to_owned(),
        params: None,
        id: Some(json!(1)),
        auth_token: Some(token),
        headers: None,
        metadata: HashMap::new(),
    };

    let warmup = f.rt.block_on(hook.authenticate(&request, &f.state));
    assert!(warmup.is_ok(), "bench JWT must authenticate: {warmup:?}");

    c.bench_function("auth_hook/jwt_bearer", |b| {
        b.iter(|| {
            f.rt.block_on(hook.authenticate(black_box(&request), &f.state))
        });
    });
}

/// Criterion config with the pprof flamegraph hook (`-- --profile-time N`).
#[cfg(unix)]
fn bench_config() -> Criterion {
    Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)))
}

/// Criterion config without a profiler: pprof does not compile on Windows,
/// so non-unix builds bench timings only, no flamegraphs.
#[cfg(not(unix))]
fn bench_config() -> Criterion {
    Criterion::default()
}

criterion_group! {
    name = benches;
    config = bench_config();
    targets = bench_registry, bench_tools_list, bench_tools_call, bench_auth_hook
}
criterion_main!(benches);
