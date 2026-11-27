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
        coach, coached_group, group_agent, join, person, ride, seed_activities, seed_ride, Person,
    };
    use crate::public_tool_surface_test::HIDDEN_TOOLS;

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use embacle_tool_host::ToolSurface;
    use pierre_chat_pipeline::ToolSessionTurn;
    use pierre_core::ai_policy::ProviderTerms;
    use pierre_core::constants::oauth::providers::{INTERVALS_ICU, STRAVA};
    use pierre_core::models::{
        Activity, ActivityBuilder, CarriedRead, ConversationTurnId, SportType,
    };
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::Transport;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_providers::ai_scope;
    use pierre_services::coach_roster::{
        gather_roster, order_roster, roster_access, OrderedMember, RosterAccess,
    };
    use pierre_tool_runtime::chat_catalogue::{
        declarations, offered_schemas, OfferedTo, SwitchedOff, TurnReaders,
    };
    use pierre_tool_runtime::coach_seat::{
        after_member_read_refusal, offer_declaration_after_member_read, MemberReads, TurnSeat,
    };
    use pierre_tool_runtime::context::TURN_DIRECT_MESSAGE;
    use pierre_tool_runtime::first_party_tools::{
        EXTERNAL_WRITE_ERROR_CODE, FIRST_PARTY_SURFACE_ERROR_CODE,
    };
    use pierre_tool_runtime::implementations::group_roster::KEPT_IN_DRAVR;
    use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
    use pierre_tool_runtime::runtime::ToolRuntime;
    use pierre_tool_runtime::security::{Audience, ToolReach};
    use pierre_tool_runtime::tool_access::CallerRole;
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

    /// What a coach tool refused for the role reads
    /// (`tool_access::coach_role_refusal`).
    const COACH_ROLE_REFUSED: &str = "is for a human coach who leads an active group";

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
            .set_coach_assistant_read(&group.to_string(), member.id, on, Timestamp::now())
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
        assert_eq!(
            coach,
            coach_tools(),
            "bug: the registry declares exactly the three coach tools"
        );
        let first_party_only: BTreeSet<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| registry.get(name).unwrap().reach() == ToolReach::FirstPartyOnly)
            .map(ToOwned::to_owned)
            .collect();
        let mut declared = platform_tools();
        declared.extend(HIDDEN_TOOLS.map(str::to_owned));
        assert_eq!(
            first_party_only, declared,
            "bug: the first-party tools are the platform and hidden ones"
        );
        assert_eq!(
            registry
                .get("publish_training_plan")
                .map(|tool| tool.reach()),
            Some(ToolReach::AppOnly),
            "bug: publishing a plan stays in Dravr's own app"
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
        assert_eq!(
            refused.result.unwrap()["error_code"],
            COACH_ROLE_REQUIRED,
            "bug: an athlete is refused the roster as not a coach"
        );
    }

    #[tokio::test]
    async fn the_web_app_still_offers_and_runs_the_coach_tools() {
        let res = create_test_server_resources().await.unwrap();
        let casey = squad(&res).await.casey;

        let coach_role = CallerRole::resolve(&res.common.repos, casey.id).await;
        assert!(coach_role.coach, "{coach_role:?}");
        let offered = |transport, role| -> BTreeSet<String> {
            offered_schemas(
                &res.mcp.tool_registry,
                TurnSeat::Subject,
                &SwitchedOff::default(),
                OfferedTo {
                    transport,
                    grant: &OAuthScope::self_grant(),
                    role,
                    readers: TurnReaders::CallerAlone,
                    member_reads: MemberReads::default(),
                },
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
            // The role the app's turn service resolves for the coach.
            let role = CallerRole::resolve(&res.common.repos, casey.id).await;
            let in_app = offered(Some(transport), role);
            assert!(
                coach_tools().is_subset(&in_app),
                "{transport:?}: {:?}",
                coach_tools().difference(&in_app).collect::<Vec<_>>()
            );
            // A caller who coaches no one is offered in the app only the
            // coach tool that also serves a group's peers, whose own gates
            // answer them: never one refused for their role (carnet#1511 I7).
            let no_role = offered(Some(transport), CallerRole::default());
            let coach_tools_offered: BTreeSet<String> =
                coach_tools().intersection(&no_role).cloned().collect();
            assert_eq!(
                coach_tools_offered,
                BTreeSet::from(["get_group_member_activities".to_owned()]),
                "{transport:?}"
            );
        }
        // A chat turn served outside the app (an API key's, the A2A text
        // turn's; an undeclared transport is external) is decided for the
        // caller's role through the one access check: the coach's lists
        // them, a caller with no role lists none (carnet#1511, plan site 5).
        for transport in [Some(Transport::ApiKey), Some(Transport::A2a), None] {
            let for_coach = offered(transport, coach_role);
            assert!(
                coach_tools().is_subset(&for_coach),
                "{transport:?}: {:?}",
                coach_tools().difference(&for_coach).collect::<Vec<_>>()
            );
            let no_role = offered(transport, CallerRole::default());
            assert!(
                no_role.is_disjoint(&coach_tools()),
                "{transport:?}: {:?}",
                no_role.intersection(&coach_tools()).collect::<Vec<_>>()
            );
            assert!(no_role.contains("get_activities"), "{transport:?}");
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
    /// was opened under and the role it was decided for, both read off its
    /// executor: the listing runs on embacle's host task, where the turn's
    /// own declaration is gone. Its calls read the same role, so an outside
    /// turn's roster names only the member who allowed the coach's
    /// assistant (carnet#1511, plan site 5).
    #[tokio::test]
    async fn the_loopback_offers_the_coach_tools_by_the_turns_role() {
        let res = create_test_server_resources().await.unwrap();
        let casey = squad(&res).await.casey;
        let coach_role = CallerRole::resolve(&res.common.repos, casey.id).await;
        let tool_runtime: Arc<dyn ToolRuntime> = res.clone();
        let bridge = HostedToolBridge::new(true, res.mcp.tool_registry.clone(), tool_runtime);
        let user_id = casey.id.to_string();
        // A direct-message turn in the coach's own conversation: the room
        // every coach tool's gate takes a call in.
        let conversation = common::own_conversation(&res, casey.id, casey.tenant).await;
        let (bridge, user_id, conversation) = (&bridge, &user_id, &conversation);
        let open = move |transport, role| {
            TURN_DIRECT_MESSAGE.scope(
                Some(true),
                ai_scope::serve_over(transport, async move {
                    bridge.turn_surface(ToolSessionTurn {
                        user_id,
                        tenant_id: casey.tenant,
                        conversation_tenant_id: casey.tenant,
                        conversation_id: conversation,
                        turn_id: ConversationTurnId(Uuid::new_v4()),
                        turn_agent_id: None,
                        budget: 64,
                        seat: TurnSeat::Subject,
                        switched_off: &SwitchedOff::default(),
                        grant: &OAuthScope::self_grant(),
                        role,
                    })
                }),
            )
        };

        let app_role = CallerRole::resolve(&res.common.repos, casey.id).await;
        let in_app = open(Transport::WebApp, app_role).await;
        let in_app_names = listed(&in_app).await;
        assert!(coach_tools().is_subset(&in_app_names), "{in_app_names:?}");

        for transport in [Transport::ApiKey, Transport::A2a] {
            let coach_turn = open(transport, coach_role).await;
            let names = listed(&coach_turn).await;
            assert!(coach_tools().is_subset(&names), "{transport:?}: {names:?}");
            // The call passes the role check and reaches the tool, whose own
            // gates answer; what the roster then shows an outside turn is
            // `coach_tools_chat_surfaces_test`'s, through a whole turn.
            let roster = coach_turn.call("get_roster_overview", &json!({})).await;
            assert!(
                !roster.text.contains(COACH_ROLE_REFUSED),
                "{transport:?}: {}",
                roster.text
            );

            let no_role = open(transport, CallerRole::default()).await;
            let names = listed(&no_role).await;
            assert!(
                names.is_disjoint(&coach_tools()),
                "{transport:?}: {names:?}"
            );
            assert!(
                names.contains("get_activities"),
                "{transport:?}: a caller with no role keeps the athlete tools: {names:?}"
            );
            // Not offered, so refused before the role check is reached
            // (carnet#1511 design call 13).
            let refused = no_role.call("get_roster_overview", &json!({})).await;
            assert!(refused.is_error, "{transport:?}: {}", refused.text);
            assert!(
                refused
                    .text
                    .contains("Tool 'get_roster_overview' not found. Available tools"),
                "{transport:?}: {}",
                refused.text
            );
        }
    }

    /// The tools `surface` lists with the `athlete` argument.
    async fn listed_naming(surface: &TurnToolSurface) -> BTreeSet<String> {
        surface
            .list_tools()
            .await
            .into_iter()
            .filter(|tool| tool.input_schema["properties"].get("athlete").is_some())
            .map(|tool| tool.name)
            .collect()
    }

    /// The loopback offers a coach's outside turn the `athlete` argument on
    /// a read only where dispatch takes a name: in a conversation confirmed
    /// private to the coach. Not where the turn says others may read it, and
    /// not in a conversation that does not resolve for the coach, where the
    /// room rule refuses every name (carnet#1422).
    #[tokio::test]
    async fn the_loopback_offers_a_name_only_where_dispatch_takes_one() {
        let res = create_test_server_resources().await.unwrap();
        let casey = squad(&res).await.casey;
        let coach_role = CallerRole::resolve(&res.common.repos, casey.id).await;
        let tool_runtime: Arc<dyn ToolRuntime> = res.clone();
        let bridge = HostedToolBridge::new(true, res.mcp.tool_registry.clone(), tool_runtime);
        let user_id = casey.id.to_string();
        let own = res
            .common
            .repos
            .chat
            .create_conversation(
                &user_id,
                casey.tenant,
                "Coach script",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let (bridge, user_id) = (&bridge, &user_id);
        let open = move |conversation_id: String, direct_message: bool| async move {
            TURN_DIRECT_MESSAGE
                .scope(
                    Some(direct_message),
                    ai_scope::serve_over(Transport::ApiKey, async {
                        bridge.turn_surface(ToolSessionTurn {
                            user_id,
                            tenant_id: casey.tenant,
                            conversation_tenant_id: casey.tenant,
                            conversation_id: &conversation_id,
                            turn_id: ConversationTurnId(Uuid::new_v4()),
                            turn_agent_id: None,
                            budget: 64,
                            seat: TurnSeat::Subject,
                            switched_off: &SwitchedOff::default(),
                            grant: &OAuthScope::self_grant(),
                            role: coach_role,
                        })
                    }),
                )
                .await
        };

        let private = listed_naming(&open(own.clone(), true).await).await;
        assert!(
            private.contains("get_activities"),
            "a coach's own direct conversation offers a name on a read: {private:?}"
        );
        for (conversation_id, direct_message, room) in [
            (own, false, "a conversation not marked a direct message"),
            (
                Uuid::new_v4().to_string(),
                true,
                "a conversation that does not resolve for the coach",
            ),
        ] {
            let naming = listed_naming(&open(conversation_id, direct_message).await).await;
            assert!(
                !naming.contains("get_activities"),
                "{room}: a read is not offered a name dispatch would refuse: {naming:?}"
            );
        }
    }

    /// A ride from Intervals.icu `days_ago`, whose terms let it reach an
    /// assistant outside Dravr.
    fn intervals_ride(id: &str, days_ago: i64) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Timestamp::now() - Elapsed::days(days_ago),
            3_600,
            INTERVALS_ICU.to_owned(),
        )
        .distance_meters(30_000.0)
        .build()
    }

    /// A coach of one group with three opted-in members: Kim syncs only
    /// from Strava, whose data stays in Dravr; Lee trained on Intervals.icu
    /// five days ago; Mo trained on Intervals.icu twenty days ago and on
    /// Strava yesterday.
    struct Provenance {
        casey: Person,
        kim: Person,
        lee: Person,
        mo: Person,
    }

    async fn provenance(res: &Arc<ServerContext>) -> Provenance {
        let casey = coach(res, "casey@provenance.test", "Casey Coach").await;
        let kim = person(res, "kim@provenance.test", "Kim Kept").await;
        let lee = person(res, "lee@provenance.test", "Lee Loop").await;
        let mo = person(res, "mo@provenance.test", "Mo Mixed").await;
        let agent = group_agent(res, &casey).await;
        let group = coached_group(res, casey.tenant, casey.id, &agent, "Provenance").await;
        for member in [&kim, &lee, &mo] {
            join(res, group, member.id, member.tenant, true).await;
            allow_assistant(res, group, member, true).await;
        }
        seed_ride(res, &kim, 1).await;
        seed_activities(res, &lee, INTERVALS_ICU, &[intervals_ride("lee-1", 5)]).await;
        seed_activities(res, &mo, INTERVALS_ICU, &[intervals_ride("mo-1", 20)]).await;
        seed_activities(res, &mo, STRAVA, &[ride("mo-2", 1)]).await;
        Provenance {
            casey,
            kim,
            lee,
            mo,
        }
    }

    /// A member whose only sources keep their data in Dravr reads to the
    /// coach's assistant as present with their data withheld, after the
    /// members it may read, and never as the quietest athlete (carnet#1511).
    #[tokio::test]
    async fn a_member_whose_data_stays_in_dravr_is_present_not_gone_quiet() {
        let res = create_test_server_resources().await.unwrap();
        let squad = provenance(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &squad.casey);

        let roster = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        assert_eq!(roster["result"]["isError"], false, "{roster}");
        let overview = &roster["result"]["structuredContent"];
        let athletes = overview["athletes"]
            .as_array()
            .unwrap_or_else(|| panic!("the athletes: {roster}"));
        let names: Vec<&str> = athletes
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names.last(),
            Some(&"Kim Kept"),
            "listed after the members whose training is read: {names:?}"
        );
        let kim = athletes.last().unwrap();
        assert_eq!(kim["status"], "shared", "{kim}");
        assert!(kim.get("training").is_none(), "nothing of theirs: {kim}");
        assert_eq!(kim["not_read"], KEPT_IN_DRAVR, "{kim}");
        let text = roster["result"]["content"].to_string();
        assert!(!text.contains("ride-1"), "no session of theirs: {roster}");
        for readable in ["Lee Loop", "Mo Mixed"] {
            let athlete = athletes
                .iter()
                .find(|a| a["name"] == readable)
                .unwrap_or_else(|| panic!("{readable}: {roster}"));
            assert!(athlete.get("training").is_some(), "{athlete}");
        }

        // In Dravr's own app the coach reads Kim's ride as before.
        let in_app = call_over(&res, &squad.casey, "get_roster_overview", Transport::WebApp).await;
        let in_app = in_app.result.unwrap();
        let kim_in_app = in_app["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "Kim Kept")
            .unwrap_or_else(|| panic!("{in_app}"))
            .clone();
        assert!(kim_in_app.get("training").is_some(), "{kim_in_app}");
        assert!(kim_in_app.get("not_read").is_none(), "{kim_in_app}");
    }

    /// The roster's order, which decides the cut at 40, reads only the
    /// sessions an outside client may be shown: a Strava ride yesterday
    /// never places Mo behind a member who trained more recently on a
    /// service the client may see (carnet#1511).
    #[tokio::test]
    async fn the_order_an_outside_client_reads_ignores_sessions_kept_in_dravr() {
        let res = create_test_server_resources().await.unwrap();
        let squad = provenance(&res).await;
        let repos = &res.common.repos;
        let RosterAccess::Granted(groups) = roster_access(repos, squad.casey.id).await.unwrap()
        else {
            panic!("Casey coaches the group");
        };
        let gathered = gather_roster(repos, squad.casey.id, &groups).await.unwrap();
        let place = |ordered: &[OrderedMember], who: Uuid| {
            ordered
                .iter()
                .position(|m| m.member.user_id == who)
                .unwrap()
        };

        let in_app = order_roster(repos, gathered.members.clone(), None)
            .await
            .unwrap();
        assert!(
            place(&in_app, squad.lee.id) < place(&in_app, squad.mo.id),
            "in the app Mo's Strava ride yesterday makes Lee the quieter"
        );
        assert!(
            in_app.iter().all(|m| !m.kept_in_dravr),
            "bug: in the app no member reads as kept in Dravr"
        );

        let registry = res.provider_registry();
        let terms: &dyn ProviderTerms = registry.as_ref();
        let outside = order_roster(repos, gathered.members, Some(terms))
            .await
            .unwrap();
        assert!(
            place(&outside, squad.mo.id) < place(&outside, squad.lee.id),
            "outside, Mo is placed by the Intervals.icu ride twenty days ago"
        );
        let mo = &outside[place(&outside, squad.mo.id)];
        assert_eq!(
            mo.last_cached_activity
                .map(|at| (Timestamp::now() - at).num_days()),
            Some(20),
            "never the day of a session kept in Dravr"
        );
        let kim = &outside[place(&outside, squad.kim.id)];
        assert!(kim.kept_in_dravr, "Kim syncs only from Strava");
        assert_eq!(
            kim.last_cached_activity, None,
            "bug: outside, Kim's Strava-only sessions give no last day"
        );
        assert_eq!(
            place(&outside, squad.kim.id),
            outside.len() - 1,
            "bug: outside, Kim, with nothing readable, is placed last"
        );
    }

    /// `tool_name` with `arguments` for `coach` through the executor bound
    /// as the A2A and MCP entry points bind it, over `transport` under
    /// `grant`.
    async fn call_with(
        res: &Arc<ServerContext>,
        coach: &Person,
        (tool_name, arguments): (&str, Value),
        transport: Transport,
        grant: Vec<OAuthScope>,
    ) -> UniversalResponse {
        let runtime: Arc<dyn ToolRuntime> = res.clone();
        UniversalExecutor::new(runtime)
            .with_scopes(grant)
            .with_transport(transport)
            .with_caller_writing_reply()
            .execute_tool(UniversalRequest {
                tool_name: tool_name.to_owned(),
                parameters: arguments,
                user_id: coach.id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(coach.tenant.to_string()),
            })
            .await
            .expect("the executor answers in band")
    }

    /// carnet#1533 at the dispatch chokepoint: a write naming an athlete
    /// over an external transport is refused for every role, before its
    /// body runs, as the listing offers no write the argument. A coach's
    /// plan write over MCP, A2A and an API key; an admin's athlete link with
    /// the admin grant, which needs the athlete it names, is not over this
    /// interface at all.
    #[tokio::test]
    async fn an_external_write_naming_an_athlete_is_refused_for_every_role() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let plan_for_alex = || {
            (
                "save_training_plan",
                json!({
                    "athlete": "Alex",
                    "outline": {
                        "goal_race": {
                            "name": "Spring Trail",
                            "date": "2027-05-16",
                            "discipline": "trail",
                            "priority": "A"
                        },
                        "strategy": "build toward the spring trail"
                    }
                }),
            )
        };
        let assert_refused = |response: &UniversalResponse, case: &str| {
            assert!(!response.success, "{case}: {response:?}");
            let result = response.result.as_ref().expect("the refusal's code");
            assert_eq!(
                result["error_code"], EXTERNAL_WRITE_ERROR_CODE,
                "{case}: {result}"
            );
            assert_eq!(result["reason"], "external_transport", "{case}: {result}");
        };
        res.common
            .repos
            .users
            .set_admin_status(squad.casey.id, true)
            .await
            .unwrap();
        let mut admin_grant = OAuthScope::self_grant();
        admin_grant.push(OAuthScope::Admin);
        for transport in [Transport::A2a, Transport::ApiKey, Transport::McpHttp] {
            let refused = call_with(
                &res,
                &squad.casey,
                ("propose_athlete_link", json!({ "athlete": "Alex" })),
                transport,
                admin_grant.clone(),
            )
            .await;
            // The link cannot be called without the athlete it names, which
            // no one is offered over this interface: the access gate refuses
            // it before the write rule is asked (carnet#1511).
            assert!(!refused.success, "{refused:?}");
            let result = refused.result.as_ref().expect("the refusal's code");
            assert_eq!(
                result["error_code"], FIRST_PARTY_SURFACE_ERROR_CODE,
                "the admin's athlete link over {transport:?}: {result}"
            );
            assert_eq!(result["reason"], "external_transport", "{result}");
            let unnamed = call_with(
                &res,
                &squad.casey,
                ("propose_athlete_link", json!({ "athlete": null })),
                transport,
                admin_grant.clone(),
            )
            .await;
            assert_ne!(
                unnamed.result.as_ref().map(|r| r["error_code"].clone()),
                Some(json!(EXTERNAL_WRITE_ERROR_CODE)),
                "a null athlete names no one: {unnamed:?}"
            );
        }
        for transport in [Transport::A2a, Transport::ApiKey, Transport::McpHttp] {
            let refused = call_with(
                &res,
                &squad.casey,
                plan_for_alex(),
                transport,
                OAuthScope::self_grant(),
            )
            .await;
            assert_refused(
                &refused,
                &format!("the coach's plan write over {transport:?}"),
            );
        }

        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &squad.casey);
        let (name, arguments) = plan_for_alex();
        let over_mcp = mcp_call(&app, &grant, name, arguments).await;
        assert_eq!(over_mcp["result"]["isError"], true, "{over_mcp}");
        assert!(
            over_mcp["result"]["content"]
                .to_string()
                .contains(EXTERNAL_WRITE_ERROR_CODE),
            "the plan write over /mcp is refused at the chokepoint: {over_mcp}"
        );

        // The write still runs from Dravr's own app.
        let in_app = call_with(
            &res,
            &squad.casey,
            ("propose_athlete_link", json!({ "athlete": "Alex" })),
            Transport::WebApp,
            admin_grant,
        )
        .await;
        let code = in_app.result.as_ref().map(|r| r["error_code"].clone());
        assert!(
            code != Some(json!(EXTERNAL_WRITE_ERROR_CODE))
                && code != Some(json!(COACH_ROLE_REQUIRED))
                && code != Some(json!(FIRST_PARTY_SURFACE_ERROR_CODE)),
            "in the app the coach's name is the tool's own: {in_app:?}"
        );
    }

    /// The writes a coach in the app names an athlete on, which write the
    /// caller's own account when they name no one.
    const NAMING_WRITES: [&str; 2] = ["prescribe_workout", "save_training_plan"];

    /// Round-12 listing-dispatch (carnet#1511 I7, calls 12 and 16): once
    /// another member's data entered a turn, a write that names no athlete
    /// lands on the caller's own account and is refused. "Names an athlete"
    /// is decided as the bodies read the argument, a non-blank string: a
    /// blank, non-string or absent `athlete` names no one. The listing
    /// offers what dispatch accepts: such a write with `athlete` required
    /// where the caller is offered the argument, else not at all. And a
    /// turn that read the member mid-turn is narrowed to the set it would
    /// have been built with under that read.
    #[tokio::test]
    async fn after_a_member_read_a_write_naming_no_one_is_neither_offered_nor_accepted() {
        let res = create_test_server_resources().await.unwrap();
        let registry = &res.mcp.tool_registry;
        // One other member drawn on: a write naming them is still taken.
        let reads = MemberReads {
            beyond_room: true,
            any: true,
            several: false,
        };
        for tool in NAMING_WRITES {
            for athlete in [
                json!(""),
                json!("   "),
                json!(false),
                json!(0),
                json!({}),
                Value::Null,
            ] {
                assert!(
                    after_member_read_refusal(
                        registry,
                        tool,
                        &json!({ "athlete": athlete }),
                        reads
                    )
                    .is_some(),
                    "bug: {tool} with athlete={athlete} writes the caller's own account after a \
                     member read"
                );
            }
            assert!(
                after_member_read_refusal(registry, tool, &json!({}), reads).is_some(),
                "bug: {tool} naming no athlete writes the caller's own account after a member read"
            );
            assert!(
                after_member_read_refusal(registry, tool, &json!({ "athlete": "Bea" }), reads)
                    .is_none(),
                "{tool} naming an athlete acts on them"
            );
        }

        let squad = squad(&res).await;
        let casey = squad.casey;
        let role = CallerRole::resolve(&res.common.repos, casey.id).await;
        let offered = |member_reads| {
            offered_schemas(
                registry,
                TurnSeat::Subject,
                &SwitchedOff::default(),
                OfferedTo {
                    transport: Some(Transport::WebApp),
                    grant: &OAuthScope::self_grant(),
                    role,
                    readers: TurnReaders::CallerAlone,
                    member_reads,
                },
                &res.common.config,
            )
        };
        let after_read = offered(reads);
        for tool in NAMING_WRITES {
            let schema = after_read
                .iter()
                .find(|schema| schema.name == tool)
                .unwrap_or_else(|| {
                    panic!("a coach in the app is offered {tool} naming an athlete")
                });
            assert!(
                schema
                    .input_schema
                    .required
                    .as_ref()
                    .is_some_and(|required| required.iter().any(|name| name == "athlete")),
                "bug: {tool} is offered with `athlete` optional though dispatch refuses it left out"
            );
        }
        let plain = offered(MemberReads::default());
        assert!(
            plain.iter().all(|schema| !schema
                .input_schema
                .required
                .as_ref()
                .is_some_and(|required| required.iter().any(|name| name == "athlete")
                    && NAMING_WRITES.contains(&schema.name.as_str()))),
            "before any member read `athlete` stays optional"
        );

        // A member read made mid-turn narrows the set the turn started with
        // to exactly the set built under it.
        let mut narrowed = declarations(plain);
        narrowed.function_declarations.retain_mut(|declaration| {
            offer_declaration_after_member_read(registry, declaration, reads)
        });
        assert_eq!(
            serde_json::to_value(&narrowed).unwrap(),
            serde_json::to_value(declarations(after_read)).unwrap(),
            "bug: a turn narrowed mid-turn offers other than one built after the read"
        );

        // carnet#1511 call 24: once the topic drew on several other members,
        // any athlete a free-text write names leaves another one drawn on,
        // so no such write is offered, and a mid-turn read narrows to the
        // same set.
        let several = MemberReads::every_read();
        let after_several = offered(several);
        for tool in NAMING_WRITES {
            assert!(
                !after_several.iter().any(|schema| schema.name == tool),
                "bug: {tool} is offered after the topic read several members, though dispatch \
                 takes no athlete for it"
            );
        }
        let mut narrowed = declarations(offered(MemberReads::default()));
        narrowed.function_declarations.retain_mut(|declaration| {
            offer_declaration_after_member_read(registry, declaration, several)
        });
        assert_eq!(
            serde_json::to_value(&narrowed).unwrap(),
            serde_json::to_value(declarations(after_several)).unwrap(),
            "bug: a turn narrowed by a live read offers other than one built after it"
        );
    }

    /// carnet#1511 call 25: a tier that lists its tools once (the loopback
    /// for the Copilot runtime and the Claude Code CLI) lists them from the
    /// reads the topic carries when the agent lists. A plain group member
    /// who read no one is offered their own-account writes from the first
    /// turn, as on main; a read made mid-turn refuses such a write at
    /// dispatch, and the refusal says why rather than "not found".
    #[tokio::test]
    async fn the_loopback_lists_a_members_own_writes_until_a_read_and_then_says_why() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let alex = squad.alex;
        let role = CallerRole::resolve(&res.common.repos, alex.id).await;
        let tool_runtime: Arc<dyn ToolRuntime> = res.clone();
        let bridge = HostedToolBridge::new(true, res.mcp.tool_registry.clone(), tool_runtime);
        let user_id = alex.id.to_string();
        let conversation = common::own_conversation(&res, alex.id, alex.tenant).await;
        let (listed_first, refused) = ai_scope::tracking(
            ai_scope::Provenance::new(),
            TURN_DIRECT_MESSAGE.scope(
                Some(true),
                ai_scope::serve_over(Transport::WebApp, async {
                    let surface = bridge.turn_surface(ToolSessionTurn {
                        user_id: &user_id,
                        tenant_id: alex.tenant,
                        conversation_tenant_id: alex.tenant,
                        conversation_id: &conversation,
                        turn_id: ConversationTurnId(Uuid::new_v4()),
                        turn_agent_id: None,
                        budget: 64,
                        seat: TurnSeat::Subject,
                        switched_off: &SwitchedOff::default(),
                        grant: &OAuthScope::self_grant(),
                        role,
                    });
                    let listed_first = listed(&surface).await;
                    ai_scope::note_member_read(CarriedRead::peer(
                        squad.casey.id,
                        Some(squad.group),
                    ));
                    let refused = surface
                        .call("remember_fact", &json!({ "fact": "my FTP is 250" }))
                        .await;
                    (listed_first, refused)
                }),
            ),
        )
        .await;
        for tool in ["remember_fact", "set_goal"] {
            assert!(
                listed_first.contains(tool),
                "bug: a group member who read no one is not listed {tool} on the loopback: \
                 {listed_first:?}"
            );
        }
        assert!(refused.is_error, "{}", refused.text);
        assert!(
            refused.text.contains("member_read_withheld"),
            "bug: a write a mid-turn read took away is refused without saying why: {}",
            refused.text
        );
    }

    /// What a tool answer holds, as one text.
    fn answer_text(response: &Value) -> String {
        response.to_string()
    }

    /// One A2A tool intent over `grant`.
    async fn a2a_call(app: &Router, grant: &str, tool: &str, parameters: Value) -> Value {
        post_json(
            app,
            "/a2a/jsonrpc",
            grant,
            rpc(
                "SendMessage",
                &json!({ "message": {
                    "messageId": format!("m-{}", Uuid::new_v4()),
                    "role": "ROLE_USER",
                    "parts": [{ "data": { "tool_name": tool, "parameters": parameters } }]
                } }),
            ),
        )
        .await
    }

    /// carnet#1511 call 30: an outside call has no topic, so the taint is
    /// the grant's. After a call by a grant that read another member, a
    /// free-text write over `/mcp` or A2A by that grant is refused for
    /// thirty minutes, and the refusal names the wait. Before any read the
    /// write runs.
    #[tokio::test]
    async fn after_a_grant_read_a_member_its_free_text_writes_wait() {
        let res = create_test_server_resources().await.unwrap();
        let squad = squad(&res).await;
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = delegated(&res, &squad.casey);
        let goal = json!({ "goal_type": "distance", "target_value": 40, "timeframe": "week" });

        let before = mcp_call(&app, &grant, "set_goal", goal.clone()).await;
        assert_ne!(
            before["result"]["isError"], true,
            "with no member read the goal is set: {before}"
        );

        let read = mcp_call(
            &app,
            &grant,
            "get_group_member_activities",
            json!({ "member": "Bea Bright" }),
        )
        .await;
        assert!(
            answer_text(&read).contains("\"member\":\"Bea Bright\""),
            "the grant read Bea: {read}"
        );

        let after = mcp_call(&app, &grant, "set_goal", goal.clone()).await;
        assert_eq!(
            after["result"]["isError"], true,
            "bug: a free-text write over /mcp ran right after the grant read another member: \
             {after}"
        );
        assert!(
            answer_text(&after).contains("30 minutes"),
            "the refusal names the wait: {after}"
        );
        let over_a2a = a2a_call(&app, &grant, "set_goal", goal).await;
        assert!(
            answer_text(&over_a2a).contains("30 minutes"),
            "bug: a free-text write over A2A ran right after the grant read another member: \
             {over_a2a}"
        );
    }
}
