// ABOUTME: For every registered tool and every role, what /mcp lists is exactly what the dispatcher lets through
// ABOUTME: Athlete, coach, and admin with and without Admin access: listed means not refused on access, unlisted means refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Listed equals callable (carnet#1511)
//!
//! The `/mcp` listing and the dispatch chokepoint both read the one access
//! predicate (`tool_access::withheld`), each building its own caller: the
//! listing from the request's context, the executor from the request it
//! dispatches. This suite holds the two paths to one answer for every
//! registered tool, without calling the predicate itself on either side.
//!
//! "Listed" is the real `tools/list` answer. "Callable" is the executor
//! built as `/mcp` builds it (the grant, `McpHttp`, the caller writing its
//! own reply). Every tool is switched off for the caller's tenant after the
//! listing, so a call that passes the access check stops at the tenant gate,
//! which sits right after it, and no tool body runs: that answer means
//! "not refused on access grounds".

use crate::common;
use crate::helpers::coach_roster_fixtures::{coach, coached_group, group_agent, person, Person};

use std::collections::BTreeSet;
use std::sync::Arc;

use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::server::McpServer;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::first_party_tools::FIRST_PARTY_SURFACE_ERROR_CODE;
use pierre_tool_runtime::guardian::TENANT_DISABLED_ERROR_CODE;
use pierre_tool_runtime::protocols::{ProtocolError, UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_access::COACH_ROLE_REQUIRED;
use serde_json::{json, Value};

/// How the dispatcher answered a call.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Answer {
    /// Past the access check: stopped at the tenant gate.
    PassedAccess,
    /// Refused on access grounds: scope, transport, role or support session.
    RefusedAccess(String),
    /// Anything else, which this suite does not expect.
    Other(String),
}

/// One caller this suite decides for.
struct Seat {
    label: &'static str,
    person: Person,
    grant: Vec<OAuthScope>,
}

async fn on_top_plan(res: &Arc<ServerContext>, who: &Person) {
    res.agent
        .database
        .repositories()
        .tenants
        .set_plan(who.tenant, "enterprise")
        .await
        .unwrap();
}

async fn seats(res: &Arc<ServerContext>) -> Vec<Seat> {
    let athlete = person(res, "athlete@listed-callable.test", "Ana Athlete").await;
    let coaching = coach(res, "coach@listed-callable.test", "Cole Coach").await;
    let agent = group_agent(res, &coaching).await;
    coached_group(res, coaching.tenant, coaching.id, &agent, "Listed squad").await;
    let admin = person(res, "admin@listed-callable.test", "Ada Admin").await;
    let narrow_admin = person(res, "admin2@listed-callable.test", "Abe Admin").await;
    for who in [&admin, &narrow_admin] {
        res.common
            .repos
            .users
            .set_admin_status(who.id, true)
            .await
            .unwrap();
    }
    for who in [&athlete, &coaching, &admin, &narrow_admin] {
        on_top_plan(res, who).await;
    }
    vec![
        Seat {
            label: "athlete",
            person: athlete,
            grant: OAuthScope::self_grant(),
        },
        Seat {
            label: "coach",
            person: coaching,
            grant: OAuthScope::self_grant(),
        },
        Seat {
            label: "admin with Admin access",
            person: admin,
            grant: OAuthScope::self_grant(),
        },
        Seat {
            label: "admin without Admin access",
            person: narrow_admin,
            grant: OAuthScope::delegable_grant(),
        },
    ]
}

async fn listed(server: &McpServer<dyn ToolRuntime>, seat: &Seat) -> BTreeSet<String> {
    let mut ctx = ToolContext::new()
        .with_user(seat.person.id.to_string())
        .with_tenant(seat.person.tenant.to_string());
    ctx.scopes = seat
        .grant
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    let request: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}
    }))
    .unwrap();
    let response = server
        .handle_request_with_context(request, &ctx)
        .await
        .expect("an answer");
    let response = serde_json::to_value(response).unwrap();
    response["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {response}"))
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Switch every catalogued tool off for `seat`'s tenant.
async fn switch_off_every_tool(res: &Arc<ServerContext>, seat: &Seat, tools: &[String]) {
    for tool in tools {
        res.mcp
            .tool_selection
            .set_tool_override(seat.person.tenant, tool, false, seat.person.id, None)
            .await
            .unwrap_or_else(|e| panic!("{tool} is catalogued, so it can be switched off: {e}"));
    }
}

fn error_code(result: Option<&Value>) -> Option<&str> {
    result.and_then(|result| result["error_code"].as_str())
}

async fn answer(res: &Arc<ServerContext>, seat: &Seat, tool: &str) -> Answer {
    let executor = UniversalToolExecutor::new(res.clone())
        .with_scopes(seat.grant.clone())
        .with_transport(Transport::McpHttp)
        .with_caller_writing_reply();
    let called = executor
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters: json!({}),
            user_id: seat.person.id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(seat.person.tenant.to_string()),
        })
        .await;
    match called {
        Ok(response) => match error_code(response.result.as_ref()) {
            Some(TENANT_DISABLED_ERROR_CODE) => Answer::PassedAccess,
            Some(code @ (FIRST_PARTY_SURFACE_ERROR_CODE | COACH_ROLE_REQUIRED)) => {
                Answer::RefusedAccess(code.to_owned())
            }
            _ => Answer::Other(format!("{response:?}")),
        },
        Err(ProtocolError::PermissionDenied { reason, .. }) => Answer::RefusedAccess(reason),
        Err(other) => Answer::Other(other.to_string()),
    }
}

#[tokio::test]
async fn every_tool_is_listed_exactly_when_the_dispatcher_lets_it_through() {
    common::init_server_config();
    let res = common::create_test_server_resources().await.unwrap();
    let server: Arc<McpServer<dyn ToolRuntime>> = build_mcp_server(res.clone());
    let registered: Vec<String> = res
        .mcp
        .tool_registry
        .tool_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert!(registered.len() > 100, "the whole registry: {registered:?}");

    let mut mismatches = Vec::new();
    let mut counts = Vec::new();
    for seat in seats(&res).await {
        let offered = listed(&server, &seat).await;
        switch_off_every_tool(&res, &seat, &registered).await;
        let mut passed = 0_usize;
        for tool in &registered {
            let answer = answer(&res, &seat, tool).await;
            match (offered.contains(tool), &answer) {
                (true, Answer::PassedAccess) => passed += 1,
                (false, Answer::RefusedAccess(_)) => {}
                (is_listed, _) => mismatches.push(format!(
                    "{}: {tool} listed={is_listed} answered {answer:?}",
                    seat.label
                )),
            }
        }
        counts.push((seat.label, offered.len(), passed));
    }
    assert!(
        mismatches.is_empty(),
        "bug: list and call disagree:\n{}",
        mismatches.join("\n")
    );

    // The roles' lists nest as JF set them: athlete < coach, and an admin
    // with Admin access is offered more than one without it.
    let listed_of = |label: &str| {
        counts
            .iter()
            .find(|(seat, ..)| *seat == label)
            .map(|(_, listed, _)| *listed)
            .unwrap()
    };
    assert!(listed_of("athlete") < listed_of("coach"), "{counts:?}");
    assert!(
        listed_of("admin without Admin access") < listed_of("admin with Admin access"),
        "{counts:?}"
    );
}
