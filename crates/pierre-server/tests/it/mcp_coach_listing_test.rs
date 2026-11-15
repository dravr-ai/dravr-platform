// ABOUTME: An external tools listing offers athlete= and the coach-only tools to a group's human coach alone
// ABOUTME: /mcp tools/list and the A2A extended card, per caller; the list is uncached and private; execution keeps its own gate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Who is listed the coach's arguments (carnet#1426)
//!
//! About fifty read tools carry an `athlete` property and `publish_training_plan`
//! serves a coach alone. Listed to every user they cost each athlete's client
//! bytes on every tool and offer a call that can only be refused, so the
//! listing carries them for a coach by the roster tool's rule: coach access
//! (`manages_roster`, or platform admin) and at least one active group.
//!
//! The listing is advice. That a non-coach who sets `athlete` is still refused
//! in band is pinned where the gate is:
//! `coach_athlete_reads_test::an_mcp_caller_who_coaches_no_one_by_that_name_is_refused`.

#[cfg(all(feature = "tools-groups", feature = "tools-memory"))]
mod mcp_coach_listing_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, person, Person,
    };

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use dravr_tronc::mcp::protocol::JsonRpcRequest;
    use dravr_tronc::mcp::tool::ToolContext;
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_mcp_server::mcp::host_seams::build_mcp_server;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    /// The one tool that declares itself coach-only.
    const COACH_ONLY_TOOL: &str = "publish_training_plan";
    /// A read tool that lists `athlete` for a coach.
    const SCOPED_READ: &str = "get_activities";

    fn grant(res: &Arc<ServerContext>, user: &Person) -> String {
        format!(
            "Bearer {}",
            common::delegated_test_token_for_tenant(res, user.id, Some(user.tenant.to_string()))
        )
    }

    async fn post(app: &Router, uri: &str, authorization: &str, method: &str) -> Value {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": {} });
        let request = Request::post(uri)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("A2A-Version", "1.0")
            .header("authorization", authorization)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{uri}: {}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn tools(app: &Router, authorization: &str) -> Vec<Value> {
        let listed = post(app, "/mcp", authorization, "tools/list").await;
        listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list answered: {listed}"))
            .clone()
    }

    fn names(tools: &[Value]) -> BTreeSet<String> {
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    fn takes_athlete(tool: &Value) -> bool {
        tool["inputSchema"]["properties"]
            .as_object()
            .is_some_and(|properties| properties.contains_key("athlete"))
    }

    fn with_athlete(tools: &[Value]) -> BTreeSet<String> {
        tools
            .iter()
            .filter(|tool| takes_athlete(tool))
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn skills(app: &Router, authorization: &str) -> BTreeSet<String> {
        let card = post(app, "/a2a/jsonrpc", authorization, "GetExtendedAgentCard").await;
        assert!(card["error"].is_null(), "{card}");
        card["result"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// `user` coaches one new group, as an invite attaches a coach.
    async fn make_coach_of_a_group(res: &Arc<ServerContext>, user: &Person, name: &str) {
        let agent = group_agent(res, user).await;
        coached_group(res, user.tenant, user.id, &agent, name).await;
    }

    #[tokio::test]
    async fn a_non_coach_is_listed_no_athlete_argument_and_no_coach_only_tool() {
        let res = create_test_server_resources().await.unwrap();
        let sam = person(&res, "sam@coach-listing.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);

        let listed = tools(&app, &grant(&res, &sam)).await;
        let listed_names = names(&listed);
        assert!(
            listed_names.contains(SCOPED_READ) && listed_names.len() > 50,
            "the rest of the catalogue is listed, or this proves nothing: {listed_names:?}"
        );
        assert!(
            !listed_names.contains(COACH_ONLY_TOOL),
            "{COACH_ONLY_TOOL} serves a coach alone"
        );
        assert_eq!(
            with_athlete(&listed),
            BTreeSet::new(),
            "no tool advertises athlete= to a user who coaches no group"
        );
    }

    #[tokio::test]
    async fn a_coach_is_listed_both() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        let app = ProviderToolRouter::build_http_app(&res);

        let listed = tools(&app, &grant(&res, &casey)).await;
        let scoped = with_athlete(&listed);
        assert!(
            scoped.contains(SCOPED_READ) && scoped.len() > 20,
            "a coach is offered athlete= on the scoped reads and the plan tools: {scoped:?}"
        );
        assert!(
            names(&listed).contains(COACH_ONLY_TOOL),
            "{COACH_ONLY_TOOL} is listed to a group's human coach"
        );
        // Hiding `athlete` from a non-coach must never leave a listed schema
        // requiring it, or the tool could not be called.
        for tool in &listed {
            let required = tool["inputSchema"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(
                !required.iter().any(|name| name == "athlete"),
                "{} requires athlete",
                tool["name"]
            );
        }
    }

    #[tokio::test]
    async fn a_non_coach_list_is_the_coach_list_minus_exactly_those() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey2@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        let sam = person(&res, "sam2@coach-listing.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);

        let for_coach = tools(&app, &grant(&res, &casey)).await;
        let for_sam = tools(&app, &grant(&res, &sam)).await;

        let mut stripped: Vec<Value> = for_coach
            .into_iter()
            .filter(|tool| tool["name"] != COACH_ONLY_TOOL)
            .collect();
        for tool in &mut stripped {
            if let Some(properties) = tool["inputSchema"]["properties"].as_object_mut() {
                properties.remove("athlete");
            }
        }
        assert_eq!(
            names(&stripped),
            names(&for_sam),
            "the two audiences differ by the coach-only tool alone"
        );
        for (coach_view, sam_view) in stripped.iter().zip(&for_sam) {
            assert_eq!(
                coach_view, sam_view,
                "{} differs by more than athlete=",
                coach_view["name"]
            );
        }
    }

    #[tokio::test]
    async fn a_user_who_becomes_a_coach_is_listed_both_on_the_next_listing() {
        let res = create_test_server_resources().await.unwrap();
        let robin = coach(&res, "robin@coach-listing.test", "Robin Becoming").await;
        let app = ProviderToolRouter::build_http_app(&res);
        let authorization = grant(&res, &robin);

        // `manages_roster` alone does not make a coach: the gate is a group.
        let before = tools(&app, &authorization).await;
        assert!(
            with_athlete(&before).is_empty(),
            "a user who coaches no group yet is offered no athlete="
        );
        assert!(
            !names(&before).contains(COACH_ONLY_TOOL),
            "a user who coaches no group yet is not listed {COACH_ONLY_TOOL}"
        );

        make_coach_of_a_group(&res, &robin, "New squad").await;

        let after = tools(&app, &authorization).await;
        assert!(
            with_athlete(&after).contains(SCOPED_READ),
            "the listing after the group is attached offers athlete= on {SCOPED_READ}"
        );
        assert!(
            names(&after).contains(COACH_ONLY_TOOL),
            "the listing after the group is attached lists {COACH_ONLY_TOOL}"
        );
    }

    /// The roster tool's rule decides (carnet#1511): an active group is not
    /// enough without coach access, `manages_roster` or platform admin.
    #[tokio::test]
    async fn a_group_coach_without_coach_access_is_listed_neither() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey4@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        res.common
            .repos
            .users
            .set_manages_roster(casey.id, false)
            .await
            .unwrap();
        let app = ProviderToolRouter::build_http_app(&res);

        let listed = tools(&app, &grant(&res, &casey)).await;
        assert!(
            !names(&listed).contains(COACH_ONLY_TOOL),
            "{COACH_ONLY_TOOL} is not listed to a group coach without coach access"
        );
        assert_eq!(
            with_athlete(&listed),
            BTreeSet::new(),
            "no tool advertises athlete= to a group coach without coach access"
        );
    }

    /// Platform admin is coach access too: an admin who coaches an active
    /// group without `manages_roster` is listed both.
    #[tokio::test]
    async fn a_group_coach_who_is_a_platform_admin_is_listed_both() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey5@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        res.common
            .repos
            .users
            .set_manages_roster(casey.id, false)
            .await
            .unwrap();
        res.common
            .repos
            .users
            .set_admin_status(casey.id, true)
            .await
            .unwrap();
        let app = ProviderToolRouter::build_http_app(&res);

        let listed = tools(&app, &grant(&res, &casey)).await;
        assert!(
            names(&listed).contains(COACH_ONLY_TOOL),
            "{COACH_ONLY_TOOL} is listed to a platform admin who coaches a group"
        );
        assert!(
            with_athlete(&listed).contains(SCOPED_READ),
            "{SCOPED_READ} offers athlete= to a platform admin who coaches a group"
        );
    }

    #[tokio::test]
    async fn the_a2a_card_offers_the_coach_only_skill_to_a_coach_alone() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey3@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        let sam = person(&res, "sam3@coach-listing.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);

        let for_sam = skills(&app, &grant(&res, &sam)).await;
        assert!(
            for_sam.contains(SCOPED_READ),
            "a non-coach's card offers the plain reads: {for_sam:?}"
        );
        assert!(
            !for_sam.contains(COACH_ONLY_TOOL),
            "a non-coach's card offers no coach-only skill: {for_sam:?}"
        );

        let for_casey = skills(&app, &grant(&res, &casey)).await;
        assert!(
            for_casey.contains(COACH_ONLY_TOOL),
            "a coach's card offers the coach-only skill: {for_casey:?}"
        );
    }

    /// A per-user list must not be advertised as shareable: the 2026-07-28
    /// revision makes `ttlMs` and `cacheScope` required on it, and `public`
    /// would let an intermediary serve a coach's list to an athlete.
    #[tokio::test]
    async fn the_per_user_list_is_uncached_and_private_under_the_modern_revision() {
        common::init_server_config();
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey4@coach-listing.test", "Casey Coach").await;
        make_coach_of_a_group(&res, &casey, "Squad").await;
        let server = build_mcp_server(res.clone());
        let mut ctx = ToolContext::new()
            .with_user(casey.id.to_string())
            .with_tenant(casey.tenant.to_string());
        ctx.scopes = OAuthScope::self_grant()
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect();

        let request: JsonRpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": { "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": { "name": "c", "version": "1" },
                "io.modelcontextprotocol/clientCapabilities": {}
            } }
        }))
        .unwrap();
        let response = server
            .handle_request_with_context(request, &ctx)
            .await
            .expect("answered");
        let result = serde_json::to_value(response).unwrap()["result"].clone();
        assert!(
            with_athlete(result["tools"].as_array().unwrap()).contains(SCOPED_READ),
            "the coach's list is the one under test: {result}"
        );
        assert_eq!(
            result["ttlMs"], 0,
            "a per-user list is never served from a cache: {}",
            result["ttlMs"]
        );
        assert_eq!(
            result["cacheScope"], "private",
            "a per-user list is never shared between users: {}",
            result["cacheScope"]
        );
    }
}
