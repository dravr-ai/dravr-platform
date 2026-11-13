// ABOUTME: Freezes the MCP handshake and tools/list, byte for byte, against committed fixtures
// ABOUTME: Any change to a tool's name, description, schema or annotations shows up as a reviewed diff
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The MCP surface is a reviewed artifact
//!
//! An agent reads `initialize` and `tools/list` and nothing else before it
//! decides what to call, so a reworded description or a dropped property is a
//! behaviour change even when every test still passes. The SDK drift check sees
//! only descriptions and input property types, needs a live server and runs in
//! CI alone. This file answers the three requests in-process and compares them
//! with `tests/fixtures/mcp_surface/`:
//!
//! - `initialize`, with the build version redacted and the instructions held
//!   to the catalogue's prompt rather than to a copy of its words;
//! - `tools/list` with the admin bit set, for a user who coaches a group: the
//!   whole catalogue, `athlete` and the coach-only tools included;
//! - the tool names `tools/list` gives without it, each schema identical;
//! - the listing of a user who coaches no group, which is that catalogue
//!   without the coach-only tools and without `athlete` (carnet#1426).
//!
//! The contremaitre description overlay starts empty and is only filled when
//! `CONTREMAITRE_REPO` is set, so these are the compiled-in descriptions — the
//! same parity rule as the SDK type regeneration.
//!
//! After an intended change, regenerate and review the diff:
//!
//! ```bash
//! MCP_SURFACE_FIXTURE_WRITE=1 cargo test --test it -- mcp_surface_fixture_test::
//! ```

use crate::common;
use crate::helpers::coach_roster_fixtures::{coached_group, group_agent, Person};

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::{env, fs};

use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::server::McpServer;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

const WRITE_ENV: &str = "MCP_SURFACE_FIXTURE_WRITE";
const REDACTED: &str = "<redacted>";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mcp_surface")
        .join(format!("{name}.json"))
}

/// Compare `actual` with the committed fixture, or write it when regenerating.
fn assert_matches_fixture(name: &str, actual: &Value) {
    let rendered = format!("{}\n", serde_json::to_string_pretty(actual).unwrap());
    let path = fixture_path(name);
    if env::var_os(WRITE_ENV).is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, rendered).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing fixture {}: {e}; run with {WRITE_ENV}=1",
            path.display()
        )
    });
    assert!(
        rendered == expected,
        "the MCP surface `{name}` changed. If intended, regenerate with \
         {WRITE_ENV}=1 and review the diff of {}",
        path.display()
    );
}

async fn call(
    server: &McpServer<dyn ToolRuntime>,
    method: &str,
    params: Value,
    ctx: &ToolContext,
) -> Value {
    let request: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    }))
    .unwrap();
    let response = server
        .handle_request_with_context(request, ctx)
        .await
        .expect("a request with an id is answered");
    serde_json::to_value(response).unwrap()
}

/// The same tenant user seen twice, with the admin bit and without it, and a
/// second user who coaches no group. A global
/// admin holds the public profile on `/mcp`, so the two views are one list. The
/// first user coaches a group, so their list is the whole catalogue: `athlete=`
/// and the coach-only tools go to a coach alone. The
/// tenant is on the top plan: `tools/list` leaves out what a plan does not cover,
/// and the fixture freezes every tool an external client can be offered.
async fn admin_and_member(
    resources: &Arc<ServerContext>,
) -> (ToolContext, ToolContext, ToolContext) {
    let (user, _) = common::create_test_tenant(resources, "surface-fixture@example.com")
        .await
        .unwrap();
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .remove(0);
    resources
        .agent
        .database
        .repositories()
        .tenants
        .set_plan(tenant.id, "enterprise")
        .await
        .unwrap();
    let mut base = ToolContext::new()
        .with_user(user.id.to_string())
        .with_tenant(tenant.id.to_string());
    // `tools/list` lists what the credential's grant covers, and the grant an
    // authenticated request carries is set by the auth hook. This is the
    // athlete's own full grant, so the fixture is the whole catalogue; the
    // narrower views are asserted in `mcp_no_provider_external_test`.
    base.scopes = OAuthScope::self_grant()
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();

    // The user coaches one group, so their list carries what a coach is offered.
    let coach = Person {
        id: user.id,
        tenant: tenant.id,
        auth: String::new(),
    };
    // Coach access and an active group: the roster tool's rule (carnet#1511).
    resources
        .common
        .repos
        .users
        .set_manages_roster(user.id, true)
        .await
        .unwrap();
    let agent = group_agent(resources, &coach).await;
    coached_group(resources, tenant.id, user.id, &agent, "Fixture squad").await;

    // Another user of the same plan who coaches no group.
    let (other, _) = common::create_test_tenant(resources, "surface-fixture-athlete@example.com")
        .await
        .unwrap();
    let other_tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(other.id)
        .await
        .unwrap()
        .remove(0);
    resources
        .agent
        .database
        .repositories()
        .tenants
        .set_plan(other_tenant.id, "enterprise")
        .await
        .unwrap();
    let mut non_coach = ToolContext::new()
        .with_user(other.id.to_string())
        .with_tenant(other_tenant.id.to_string());
    non_coach.scopes.clone_from(&base.scopes);

    (base.clone().as_admin(true), base.as_admin(false), non_coach)
}

#[tokio::test]
async fn mcp_surface_matches_the_committed_fixtures() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let server: Arc<McpServer<dyn ToolRuntime>> = build_mcp_server(resources.clone());

    let mut initialize = call(
        &server,
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "clientInfo": { "name": "surface-fixture", "version": "1.0.0" },
            "capabilities": {}
        }),
        &ToolContext::default(),
    )
    .await;
    // The crate version moves on every release and the OAuth endpoints follow
    // the configured `BASE_URL` (a tunnel, a deploy); neither is the surface.
    initialize["result"]["serverInfo"]["version"] = json!(REDACTED);
    if let Some(oauth) =
        initialize["result"]["capabilities"]["experimental"]["oauth2"].as_object_mut()
    {
        for endpoint in oauth.values_mut() {
            let url = url::Url::parse(endpoint.as_str().unwrap()).unwrap();
            *endpoint = json!(format!("<base_url>{}", url.path()));
        }
    }
    // The instructions are authored and reviewed in contremaitre, and its bump
    // lane moves them with no platform change, so the surface owes serving them
    // rather than freezing their words here.
    assert_eq!(
        initialize["result"]["instructions"],
        json!(resources
            .mcp
            .prompt_registry
            .mcp_server_instructions_prompt()
            .trim())
    );
    initialize["result"]["instructions"] = json!(REDACTED);
    assert_matches_fixture("initialize", &initialize["result"]);

    let (admin, member, non_coach) = admin_and_member(&resources).await;
    let admin_list = call(&server, "tools/list", json!({}), &admin).await;
    assert_matches_fixture("tools_list_admin", &admin_list["result"]);
    // The catalogue view lists a subset of the same schemas, so only its names
    // are frozen; each entry must be byte-identical to the admin one. The
    // catalogue path once skipped the contremaitre description overlay.
    let member_list = call(&server, "tools/list", json!({}), &member).await;
    let admin_by_name: BTreeMap<&str, &Value> = admin_list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| (tool["name"].as_str().unwrap(), tool))
        .collect();
    let member_names: Vec<&str> = member_list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            let name = tool["name"].as_str().unwrap();
            assert_eq!(
                Some(&tool),
                admin_by_name.get(name),
                "the member schema of `{name}` differs from the admin one"
            );
            name
        })
        .collect();
    assert_matches_fixture("tools_list_member_names", &json!(member_names));

    // A user who coaches no group is listed that catalogue without the
    // coach-only tools and without `athlete`, and nothing else is different
    // (carnet#1426). Frozen as a derivation, not as a second 1.3 MB fixture.
    let non_coach_list = call(&server, "tools/list", json!({}), &non_coach).await;
    let mut expected: Vec<Value> = admin_list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tool| tool["name"] != "publish_training_plan")
        .cloned()
        .collect();
    let mut athlete_properties = 0_usize;
    for tool in &mut expected {
        if let Some(properties) = tool["inputSchema"]["properties"].as_object_mut() {
            athlete_properties += usize::from(properties.remove("athlete").is_some());
        }
    }
    assert!(
        athlete_properties > 20,
        "the coach list carries athlete= on the scoped reads, or this proves nothing"
    );
    assert_eq!(
        json!(expected),
        non_coach_list["result"]["tools"],
        "a non-coach's list is the coach's without publish_training_plan and athlete="
    );
    // The public profile's names: what every external client who coaches no
    // group is offered, a global admin included, and so what the generated
    // SDK types describe. The SDK drift check compares the types against it.
    let public_names: Vec<&str> = non_coach_list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_matches_fixture("tools_list_public_names", &json!(public_names));

    // Both views are in name order: a reordered list is a different prompt
    // prefix for every client that caches it.
    for (view, list) in [("admin", &admin_list), ("member", &member_list)] {
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "the {view} tools/list is not in name order");
    }
}
