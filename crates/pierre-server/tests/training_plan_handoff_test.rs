// ABOUTME: The catalogue's plan handoffs through the tools: a taper or fuelling agent reads and extends the season another agent laid
// ABOUTME: One athlete, conversations with several agents; asserts one plan row, week authors, and the refusal over another agent's outline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The athlete has one season plan, whichever agent laid it. An endurance
//! agent lays it; the taper agent the catalogue hands off to reads that same
//! plan, adds its weeks to it, and may re-lay its outline only when the
//! athlete asked — `replace_season`. Every assertion reads the rows back
//! through the repository, so a tool that answered well and stored a second
//! plan would fail here.

mod common;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::FactKind;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const ENDURANCE_TITLE: &str = "Endurance Agent";
const TAPER_TITLE: &str = "Taper Builder";
const FUELLING_TITLE: &str = "Fuelling Agent";

/// One athlete, their tenant, and a conversation with each of three agents.
struct Athlete {
    resources: Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
    endurance: String,
    taper: String,
    fuelling: String,
    endurance_dm: String,
    taper_dm: String,
    fuelling_dm: String,
}

async fn agent_titled(
    resources: &ServerContext,
    user: Uuid,
    tenant: TenantId,
    title: &str,
) -> String {
    resources
        .common
        .repos
        .agents
        .create_system_agent(
            user,
            tenant,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: format!("You are the {title}."),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Global,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

async fn conversation_with(
    resources: &ServerContext,
    user: Uuid,
    tenant: TenantId,
    agent: &str,
) -> String {
    resources
        .common
        .repos
        .chat
        .create_conversation(
            &user.to_string(),
            tenant,
            "handoff",
            "gemini-2.0-flash",
            Some(agent),
            None,
        )
        .await
        .unwrap()
        .id
}

async fn athlete() -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user)
        .await
        .unwrap()
        .first()
        .expect("the athlete has a tenant")
        .id;
    let endurance = agent_titled(&resources, user, tenant, ENDURANCE_TITLE).await;
    let taper = agent_titled(&resources, user, tenant, TAPER_TITLE).await;
    let fuelling = agent_titled(&resources, user, tenant, FUELLING_TITLE).await;
    let endurance_dm = conversation_with(&resources, user, tenant, &endurance).await;
    let taper_dm = conversation_with(&resources, user, tenant, &taper).await;
    let fuelling_dm = conversation_with(&resources, user, tenant, &fuelling).await;
    Athlete {
        resources,
        user,
        tenant,
        endurance,
        taper,
        fuelling,
        endurance_dm,
        taper_dm,
        fuelling_dm,
    }
}

fn tool_context(user: Uuid, tenant: TenantId) -> ToolContext {
    ToolContext::new()
        .with_user(user.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer")
}

fn structured(response: &ToolResponse) -> Value {
    response
        .structured_content
        .clone()
        .expect("tool result carries structured content")
}

/// Call a plan tool as the athlete, inside `conversation` when one is given
/// (a chat turn) or with none at all (a direct MCP call).
async fn call<T: McpTool<dyn ToolRuntime>>(
    a: &Athlete,
    tool: T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = a.resources.clone();
    let ctx = tool_context(a.user, a.tenant);
    let response = CONVERSATION_ID
        .scope(conversation.map(ToOwned::to_owned), async {
            tool.execute(&runtime, &ctx, args).await
        })
        .await;
    structured(&response)
}

fn outline(race: &str) -> Value {
    json!({
        "goal_race": { "name": race, "date": "2026-12-06", "discipline": "trail", "priority": "A" },
        "strategy": format!("build to {race}"),
        "phases": [
            {"kind": "build", "start": "2026-10-05", "weeks": 6, "intent": "long runs grow"},
            {"kind": "taper", "start": "2026-11-16", "weeks": 3, "intent": "shed fatigue"}
        ]
    })
}

fn week(week_start: &str, workout: &str) -> Value {
    json!({
        "week_start": week_start,
        "focus": workout,
        "days": [
            { "date": week_start, "sport": "run", "workout": workout, "duration_min": 60, "intensity": "Z2" }
        ]
    })
}

fn error_text(payload: &Value) -> String {
    payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected a refusal, got: {payload}"))
        .to_owned()
}

fn ok(payload: &Value) -> &Value {
    assert!(
        payload.get("error").is_none(),
        "the call must succeed: {payload}"
    );
    payload
}

fn plan_id(payload: &Value) -> String {
    payload
        .get("plan_id")
        .or_else(|| payload.pointer("/plan/id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a plan id: {payload}"))
        .to_owned()
}

/// The endurance agent lays the season, with one week of its own.
async fn endurance_lays_the_season(a: &Athlete) -> String {
    let saved = call(
        a,
        SaveTrainingPlanTool,
        Some(&a.endurance_dm),
        json!({ "outline": outline("Harricana 80"), "weeks": [week("2026-10-05", "endurance long run")] }),
    )
    .await;
    plan_id(ok(&saved))
}

/// The `authors` entry for `agent_id`.
fn author<'a>(fetched: &'a Value, agent_id: &str) -> &'a Value {
    fetched
        .get("authors")
        .and_then(Value::as_array)
        .and_then(|authors| {
            authors
                .iter()
                .find(|entry| entry.get("agent_id").and_then(Value::as_str) == Some(agent_id))
        })
        .unwrap_or_else(|| panic!("{agent_id} among the authors: {fetched}"))
}

/// Who wrote each active week of the season, by `week_start`.
async fn week_authors(a: &Athlete, plan: &str) -> Vec<(String, Option<String>)> {
    a.resources
        .common
        .repos
        .training_plans
        .list_plan_weeks(&a.tenant.to_string(), &a.user.to_string(), plan, false)
        .await
        .unwrap()
        .into_iter()
        .map(|w| (w.week_start, w.author_agent_id))
        .collect()
}

/// Every active plan row the athlete holds — the index allows one.
async fn active_plan_rows(a: &Athlete) -> i64 {
    let sql = "SELECT COUNT(*) FROM training_plans \
               WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'";
    match a.resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(sql)
            .bind(a.tenant.to_string())
            .bind(a.user.to_string())
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query_scalar(sql)
            .bind(a.tenant.to_string())
            .bind(a.user.to_string())
            .fetch_one(pg.pool())
            .await
            .unwrap(),
    }
}

async fn goal_fact_ids(a: &Athlete) -> Vec<String> {
    let mut ids: Vec<String> = a
        .resources
        .common
        .repos
        .memory
        .list_user_facts(
            a.tenant,
            &a.user.to_string(),
            None,
            Some(FactKind::Goal),
            200,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|fact| fact.id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn the_taper_conversation_reads_the_season_the_endurance_agent_laid() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;

    let fetched = call(&a, GetTrainingPlanTool, Some(&a.taper_dm), json!({})).await;
    assert_eq!(
        plan_id(ok(&fetched)),
        season,
        "the taper agent reads the same plan"
    );
    let laid = author(&fetched, &a.endurance);
    assert_eq!(
        laid.get("name").and_then(Value::as_str),
        Some(ENDURANCE_TITLE)
    );
    assert_eq!(laid.get("laid_the_season"), Some(&json!(true)));
    assert_eq!(laid.get("is_you"), Some(&json!(false)));
    assert_eq!(
        fetched
            .pointer("/plan/author_agent_id")
            .and_then(Value::as_str),
        Some(a.endurance.as_str())
    );

    // The endurance agent reading its own season is told it laid it.
    let own = call(&a, GetTrainingPlanTool, Some(&a.endurance_dm), json!({})).await;
    assert_eq!(author(&own, &a.endurance).get("is_you"), Some(&json!(true)));
}

#[tokio::test]
async fn a_taper_weeks_only_save_lands_on_the_same_season_authored_by_taper() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;

    let saved = call(
        &a,
        SaveTrainingPlanTool,
        Some(&a.taper_dm),
        json!({ "weeks": [week("2026-11-16", "taper opener"), week("2026-11-23", "taper sharpen")] }),
    )
    .await;
    assert_eq!(plan_id(ok(&saved)), season);
    assert!(
        saved.get("superseded_plan_id").is_some_and(Value::is_null),
        "{saved}"
    );
    assert_eq!(
        week_authors(&a, &season).await,
        vec![
            ("2026-10-05".to_owned(), Some(a.endurance.clone())),
            ("2026-11-16".to_owned(), Some(a.taper.clone())),
            ("2026-11-23".to_owned(), Some(a.taper.clone())),
        ]
    );
    assert_eq!(active_plan_rows(&a).await, 1, "the athlete holds one plan");

    // Both agents now appear among the season's authors.
    let fetched = call(&a, GetTrainingPlanTool, Some(&a.taper_dm), json!({})).await;
    let taper = author(&fetched, &a.taper);
    assert_eq!(taper.get("is_you"), Some(&json!(true)));
    assert_eq!(taper.get("laid_the_season"), Some(&json!(false)));
    assert_eq!(taper.get("name").and_then(Value::as_str), Some(TAPER_TITLE));
}

#[tokio::test]
async fn a_taper_outline_save_is_refused_naming_the_endurance_agent_and_writes_nothing() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;
    let facts_before = goal_fact_ids(&a).await;

    let refused = call(
        &a,
        SaveTrainingPlanTool,
        Some(&a.taper_dm),
        json!({ "outline": outline("Taper Tune-up"), "weeks": [week("2026-11-16", "taper opener")] }),
    )
    .await;
    let err = error_text(&refused);
    assert!(
        err.contains(&format!("{ENDURANCE_TITLE} laid out the athlete's season"))
            && err.contains("replace_season")
            && err.contains("without an outline"),
        "the refusal names who laid the season and both ways forward: {err}"
    );

    let fetched = call(&a, GetTrainingPlanTool, Some(&a.taper_dm), json!({})).await;
    assert_eq!(plan_id(&fetched), season);
    assert_eq!(
        fetched
            .pointer("/plan/goal_race/name")
            .and_then(Value::as_str),
        Some("Harricana 80")
    );
    assert_eq!(
        week_authors(&a, &season).await,
        vec![("2026-10-05".to_owned(), Some(a.endurance.clone()))],
        "not even the weeks of a refused save are written"
    );
    assert_eq!(active_plan_rows(&a).await, 1);
    assert_eq!(
        goal_fact_ids(&a).await,
        facts_before,
        "no goal fact is written either"
    );
}

#[tokio::test]
async fn with_replace_season_the_taper_outline_supersedes_and_carries_the_authors() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;
    call(
        &a,
        SaveTrainingPlanTool,
        Some(&a.fuelling_dm),
        json!({ "weeks": [week("2026-10-12", "fuelling rehearsal")] }),
    )
    .await;

    let relaid = call(
        &a,
        SaveTrainingPlanTool,
        Some(&a.taper_dm),
        json!({ "outline": outline("Taper Tune-up"), "replace_season": true }),
    )
    .await;
    let relaid_id = plan_id(ok(&relaid));
    assert_ne!(relaid_id, season);
    assert_eq!(
        relaid.get("superseded_plan_id").and_then(Value::as_str),
        Some(season.as_str())
    );

    let fetched = call(&a, GetTrainingPlanTool, Some(&a.endurance_dm), json!({})).await;
    assert_eq!(plan_id(&fetched), relaid_id);
    assert_eq!(
        fetched
            .pointer("/plan/author_agent_id")
            .and_then(Value::as_str),
        Some(a.taper.as_str()),
        "the outline is authored by the agent that re-laid it"
    );
    assert_eq!(
        week_authors(&a, &relaid_id).await,
        vec![
            ("2026-10-05".to_owned(), Some(a.endurance.clone())),
            ("2026-10-12".to_owned(), Some(a.fuelling.clone())),
        ],
        "carried weeks keep their authors"
    );
    assert_eq!(active_plan_rows(&a).await, 1);
}

#[tokio::test]
async fn an_agnostic_season_is_re_laid_without_the_flag() {
    let a = athlete().await;
    // A direct MCP call that names no agent lays an agnostic season.
    let direct = call(
        &a,
        SaveTrainingPlanTool,
        None,
        json!({ "outline": outline("Open Season") }),
    )
    .await;
    let agnostic = plan_id(ok(&direct));
    let fetched = call(&a, GetTrainingPlanTool, None, json!({})).await;
    assert!(
        fetched
            .pointer("/plan/author_agent_id")
            .is_some_and(Value::is_null),
        "{fetched}"
    );
    assert!(
        fetched.get("authors").is_none(),
        "no agent to name: {fetched}"
    );

    let taken = call(
        &a,
        SaveTrainingPlanTool,
        Some(&a.taper_dm),
        json!({ "outline": outline("Taper Tune-up") }),
    )
    .await;
    assert_eq!(
        ok(&taken).get("superseded_plan_id").and_then(Value::as_str),
        Some(agnostic.as_str())
    );
    assert_eq!(active_plan_rows(&a).await, 1);
}

#[tokio::test]
async fn a_direct_call_with_no_agent_is_refused_over_an_agent_laid_season() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;

    let refused = call(
        &a,
        SaveTrainingPlanTool,
        None,
        json!({ "outline": outline("Direct") }),
    )
    .await;
    let err = error_text(&refused);
    assert!(
        err.contains(ENDURANCE_TITLE) && err.contains("replace_season"),
        "{err}"
    );
    let fetched = call(&a, GetTrainingPlanTool, None, json!({})).await;
    assert_eq!(plan_id(&fetched), season);

    // A direct call's weeks still attach to the season, authored by no one.
    let weeks = call(
        &a,
        SaveTrainingPlanTool,
        None,
        json!({ "weeks": [week("2026-10-19", "direct week")] }),
    )
    .await;
    assert_eq!(plan_id(ok(&weeks)), season);
    assert_eq!(
        week_authors(&a, &season).await,
        vec![
            ("2026-10-05".to_owned(), Some(a.endurance.clone())),
            ("2026-10-19".to_owned(), None),
        ]
    );
}

#[tokio::test]
async fn a_fuelling_conversation_reads_the_season() {
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;

    let fetched = call(&a, GetTrainingPlanTool, Some(&a.fuelling_dm), json!({})).await;
    assert_eq!(plan_id(ok(&fetched)), season);
    let authors = fetched
        .get("authors")
        .and_then(Value::as_array)
        .expect("the season names its author");
    assert_eq!(authors.len(), 1, "{fetched}");
    assert_eq!(authors[0].get("is_you"), Some(&json!(false)));
    assert_eq!(
        fetched
            .pointer("/weeks/0/author_agent_id")
            .and_then(Value::as_str),
        Some(a.endurance.as_str())
    );
}

fn save_request(a: &Athlete, params: Value) -> UniversalRequest {
    UniversalRequest {
        tool_name: "save_training_plan".to_owned(),
        parameters: params,
        user_id: a.user.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(a.tenant.to_string()),
    }
}

/// A `@taper` turn in the endurance conversation: the executor is bound to
/// the endurance conversation and to the taper agent the turn answers as.
#[tokio::test]
async fn an_executor_bound_to_the_endurance_conversation_with_the_taper_turn_agent_writes_as_taper()
{
    let a = athlete().await;
    let season = endurance_lays_the_season(&a).await;
    let executor = UniversalToolExecutor::new(a.resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_conversation_id(a.endurance_dm.clone())
        .with_conversation_tenant(a.tenant.as_uuid())
        .with_turn_agent(Some(a.taper.clone()));

    let saved = executor
        .execute_tool(save_request(
            &a,
            json!({ "weeks": [week("2026-11-16", "taper via mention")] }),
        ))
        .await
        .unwrap();
    assert!(saved.success, "{:?}", saved.error);
    assert_eq!(
        week_authors(&a, &season).await,
        vec![
            ("2026-10-05".to_owned(), Some(a.endurance.clone())),
            ("2026-11-16".to_owned(), Some(a.taper.clone())),
        ],
        "the turn's agent wins over the conversation's"
    );

    // The executor raises an invalid-input refusal as invalid parameters.
    let refused = executor
        .execute_tool(save_request(
            &a,
            json!({ "outline": outline("Taper Tune-up") }),
        ))
        .await;
    let text = match refused {
        Ok(response) => {
            assert!(!response.success, "{response:?}");
            format!("{:?} {:?}", response.error, response.result)
        }
        Err(refusal) => refusal.to_string(),
    };
    assert!(
        text.contains(ENDURANCE_TITLE) && text.contains("replace_season"),
        "the taper turn may not re-lay the endurance season unasked: {text}"
    );
}
