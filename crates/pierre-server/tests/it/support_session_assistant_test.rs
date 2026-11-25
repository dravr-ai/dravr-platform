// ABOUTME: An admin's AI assistant acts as a user over /mcp and /a2a: a support session bound to the credential that starts it
// ABOUTME: Only that grant or key acts as the user; revoking it ends the session; never of an admin; every call audited

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Support sessions through an admin's assistant (carnet#1511)
//!
//! JF, 2026-11-19: "Build it over MCP/A2A". An admin's assistant starts a
//! support session with `admin_impersonate_start`, and from the next call
//! on the credential that started it lists and runs exactly what the user's
//! own assistant does; `admin_impersonate_end` ends it. JF, Q21: "the
//! session is bound to the grant or API key that started it", and revoking
//! the grant ends the session. Each call drives the real entry points,
//! `POST /mcp` and `POST /a2a/jsonrpc`; a revocation is the repository write
//! every revoking route makes.

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
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_auth::auth::{DelegatedGrant, TokenLineage, IMPERSONATION_WRITE_REFUSED};
    use pierre_core::models::{
        GrantActor, GrantApproval, OAuth2Client, OAuth2RefreshToken, OAuthClientGrant,
    };
    use pierre_core::permissions::impersonation::{
        BoundCredential, ImpersonationSession, ENDED_ACTION, EXPIRED_ACTION, STARTED_ACTION,
    };
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::repositories::admin_audit::AuditEventFilter;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::support_session::{END_TOOL, START_TOOL};
    use secrecy::SecretString;
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
            .set_coach_assistant_read(&group.to_string(), bea.id, true, Timestamp::now())
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

    /// One of the admin's API keys: the bearer an assistant sends and the
    /// key's id, which a session it starts is bound to.
    struct Key {
        bearer: String,
        id: String,
    }

    /// One authorization of [`ADMIN_CLIENT`]: its refresh family, stored as
    /// the token endpoint stores it, and an access token minted in it.
    struct Grant {
        bearer: String,
        family: String,
        refresh: String,
    }

    impl World {
        /// The admin's own API key: the self grant, `admin` included.
        async fn admin_key(&self) -> String {
            self.admin_key_named("admin assistant").await.bearer
        }

        /// A new API key of the admin's, named `name`.
        async fn admin_key_named(&self, name: &str) -> Key {
            let key = common::issue_test_api_key(&self.res.agent.database, self.admin.id, name)
                .await
                .unwrap();
            let id = self
                .res
                .common
                .repos
                .api_keys
                .get_for_user(self.admin.id)
                .await
                .unwrap()
                .into_iter()
                .find(|stored| stored.name == name)
                .expect("the key just issued")
                .id;
            Key {
                bearer: format!("Bearer {key}"),
                id,
            }
        }

        /// A new authorization of [`ADMIN_CLIENT`] by `who` with `scopes`:
        /// a refresh family of its own, and an access token minted in it.
        /// Each call is another grant of the same application.
        async fn grant(&self, who: &Person, scopes: &[&str]) -> Grant {
            let oauth = &self.res.common.repos.oauth2_server;
            if oauth.get_client(ADMIN_CLIENT).await.unwrap().is_none() {
                oauth
                    .store_client(&OAuth2Client {
                        id: Uuid::new_v4().to_string(),
                        client_id: ADMIN_CLIENT.to_owned(),
                        client_secret_hash: String::new(),
                        redirect_uris: Vec::new(),
                        grant_types: vec![
                            "authorization_code".to_owned(),
                            "refresh_token".to_owned(),
                        ],
                        response_types: vec!["code".to_owned()],
                        client_name: Some("Admin assistant".to_owned()),
                        client_uri: None,
                        scope: None,
                        created_at: Timestamp::now(),
                        expires_at: None,
                    })
                    .await
                    .unwrap();
            }
            let family = Uuid::new_v4().to_string();
            let refresh = format!("rt_{}", Uuid::new_v4().simple());
            let now = Timestamp::now();
            oauth
                .store_refresh_token(&OAuth2RefreshToken {
                    token: SecretString::from(refresh.clone()),
                    client_id: ADMIN_CLIENT.to_owned(),
                    user_id: who.id,
                    tenant_id: who.tenant.to_string(),
                    scope: Some(scopes.join(" ")),
                    expires_at: now + Elapsed::days(1),
                    created_at: now,
                    revoked: false,
                    family_id: family.clone(),
                    resource: None,
                    family_created_at: Some(now),
                    client_grant_id: None,
                    dpop_jkt: None,
                })
                .await
                .unwrap();
            let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
            let token = self
                .res
                .auth
                .auth_manager
                .generate_oauth_access_token_in_grant(
                    &self.res.auth.jwks_manager,
                    &DelegatedGrant {
                        client_id: ADMIN_CLIENT,
                        scopes: &scopes,
                        audience: None,
                    },
                    &TokenLineage {
                        family_id: &family,
                        client_grant_id: None,
                        dpop_jkt: None,
                    },
                    &who.id,
                    Some(who.tenant.to_string()),
                )
                .unwrap();
            Grant {
                bearer: format!("Bearer {token}"),
                family,
                refresh,
            }
        }

        /// An admin grant of [`ADMIN_CLIENT`]: every delegable scope and
        /// `admin`, as an admin who ticked Admin access holds.
        async fn admin_grant(&self) -> Grant {
            let mut every = OAuthScope::delegable_as_str();
            every.push("admin");
            self.grant(&self.admin, &every).await
        }

        /// The session `id`, as stored.
        async fn session(&self, id: &str) -> ImpersonationSession {
            self.res
                .common
                .repos
                .impersonation
                .get_session(id)
                .await
                .unwrap()
                .expect("the session row")
        }

        /// The live session the admin started with `credential`.
        async fn bound(&self, credential: &BoundCredential) -> Option<ImpersonationSession> {
            self.res
                .common
                .repos
                .impersonation
                .get_active_bound_session(self.admin.id, credential)
                .await
                .unwrap()
        }

        /// The `impersonation.ended` rows naming session `id`.
        async fn ended_rows(&self, id: &str) -> Vec<Value> {
            self.trail(ENDED_ACTION)
                .await
                .into_iter()
                .filter(|row| row["metadata"]["impersonation_session_id"] == id)
                .collect()
        }

        /// Whether `authorization` reads the coach's member by name: the
        /// roster as the coach's assistant sees it.
        async fn sees_as_coach(&self, authorization: &str) -> bool {
            let roster = self
                .call(authorization, "get_roster_overview", json!({}))
                .await;
            text(&roster).contains("Bea Bright")
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
    async fn only_a_live_session_this_credential_started_changes_it() {
        let world = world().await;
        let own_key = world.admin_key_named("admin assistant").await;
        let key = own_key.bearer.clone();
        let sessions = &world.res.common.repos.impersonation;

        let web = ImpersonationSession::new(world.admin.id, world.casey.id, Some("web".to_owned()));
        sessions.create_session(&web).await.unwrap();
        let listed = world.listed(&key).await;
        assert!(listed.contains(START_TOOL), "still the admin: {listed:?}");

        sessions.end_all_sessions(world.admin.id).await.unwrap();
        let stale = ImpersonationSession {
            started_at: Timestamp::now() - Elapsed::minutes(61),
            ..ImpersonationSession::bound_to(
                world.admin.id,
                world.casey.id,
                Some("stale".to_owned()),
                &BoundCredential::ApiKey(own_key.id.clone()),
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

        // And a session bound to another credential of the admin's leaves
        // this one alone.
        let elsewhere = ImpersonationSession::bound_to(
            world.admin.id,
            world.casey.id,
            Some("elsewhere".to_owned()),
            &BoundCredential::ApiKey("some_other_key".to_owned()),
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

        let admin_grant = world.admin_grant().await;
        let grant = admin_grant.bearer.clone();
        let started = world
            .a2a(
                &grant,
                START_TOOL,
                json!({ "user": "casey@support-session.test", "reason": "carnet #42" }),
            )
            .await;
        assert!(text(&started).contains("\"session_id\""), "{started}");
        let session = world
            .bound(&BoundCredential::Grant(admin_grant.family.clone()))
            .await
            .expect("bound to the agent's grant");
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

    /// JF Q21 over `/mcp` with API keys: the session is the key's that
    /// started it. Another key of the same admin acts as the admin, and the
    /// start names the key on the trail.
    #[tokio::test]
    async fn only_the_key_that_started_a_session_acts_as_the_user() {
        let world = world().await;
        let first = world.admin_key_named("first key").await;
        let second = world.admin_key_named("second key").await;

        let started = world
            .start(&first.bearer, "casey@support-session.test")
            .await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        let session = world
            .bound(&BoundCredential::ApiKey(first.id.clone()))
            .await
            .expect("bound to the first key");
        assert_eq!(
            session.api_key_id.as_deref(),
            Some(first.id.as_str()),
            "bug: the session names another key than the one that started it"
        );
        assert_eq!(
            session.grant_id, None,
            "bug: a key's session names no grant"
        );

        assert!(
            world.sees_as_coach(&first.bearer).await,
            "the first key is the coach"
        );
        let listed = world.listed(&second.bearer).await;
        assert!(
            listed.contains(START_TOOL),
            "bug: another key of the same admin is the admin: {listed:?}"
        );
        assert!(
            !world.sees_as_coach(&second.bearer).await,
            "bug: another key of the same admin never acts as the coach"
        );

        let started_rows = world.trail(STARTED_ACTION).await;
        assert_eq!(started_rows.len(), 1, "{started_rows:#?}");
        assert_eq!(
            started_rows[0]["metadata"]["api_key_id"], first.id,
            "bug: the start names the key it is bound to"
        );
    }

    /// JF Q21 over `/mcp` with OAuth: the session is the grant's (the
    /// refresh family) that started it. Another authorization of the same
    /// application acts as the admin.
    #[tokio::test]
    async fn only_the_grant_that_started_a_session_acts_as_the_user() {
        let world = world().await;
        let first = world.admin_grant().await;
        let second = world.admin_grant().await;
        assert_ne!(
            first.family, second.family,
            "two authorizations, two grants"
        );

        let started = world
            .start(&first.bearer, "casey@support-session.test")
            .await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        let session = world
            .bound(&BoundCredential::Grant(first.family.clone()))
            .await
            .expect("bound to the first grant");
        assert_eq!(
            session.api_key_id, None,
            "bug: a grant's session names no key"
        );

        assert!(
            world.sees_as_coach(&first.bearer).await,
            "the first grant is the coach"
        );
        let listed = world.listed(&second.bearer).await;
        assert!(
            listed.contains(START_TOOL),
            "bug: another grant of the same application is the admin: {listed:?}"
        );
        assert!(
            !world.sees_as_coach(&second.bearer).await,
            "bug: another grant of the same application never acts as the coach"
        );
        let key = world.admin_key().await;
        assert!(
            !world.sees_as_coach(&key).await,
            "bug: the admin's API key never acts as a grant's session"
        );
        assert_eq!(
            world.trail(STARTED_ACTION).await[0]["metadata"]["grant_id"],
            first.family,
            "bug: the start names the grant it is bound to"
        );
    }

    /// Deactivating the key ends its session, in the same write, with a row
    /// on the trail; the other keys never were the user.
    #[tokio::test]
    async fn deactivating_the_key_ends_its_support_session() {
        let world = world().await;
        let key = world.admin_key_named("revoked key").await;
        let other = world.admin_key_named("kept key").await;
        let started = world.start(&key.bearer, "casey@support-session.test").await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        let session = world
            .bound(&BoundCredential::ApiKey(key.id.clone()))
            .await
            .expect("live");

        world
            .res
            .common
            .repos
            .api_keys
            .deactivate(&key.id, world.admin.id)
            .await
            .unwrap();

        let row = world.session(&session.id).await;
        assert!(!row.is_active, "bug: deactivating the key ends the session");
        assert!(row.ended_at.is_some(), "bug: the end is stamped");
        let ended = world.ended_rows(&session.id).await;
        assert_eq!(ended.len(), 1, "bug: the end is one audit row: {ended:#?}");
        assert_eq!(
            ended[0]["metadata"]["api_key_id"], key.id,
            "bug: the end row names another key"
        );
        assert_eq!(
            ended[0]["metadata"]["credential_kind"], "api_key",
            "bug: the end row names the kind of credential revoked"
        );
        assert_eq!(
            ended[0]["metadata"]["client_id"],
            Value::Null,
            "bug: an API key names no client"
        );
        assert_eq!(
            ended[0]["actor_user_id"],
            world.admin.id.to_string(),
            "bug: the end row names another actor than the admin"
        );

        let (status, _) = world
            .post("/mcp", &key.bearer, rpc("tools/list", &json!({})))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "the key is gone");
        assert!(
            world.listed(&other.bearer).await.contains(START_TOOL),
            "the admin's other key is the admin"
        );
    }

    /// Revoking the grant's refresh family (a replayed token, the client's
    /// own revocation of it) ends its session and audits the end. The
    /// family's access token still authenticates until it expires, and a
    /// call with it acts as the admin, never as the user.
    #[tokio::test]
    async fn revoking_the_grants_family_ends_its_support_session() {
        let world = world().await;
        let grant = world.admin_grant().await;
        let kept = world.admin_grant().await;
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let session = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");

        let revoked = world
            .res
            .common
            .repos
            .oauth2_server
            .revoke_refresh_token_family(&grant.refresh)
            .await
            .unwrap();
        assert_eq!(revoked, 1, "the family's one token");

        let row = world.session(&session.id).await;
        assert!(!row.is_active, "bug: revoking the family ends the session");
        let ended = world.ended_rows(&session.id).await;
        assert_eq!(ended.len(), 1, "bug: the end is one audit row: {ended:#?}");
        assert_eq!(
            ended[0]["metadata"]["grant_id"], grant.family,
            "bug: the end row names another grant"
        );
        assert_eq!(
            ended[0]["metadata"]["credential_kind"], "oauth_grant",
            "bug: the end row names the kind of credential revoked"
        );
        assert_eq!(
            ended[0]["metadata"]["client_id"],
            Value::Null,
            "bug: a family revoked by its token names no client it was not told"
        );

        let listed = world.listed(&grant.bearer).await;
        assert!(
            listed.contains(START_TOOL),
            "bug: a call after the revocation is the admin's: {listed:?}"
        );
        assert!(
            !world.sees_as_coach(&grant.bearer).await,
            "bug: a call after the revocation never acts as the user"
        );
        // Nor can the revoked grant's token, still good for its hour, start
        // a session again.
        let restarted = world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        assert_eq!(
            restarted["result"]["isError"], true,
            "bug: a revoked grant starts no session: {restarted}"
        );
        assert!(
            world
                .bound(&BoundCredential::Grant(grant.family.clone()))
                .await
                .is_none(),
            "bug: no session is bound to a revoked grant"
        );

        // The application's other grant still refreshes: only the family
        // that started the session was revoked.
        let other = world
            .res
            .common
            .repos
            .oauth2_server
            .get_refresh_token_by_value(&kept.refresh)
            .await
            .unwrap()
            .expect("the other grant's token");
        assert!(!other.revoked, "another grant is untouched");
    }

    /// Withdrawing the consent behind the grant at Connected apps ends the
    /// session in the same write, with a row on the trail, and the grant's
    /// token is refused from then on.
    #[tokio::test]
    async fn revoking_the_consent_ends_its_support_session() {
        let world = world().await;
        let grant = world.admin_grant().await;
        let consent = Uuid::new_v4().to_string();
        let oauth = &world.res.common.repos.oauth2_server;
        oauth
            .store_client_grant(
                &OAuthClientGrant {
                    id: consent.clone(),
                    user_id: world.admin.id.to_string(),
                    tenant_id: world.admin.tenant.to_string(),
                    client_id: ADMIN_CLIENT.to_owned(),
                    scope: "admin".to_owned(),
                    granted_at: Timestamp::now(),
                    revoked_at: None,
                    consent_version: None,
                    consent_locale: None,
                    authorization_details: None,
                    expires_at: None,
                },
                &GrantApproval::by_user(),
            )
            .await
            .unwrap();
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let session = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");

        assert!(
            oauth
                .revoke_client_grant(
                    &consent,
                    &world.admin.id.to_string(),
                    &world.admin.tenant.to_string(),
                    GrantActor::User,
                )
                .await
                .unwrap(),
            "bug: the consent was not revoked"
        );
        world
            .res
            .account_service()
            .grant_standings()
            .forget(
                world.admin.id,
                &world.admin.tenant.to_string(),
                ADMIN_CLIENT,
            )
            .await;

        let row = world.session(&session.id).await;
        assert!(!row.is_active, "bug: revoking the consent ends the session");
        let ended = world.ended_rows(&session.id).await;
        assert_eq!(ended.len(), 1, "bug: the end is one audit row: {ended:#?}");
        assert_eq!(
            ended[0]["metadata"]["grant_id"], grant.family,
            "bug: the end row names another grant"
        );

        let (status, body) = world
            .post("/mcp", &grant.bearer, rpc("tools/list", &json!({})))
            .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the withdrawn consent's token is refused: {body}"
        );
    }

    /// A refresh consumes the presented token before it stores the
    /// successor. Between the two the family holds no live token, and a call
    /// then still acts as the user: the session is neither ended nor left.
    #[tokio::test]
    async fn a_refresh_in_flight_never_ends_the_support_session() {
        let world = world().await;
        let grant = world.admin_grant().await;
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let session = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");
        let consumed = world
            .res
            .common
            .repos
            .oauth2_server
            .consume_refresh_token(&grant.refresh, ADMIN_CLIENT, Timestamp::now())
            .await
            .unwrap();
        assert!(consumed.is_some(), "the rotation consumed the token");

        assert!(
            world.sees_as_coach(&grant.bearer).await,
            "bug: a call mid-rotation still acts as the user"
        );
        assert!(
            world.session(&session.id).await.is_active,
            "bug: a rotation never ends the session"
        );
    }

    /// A client that accepts a stream (`text/event-stream`, as MCP clients
    /// send) is served the session as one that accepts JSON only.
    #[tokio::test]
    async fn a_streaming_client_is_served_its_support_session() {
        let world = world().await;
        let key = world.admin_key().await;
        world.start(&key, "casey@support-session.test").await;
        let request = Request::post("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", &key)
            .body(Body::from(
                rpc(
                    "tools/call",
                    &json!({ "name": "get_roster_overview", "arguments": {} }),
                )
                .to_string(),
            ))
            .unwrap();
        let response = world.app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "bug: the session's call was refused"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            body.contains("Bea Bright"),
            "bug: a streaming client's call acts as the user: {body}"
        );
    }

    /// The client's own revocation (RFC 7009, which withdraws its access)
    /// and the application's deletion each end the session bound to its
    /// grant, audited.
    #[tokio::test]
    async fn the_clients_revocation_and_its_deletion_end_its_support_session() {
        let world = world().await;
        let oauth = &world.res.common.repos.oauth2_server;

        let grant = world.admin_grant().await;
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let withdrawn = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");
        oauth
            .withdraw_client_access(
                &world.admin.id.to_string(),
                &world.admin.tenant.to_string(),
                ADMIN_CLIENT,
                "admin",
            )
            .await
            .unwrap();
        assert!(
            !world.session(&withdrawn.id).await.is_active,
            "bug: the client's revocation ends the session"
        );
        let ended = world.ended_rows(&withdrawn.id).await;
        assert_eq!(ended.len(), 1, "bug: the end is not one audit row");
        assert_eq!(
            ended[0]["metadata"]["credential_kind"], "oauth_grant",
            "bug: the end row names the kind of credential revoked: {ended:#?}"
        );
        assert_eq!(
            ended[0]["metadata"]["client_id"], ADMIN_CLIENT,
            "bug: a consent withdrawn from a client names that client: {ended:#?}"
        );

        let grant = world.admin_grant().await;
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let deleted = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");
        assert!(
            oauth.delete_client(ADMIN_CLIENT).await.unwrap(),
            "bug: the application was not deleted"
        );
        assert!(
            !world.session(&deleted.id).await.is_active,
            "bug: deleting the application ends the session"
        );
        assert_eq!(
            world.ended_rows(&deleted.id).await.len(),
            1,
            "bug: the end is not one audit row"
        );
    }

    /// A grant that ran out while its access token still authenticates
    /// ends its session at the next call, which is the admin's.
    #[tokio::test]
    async fn a_grant_that_ran_out_ends_its_support_session_at_the_next_call() {
        let world = world().await;
        let grant = world.admin_grant().await;
        world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        let session = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("live");
        expire_family(&world, &grant.family).await;

        let listed = world.listed(&grant.bearer).await;
        assert!(listed.contains(START_TOOL), "the admin again: {listed:?}");
        assert!(
            !world.session(&session.id).await.is_active,
            "bug: the session outlived its grant"
        );
        assert_eq!(
            world.ended_rows(&session.id).await.len(),
            1,
            "bug: the end is not one audit row"
        );
    }

    /// The same binding over `/a2a`: another grant of the same application
    /// acts as the admin, and revoking the family that started the session
    /// ends it, audited, so the agent's next call is the admin's.
    #[tokio::test]
    async fn over_a2a_only_the_grant_that_started_a_session_acts_as_the_user() {
        let world = world().await;
        let grant = world.admin_grant().await;
        let other = world.admin_grant().await;
        let started = world
            .a2a(
                &grant.bearer,
                START_TOOL,
                json!({ "user": "casey@support-session.test", "reason": "carnet #42" }),
            )
            .await;
        assert!(text(&started).contains("\"session_id\""), "{started}");
        let session = world
            .bound(&BoundCredential::Grant(grant.family.clone()))
            .await
            .expect("bound to the agent's grant");

        let roster = world
            .a2a(&grant.bearer, "get_roster_overview", json!({}))
            .await;
        assert!(text(&roster).contains("Bea Bright"), "{roster}");
        let roster = world
            .a2a(&other.bearer, "get_roster_overview", json!({}))
            .await;
        assert!(
            !text(&roster).contains("Bea Bright"),
            "bug: another grant of the same application is the admin over A2A: {roster}"
        );

        world
            .res
            .common
            .repos
            .oauth2_server
            .revoke_refresh_token_family(&grant.refresh)
            .await
            .unwrap();
        assert!(
            !world.session(&session.id).await.is_active,
            "bug: the session outlived its grant"
        );
        assert_eq!(
            world.ended_rows(&session.id).await.len(),
            1,
            "bug: the end is not one audit row"
        );
        let roster = world
            .a2a(&grant.bearer, "get_roster_overview", json!({}))
            .await;
        assert!(
            !text(&roster).contains("Bea Bright"),
            "bug: after the revocation the agent is the admin: {roster}"
        );
    }

    /// Push every token of `family` past its expiry, as a grant whose life
    /// ran out.
    async fn expire_family(world: &World, family: &str) {
        const EXPIRE: &str =
            "UPDATE oauth2_refresh_tokens SET expires_at = $1 WHERE family_id = $2";
        let past = Timestamp::now() - Elapsed::minutes(1);
        match world.res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(EXPIRE)
                    .bind(past)
                    .bind(family)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(EXPIRE)
                    .bind(past)
                    .bind(family)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    // ── The credential on the trail (carnet#1511, plan step 11) ──────────

    /// A plain admin-only read every admin may call.
    const ADMIN_READ: &str = "admin_list_system_agents";

    /// The newest `tool.<name>` row naming the admin.
    async fn newest_call_row(world: &World, tool: &str) -> Value {
        world
            .trail(&format!("tool.{tool}"))
            .await
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("a row for {tool}"))
    }

    /// A client-credentials token of an A2A client the admin registered.
    async fn admin_machine_client(world: &World) -> (String, String) {
        let credentials = world
            .res
            .a2a
            .a2a_client_manager
            .register_client(
                pierre_a2a::ClientRegistrationRequest {
                    name: "admin machine client".into(),
                    description: "an admin's A2A machine client".into(),
                    capabilities: vec!["fitness-data-analysis".into()],
                    redirect_uris: vec![],
                    contact_email: "machine@support-session.test".into(),
                },
                world.admin.id,
            )
            .await
            .unwrap();
        let scopes: Vec<String> = OAuthScope::delegable_as_str()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let token = world
            .res
            .auth
            .auth_manager
            .generate_client_credentials_token(
                &world.res.auth.jwks_manager,
                &credentials.client_id,
                &scopes,
                None,
                None,
            )
            .unwrap();
        (credentials.client_id, format!("Bearer {token}"))
    }

    /// Whether `row`, serialized, holds `bearer`'s token: ids and kinds
    /// only ever reach the trail.
    fn holds_token(row: &Value, bearer: &str) -> bool {
        row.to_string()
            .contains(bearer.trim_start_matches("Bearer "))
    }

    /// An admin-only call over `/mcp` and over A2A through an OAuth grant
    /// names the application and the kind of credential; the token itself
    /// is never written.
    #[tokio::test]
    async fn an_admin_tool_call_through_an_oauth_grant_names_its_client_and_kind() {
        let world = world().await;
        let grant = world.admin_grant().await;

        let listed = world.call(&grant.bearer, ADMIN_READ, json!({})).await;
        assert_eq!(listed["result"]["isError"], false, "{listed}");
        let row = newest_call_row(&world, ADMIN_READ).await;
        assert_eq!(row["metadata"]["transport"], "mcp_http", "{row}");
        assert_eq!(
            row["metadata"]["credential_kind"], "oauth_grant",
            "bug: the row names the kind of credential: {row}"
        );
        assert_eq!(
            row["metadata"]["client_id"], ADMIN_CLIENT,
            "bug: the row names the OAuth client the token was issued to: {row}"
        );
        assert!(
            !holds_token(&row, &grant.bearer),
            "bug: a token on the trail"
        );

        let answered = world.a2a(&grant.bearer, ADMIN_READ, json!({})).await;
        assert!(answered.get("error").is_none(), "{answered}");
        let row = newest_call_row(&world, ADMIN_READ).await;
        assert_eq!(row["metadata"]["transport"], "a2a", "{row}");
        assert_eq!(row["metadata"]["credential_kind"], "oauth_grant", "{row}");
        assert_eq!(
            row["metadata"]["client_id"], ADMIN_CLIENT,
            "bug: the A2A user token keeps its client: {row}"
        );
        assert!(
            !holds_token(&row, &grant.bearer),
            "bug: a token on the trail"
        );
    }

    /// Through an API key the row names the kind and no client: no
    /// registered client holds a key.
    #[tokio::test]
    async fn an_admin_tool_call_through_an_api_key_names_the_kind_and_no_client() {
        let world = world().await;
        let key = world.admin_key_named("audit key").await;

        let listed = world.call(&key.bearer, ADMIN_READ, json!({})).await;
        assert_eq!(listed["result"]["isError"], false, "{listed}");
        let row = newest_call_row(&world, ADMIN_READ).await;
        assert_eq!(row["metadata"]["credential_kind"], "api_key", "{row}");
        assert_eq!(
            row["metadata"]["client_id"],
            Value::Null,
            "bug: an API key names no client: {row}"
        );
        assert!(!holds_token(&row, &key.bearer), "bug: a key on the trail");
    }

    /// A2A takes no API key; its other credential, an admin's machine
    /// client, is refused the admin tools (its grant never holds `admin`),
    /// and the refused row names the client and its kind.
    #[tokio::test]
    async fn an_admin_tool_call_through_a2a_client_credentials_names_the_client() {
        let world = world().await;
        let (client_id, bearer) = admin_machine_client(&world).await;

        world.a2a(&bearer, ADMIN_READ, json!({})).await;
        let row = newest_call_row(&world, ADMIN_READ).await;
        assert_eq!(row["outcome"], "refused", "{row}");
        assert_eq!(row["metadata"]["transport"], "a2a", "{row}");
        assert_eq!(
            row["metadata"]["credential_kind"], "a2a_client_credentials",
            "{row}"
        );
        assert_eq!(row["metadata"]["client_id"], client_id, "{row}");
        assert!(!holds_token(&row, &bearer), "bug: a token on the trail");
    }

    /// A support session's start and end name the credential's kind and
    /// client, through an OAuth grant and through an API key.
    #[tokio::test]
    async fn a_support_sessions_start_and_end_name_the_credential_kind_and_client() {
        let world = world().await;
        let grant = world.admin_grant().await;
        let started = world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        let row = &world.trail(STARTED_ACTION).await[0];
        assert_eq!(row["metadata"]["credential_kind"], "oauth_grant", "{row}");
        assert_eq!(row["metadata"]["client_id"], ADMIN_CLIENT, "{row}");
        assert_eq!(row["metadata"]["grant_id"], grant.family, "{row}");
        assert!(
            !holds_token(row, &grant.bearer),
            "bug: a token on the trail"
        );
        world.call(&grant.bearer, END_TOOL, json!({})).await;
        let row = &world.trail(ENDED_ACTION).await[0];
        assert_eq!(row["metadata"]["credential_kind"], "oauth_grant", "{row}");
        assert_eq!(row["metadata"]["client_id"], ADMIN_CLIENT, "{row}");

        let key = world.admin_key_named("support key").await;
        world.start(&key.bearer, "casey@support-session.test").await;
        let row = &world.trail(STARTED_ACTION).await[0];
        assert_eq!(row["metadata"]["credential_kind"], "api_key", "{row}");
        assert_eq!(row["metadata"]["client_id"], Value::Null, "{row}");
        assert_eq!(row["metadata"]["api_key_id"], key.id, "{row}");
    }

    /// The 11-14 engineering default: in a support session a call whose row
    /// cannot be written is refused, a plain read included, over `/mcp` and
    /// over A2A alike.
    #[tokio::test]
    async fn a_call_in_a_support_session_whose_audit_row_fails_is_refused() {
        let world = world().await;
        let grant = world.admin_grant().await;
        let started = world
            .start(&grant.bearer, "casey@support-session.test")
            .await;
        assert_eq!(started["result"]["isError"], false, "{started}");
        assert!(
            world.sees_as_coach(&grant.bearer).await,
            "the session serves the coach"
        );
        let roster = world
            .a2a(&grant.bearer, "get_roster_overview", json!({}))
            .await;
        assert!(text(&roster).contains("Bea Bright"), "{roster}");

        match world.res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query("DROP TABLE audit_events")
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query("DROP TABLE audit_events CASCADE")
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }

        let refused = world
            .call(&grant.bearer, "get_roster_overview", json!({}))
            .await;
        assert_eq!(
            refused["result"]["isError"], true,
            "bug: an unrecorded call in a support session ran over /mcp: {refused}"
        );
        assert!(!text(&refused).contains("Bea Bright"), "{refused}");
        let refused = world
            .a2a(&grant.bearer, "get_roster_overview", json!({}))
            .await;
        assert!(
            text(&refused).contains("audit trail"),
            "bug: an unrecorded call in a support session ran over A2A: {refused}"
        );
        assert!(!text(&refused).contains("Bea Bright"), "{refused}");
    }
}
