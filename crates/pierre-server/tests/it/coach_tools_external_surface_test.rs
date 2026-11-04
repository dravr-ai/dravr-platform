// ABOUTME: The coach family (roster, group-member reads, athlete link) is listed and run on Dravr's own surfaces only
// ABOUTME: /mcp tools/list, the A2A card and an API-key chat catalogue leave it out, an external call is refused, the web app runs it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Coach tools stay inside Dravr
//!
//! The coach tools read other athletes' training for the coach of their group,
//! under consents those athletes gave inside Dravr. A coach who connects an
//! MCP client or an A2A agent is outside that agreement, so neither is offered
//! these tools, and a call that names one anyway is refused before anything
//! about the athletes is read. The web app's chat keeps them.
//!
//! Each case drives the real entry point: `POST /mcp` with the coach's
//! delegated grant, the A2A extended card, the executor A2A dispatches
//! through, the chat catalogue and Copilot loopback by the turn's transport,
//! and the web app's catalogue and executor.

#[cfg(feature = "tools-groups")]
mod coach_tools_external_surface_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, join, person, seed_ride, Person,
    };

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use embacle_tool_host::ToolSurface;
    use pierre_chat_pipeline::ToolSessionTurn;
    use pierre_core::models::ConversationTurnId;
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_providers::ai_scope;
    use pierre_tool_runtime::chat_catalogue::{offered_schemas, SwitchedOff};
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use pierre_tool_runtime::first_party_tools::FIRST_PARTY_SURFACE_ERROR_CODE;
    use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// The coach family, by name, as design §7 lists it.
    const COACH_TOOLS: [&str; 5] = [
        "get_group_member_activities",
        "get_group_member_plan_status",
        "get_provider_roster",
        "get_roster_overview",
        "propose_athlete_link",
    ];

    /// A coach of one group whose athlete shares with them and has a ride.
    async fn coach_with_an_athlete(res: &Arc<ServerContext>) -> Person {
        let casey = coach(res, "casey@external-surface.test", "Casey Coach").await;
        let alex = person(res, "alex@external-surface.test", "Alex Athlete").await;
        let agent = group_agent(res, &casey).await;
        let group = coached_group(res, casey.tenant, casey.id, &agent, "Squad").await;
        join(res, group, alex.id, alex.tenant, true).await;
        seed_ride(res, &alex, 2).await;
        casey
    }

    /// The grant an MCP client or an A2A agent holds for `coach`.
    fn delegated(res: &Arc<ServerContext>, coach: &Person) -> String {
        format!(
            "Bearer {}",
            common::delegated_test_token_for_tenant(res, coach.id, Some(coach.tenant.to_string()))
        )
    }

    async fn post_json(app: &Router, uri: &str, authorization: &str, body: Value) -> Value {
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

    fn rpc(method: &str, params: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    fn names(items: &Value) -> BTreeSet<String> {
        items
            .as_array()
            .unwrap_or_else(|| panic!("a list: {items}"))
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_owned())
            .collect()
    }

    fn coach_tools() -> BTreeSet<String> {
        COACH_TOOLS.iter().map(|name| (*name).to_owned()).collect()
    }

    async fn call_over(
        res: &Arc<ServerContext>,
        coach: &Person,
        tool_name: &str,
        transport: Transport,
    ) -> UniversalResponse {
        let runtime: Arc<dyn ToolRuntime> = res.clone();
        UniversalExecutor::new(runtime)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(transport)
            .execute_tool(UniversalRequest {
                tool_name: tool_name.to_owned(),
                parameters: json!({}),
                user_id: coach.id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(coach.tenant.to_string()),
            })
            .await
            .expect("the executor answers in band")
    }

    fn assert_refused_as_first_party(response: &UniversalResponse, tool_name: &str) {
        assert!(!response.success, "{tool_name}: {response:?}");
        let result = response.result.as_ref().expect("the refusal's code");
        assert_eq!(result["error_code"], FIRST_PARTY_SURFACE_ERROR_CODE);
        assert_eq!(result["reason"], "external_transport");
        assert!(response
            .error
            .as_deref()
            .unwrap()
            .contains("not available over this interface"));
    }

    #[tokio::test]
    async fn the_registry_keeps_exactly_the_coach_family_on_dravrs_own_surfaces() {
        let res = create_test_server_resources().await.unwrap();
        let registry = &res.mcp.tool_registry;
        let first_party_only: BTreeSet<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| registry.transport_policy(name) == Some(TransportPolicy::FirstPartyOnly))
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(first_party_only, coach_tools());
    }

    #[tokio::test]
    async fn mcp_lists_no_coach_tool_and_refuses_a_call_to_one() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach_with_an_athlete(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &casey);

        let listed = post_json(&app, "/mcp", &grant, rpc("tools/list", &json!({}))).await;
        let listed = names(&listed["result"]["tools"]);
        assert!(
            listed.len() > 50,
            "the rest of the catalogue is listed: {listed:?}"
        );
        assert!(
            listed.is_disjoint(&coach_tools()),
            "{:?}",
            listed.intersection(&coach_tools()).collect::<Vec<_>>()
        );

        let called = post_json(
            &app,
            "/mcp",
            &grant,
            rpc(
                "tools/call",
                &json!({ "name": "get_roster_overview", "arguments": {} }),
            ),
        )
        .await;
        let result = &called["result"];
        assert_eq!(result["isError"], true, "{called}");
        let text = result["content"].to_string();
        assert!(
            text.contains("not available over this interface"),
            "{called}"
        );
        assert!(text.contains(FIRST_PARTY_SURFACE_ERROR_CODE), "{called}");
        assert!(
            !text.contains("Alex Athlete"),
            "nothing about the athletes is read before the refusal: {called}"
        );
    }

    #[tokio::test]
    async fn the_a2a_card_offers_no_coach_skill_and_an_a2a_call_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach_with_an_athlete(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);

        let card = post_json(
            &app,
            "/a2a/jsonrpc",
            &delegated(&res, &casey),
            rpc("GetExtendedAgentCard", &json!({})),
        )
        .await;
        assert!(card["error"].is_null(), "{card}");
        let skills: BTreeSet<String> = card["result"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(skills.contains("get_activities"), "{skills:?}");
        assert!(skills.is_disjoint(&coach_tools()), "{skills:?}");

        for tool_name in COACH_TOOLS {
            for transport in [Transport::A2a, Transport::McpHttp, Transport::ApiKey] {
                let refused = call_over(&res, &casey, tool_name, transport).await;
                assert_refused_as_first_party(&refused, tool_name);
            }
        }
    }

    #[tokio::test]
    async fn the_web_app_still_offers_and_runs_the_coach_tools() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach_with_an_athlete(&res).await;

        let offered = |transport| -> BTreeSet<String> {
            offered_schemas(
                &res.mcp.tool_registry,
                TurnSeat::Subject,
                &SwitchedOff::default(),
                transport,
            )
            .into_iter()
            .map(|schema| schema.name)
            .collect()
        };
        for transport in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
        ] {
            let in_app = offered(Some(transport));
            assert!(
                coach_tools().is_subset(&in_app),
                "{transport:?}: {:?}",
                coach_tools().difference(&in_app).collect::<Vec<_>>()
            );
        }
        // A chat turn served over an API key lists none of them, as its
        // calls to them are refused; an undeclared transport is external.
        for transport in [Some(Transport::ApiKey), None] {
            let external = offered(transport);
            assert!(
                external.is_disjoint(&coach_tools()),
                "{transport:?}: {:?}",
                external.intersection(&coach_tools()).collect::<Vec<_>>()
            );
            assert!(external.contains("get_activities"), "{transport:?}");
        }

        for transport in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
        ] {
            let roster = call_over(&res, &casey, "get_roster_overview", transport).await;
            assert!(roster.success, "{transport:?}: {roster:?}");
            assert!(
                roster.result.unwrap().to_string().contains("Alex Athlete"),
                "the coach reads their athlete over {transport:?}"
            );
        }
    }

    async fn listed(surface: &TurnToolSurface) -> BTreeSet<String> {
        surface
            .list_tools()
            .await
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    /// The Copilot loopback lists the coach tools by the transport the turn
    /// was opened under, read off its executor: the listing runs on embacle's
    /// host task, where the turn's own declaration is gone.
    #[tokio::test]
    async fn the_loopback_lists_the_coach_tools_only_for_an_in_app_turn() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach_with_an_athlete(&res).await;
        let tool_runtime: Arc<dyn ToolRuntime> = res.clone();
        let bridge = HostedToolBridge::new(true, res.mcp.tool_registry.clone(), tool_runtime);
        let user_id = casey.id.to_string();
        let open = |transport| {
            ai_scope::serve_over(transport, async {
                bridge.turn_surface(ToolSessionTurn {
                    user_id: &user_id,
                    tenant_id: casey.tenant,
                    conversation_id: "conv-loopback",
                    turn_id: ConversationTurnId(Uuid::new_v4()),
                    turn_agent_id: None,
                    budget: 64,
                    seat: TurnSeat::Subject,
                    switched_off: &SwitchedOff::default(),
                })
            })
        };

        let in_app = open(Transport::WebApp).await;
        let in_app_names = listed(&in_app).await;
        assert!(coach_tools().is_subset(&in_app_names), "{in_app_names:?}");

        let api_key = open(Transport::ApiKey).await;
        let api_key_names = listed(&api_key).await;
        assert!(
            api_key_names.is_disjoint(&coach_tools()),
            "{api_key_names:?}"
        );
        assert!(api_key_names.contains("get_activities"));
        let refused = api_key.call("get_roster_overview", &json!({})).await;
        assert!(refused.is_error, "{}", refused.text);
    }
}
