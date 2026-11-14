// ABOUTME: What an external client on /mcp reads and is offered: no medical facts, no first-party tools, uploads elected like any source
// ABOUTME: Drives POST /mcp with a delegated grant against a seeded athlete, beside the first-party answer to the same call

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The public tool surface
//!
//! A scripted external client on an uploads-only account found these on
//! production: PAR-Q flags in `recall_user_memory` and `export_dossier`, tools
//! listed and then refused, and a false "no provider connected" beside 28
//! uploaded activities. Each test here drives the real entry point and fails
//! without the change it pins.

use crate::common;

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use pierre_core::models::{DataSource, DeviceType, StoredHealthMetrics, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::WellnessWriteMode;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_memory::{FactKind, FactSource};
use pierre_providers::ai_scope;
use pierre_services::parq;
use pierre_test_support::fit::FitWorkout;
use pierre_tool_runtime::first_party_tools::FIRST_PARTY_SURFACE_ERROR_CODE;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// Design §7's tools kept on Dravr's own surfaces, beyond the coach family:
/// agent store and CRUD, memory and self-steering; and the Claude directory
/// pre-submission set (carnet#1565): the agent reads, the playbook list, the
/// configuration reads and the two prescriptive nutrition tools.
pub(crate) const HIDDEN_TOOLS: [&str; 29] = [
    "create_agent",
    "update_agent",
    "activate_agent",
    "deactivate_agent",
    "hide_agent",
    "show_agent",
    "delete_agent",
    "toggle_agent_favorite",
    "install_agent_from_store",
    "browse_agent_store",
    "search_agent_store",
    "commitment_create",
    "commitment_cancel",
    "remember_fact",
    "forget_playbook",
    "list_agents",
    "get_agent",
    "search_agents",
    "get_active_agent",
    "list_hidden_agents",
    "list_coaching_playbooks",
    "get_configuration_catalog",
    "get_configuration_profiles",
    "get_user_configuration",
    "validate_configuration",
    "get_fitness_config",
    "list_fitness_configs",
    "calculate_daily_nutrition",
    "get_nutrient_timing",
];

/// The refusals the external client read beside uploaded activities.
const FALSE_REFUSALS: [&str; 3] = [
    "No fitness provider connected",
    "messaging features",
    "connect_provider",
];

struct Athlete {
    id: Uuid,
    tenant: TenantId,
    session: String,
}

/// An athlete on the top plan, so a plan refusal never stands in for the
/// behaviour under test.
async fn athlete(res: &Arc<ServerContext>) -> Athlete {
    let email = format!("public-surface-{}@example.com", Uuid::new_v4());
    let (id, user, tenant) =
        common::create_test_user_with_plan(&res.agent.database, &email, "enterprise")
            .await
            .unwrap();
    let session = common::generate_test_token(res, &user).await;
    Athlete {
        id,
        tenant,
        session,
    }
}

fn grant(res: &ServerContext, who: &Athlete) -> String {
    format!(
        "Bearer {}",
        common::delegated_test_token_for_tenant(res, who.id, Some(who.tenant.to_string()))
    )
}

async fn rpc(app: &Router, authorization: &str, method: &str, params: Value) -> Value {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("authorization", authorization)
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

async fn mcp_call(app: &Router, authorization: &str, name: &str, arguments: Value) -> Value {
    rpc(
        app,
        authorization,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
    .await
}

async fn listed(app: &Router, authorization: &str) -> BTreeSet<String> {
    rpc(app, authorization, "tools/list", json!({})).await["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The same tool on Dravr's own web app.
async fn in_app(
    res: &Arc<ServerContext>,
    who: &Athlete,
    tool: &str,
    parameters: Value,
) -> UniversalResponse {
    let runtime: Arc<dyn ToolRuntime> = res.clone();
    UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: who.id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(who.tenant.to_string()),
        })
        .await
        .expect("the executor answers in band")
}

async fn seed_parq(res: &ServerContext, who: &Athlete) {
    let yes = vec!["heart_condition".to_owned(), "medication".to_owned()];
    let raised = parq::persist_parq_flags(
        res.common.repos.memory.as_ref(),
        who.tenant,
        &who.id.to_string(),
        &yes,
    )
    .await
    .unwrap();
    assert_eq!(raised, 2);
}

fn assert_no_medical_fact(answer: &Value) {
    let text = answer.to_string();
    for leak in ["heart_condition", "medication", "parq_yes"] {
        assert!(!text.contains(leak), "bug: {leak} left over /mcp: {text}");
    }
}

#[tokio::test]
async fn recall_over_mcp_never_returns_a_medical_fact() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    seed_parq(&res, &who).await;
    let app = ProviderToolRouter::build_http_app(&res);
    let authorization = grant(&res, &who);

    for arguments in [json!({}), json!({"kind": "medical"})] {
        let answer = mcp_call(&app, &authorization, "recall_user_memory", arguments).await;
        assert_eq!(answer["result"]["isError"], false, "{answer}");
        assert_eq!(answer["result"]["structuredContent"]["count"], 0);
        assert_no_medical_fact(&answer);
    }

    // The first-party path still reads them, or this would pass on an empty row.
    let own = in_app(&res, &who, "recall_user_memory", json!({})).await;
    assert!(own.success, "{own:?}");
    let body = own.result.unwrap().to_string();
    assert!(body.contains("heart_condition"), "{body}");
    assert!(body.contains("medication"), "{body}");
}

#[tokio::test]
async fn export_dossier_over_mcp_has_no_medical_bucket() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    seed_parq(&res, &who).await;
    let app = ProviderToolRouter::build_http_app(&res);

    let answer = mcp_call(&app, &grant(&res, &who), "export_dossier", json!({})).await;
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    assert!(
        answer["result"]["structuredContent"]["medical"].is_null(),
        "{answer}"
    );
    assert_no_medical_fact(&answer);

    let own = in_app(&res, &who, "export_dossier", json!({})).await;
    assert!(own.success, "{own:?}");
    let body = own.result.unwrap();
    assert_eq!(body["medical"].as_array().map(Vec::len), Some(2), "{body}");
}

/// The read every prompt and tool goes through: an external read of the fact
/// lists holds no medical row however it asks, so the A2A text turn's context,
/// composed from the same dossier under `Transport::A2a`, has none either.
#[tokio::test]
async fn every_external_read_of_the_fact_lists_leaves_medical_rows_out() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    seed_parq(&res, &who).await;
    let memory = &res.common.repos.memory;
    let user = who.id.to_string();

    for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
        let readable = ai_scope::serve_over(transport, async { ai_scope::readable_policy() }).await;
        assert_eq!(readable, TransportPolicy::AnyTransport, "{transport:?}");
        let dossier = ai_scope::serve_over(transport, async {
            res.common
                .repos
                .dossier
                .compose_dossier(
                    who.tenant,
                    who.id,
                    ai_scope::readable_policy(),
                    &ai_scope::admit_derived,
                )
                .await
                .unwrap()
        })
        .await;
        assert!(dossier.medical.is_empty(), "{transport:?}");
    }

    let external = TransportPolicy::AnyTransport;
    let by_kind = memory
        .list_user_facts(
            who.tenant,
            &user,
            None,
            Some(FactKind::Medical),
            10,
            external,
        )
        .await
        .unwrap();
    assert!(by_kind.is_empty(), "{by_kind:?}");
    let all = memory
        .list_user_facts(who.tenant, &user, None, None, 10, external)
        .await
        .unwrap();
    assert!(all.is_empty(), "{all:?}");
    let by_source = memory
        .list_user_facts_by_source(who.tenant, &user, FactSource::Onboarding, 10, external)
        .await
        .unwrap();
    assert!(by_source.is_empty(), "{by_source:?}");

    // The nutrition gate and Dravr's own prompt read them all the same.
    let own = memory
        .list_user_facts(
            who.tenant,
            &user,
            None,
            Some(FactKind::Medical),
            10,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(own.len(), 2);
}

#[tokio::test]
async fn blood_pressure_and_glucose_stay_off_external_transports() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let source_id = res
        .common
        .repos
        .data_sources
        .upsert_data_source(
            &who.tenant,
            &DataSource {
                id: String::new(),
                user_id: who.id.to_string(),
                provider: "upload".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let date = Utc::now() - Duration::days(1);
    let reading = StoredHealthMetrics {
        id: String::new(),
        user_id: who.id.to_string(),
        data_source_id: source_id,
        date: date.date_naive(),
        weight_kg: Some(72.5),
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: Some(141),
        diastolic_bp: Some(92),
        blood_glucose: Some(118.0),
        vo2_max: None,
        source_name: "upload".to_owned(),
        recorded_at: date,
    };
    res.common
        .repos
        .health_snapshots
        .upsert_health_snapshot(
            &who.tenant,
            &reading,
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&res);

    let answer = mcp_call(&app, &grant(&res, &who), "get_health_snapshots", json!({})).await;
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let snapshots = &answer["result"]["structuredContent"]["snapshots"];
    assert_eq!(snapshots.as_array().map(Vec::len), Some(1), "{answer}");
    assert_eq!(snapshots[0]["weight_kg"], 72.5, "{answer}");
    for clinical in ["systolic_bp", "diastolic_bp", "blood_glucose"] {
        assert!(snapshots[0][clinical].is_null(), "{clinical}: {answer}");
    }

    let own = in_app(&res, &who, "get_health_snapshots", json!({})).await;
    let body = own.result.unwrap();
    assert_eq!(body["snapshots"][0]["systolic_bp"], 141, "{body}");
}

#[tokio::test]
async fn the_first_party_tools_are_neither_listed_nor_run_over_mcp() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let app = ProviderToolRouter::build_http_app(&res);
    let authorization = grant(&res, &who);

    let names = listed(&app, &authorization).await;
    assert!(names.contains("verify_claim"), "D14 keeps it: {names:?}");
    assert!(names.contains("recall_user_memory"), "{names:?}");
    assert!(names.contains("export_dossier"), "{names:?}");
    for hidden in HIDDEN_TOOLS {
        assert!(!names.contains(hidden), "{hidden} is listed");
        let refused = mcp_call(&app, &authorization, hidden, json!({})).await;
        assert_eq!(refused["result"]["isError"], true, "{hidden}: {refused}");
        let text = refused["result"]["content"].to_string();
        assert!(
            text.contains(FIRST_PARTY_SURFACE_ERROR_CODE),
            "{hidden}: {refused}"
        );
    }
}

/// `remember_fact` is the write path a free-text medical fact had: refused
/// externally before anything is stored.
#[tokio::test]
async fn an_external_client_cannot_store_a_medical_fact() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let app = ProviderToolRouter::build_http_app(&res);

    // Arguments the tool would accept: only the refusal stops the write.
    let answer = mcp_call(
        &app,
        &grant(&res, &who),
        "remember_fact",
        json!({
            "kind": "medical",
            "predicate_code": "states",
            "object": "takes a beta blocker",
            "confidence": 0.9,
        }),
    )
    .await;
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    assert!(
        answer["result"]["content"]
            .to_string()
            .contains(FIRST_PARTY_SURFACE_ERROR_CODE),
        "{answer}"
    );
    // Outside a conversation the tool writes a draft, so read both.
    let memory = &res.common.repos.memory;
    let user = who.id.to_string();
    let readable = TransportPolicy::FirstPartyOnly;
    let stored = memory
        .list_user_facts(who.tenant, &user, None, None, 10, readable)
        .await
        .unwrap();
    assert!(stored.is_empty(), "{stored:?}");
    let drafts = memory
        .list_draft_user_facts(who.tenant, &user, 10, readable)
        .await
        .unwrap();
    assert!(drafts.is_empty(), "{drafts:?}");
}

#[tokio::test]
async fn a_global_admin_holds_the_public_profile_on_mcp() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let app = ProviderToolRouter::build_http_app(&res);
    let authorization = grant(&res, &who);
    let as_athlete = listed(&app, &authorization).await;

    res.agent
        .database
        .repositories()
        .users
        .set_admin_status(who.id, true)
        .await
        .unwrap();
    let as_admin = listed(&app, &authorization).await;
    assert_eq!(
        as_admin, as_athlete,
        "the listing must not move with the role"
    );
    assert!(
        !as_admin.iter().any(|name| name.starts_with("admin_")),
        "{as_admin:?}"
    );

    // An API key carries the self grant, `admin` scope included: the listing is
    // the same for it too.
    let key = common::issue_test_api_key(&res.agent.database, who.id, "admin key")
        .await
        .unwrap();
    assert_eq!(listed(&app, &format!("Bearer {key}")).await, as_athlete);
}

#[tokio::test]
async fn the_a2a_card_offers_none_of_the_first_party_tools() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let app = ProviderToolRouter::build_http_app(&res);
    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("A2A-Version", "1.0")
        .header("authorization", grant(&res, &who))
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": "GetExtendedAgentCard", "params": {}})
                .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let card: Value = serde_json::from_slice(&bytes).unwrap();
    let skills: BTreeSet<String> = card["result"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(skills.contains("get_activities"), "{skills:?}");
    for hidden in HIDDEN_TOOLS {
        assert!(!skills.contains(hidden), "{hidden} is a skill");
    }
}

/// 07:00 UTC `days_ago` days before today.
fn morning(days_ago: i64) -> DateTime<Utc> {
    let day = Utc::now().date_naive() - Duration::days(days_ago);
    Utc.from_utc_datetime(&day.and_time(NaiveTime::from_hms_opt(7, 0, 0).unwrap()))
}

/// Upload the ride of `days_ago` through the athlete's own route; its id.
async fn upload_ride(res: &Arc<ServerContext>, who: &Athlete, days_ago: i64) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/me/activities/upload")
        .header("authorization", format!("Bearer {}", who.session))
        .header("content-type", "application/octet-stream")
        .body(Body::from(
            FitWorkout::ride(morning(days_ago), 1_800).encode(),
        ))
        .unwrap();
    let response = athlete_home_routes()
        .with_state(Arc::clone(res))
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{status}: {body}");
    body["activities"][0]["id"].as_str().unwrap().to_owned()
}

/// Every tool the external client read a false refusal from, none naming a
/// provider: an uploads-only athlete has a source, and it answers.
#[tokio::test]
async fn an_uploads_only_athlete_is_answered_from_their_uploads_over_mcp() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    let first = upload_ride(&res, &who, 3).await;
    let second = upload_ride(&res, &who, 1).await;
    let app = ProviderToolRouter::build_http_app(&res);
    let authorization = grant(&res, &who);

    let by_id = json!({"activity_id": second});
    let cases = [
        ("get_stats", json!({})),
        ("calculate_fitness_score", json!({})),
        ("analyze_activity", by_id.clone()),
        ("get_activity_intelligence", by_id.clone()),
        (
            "compare_activities",
            json!({"activity_id": second, "compare_activity_id": first}),
        ),
        ("calculate_metrics", by_id.clone()),
        ("suggest_rest_day", json!({})),
        ("calculate_recovery_score", json!({})),
        ("generate_recommendations", json!({})),
        ("predict_performance", json!({})),
        ("suggest_goals", json!({})),
        ("extract_activity_streams", by_id.clone()),
        ("export_intervals", by_id.clone()),
        ("export_routes", by_id.clone()),
        ("export_latest_snapshot", json!({})),
    ];
    let mut wrong = Vec::new();
    for (tool, arguments) in cases {
        let answer = mcp_call(&app, &authorization, tool, arguments).await;
        let text = answer.to_string();
        if answer["result"]["isError"] != false {
            wrong.push(format!("{tool} failed: {text}"));
        }
        if let Some(refusal) = FALSE_REFUSALS.iter().find(|r| text.contains(**r)) {
            wrong.push(format!("{tool}: \"{refusal}\" in {text}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The elected source is named, so the score is traced to the uploads.
#[tokio::test]
async fn the_fitness_score_says_it_read_the_uploads() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    upload_ride(&res, &who, 2).await;
    let app = ProviderToolRouter::build_http_app(&res);

    let answer = mcp_call(
        &app,
        &grant(&res, &who),
        "calculate_fitness_score",
        json!({}),
    )
    .await;
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    assert_eq!(
        answer["result"]["structuredContent"]["providers_used"]["activity_provider"], "upload",
        "{answer}"
    );
}

/// A history shorter than the chronic warm-up has no daily row to vouch for,
/// and the answer says so instead of a bare `days: []`.
#[tokio::test]
async fn a_short_upload_history_explains_its_empty_training_history() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    for days_ago in [38, 20, 3] {
        upload_ride(&res, &who, days_ago).await;
    }
    let app = ProviderToolRouter::build_http_app(&res);

    let answer = mcp_call(&app, &grant(&res, &who), "get_training_history", json!({})).await;
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let result = &answer["result"]["structuredContent"];
    assert_eq!(result["days"], json!([]), "{answer}");
    let note = result["coverage_note"].as_str().expect("the reason");
    assert!(note.contains("Stored history only reaches back"), "{note}");
}

/// A history past the warm-up reads its rows, and carries no note.
#[tokio::test]
async fn a_deep_upload_history_reads_its_rows_over_mcp() {
    let res = common::create_test_server_resources().await.unwrap();
    let who = athlete(&res).await;
    for days_ago in [100, 70, 40, 10, 2] {
        upload_ride(&res, &who, days_ago).await;
    }
    let app = ProviderToolRouter::build_http_app(&res);

    let answer = mcp_call(
        &app,
        &grant(&res, &who),
        "get_training_history",
        json!({"from": (Utc::now() - Duration::days(14)).format("%Y-%m-%d").to_string()}),
    )
    .await;
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let result = &answer["result"]["structuredContent"];
    assert!(
        result["days"]
            .as_array()
            .is_some_and(|days| !days.is_empty()),
        "{answer}"
    );
    assert!(result["coverage_note"].is_null(), "{answer}");
}
