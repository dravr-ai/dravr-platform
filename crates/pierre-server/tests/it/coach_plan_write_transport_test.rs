// ABOUTME: A coach writes a coached athlete's plan from Dravr's own surfaces only — never over MCP, A2A or an API key
// ABOUTME: Drives save/publish over the real /mcp and A2A doors and every plan-writing athlete= tool over every transport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # A coach's plan writes stay inside Dravr (carnet#1533)
//!
//! A group's human coach names a coached athlete with `athlete=` to write
//! that athlete's plan. The athlete consented to their coach inside Dravr; a
//! client the coach connects over MCP, A2A or an API key is outside that
//! agreement, so it writes no athlete's plan — exactly as it writes no
//! athlete's calendar. What it reads through `athlete=` is unchanged, and the
//! same write from Dravr's own app or channels still lands.
//!
//! Each case drives a real entry point: `POST /mcp` `tools/call` with the
//! coach's delegated grant, A2A `SendMessage` carrying a tool intent, the
//! plan card's REST publish and discard called with an API key, and the
//! executor every door dispatches through, bound to each transport.

#[cfg(feature = "tools-groups")]
mod coach_plan_write_transport_tests {
    use crate::common::{self, create_test_server_resources, in_web_turn};
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, join, person, Person,
    };

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use chrono::{Datelike, Days, NaiveDate, Utc};
    use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
    use pierre_core::constants::oauth::providers::INTERVALS_ICU;
    use pierre_core::models::{ConnectionType, UserOAuthToken, API_KEY_TOKEN_TYPE};
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::Transport;
    use pierre_database::repositories::training_plans::DraftOwner;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_providers::ai_scope;
    use pierre_tool_runtime::context::CONVERSATION_ID;
    use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
    use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
    use pierre_tool_runtime::runtime::ToolRuntime;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    /// The machine-readable reason a write refused over an external transport
    /// carries, the calendar's and the plan's alike.
    const EXTERNAL: &str = "external_transport";

    /// Every transport, Dravr's own and external alike.
    const TRANSPORTS: [Transport; 7] = [
        Transport::WebApp,
        Transport::MobileApp,
        Transport::Messaging,
        Transport::PlatformJob,
        Transport::McpHttp,
        Transport::A2a,
        Transport::ApiKey,
    ];

    /// Every tool that writes a coached athlete's plan, or what it reaches,
    /// when `athlete=` names them — and the arguments that get it past its own
    /// validation to the question of whose plan it writes.
    fn plan_writes() -> Vec<(&'static str, Vec<Value>)> {
        vec![
            ("save_training_plan", vec![save_args()]),
            (
                "publish_training_plan",
                vec![
                    json!({ "athlete": ATHLETE, "action": "publish" }),
                    json!({ "athlete": ATHLETE, "action": "discard" }),
                ],
            ),
            ("push_training_plan", vec![json!({ "athlete": ATHLETE })]),
            (
                "prescribe_workout",
                vec![
                    json!({ "athlete": ATHLETE, "date": "2030-01-07", "session": tempo_session() }),
                ],
            ),
            (
                "withdraw_prescribed_workout",
                vec![
                    json!({ "athlete": ATHLETE, "prescription_id": uuid::Uuid::new_v4().to_string() }),
                ],
            ),
            (
                "set_preferred_calendar",
                vec![json!({ "athlete": ATHLETE, "provider": "intervals_icu" })],
            ),
            ("propose_athlete_link", vec![json!({ "athlete": ATHLETE })]),
        ]
    }

    const ATHLETE: &str = "Alex";

    /// The tools that read a coached athlete's plan through the plan scope
    /// with `athlete=`: open over every transport (JF, carnet#1533).
    const PLAN_READS: [&str; 3] = [
        "get_training_plan",
        "recommend_plan_flavour",
        "get_planned_workouts",
    ];

    /// A coach of one group whose athlete shares with them.
    struct World {
        res: Arc<ServerContext>,
        coach: Person,
        athlete: Person,
        group: uuid::Uuid,
    }

    impl World {
        async fn new() -> Self {
            let res = create_test_server_resources().await.unwrap();
            let casey = coach(&res, "casey@plan-write-transport.test", "Casey Coach").await;
            let alex = person(&res, "alex@plan-write-transport.test", "Alex Athlete").await;
            let agent = group_agent(&res, &casey).await;
            let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
            join(&res, group, alex.id, alex.tenant, true).await;
            // The coach's own Intervals.icu key: without a calendar the
            // calendar tools answer before whose calendar it is.
            let now = Utc::now();
            let repos = &res.common.repos;
            repos
                .oauth_tokens
                .upsert_token(&UserOAuthToken {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: casey.id,
                    tenant_id: casey.tenant.to_string(),
                    provider: INTERVALS_ICU.to_owned(),
                    access_token: SecretString::from("coach-key".to_owned()),
                    refresh_token: None,
                    token_type: API_KEY_TOKEN_TYPE.to_owned(),
                    expires_at: None,
                    scope: None,
                    provider_user_id: Some("i100".to_owned()),
                    oauth_app_client_id: None,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
            repos
                .provider_connections
                .register_connection(
                    casey.id,
                    casey.tenant,
                    INTERVALS_ICU,
                    &ConnectionType::Manual,
                    None,
                )
                .await
                .unwrap();
            Self {
                res,
                coach: casey,
                athlete: alex,
                group,
            }
        }

        fn app(&self) -> Router {
            ProviderToolRouter::build_http_app(&self.res)
        }

        /// The coach's thread bound to the athlete, opened from the app.
        async fn open_thread(&self) -> String {
            let opened =
                AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", self.athlete.id))
                    .header("authorization", &self.coach.auth)
                    .json(&json!({ "group_id": self.group }))
                    .send(ChatRoutes::routes(Arc::clone(&self.res)))
                    .await;
            assert_eq!(
                opened.status_code(),
                StatusCode::OK,
                "fixture: the coach opens a thread bound to the athlete"
            );
            opened.json::<Value>()["id"].as_str().unwrap().to_owned()
        }

        /// A chat route called with `authorization`: its status and body.
        async fn rest(&self, request: AxumTestRequest, authorization: &str) -> (StatusCode, Value) {
            let response = request
                .header("authorization", authorization)
                .send(ChatRoutes::routes(Arc::clone(&self.res)))
                .await;
            let status = response.status_code();
            let body = if status == StatusCode::NO_CONTENT {
                Value::Null
            } else {
                response.json()
            };
            (status, body)
        }

        /// The plan card's Publish button on `thread`.
        async fn publish_route(&self, thread: &str, authorization: &str) -> (StatusCode, Value) {
            self.rest(
                AxumTestRequest::post(&format!(
                    "/api/chat/conversations/{thread}/subject-plan/publish"
                ))
                .json(&json!({ "overwrite_athlete_changes": false })),
                authorization,
            )
            .await
        }

        /// The plan card's Discard button on `thread`.
        async fn discard_route(&self, thread: &str, authorization: &str) -> (StatusCode, Value) {
            self.rest(
                AxumTestRequest::delete(&format!(
                    "/api/chat/conversations/{thread}/subject-plan/draft"
                )),
                authorization,
            )
            .await
        }

        /// The coach drafts the athlete's plan from the app.
        async fn draft_from_the_app(&self) {
            let drafted = self
                .call_over("save_training_plan", save_args(), Transport::WebApp)
                .await;
            assert!(
                drafted.success,
                "fixture: the coach drafts the athlete's plan from the app: {drafted:?}"
            );
        }

        /// The grant an MCP client or an A2A agent holds for the coach.
        fn grant(&self) -> String {
            format!(
                "Bearer {}",
                common::delegated_test_token_for_tenant(
                    &self.res,
                    self.coach.id,
                    Some(self.coach.tenant.to_string())
                )
            )
        }

        /// Whether the coach holds a draft of the athlete's plan.
        async fn has_draft(&self) -> bool {
            let tenant = self.athlete.tenant.to_string();
            let athlete = self.athlete.id.to_string();
            let coach = self.coach.id.to_string();
            self.res
                .common
                .repos
                .training_plans
                .get_draft_plan(DraftOwner {
                    tenant_id: &tenant,
                    user_id: &athlete,
                    coach_user_id: &coach,
                })
                .await
                .unwrap()
                .is_some()
        }

        /// Whether the athlete has a live season.
        async fn has_live_plan(&self) -> bool {
            self.res
                .common
                .repos
                .training_plans
                .get_active_plan(
                    &self.athlete.tenant.to_string(),
                    &self.athlete.id.to_string(),
                )
                .await
                .unwrap()
                .is_some()
        }

        /// `tool` run as the coach in `conversation`, as a chat turn runs it.
        async fn in_thread<T: McpTool<dyn ToolRuntime>>(
            &self,
            tool: &T,
            conversation: &str,
            args: Value,
        ) -> Value {
            let runtime: Arc<dyn ToolRuntime> = self.res.clone();
            let ctx = ToolContext::new()
                .with_user(self.coach.id.to_string())
                .with_tenant(self.coach.tenant.to_string())
                .with_auth_method("jwt_bearer");
            CONVERSATION_ID
                .scope(
                    Some(conversation.to_owned()),
                    in_web_turn(&self.res, self.coach.id, self.coach.tenant, async {
                        tool.execute(&runtime, &ctx, args)
                            .await
                            .structured_content
                            .expect("tool result carries structured content")
                    }),
                )
                .await
        }

        /// `tool` run as the coach by the executor every door dispatches
        /// through, bound to `transport`.
        async fn call_over(
            &self,
            tool: &str,
            args: Value,
            transport: Transport,
        ) -> UniversalResponse {
            self.try_call_over(tool, args, transport)
                .await
                .expect("the executor answers in band")
        }

        /// [`Self::call_over`], with a failure raised out of band (a
        /// calendar the coach cannot reach, say) as its message.
        async fn try_call_over(
            &self,
            tool: &str,
            args: Value,
            transport: Transport,
        ) -> Result<UniversalResponse, String> {
            let runtime: Arc<dyn ToolRuntime> = self.res.clone();
            let mut executor = UniversalExecutor::new(runtime)
                .with_scopes(OAuthScope::self_grant())
                .with_transport(transport);
            if !transport.is_first_party() {
                executor = executor.with_caller_writing_reply();
            }
            executor
                .execute_tool(UniversalRequest {
                    tool_name: tool.to_owned(),
                    parameters: args,
                    user_id: self.coach.id.to_string(),
                    protocol: "test".to_owned(),
                    tenant_id: Some(self.coach.tenant.to_string()),
                })
                .await
                .map_err(|e| e.to_string())
        }
    }

    fn monday(weeks: u64) -> NaiveDate {
        let today = Utc::now().date_naive();
        let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
        this + Days::new(7 * weeks)
    }

    fn week(start: NaiveDate) -> Value {
        let days: Vec<Value> = (0..7)
            .map(|offset| {
                json!({
                    "date": (start + Days::new(offset)).format("%Y-%m-%d").to_string(),
                    "sport": "run",
                    "workout": "easy aerobic run",
                    "duration_min": 40,
                    "intensity": "Z2",
                })
            })
            .collect();
        json!({
            "week_start": start.format("%Y-%m-%d").to_string(),
            "focus": "aerobic base",
            "days": days,
        })
    }

    fn outline() -> Value {
        json!({
            "goal_race": { "name": "Spring Trail", "date": "2027-05-16", "discipline": "trail", "priority": "A" },
            "strategy": "build toward the spring trail",
        })
    }

    /// A valid save of the athlete's season: an outline and one week.
    fn save_args() -> Value {
        json!({ "athlete": ATHLETE, "outline": outline(), "weeks": [week(monday(1))] })
    }

    fn tempo_session() -> Value {
        json!({
            "name": "Coach tempo 40",
            "sport": "run",
            "intensity_distribution": "threshold",
            "structure": [
                { "label": "Warm-up", "duration_seconds": 600, "target_zone": "Z1" },
                { "label": "Tempo", "duration_seconds": 1200, "target_zone": "Z3" },
                { "label": "Cool-down", "duration_seconds": 600, "target_zone": "Z1" }
            ]
        })
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

    /// `tools/call` over `POST /mcp`: the call's `result`.
    async fn mcp_call(app: &Router, grant: &str, tool: &str, arguments: Value) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let answer = post_json(app, "/mcp", grant, body).await;
        assert!(answer["error"].is_null(), "{answer}");
        answer["result"].clone()
    }

    /// A2A `SendMessage` carrying a tool intent: the task it answers with.
    async fn a2a_call(app: &Router, grant: &str, tool: &str, parameters: Value) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": format!("m-{}", uuid::Uuid::new_v4()),
                    "role": "ROLE_USER",
                    "parts": [{ "data": { "tool_name": tool, "parameters": parameters } }]
                }
            },
        });
        let answer = post_json(app, "/a2a/jsonrpc", grant, body).await;
        assert!(answer["error"].is_null(), "{answer}");
        answer["result"]["task"].clone()
    }

    /// Whether a `tools/call` result is the external-transport refusal.
    fn mcp_refused(result: &Value) -> bool {
        result["isError"] == true && result["content"].to_string().contains(EXTERNAL)
    }

    /// Whether an A2A task failed with the external-transport refusal.
    fn a2a_refused(task: &Value) -> bool {
        task["status"]["state"] == "TASK_STATE_FAILED"
            && task["status"]["message"]["parts"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("not over this interface"))
    }

    fn reason(response: &UniversalResponse) -> Option<&str> {
        response.result.as_ref()?.get("reason")?.as_str()
    }

    /// Over `POST /mcp` and A2A — the doors an MCP client and an A2A agent
    /// use — a coach who coaches the athlete writes no draft of their plan,
    /// publishes none and discards none. What the coach reads of it, and
    /// their own plan, still answer on the same doors with the same grant.
    #[tokio::test]
    async fn a_coachs_plan_write_for_an_athlete_over_mcp_or_a2a_is_refused() {
        let w = World::new().await;
        let app = w.app();
        let grant = w.grant();

        // Controls: on each door the grant reaches a write of the coach's own
        // plan, and a read of the athlete's.
        let own = json!({ "outline": outline(), "weeks": [week(monday(1))] });
        let saved = mcp_call(&app, &grant, "save_training_plan", own.clone()).await;
        assert_ne!(
            saved["isError"], true,
            "the coach saves their own plan: {saved}"
        );
        let task = a2a_call(&app, &grant, "save_training_plan", own).await;
        assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{task}");
        let read = mcp_call(
            &app,
            &grant,
            "get_training_plan",
            json!({ "athlete": ATHLETE }),
        )
        .await;
        assert_ne!(read["isError"], true, "athlete= reads stay open: {read}");
        let task = a2a_call(
            &app,
            &grant,
            "get_training_plan",
            json!({ "athlete": ATHLETE }),
        )
        .await;
        assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{task}");

        // Every door and write is tried before any verdict, so a regression
        // names each one it reopens.
        let mut written = Vec::new();
        let saved = mcp_call(&app, &grant, "save_training_plan", save_args()).await;
        if !mcp_refused(&saved) {
            written.push(format!("save over /mcp: {saved}"));
        }
        if w.has_draft().await {
            written.push("save over /mcp wrote a draft".to_owned());
        }
        let task = a2a_call(&app, &grant, "save_training_plan", save_args()).await;
        if !a2a_refused(&task) {
            written.push(format!("save over A2A: {task}"));
        }
        if w.has_draft().await {
            written.push("save over A2A wrote a draft".to_owned());
        }

        // A draft the coach wrote in the app is not published or discarded
        // from outside.
        let drafted = w
            .call_over("save_training_plan", save_args(), Transport::WebApp)
            .await;
        assert!(
            drafted.success,
            "fixture: the coach drafts the athlete's plan from the app: {drafted:?}"
        );
        for action in ["publish", "discard"] {
            let args = json!({ "athlete": ATHLETE, "action": action });
            let over_mcp = mcp_call(&app, &grant, "publish_training_plan", args.clone()).await;
            if !mcp_refused(&over_mcp) {
                written.push(format!("{action} over /mcp: {over_mcp}"));
            }
            let over_a2a = a2a_call(&app, &grant, "publish_training_plan", args).await;
            if !a2a_refused(&over_a2a) {
                written.push(format!("{action} over A2A: {over_a2a}"));
            }
        }
        if !w.has_draft().await {
            written.push("an external client published or discarded the draft".to_owned());
        }
        if w.has_live_plan().await {
            written.push("an external client published the athlete's plan".to_owned());
        }
        assert!(
            written.is_empty(),
            "bug: an external client wrote the athlete's plan:\n{}",
            written.join("\n")
        );
    }

    /// The coach's thread bound to the athlete writes their plan with no
    /// `athlete` named, so over an external transport it is refused exactly
    /// as a named athlete is — and lands from the app.
    #[tokio::test]
    async fn a_thread_bound_to_the_athlete_writes_their_plan_only_from_dravr() {
        let w = World::new().await;
        let thread = w.open_thread().await;
        let unnamed = json!({ "outline": outline(), "weeks": [week(monday(1))] });

        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            let refused = ai_scope::serve_over(
                transport,
                w.in_thread(&SaveTrainingPlanTool, &thread, unnamed.clone()),
            )
            .await;
            assert_eq!(refused["reason"], EXTERNAL, "{transport:?}: {refused}");
        }
        assert!(
            !w.has_draft().await,
            "bug: an external client wrote the thread's athlete's plan"
        );

        let saved = ai_scope::serve_over(
            Transport::WebApp,
            w.in_thread(&SaveTrainingPlanTool, &thread, unnamed),
        )
        .await;
        assert_eq!(saved["draft"], true, "{saved}");
        assert!(
            w.has_draft().await,
            "bug: the bound thread's save from the app wrote no draft"
        );
    }

    /// The plan card's Publish and Discard routes run the draft publish and
    /// discard themselves, not through the plan tools: an API key the coach
    /// holds is refused on both, with the plan tools' reason, and the draft
    /// stays as it was. The coach's own session from the app publishes and
    /// discards on the same routes.
    #[tokio::test]
    async fn the_plan_cards_publish_and_discard_refuse_an_api_key_and_answer_the_app() {
        let w = World::new().await;
        let thread = w.open_thread().await;
        w.draft_from_the_app().await;
        let key = common::issue_test_api_key(&w.res.agent.database, w.coach.id, "coach script")
            .await
            .unwrap();

        // Both routes are tried before any verdict, so a regression names
        // each one it reopens. An API key is the whole header, not a bearer.
        let mut written = Vec::new();
        for (route, (status, body)) in [
            ("publish", w.publish_route(&thread, &key).await),
            ("discard", w.discard_route(&thread, &key).await),
        ] {
            if status != StatusCode::FORBIDDEN || body["details"]["reason"] != EXTERNAL {
                written.push(format!("{route} with an API key: {status} {body}"));
            }
        }
        if !w.has_draft().await {
            written.push("an API key published or discarded the draft".to_owned());
        }
        if w.has_live_plan().await {
            written.push("an API key published the athlete's plan".to_owned());
        }
        assert!(
            written.is_empty(),
            "bug: an API key wrote the athlete's plan through the plan card:\n{}",
            written.join("\n")
        );

        // The coach's session from the app discards the draft …
        let (status, body) = w.discard_route(&thread, &w.coach.auth).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "bug: the app's discard was refused: {body}"
        );
        assert!(
            !w.has_draft().await,
            "bug: the app's discard left the draft"
        );
        assert!(
            !w.has_live_plan().await,
            "bug: the app's discard published the plan"
        );

        // … and publishes the next one.
        w.draft_from_the_app().await;
        let (status, body) = w.publish_route(&thread, &w.coach.auth).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "bug: the app's publish was refused: {body}"
        );
        assert!(
            w.has_live_plan().await,
            "bug: the app's publish left no live plan: {body}"
        );
        assert!(
            !w.has_draft().await,
            "bug: the app's publish left the draft"
        );
    }

    /// The same writes from Dravr's own app and channels still land: the
    /// coach saves a draft and publishes it.
    #[tokio::test]
    async fn a_coachs_plan_write_for_an_athlete_from_dravrs_own_surfaces_lands() {
        for transport in [Transport::WebApp, Transport::Messaging] {
            let w = World::new().await;
            let saved = w
                .call_over("save_training_plan", save_args(), transport)
                .await;
            assert!(saved.success, "{transport:?}: {saved:?}");
            assert_eq!(saved.result.as_ref().unwrap()["draft"], true, "{saved:?}");
            assert!(
                w.has_draft().await,
                "bug: a save over {transport:?} wrote no draft"
            );

            let published = w
                .call_over(
                    "publish_training_plan",
                    json!({ "athlete": ATHLETE, "action": "publish" }),
                    transport,
                )
                .await;
            assert!(published.success, "{transport:?}: {published:?}");
            assert!(
                w.has_live_plan().await,
                "bug: a publish over {transport:?} left no live plan"
            );
        }
    }

    /// Every tool that writes for an athlete it names is refused over every
    /// external transport and never for that reason on Dravr's own, and the
    /// table is the registry's whole set of such tools: a new one fails here
    /// until it is listed, and so decided.
    #[tokio::test]
    async fn every_athlete_naming_plan_write_is_refused_over_every_external_transport() {
        let w = World::new().await;
        let registry = &w.res.mcp.tool_registry;
        let declared: BTreeSet<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| registry.may_change_data(name))
            .filter(|name| {
                // The schema as `tools/list` advertises it, so an `athlete`
                // the registry adds for a read-subject tool counts too.
                registry
                    .list_schemas_by_names(&[name])
                    .first()
                    .and_then(|schema| schema.input_schema.properties.as_ref())
                    .is_some_and(|properties| properties.contains_key("athlete"))
            })
            .map(ToOwned::to_owned)
            .collect();
        let table = plan_writes();
        let covered: BTreeSet<String> = table.iter().map(|(name, _)| (*name).to_owned()).collect();
        assert_eq!(
            declared, covered,
            "every athlete-naming write is in the table"
        );
        // The plan scope's reads declare no write, so the gate leaves them be.
        for read in PLAN_READS {
            assert!(
                registry.get(read).is_some_and(|tool| !tool
                    .capabilities()
                    .contains(ToolCapabilities::WRITES_DATA)),
                "bug: the plan read {read} declares a write"
            );
            assert!(
                !registry.may_change_data(read),
                "bug: the registry counts the plan read {read} as a write"
            );
        }

        let drafted = w
            .call_over("save_training_plan", save_args(), Transport::WebApp)
            .await;
        assert!(
            drafted.success,
            "fixture: the coach drafts the athlete's plan from the app: {drafted:?}"
        );

        let external: Vec<Transport> = TRANSPORTS
            .into_iter()
            .filter(|t| !t.is_first_party())
            .collect();
        let first_party: Vec<Transport> = TRANSPORTS
            .into_iter()
            .filter(|t| t.is_first_party())
            .collect();
        for (tool, calls) in &table {
            for args in calls {
                for &transport in &external {
                    let refused = w.call_over(tool, args.clone(), transport).await;
                    assert!(!refused.success, "{tool} over {transport:?}: {refused:?}");
                    assert_eq!(
                        reason(&refused),
                        Some(EXTERNAL),
                        "{tool} {args} over {transport:?}: {refused:?}"
                    );
                }
            }
        }
        assert!(w.has_draft().await, "no external call touched the draft");
        assert!(!w.has_live_plan().await, "no external call published");

        // The coach still reads the athlete's plan through `athlete=` on
        // every transport.
        for transport in TRANSPORTS {
            let read = w
                .call_over(
                    "get_training_plan",
                    json!({ "athlete": ATHLETE }),
                    transport,
                )
                .await;
            assert!(
                read.success,
                "get_training_plan over {transport:?}: {read:?}"
            );
        }

        for (tool, calls) in &table {
            for args in calls {
                for &transport in &first_party {
                    // The write may still fail for its own reasons here (no
                    // calendar the coach reaches, no draft left), never this one.
                    match w.try_call_over(tool, args.clone(), transport).await {
                        Ok(answered) => assert_ne!(
                            reason(&answered),
                            Some(EXTERNAL),
                            "{tool} {args} over {transport:?}: {answered:?}"
                        ),
                        Err(failure) => assert!(
                            !failure.contains("not over this interface"),
                            "{tool} {args} over {transport:?}: {failure}"
                        ),
                    }
                }
            }
        }
    }
}
