// ABOUTME: The coach tools reach the AI assistant a human coach connects: listed and run over MCP and A2A for a coach alone
// ABOUTME: An athlete is offered none; the coach's assistant sees only members who allowed it; the web app runs them as before

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Coach tools over MCP and A2A (carnet#1511)
//!
//! Three tool lists, and the role decides: an athlete's MCP client or A2A
//! agent is offered the athlete tools, a human coach's also the three coach
//! tools (the roster overview, a member's activities, a member's plan
//! status). The coach's assistant is shown a member only once that member
//! allowed it from Dravr's own app; the others are a count, and a withdrawal
//! takes effect on the next call. The two coaching-platform tools stay in
//! Dravr's own app for a coach. The web app's chat is unchanged.
//!
//! Each case drives the real entry point: `POST /mcp` with a delegated grant,
//! the A2A extended card, the executor A2A dispatches through, the chat
//! catalogue and Copilot loopback by the turn's transport, and the web app's
//! catalogue and executor.

#[cfg(feature = "tools-groups")]
mod coach_tools_external_surface_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, join, person, seed_ride, Person,
    };
    use crate::public_tool_surface_test::HIDDEN_TOOLS;

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use chrono::Utc;
    use embacle_tool_host::ToolSurface;
    use pierre_chat_pipeline::ToolSessionTurn;
    use pierre_core::models::ConversationTurnId;
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::Transport;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_providers::ai_scope;
    use pierre_tool_runtime::chat_catalogue::{offered_schemas, SwitchedOff};
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use pierre_tool_runtime::first_party_tools::FIRST_PARTY_SURFACE_ERROR_CODE;
    use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
    use pierre_tool_runtime::runtime::ToolRuntime;
    use pierre_tool_runtime::security::{Audience, ToolReach};
    use pierre_tool_runtime::tool_access::COACH_ROLE_REQUIRED;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// The three coach tools (carnet#1511).
    const COACH_TOOLS: [&str; 3] = [
        "get_group_member_activities",
        "get_group_member_plan_status",
        "get_roster_overview",
    ];

    /// The two coaching-platform tools, kept in Dravr's own app for a coach.
    const PLATFORM_TOOLS: [&str; 2] = ["get_provider_roster", "propose_athlete_link"];

    /// A coach of one group, its two members sharing with the coach, each
    /// with a ride: Alex has not allowed the coach's assistant, Bea has.
    struct Squad {
        casey: Person,
        alex: Person,
        group: Uuid,
    }

    async fn squad(res: &Arc<ServerContext>) -> Squad {
        let casey = coach(res, "casey@external-surface.test", "Casey Coach").await;
        let alex = person(res, "alex@external-surface.test", "Alex Athlete").await;
        let bea = person(res, "bea@external-surface.test", "Bea Bright").await;
        let agent = group_agent(res, &casey).await;
        let group = coached_group(res, casey.tenant, casey.id, &agent, "Squad").await;
        join(res, group, alex.id, alex.tenant, true).await;
        join(res, group, bea.id, bea.tenant, true).await;
        allow_assistant(res, group, &bea, true).await;
        seed_ride(res, &alex, 2).await;
        seed_ride(res, &bea, 3).await;
        Squad { casey, alex, group }
    }

    /// The member turns their coach's assistant on or off.
    async fn allow_assistant(res: &Arc<ServerContext>, group: Uuid, member: &Person, on: bool) {
        res.common
            .repos
            .groups
            .set_coach_assistant_read(&group.to_string(), member.id, on, Utc::now())
            .await
            .unwrap()
            .expect("a live member");
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

    fn platform_tools() -> BTreeSet<String> {
        PLATFORM_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    async fn mcp_call(app: &Router, grant: &str, name: &str, arguments: Value) -> Value {
        post_json(
            app,
            "/mcp",
            grant,
            rpc(
                "tools/call",
                &json!({ "name": name, "arguments": arguments }),
            ),
        )
        .await
    }

    async fn call_over(
        res: &Arc<ServerContext>,
        coach: &Person,
        tool_name: &str,
        transport: Transport,
    ) -> UniversalResponse {
        let runtime: Arc<dyn ToolRuntime> = res.clone();
        // Bound as the A2A and MCP entry points bind it.
        UniversalExecutor::new(runtime)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(transport)
            .with_caller_writing_reply()
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
    async fn the_registry_declares_three_coach_tools_and_keeps_the_rest_in_the_app() {
        let res = create_test_server_resources().await.unwrap();
        let registry = &res.mcp.tool_registry;
        let coach: BTreeSet<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| registry.get(name).unwrap().audience() == Audience::Coach)
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(coach, coach_tools());
        let first_party_only: BTreeSet<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| registry.reach(name) == Some(ToolReach::FirstPartyOnly))
            .map(ToOwned::to_owned)
            .collect();
        let mut declared = platform_tools();
        declared.extend(HIDDEN_TOOLS.map(str::to_owned));
        assert_eq!(first_party_only, declared);
        assert_eq!(
            registry.reach("publish_training_plan"),
            Some(ToolReach::AppOnly)
        );
    }

    #[tokio::test]
    async fn a_coachs_mcp_client_is_listed_the_coach_tools_and_sees_who_allowed_it() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &squad.casey);

        let listed = post_json(&app, "/mcp", &grant, rpc("tools/list", &json!({}))).await;
        let listed = names(&listed["result"]["tools"]);
        assert!(coach_tools().is_subset(&listed), "{listed:?}");
        assert!(listed.is_disjoint(&platform_tools()), "{listed:?}");
        assert!(!listed.contains("publish_training_plan"), "{listed:?}");

        let roster = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        let text = roster["result"]["content"].to_string();
        assert_eq!(roster["result"]["isError"], false, "{roster}");
        assert!(
            text.contains("Bea Bright"),
            "the member who allowed it: {roster}"
        );
        assert!(
            !text.contains("Alex Athlete"),
            "a member who has not allowed it is a count: {roster}"
        );
        assert!(
            text.contains("1 member has not allowed AI tools"),
            "{roster}"
        );

        let alex = mcp_call(
            &app,
            &grant,
            "get_group_member_activities",
            json!({ "member": "Alex" }),
        )
        .await;
        assert_eq!(alex["result"]["isError"], true, "{alex}");
        assert!(
            alex["result"]["content"].to_string().contains("no_match"),
            "{alex}"
        );
        let bea = mcp_call(
            &app,
            &grant,
            "get_group_member_activities",
            json!({ "member": "Bea" }),
        )
        .await;
        assert_eq!(bea["result"]["isError"], false, "{bea}");

        // Allowing it and withdrawing it take effect on the very next call.
        allow_assistant(&res, squad.group, &squad.alex, true).await;
        let roster = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        assert!(
            roster["result"]["content"]
                .to_string()
                .contains("Alex Athlete"),
            "{roster}"
        );
        allow_assistant(&res, squad.group, &squad.alex, false).await;
        let roster = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        assert!(
            !roster["result"]["content"]
                .to_string()
                .contains("Alex Athlete"),
            "{roster}"
        );
        let plan = mcp_call(
            &app,
            &grant,
            "get_group_member_plan_status",
            json!({ "member": "Alex" }),
        )
        .await;
        assert_eq!(plan["result"]["isError"], true, "{plan}");
        assert!(
            plan["result"]["content"].to_string().contains("no_match"),
            "{plan}"
        );
    }

    #[tokio::test]
    async fn an_athletes_assistant_is_offered_no_coach_tool_and_is_refused_one() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &squad.alex);

        let listed = post_json(&app, "/mcp", &grant, rpc("tools/list", &json!({}))).await;
        let listed = names(&listed["result"]["tools"]);
        assert!(
            listed.len() > 50,
            "the athlete tools are listed: {listed:?}"
        );
        assert!(listed.is_disjoint(&coach_tools()), "{listed:?}");

        let called = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        assert_eq!(called["result"]["isError"], true, "{called}");
        let text = called["result"]["content"].to_string();
        assert!(text.contains(COACH_ROLE_REQUIRED), "{called}");
        assert!(!text.contains("Bea Bright"), "{called}");
    }

    #[tokio::test]
    async fn the_a2a_card_offers_the_coach_skills_to_a_coach_alone() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let skills = |card: &Value| -> BTreeSet<String> {
            card["result"]["skills"]
                .as_array()
                .unwrap()
                .iter()
                .map(|skill| skill["id"].as_str().unwrap().to_owned())
                .collect()
        };

        for (who, coaches) in [(&squad.casey, true), (&squad.alex, false)] {
            let card = post_json(
                &app,
                "/a2a/jsonrpc",
                &delegated(&res, who),
                rpc("GetExtendedAgentCard", &json!({})),
            )
            .await;
            assert!(card["error"].is_null(), "{card}");
            let offered = skills(&card);
            assert!(offered.contains("get_activities"), "{offered:?}");
            assert_eq!(coach_tools().is_subset(&offered), coaches, "{offered:?}");
            assert!(offered.is_disjoint(&platform_tools()), "{offered:?}");
        }

        // The A2A executor answers by the same predicate: a coach's call
        // runs, a platform tool stays in the app.
        let roster = call_over(&res, &squad.casey, "get_roster_overview", Transport::A2a).await;
        assert!(roster.success, "{roster:?}");
        for tool_name in PLATFORM_TOOLS {
            let refused = call_over(&res, &squad.casey, tool_name, Transport::A2a).await;
            assert_refused_as_first_party(&refused, tool_name);
        }
        let refused = call_over(&res, &squad.alex, "get_roster_overview", Transport::A2a).await;
        assert!(!refused.success, "{refused:?}");
        assert_eq!(refused.result.unwrap()["error_code"], COACH_ROLE_REQUIRED);
    }

    #[tokio::test]
    async fn the_web_app_still_offers_and_runs_the_coach_tools() {
        let res = create_test_server_resources().await.unwrap();
        let casey = squad(&res).await.casey;

        let offered = |transport| -> BTreeSet<String> {
            offered_schemas(
                &res.mcp.tool_registry,
                TurnSeat::Subject,
                &SwitchedOff::default(),
                transport,
                &OAuthScope::self_grant(),
                &res.common.config,
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
        // A chat turn served over an API key is decided for no role, so it
        // lists none of them and its calls to them are refused; an
        // undeclared transport is external. The coach list is the MCP and
        // A2A listings'.
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
                "in the app the coach reads every athlete who shares, allowed or not, over {transport:?}"
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
        let casey = squad(&res).await.casey;
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
                    grant: &OAuthScope::self_grant(),
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
