// ABOUTME: An admin's AI assistant acts as a user over /mcp and /a2a: a support session bound to the connection that starts it
// ABOUTME: Refused without admin in the grant; never of an admin; leaves nothing behind; every call audited; ends with the session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Support sessions through an admin's assistant (carnet#1511)
//!
//! JF, 2026-11-19: "Build it over MCP/A2A". An admin's assistant starts a
//! support session with `admin_impersonate_start`, and from the next call
//! on the connection that started it lists and runs exactly what the user's
//! own assistant does; `admin_impersonate_end` ends it. Each case drives the
//! real entry points, `POST /mcp` and `POST /a2a/jsonrpc`.

#[cfg(feature = "tools-groups")]
mod support_session_assistant_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, join, person, seed_ride, Person,
    };

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use chrono::{Duration, Utc};
    use pierre_auth::auth::{DelegatedGrant, IMPERSONATION_WRITE_REFUSED};
    use pierre_core::permissions::impersonation::{ImpersonationSession, API_KEY_BINDING};
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_database::repositories::admin_audit::AuditEventFilter;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::support_session::{
        ENDED_ACTION, END_TOOL, EXPIRED_ACTION, STARTED_ACTION, START_TOOL,
    };
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// The OAuth application an admin's agent connects through.
    const ADMIN_CLIENT: &str = "admin_assistant_test";

    /// A coach with one group: Bea allowed the coach's assistant, Alex did
    /// not. And an admin who connects their own assistant.
    struct World {
        res: Arc<ServerContext>,
        app: Router,
        admin: Person,
        casey: Person,
        alex: Person,
    }

    async fn world() -> World {
        let res = create_test_server_resources().await.unwrap();
        let admin = person(&res, "ada@support-session.test", "Ada Admin").await;
        res.common
            .repos
            .users
            .set_admin_status(admin.id, true)
            .await
            .unwrap();
        let casey = coach(&res, "casey@support-session.test", "Casey Coach").await;
        let alex = person(&res, "alex@support-session.test", "Alex Athlete").await;
        let bea = person(&res, "bea@support-session.test", "Bea Bright").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        join(&res, group, bea.id, bea.tenant, true).await;
        res.common
            .repos
            .groups
            .set_coach_assistant_read(&group.to_string(), bea.id, true, Utc::now())
            .await
            .unwrap()
            .expect("a live member");
        seed_ride(&res, &alex, 2).await;
        seed_ride(&res, &bea, 3).await;
        let app = ProviderToolRouter::build_http_app(&res);
        World {
            res,
            app,
            admin,
            casey,
            alex,
        }
    }

    impl World {
        /// The admin's own API key: the self grant, `admin` included.
        async fn admin_key(&self) -> String {
            let key = common::issue_test_api_key(
                &self.res.agent.database,
                self.admin.id,
                "admin assistant",
            )
            .await
            .unwrap();
            format!("Bearer {key}")
        }

        /// A token an OAuth application holds for `who`, with `scopes`.
        fn oauth(&self, who: &Person, scopes: &[&str]) -> String {
            let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
            let token = self
                .res
                .auth
                .auth_manager
                .generate_oauth_access_token(
                    &self.res.auth.jwks_manager,
                    &DelegatedGrant {
                        client_id: ADMIN_CLIENT,
                        scopes: &scopes,
                        audience: None,
                    },
                    &who.id,
                    &[],
                    Some(who.tenant.to_string()),
                )
                .unwrap();
            format!("Bearer {token}")
        }

        async fn post(&self, uri: &str, authorization: &str, body: Value) -> (StatusCode, Value) {
            let request = Request::post(uri)
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .header("A2A-Version", "1.0")
                .header("authorization", authorization)
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = self.app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                    Value::String(String::from_utf8_lossy(&bytes).into_owned())
                }),
            )
        }

        async fn listed(&self, authorization: &str) -> BTreeSet<String> {
            let (status, body) = self
                .post("/mcp", authorization, rpc("tools/list", &json!({})))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["result"]["tools"]
                .as_array()
                .unwrap_or_else(|| panic!("a list: {body}"))
                .iter()
                .map(|tool| tool["name"].as_str().unwrap().to_owned())
                .collect()
        }

        async fn call(&self, authorization: &str, name: &str, arguments: Value) -> Value {
            let (status, body) = self
                .post(
                    "/mcp",
                    authorization,
                    rpc(
                        "tools/call",
                        &json!({ "name": name, "arguments": arguments }),
                    ),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{name}: {body}");
            body
        }

        async fn start(&self, authorization: &str, user: &str) -> Value {
            self.call(
                authorization,
                START_TOOL,
                json!({ "user": user, "reason": "carnet support request #42" }),
            )
            .await
        }

        /// One A2A tool intent, as the agent sends it.
        async fn a2a(&self, authorization: &str, tool: &str, parameters: Value) -> Value {
            let (status, body) = self
                .post(
                    "/a2a/jsonrpc",
                    authorization,
                    rpc(
                        "SendMessage",
                        &json!({
                            "message": {
                                "messageId": format!("m-{}", Uuid::new_v4()),
                                "role": "ROLE_USER",
                                "parts": [{ "data": { "tool_name": tool, "parameters": parameters } }]
                            }
                        }),
                    ),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{tool}: {body}");
            body
        }

        async fn trail(&self, action_prefix: &str) -> Vec<Value> {
            self.res
                .common
                .repos
                .admin_audit
                .list(
                    &AuditEventFilter {
                        action_prefix: Some(action_prefix.to_owned()),
                        actor_user_id: Some(self.admin.id),
                        ..AuditEventFilter::default()
                    },
                    200,
                )
                .await
                .unwrap()
                .into_iter()
                .map(|row| serde_json::to_value(row).unwrap())
                .collect()
        }
    }

    fn rpc(method: &str, params: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    fn text(response: &Value) -> String {
        response.to_string()
    }

    /// Over `/mcp` with the admin's API key: the session lists and serves
    /// the coach's assistant's view, members' opt-in included; it leaves
    /// nothing behind; every call is on the trail; it ends on request.
    #[tokio::test]
    async fn an_admins_assistant_acts_as_a_coach_over_mcp_until_it_ends() {
        let world = world().await;
        let key = world.admin_key().await;

        let own = world.listed(&key).await;
        assert!(own.contains(START_TOOL), "{own:?}");
        let roster = world.call(&key, "get_roster_overview", json!({})).await;
        assert!(
            !text(&roster).contains("Bea Bright"),
            "the admin leads no group: {roster}"
        );

        let started = world.start(&key, "casey@support-session.test").await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        assert!(
            text(&started).contains(&world.casey.id.to_string()),
            "{started}"
        );

        // The coach's list: the coach tools, the end tool, no other admin
        // tool, no export.
        let as_coach = world.listed(&key).await;
        assert!(as_coach.contains("get_roster_overview"), "{as_coach:?}");
        assert!(as_coach.contains(END_TOOL), "{as_coach:?}");
        assert!(!as_coach.contains(START_TOOL), "{as_coach:?}");
        assert!(
            as_coach
                .iter()
                .all(|name| !name.starts_with("admin_") || name == END_TOOL),
            "no admin tool is the coach's: {as_coach:?}"
        );
        for export in [
            "export_dossier",
            "export_latest_snapshot",
            "export_intervals",
            "export_routes",
        ] {
            if own.contains(export) {
                assert!(
                    !as_coach.contains(export),
                    "{export} is withheld: {as_coach:?}"
                );
            }
        }

        // Served as the coach's own assistant: Bea, who allowed it, by name;
        // Alex, who did not, as a count.
        let roster = world.call(&key, "get_roster_overview", json!({})).await;
        assert_eq!(roster["result"]["isError"], false, "{roster}");
        assert!(text(&roster).contains("Bea Bright"), "{roster}");
        assert!(!text(&roster).contains("Alex Athlete"), "{roster}");

        // Nothing that outlives the session: a provider connection is refused
        // and the attempt is on the trail.
        let connect = world
            .call(&key, "connect_provider", json!({ "provider": "strava" }))
            .await;
        assert!(
            text(&connect).contains(IMPERSONATION_WRITE_REFUSED),
            "{connect}"
        );
        assert!(
            !world.trail("impersonation.write_refused").await.is_empty(),
            "the refused attempt names the admin"
        );

        // Every call of the session is a row naming the admin and the coach.
        let calls = world.trail("tool.get_roster_overview").await;
        let in_session: Vec<&Value> = calls
            .iter()
            .filter(|row| row["metadata"]["impersonation_session_id"].is_string())
            .collect();
        assert_eq!(in_session.len(), 1, "{calls:#?}");
        assert_eq!(
            in_session[0]["metadata"]["target"]["user_id"],
            world.casey.id.to_string(),
            "bug: the session's call names the coach as its target"
        );
        assert_eq!(
            in_session[0]["actor_user_id"],
            world.admin.id.to_string(),
            "bug: the session's call names the admin as its actor"
        );
        assert_eq!(
            world.trail(STARTED_ACTION).await.len(),
            1,
            "bug: the start is one audit row"
        );

        let ended = world.call(&key, END_TOOL, json!({})).await;
        assert_eq!(ended["result"]["isError"], false, "{ended}");
        assert!(text(&ended).contains("\"ended\":true"), "{ended}");
        assert_eq!(
            world.trail(ENDED_ACTION).await.len(),
            1,
            "bug: the end is one audit row"
        );
        let back = world.listed(&key).await;
        assert!(back.contains(START_TOOL), "the admin again: {back:?}");
        let roster = world.call(&key, "get_roster_overview", json!({})).await;
        assert!(!text(&roster).contains("Bea Bright"), "{roster}");
    }

    /// Without `admin` in the grant there is no support session: a
    /// connection granted the ordinary scopes is offered neither tool and is
    /// refused the start; a non-admin is refused it whatever the grant.
    #[tokio::test]
    async fn a_support_session_needs_an_admin_whose_grant_holds_admin() {
        let world = world().await;
        let delegated = OAuthScope::delegable_as_str();
        let read_write = world.oauth(&world.admin, &delegated);
        let listed = world.listed(&read_write).await;
        assert!(!listed.contains(START_TOOL), "{listed:?}");
        let (status, refused) = world
            .post(
                "/mcp",
                &read_write,
                rpc(
                    "tools/call",
                    &json!({ "name": START_TOOL, "arguments": { "user": "casey@support-session.test", "reason": "x" } }),
                ),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "insufficient scope: {refused}"
        );
        assert!(
            world
                .res
                .common
                .repos
                .impersonation
                .get_active_session(world.admin.id)
                .await
                .unwrap()
                .is_none(),
            "no session was started"
        );

        // A coach holding every scope is still no admin.
        let mut every = delegated.clone();
        every.push("admin");
        let coach_grant = world.oauth(&world.casey, &every);
        let refused = world
            .call(
                &coach_grant,
                START_TOOL,
                json!({ "user": "alex@support-session.test", "reason": "x" }),
            )
            .await;
        assert_eq!(refused["result"]["isError"], true, "{refused}");
    }

    /// Never another admin, never yourself.
    #[tokio::test]
    async fn an_admin_is_never_the_user_a_support_session_acts_as() {
        let world = world().await;
        let key = world.admin_key().await;
        world
            .res
            .common
            .repos
            .users
            .set_admin_status(world.alex.id, true)
            .await
            .unwrap();
        for user in ["alex@support-session.test", "ada@support-session.test"] {
            let refused = world.start(&key, user).await;
            assert_eq!(refused["result"]["isError"], true, "{user}: {refused}");
        }
        assert!(
            world.trail(STARTED_ACTION).await.is_empty(),
            "bug: a refused start writes no audit row"
        );
    }

    /// A session the web console started never turns the admin's assistant
    /// into the user; one past its hour is ended on the next call, and the
    /// expiry is on the trail.
    #[tokio::test]
    async fn only_a_live_session_this_connection_started_changes_it() {
        let world = world().await;
        let key = world.admin_key().await;
        let sessions = &world.res.common.repos.impersonation;

        let web = ImpersonationSession::new(world.admin.id, world.casey.id, Some("web".to_owned()));
        sessions.create_session(&web).await.unwrap();
        let listed = world.listed(&key).await;
        assert!(listed.contains(START_TOOL), "still the admin: {listed:?}");

        sessions.end_all_sessions(world.admin.id).await.unwrap();
        let stale = ImpersonationSession {
            started_at: Utc::now() - Duration::minutes(61),
            ..ImpersonationSession::bound_to(
                world.admin.id,
                world.casey.id,
                Some("stale".to_owned()),
                API_KEY_BINDING.to_owned(),
            )
        };
        sessions.create_session(&stale).await.unwrap();
        let listed = world.listed(&key).await;
        assert!(listed.contains(START_TOOL), "the hour is over: {listed:?}");
        let row = sessions.get_session(&stale.id).await.unwrap().unwrap();
        assert!(!row.is_active, "ended by the lookup past its hour");
        assert_eq!(
            world.trail(EXPIRED_ACTION).await.len(),
            1,
            "bug: the expiry is one audit row"
        );

        // And a session bound to another connection of the admin's leaves
        // this one alone.
        let elsewhere = ImpersonationSession::bound_to(
            world.admin.id,
            world.casey.id,
            Some("elsewhere".to_owned()),
            "some_other_client".to_owned(),
        );
        sessions.create_session(&elsewhere).await.unwrap();
        assert!(
            world.listed(&key).await.contains(START_TOOL),
            "still the admin"
        );
    }

    /// Over `/a2a`: an agent whose grant holds `admin` acts as the coach
    /// through tool calls, its card is the coach's, a text turn is refused
    /// while the session lives, and it ends on request. Without `admin` the
    /// start is refused.
    #[tokio::test]
    async fn an_admins_agent_acts_as_a_coach_over_a2a() {
        let world = world().await;
        let delegated = OAuthScope::delegable_as_str();
        let refused_grant = world.oauth(&world.admin, &delegated);
        let refused = world
            .a2a(
                &refused_grant,
                START_TOOL,
                json!({ "user": "casey@support-session.test", "reason": "x" }),
            )
            .await;
        assert!(!text(&refused).contains("\"session_id\""), "{refused}");
        assert!(world.trail(STARTED_ACTION).await.is_empty(), "{refused}");

        let mut every = delegated.clone();
        every.push("admin");
        let grant = world.oauth(&world.admin, &every);
        let started = world
            .a2a(
                &grant,
                START_TOOL,
                json!({ "user": "casey@support-session.test", "reason": "carnet #42" }),
            )
            .await;
        assert!(text(&started).contains("\"session_id\""), "{started}");
        let session = world
            .res
            .common
            .repos
            .impersonation
            .get_active_bound_session(world.admin.id, ADMIN_CLIENT)
            .await
            .unwrap()
            .expect("bound to the agent's application");
        assert_eq!(
            session.target_user_id, world.casey.id,
            "bug: the A2A session acts as the coach it named"
        );

        let roster = world.a2a(&grant, "get_roster_overview", json!({})).await;
        assert!(text(&roster).contains("Bea Bright"), "{roster}");
        assert!(!text(&roster).contains("Alex Athlete"), "{roster}");

        let (status, card) = world
            .post(
                "/a2a/jsonrpc",
                &grant,
                rpc("GetExtendedAgentCard", &json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{card}");
        let skills: BTreeSet<String> = card["result"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(skills.contains(END_TOOL), "{skills:?}");
        assert!(!skills.contains(START_TOOL), "{skills:?}");

        let calls = world.trail("tool.get_roster_overview").await;
        assert!(
            calls
                .iter()
                .any(|row| row["metadata"]["target"]["user_id"] == world.casey.id.to_string()),
            "{calls:#?}"
        );

        let ended = world.a2a(&grant, END_TOOL, json!({})).await;
        assert!(text(&ended).contains("\"ended\":true"), "{ended}");
        let roster = world.a2a(&grant, "get_roster_overview", json!({})).await;
        assert!(!text(&roster).contains("Bea Bright"), "{roster}");
    }
}
