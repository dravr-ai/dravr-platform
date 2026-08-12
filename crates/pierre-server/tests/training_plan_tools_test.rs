// ABOUTME: Integration tests for get/save_training_plan MCP tools + prompt injection
// ABOUTME: Content-asserting: real plan payloads through the executor, goal-fact write-back, prompt text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::{
    Activity, ActivityBuilder, GuidedFlow, OnboardingState, Pillar, SportType, TenantId,
};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_llm::FunctionDeclaration;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::tool_execution::generate_tool_catalog;
use pierre_tool_runtime::McpTool;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

mod common;

// ============================================================================
// Notify-event capture
// ============================================================================

/// One `tracing` event with its structured fields, so a test can assert on the
/// `notify` events the save emits rather than only on what it returns.
#[derive(Clone, Debug)]
struct CapturedEvent {
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent {
                fields: visitor.fields,
            });
        }
    }
}

/// Capture every event emitted on this thread until the guard is dropped.
fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("plan_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let user_tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
}

fn make_request(
    tool: &str,
    params: Value,
    user_id: Uuid,
    tenant_id: Option<&str>,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: tenant_id.map(str::to_owned),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// The plan Raph asked for on 2026-07-11: outline to Aug 8 + two detailed
/// weeks. Day content is concrete so a returns-empty stub cannot pass.
fn full_plan_payload() -> Value {
    json!({
        "coach_id": "endurance-coach",
        "outline": {
            "goal_race": {
                "name": "Big Red",
                "date": "2026-08-08",
                "discipline": "gravel",
                "priority": "A"
            },
            "strategy": "rest week done after Buckland; rebuild volume 2 weeks, race-specific tempo, taper into Aug 8",
            "blocks": [
                {"phase": "build", "start": "2026-07-13", "weeks": 3, "intent": "volume back up, one moderate day/week", "target_hours": 9.0},
                {"phase": "taper", "start": "2026-08-03", "weeks": 1, "intent": "shorter, sharper, more rest"}
            ]
        },
        "weeks": [
            {
                "week_start": "2026-07-13",
                "focus": "volume back up",
                "days": [
                    {"date": "2026-07-13", "sport": "rest", "workout": "off — legs up"},
                    {"date": "2026-07-14", "sport": "gravel", "workout": "tempo 3x8min", "duration_min": 60, "intensity": "3x8min @ 88-93% FTP"},
                    {"date": "2026-07-15", "sport": "mtb", "workout": "endurance, low HR on climbs", "duration_min": 105, "intensity": "Z2"}
                ]
            },
            {
                "week_start": "2026-07-20",
                "focus": "moderate build",
                "days": [
                    {"date": "2026-07-20", "sport": "rest", "workout": "off"},
                    {"date": "2026-07-22", "sport": "gravel", "workout": "long endurance", "duration_min": 150, "intensity": "Z2"}
                ]
            }
        ]
    })
}

#[tokio::test]
async fn save_full_plan_then_get_roundtrip_with_goal_fact_writeback() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);
    let result = save.result.expect("save result");
    assert_eq!(result["weeks_saved"], 2);
    let plan_id = result["plan_id"].as_str().expect("plan_id").to_owned();
    let goal_fact_id = result["goal_fact_id"]
        .as_str()
        .expect("goal-fact write-back must link a fact id")
        .to_owned();
    assert!(!goal_fact_id.is_empty());

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(get.success, "get failed: {:?}", get.error);
    let fetched = get.result.expect("get result");
    assert_eq!(fetched["plan"]["id"].as_str(), Some(plan_id.as_str()));
    assert_eq!(fetched["plan"]["goal_race"]["name"], "Big Red");
    assert_eq!(fetched["plan"]["goal_race"]["date"], "2026-08-08");
    assert_eq!(fetched["plan"]["blocks"][0]["phase"], "build");
    assert_eq!(
        fetched["plan"]["goal_fact_id"].as_str(),
        Some(goal_fact_id.as_str())
    );
    let weeks = fetched["weeks"].as_array().expect("weeks array");
    assert_eq!(weeks.len(), 2);
    assert_eq!(weeks[0]["days"][1]["intensity"], "3x8min @ 88-93% FTP");
    assert_eq!(weeks[1]["days"][1]["duration_min"], 150);

    // §3b write-back: the goal race became a pillar Goal fact.
    let dossier = executor
        .resources
        .repos()
        .dossier
        .compose_dossier(TenantId::from_uuid(Uuid::parse_str(&tenant_id)?), user_id)
        .await?;
    let has_goal = format!("{dossier:?}").contains("Big Red");
    assert!(
        has_goal,
        "goal race must be written back as a pillar Goal fact"
    );
    Ok(())
}

#[tokio::test]
async fn adjust_single_week_without_outline_supersedes_that_week() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;

    // "Move Tuesday's tempo to Wednesday" — week-only re-save, no outline.
    let adjust = executor
        .execute_tool(make_request(
            "save_training_plan",
            json!({
                "coach_id": "endurance-coach",
                "weeks": [{
                    "week_start": "2026-07-13",
                    "focus": "volume back up",
                    "adjustment_reason": "legs heavy Tuesday — tempo moved to Wednesday",
                    "days": [
                        {"date": "2026-07-13", "sport": "rest", "workout": "off — legs up"},
                        {"date": "2026-07-15", "sport": "gravel", "workout": "tempo 3x8min", "duration_min": 60, "intensity": "3x8min @ 88-93% FTP"},
                        {"date": "2026-07-14", "sport": "mtb", "workout": "endurance easy", "duration_min": 90, "intensity": "Z2"}
                    ]
                }]
            }),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(adjust.success, "adjust failed: {:?}", adjust.error);
    let result = adjust.result.expect("adjust result");
    assert_eq!(result["weeks_saved"], 1);
    assert_eq!(result["weeks"][0]["superseded"], true);

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let fetched = get.result.expect("get result");
    let weeks = fetched["weeks"].as_array().expect("weeks");
    assert_eq!(weeks.len(), 2, "still one active row per week");
    assert_eq!(
        weeks[0]["adjustment_reason"],
        "legs heavy Tuesday — tempo moved to Wednesday",
    );
    assert_eq!(weeks[0]["days"][1]["date"], "2026-07-15");
    assert_eq!(weeks[1]["focus"], "moderate build", "week 2 untouched");
    Ok(())
}

#[tokio::test]
async fn float_shaped_numbers_from_the_llm_are_accepted() -> Result<()> {
    // Live QA 2026-07-12 (conv 79ca0840): seven save_training_plan calls
    // failed in 22-67ms — validation-layer rejections. The schema declares
    // duration_min/weeks/target_hours as "number", and LLMs routinely emit
    // 60.0 for a number; serde rejects floats for u32/u8. The tool must
    // accept whole-valued floats wherever the schema says "number".
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let mut payload = full_plan_payload();
    payload["outline"]["blocks"][0]["weeks"] = json!(3.0);
    payload["weeks"][0]["days"][1]["duration_min"] = json!(60.0);
    payload["weeks"][1]["days"][1]["duration_min"] = json!(150.0);

    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        save.success,
        "float-shaped numbers must save: {:?}",
        save.error
    );
    let result = save.result.expect("save result");
    assert_eq!(result["weeks_saved"], 2);

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let fetched = get.result.expect("get result");
    assert_eq!(fetched["plan"]["blocks"][0]["weeks"], 3);
    assert_eq!(fetched["weeks"][0]["days"][1]["duration_min"], 60);
    Ok(())
}

#[tokio::test]
async fn invalid_week_date_rejects_whole_save_with_no_partial_writes() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let mut payload = full_plan_payload();
    payload["weeks"][1]["days"][1]["date"] = json!("2026-7-22"); // non-canonical

    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await;
    let refused = match save {
        Err(_) => true,
        Ok(resp) => !resp.success,
    };
    assert!(refused, "malformed date must reject the save");

    // Validation runs before any write: not even the (valid) outline landed.
    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let fetched = get.result.expect("get result");
    assert!(
        fetched["plan"].is_null(),
        "no partial plan may survive a rejected save"
    );
    Ok(())
}

#[tokio::test]
async fn saved_plan_is_injected_into_the_system_prompt() -> Result<()> {
    use pierre_chat_pipeline::stages::memory::inject_training_plan;

    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;

    let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 14).expect("valid date");
    let prompt = inject_training_plan(
        executor.resources.repos().training_plans.as_ref(),
        &tenant_id,
        &user_id.to_string(),
        Some("endurance-coach"),
        today,
        false,
        "BASE PROMPT".to_owned(),
    )
    .await;

    assert!(prompt.starts_with("BASE PROMPT"));
    assert!(prompt.contains("## Current training plan"));
    assert!(prompt.contains("Big Red (gravel) on 2026-08-08 — 25 days out"));
    assert!(prompt.contains("[current] build × 3wk from 2026-07-13"));
    assert!(prompt.contains("This week (starting 2026-07-13) — focus: volume back up"));
    assert!(prompt.contains("tempo 3x8min"));
    assert!(prompt.contains("Next week (starting 2026-07-20)"));

    // A user with no plan gets the prompt back untouched — no empty section.
    let other_user = Uuid::new_v4();
    let untouched = inject_training_plan(
        executor.resources.repos().training_plans.as_ref(),
        &tenant_id,
        &other_user.to_string(),
        Some("endurance-coach"),
        today,
        false,
        "BASE PROMPT".to_owned(),
    )
    .await;
    assert_eq!(untouched, "BASE PROMPT");

    // Mid pillar-walk the section is suppressed even WITH a saved plan: the
    // onboarding directive says "do not deliver a full coaching plan yet",
    // and a trailing plan block overriding it is exactly how the 2026-07-12
    // QA walk lost its remaining pillars.
    let during_walk = inject_training_plan(
        executor.resources.repos().training_plans.as_ref(),
        &tenant_id,
        &user_id.to_string(),
        Some("endurance-coach"),
        today,
        true,
        "BASE PROMPT".to_owned(),
    )
    .await;
    assert_eq!(during_walk, "BASE PROMPT");
    Ok(())
}

/// Count the athlete's coach-agnostic `target race` Goal facts — the row the
/// pillar loop is supposed to converge on exactly one of.
async fn agnostic_goal_facts(
    executor: &UniversalToolExecutor,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<Vec<(String, String)>> {
    let tenant = TenantId::from_uuid(Uuid::parse_str(tenant_id)?);
    let facts = executor
        .resources
        .repos()
        .memory
        .list_user_facts(
            tenant,
            &user_id.to_string(),
            None,
            Some(pierre_memory::FactKind::Goal),
            200,
        )
        .await?;
    Ok(facts
        .into_iter()
        .filter(|f| f.coach_id.is_none() && f.predicate == "target race")
        .map(|f| (f.id, f.object))
        .collect())
}

#[tokio::test]
async fn resaving_an_outline_does_not_duplicate_the_goal_fact() -> Result<()> {
    // F3: re-sending the same outline (the documented normal flow) without
    // echoing goal_fact_id back must NOT mint a second Goal fact.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let first = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let first_fact = first.result.expect("result")["goal_fact_id"]
        .as_str()
        .expect("goal fact")
        .to_owned();

    // Re-save the identical outline — no goal_fact_id supplied.
    let second = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let second_fact = second.result.expect("result")["goal_fact_id"]
        .as_str()
        .expect("goal fact")
        .to_owned();

    assert_eq!(
        first_fact, second_fact,
        "an identical re-save must reuse the same goal fact"
    );
    let facts = agnostic_goal_facts(&executor, &tenant_id, user_id).await?;
    assert_eq!(
        facts.len(),
        1,
        "exactly one coach-agnostic goal fact, got {facts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn changing_the_goal_race_keeps_one_current_goal_fact() -> Result<()> {
    // F3: a genuinely different goal race replaces the prior agnostic goal
    // fact — still exactly one row, now reflecting the new race.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;

    let mut changed = full_plan_payload();
    changed["outline"]["goal_race"]["name"] = json!("Autumn Gravel");
    changed["outline"]["goal_race"]["date"] = json!("2026-09-19");
    executor
        .execute_tool(make_request(
            "save_training_plan",
            changed,
            user_id,
            Some(&tenant_id),
        ))
        .await?;

    let facts = agnostic_goal_facts(&executor, &tenant_id, user_id).await?;
    assert_eq!(facts.len(), 1, "still exactly one goal fact, got {facts:?}");
    assert!(
        facts[0].1.contains("Autumn Gravel"),
        "goal fact must reflect the new race: {facts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn oversized_strategy_is_rejected_with_no_write() -> Result<()> {
    // F6: an unbounded field would inflate every future system prompt.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let mut payload = full_plan_payload();
    payload["outline"]["strategy"] = json!("x".repeat(5_000));
    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await;
    let refused = match save {
        Err(_) => true,
        Ok(resp) => !resp.success,
    };
    assert!(refused, "an oversized strategy must be rejected");

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        get.result.expect("result")["plan"].is_null(),
        "no plan may survive a rejected oversized save"
    );
    Ok(())
}

#[tokio::test]
async fn unknown_goal_fact_id_is_dropped_and_replaced() -> Result<()> {
    // F7: an LLM-supplied goal_fact_id that isn't a real fact of this athlete
    // must not be persisted as a link; a real one is minted instead.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let mut payload = full_plan_payload();
    payload["goal_fact_id"] = json!("00000000-0000-0000-0000-000000000000");
    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);
    let linked = save.result.expect("result")["goal_fact_id"]
        .as_str()
        .expect("goal fact")
        .to_owned();
    assert_ne!(
        linked, "00000000-0000-0000-0000-000000000000",
        "a bogus goal_fact_id must be dropped, not stored"
    );
    let facts = agnostic_goal_facts(&executor, &tenant_id, user_id).await?;
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].0, linked, "the linked fact must be the minted one");
    Ok(())
}

#[tokio::test]
async fn a_real_but_non_goal_fact_id_is_not_linked() -> Result<()> {
    // F7 regression: a plan links only to a Goal fact, and the staleness read
    // sees only Goal facts. An LLM-supplied goal_fact_id that is a real but
    // NON-Goal fact of this athlete must be dropped (else the plan would read
    // stale forever), and a real Goal fact minted in its place.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);

    // A genuine non-Goal fact the LLM could echo back from recalled memory.
    let schedule_fact = executor
        .resources
        .repos()
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_id.to_string(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Schedule,
            pillar: Some(Pillar::TrainingAndMovement),
            subject: "you",
            predicate: "can train on",
            object: "Tuesday and Thursday evenings",
            confidence: 0.9,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await?;

    let mut payload = full_plan_payload();
    payload["goal_fact_id"] = json!(schedule_fact.id);
    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);
    let linked = save.result.expect("result")["goal_fact_id"]
        .as_str()
        .expect("goal fact")
        .to_owned();
    assert_ne!(
        linked, schedule_fact.id,
        "a non-Goal fact id must not be linked as the plan's goal"
    );

    // The plan is fresh, so the read must NOT flag it stale.
    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert_eq!(
        get.result.expect("result")["goal_stale"],
        false,
        "a plan linked to a real minted Goal fact must not read stale"
    );
    Ok(())
}

#[tokio::test]
async fn get_flags_goal_stale_when_the_linked_goal_is_gone() -> Result<()> {
    // F7: the plan snapshots the goal; when the living goal fact disappears
    // the read flags the snapshot stale so the coach re-confirms.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let goal_fact_id = save.result.expect("result")["goal_fact_id"]
        .as_str()
        .expect("goal fact")
        .to_owned();

    // A fresh plan is not stale.
    let fresh = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert_eq!(fresh.result.expect("result")["goal_stale"], false);

    // Remove the living goal fact, then the snapshot reads stale.
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let removed = executor
        .resources
        .repos()
        .memory
        .delete_user_fact(&goal_fact_id, tenant, &user_id.to_string())
        .await?;
    assert!(removed, "goal fact should have been deleted");

    let stale = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert_eq!(
        stale.result.expect("result")["goal_stale"],
        true,
        "a plan whose goal fact is gone must read stale"
    );
    Ok(())
}

/// A conversation mid guided pillar walk refuses `save_training_plan` outright.
///
/// The pipeline already withholds the tool from the prompt's "Available Tools"
/// list and from the native function declarations for the duration of the walk,
/// but neither covers the native-MCP path (an ACP subprocess reads `tools/list`
/// straight off `/mcp`) or a direct MCP call. This is the enforcement half: no
/// plan is written, and the same call succeeds once the walk ends.
#[tokio::test]
async fn save_refuses_while_the_conversation_is_mid_profile_walk() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let repos = executor.resources.repos();

    let conversation = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "profile walk",
            "gemini-2.0-flash",
            // No coach row is seeded in this fixture; the walk gate keys on the
            // conversation's onboarding_state, not on which coach is bound.
            None,
            None,
        )
        .await?;

    let activated = repos
        .chat
        .set_conversation_onboarding_state(
            &conversation.id,
            Some(&OnboardingState::start_now_column(GuidedFlow::Pillars)),
            tenant,
        )
        .await?;
    assert!(activated, "onboarding state should have been written");

    // Scope the conversation the way the chat pipeline does, so the tool sees it.
    let scoped = Arc::new(
        UniversalExecutor::new(Arc::clone(&executor.resources))
            .with_conversation_id(conversation.id.clone()),
    );
    let refused = scoped
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await;
    let error = refused
        .expect_err("save_training_plan must be refused during the walk")
        .to_string();
    assert!(
        error.contains("profile walk"),
        "refusal must tell the model why, got: {error}"
    );
    assert!(
        repos
            .training_plans
            .get_active_plan(&tenant_id, &user_id.to_string(), None)
            .await?
            .is_none(),
        "the refused save must not have written a plan"
    );

    // Walk over → the identical call goes through.
    repos
        .chat
        .set_conversation_onboarding_state(&conversation.id, None, tenant)
        .await?;
    let saved = scoped
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        saved.success,
        "save must succeed once the walk is over, got: {:?}",
        saved.result
    );
    assert_eq!(
        saved.result.expect("result")["goal_race"],
        "Big Red on 2026-08-08"
    );
    Ok(())
}

/// The exact payload the coach emitted on 2026-07-28 (conversation
/// 043fdd88…): `goal_race` as free text, no `blocks`, and weeks keyed by
/// `week_label` / `day` / `session`. Every field name is invented, because the
/// text tool catalog rendered only `outline (object)` / `weeks (array)` and
/// never showed the nested schema.
fn freeform_payload_from_the_2026_07_28_failure() -> Value {
    json!({
        "outline": {
            "goal_race": "Big Red (140km gravel, 1800m D+) le 8 août",
            "strategy": "Remontée de forme vélo montagne/gravel + trail"
        },
        "weeks": [{
            "week_label": "Semaine 1 (jusqu'au 4 août)",
            "days": [
                {"day": "Lundi", "session": "Vélo Z1 45min facile"},
                {"day": "Mardi", "session": "Sub-seuil 3x10min @85-90% FTP"}
            ]
        }]
    })
}

#[tokio::test]
async fn freeform_shape_is_rejected_with_the_real_shape_attached() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // The rejection has to teach the shape, not just name the failure: the
    // tool loop runs another LLM turn on the error, so the field names the
    // model got wrong must be in the message for a retry to land. Both halves
    // were wrong here, so both skeletons must come back in the one reply.
    let message = outcome_text(
        &executor,
        make_request(
            "save_training_plan",
            freeform_payload_from_the_2026_07_28_failure(),
            user_id,
            Some(&tenant_id),
        ),
    )
    .await;
    for field in [
        "goal_race",
        "discipline",
        "priority",
        "week_start",
        "sport",
        "workout",
    ] {
        assert!(
            message.contains(field),
            "rejection must name `{field}` so the retry can fix it: {message}"
        );
    }
    // Format hints ride along on the leaves — a bare field name is not enough
    // to know a date is wanted where a French week label was sent.
    assert!(
        message.contains("YYYY-MM-DD"),
        "rejection must carry the date format: {message}"
    );

    // Nothing was written — a rejected shape must not leave a half-plan.
    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        get.result.expect("get result")["plan"].is_null(),
        "a rejected payload must persist nothing"
    );
    Ok(())
}

/// Text of a tool outcome, however the executor surfaces it — a transport-level
/// `Err` or an `Ok` carrying `success: false`.
async fn outcome_text(executor: &UniversalToolExecutor, request: UniversalRequest) -> String {
    match executor.execute_tool(request).await {
        Err(e) => e.to_string(),
        Ok(response) => format!("{:?}{:?}", response.error, response.result),
    }
}

#[tokio::test]
async fn outline_without_blocks_saves_and_reads_back() -> Result<()> {
    // Phil's real case: "two weeks to Big Red, hold form then taper" has no
    // mesocycle structure. Requiring at least one block made such a plan
    // unsaveable until the coach invented one.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "save_training_plan",
            json!({
                "coach_id": "endurance-coach",
                "outline": {
                    "goal_race": {
                        "name": "Big Red",
                        "date": "2026-08-08",
                        "discipline": "gravel",
                        "priority": "A"
                    },
                    "strategy": "Remise en forme gravel/MTB + trail, densité modérée-forte, pas de focus perf"
                },
                "weeks": [{
                    "week_start": "2026-08-03",
                    "focus": "décharge avant Big Red",
                    "days": [
                        {"date": "2026-08-04", "sport": "gravel", "workout": "sub-seuil 2x8min", "duration_min": 60, "intensity": "2x8min @ 85% FTP"},
                        {"date": "2026-08-07", "sport": "rest", "workout": "repos complet, veille de course"}
                    ]
                }]
            }),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        saved.success,
        "a blockless outline must save: {:?}",
        saved.error
    );

    let fetched = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?
        .result
        .expect("get result");
    assert_eq!(fetched["plan"]["goal_race"]["name"], "Big Red");
    assert_eq!(
        fetched["plan"]["blocks"].as_array().map(Vec::len),
        Some(0),
        "no blocks were sent, so none are stored"
    );
    // The prescription itself survived — the point of saving at all.
    assert_eq!(
        fetched["weeks"][0]["days"][0]["intensity"],
        "2x8min @ 85% FTP"
    );
    assert_eq!(fetched["weeks"][0]["days"][1]["sport"], "rest");
    Ok(())
}

/// Write one coach-agnostic `target race` Goal fact directly, the way an
/// earlier save (or a `/pillars` walk) leaves one behind.
async fn seed_agnostic_goal_fact(
    executor: &UniversalToolExecutor,
    tenant: TenantId,
    user_id: Uuid,
    object: &str,
) -> Result<String> {
    let fact = executor
        .resources
        .repos()
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_id.to_string(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: Some(Pillar::TrainingAndMovement),
            subject: "you",
            predicate: "target race",
            object,
            confidence: 0.95,
            source: FactSource::Coach,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await?;
    Ok(fact.id)
}

#[tokio::test]
async fn goal_convergence_retires_leftovers_even_when_the_goal_is_unchanged() -> Result<()> {
    // F10: the goal fact is written before the plan bundle (the plan row stores
    // its id) and the facts it replaces are erased only after the bundle
    // commits — so a bundle that fails leaves an extra agnostic goal fact
    // behind. The next save has to converge anyway, including the case where
    // the athlete's goal did not change and the matching fact already exists.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);

    // The fact this payload's goal race converges on, already stored…
    let matching = seed_agnostic_goal_fact(
        &executor,
        tenant,
        user_id,
        "Big Red (gravel) on 2026-08-08 — priority A",
    )
    .await?;
    // …alongside the goal it replaced, which an interrupted save left behind.
    let leftover = seed_agnostic_goal_fact(
        &executor,
        tenant,
        user_id,
        "Spring Classic (road) on 2026-04-11 — priority A",
    )
    .await?;

    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);
    assert_eq!(
        save.result.expect("result")["goal_fact_id"].as_str(),
        Some(matching.as_str()),
        "an unchanged goal must reuse the fact already stored"
    );

    let facts = agnostic_goal_facts(&executor, &tenant_id, user_id).await?;
    assert_eq!(
        facts.len(),
        1,
        "the superseded goal fact must be retired, got {facts:?}"
    );
    assert_eq!(facts[0].0, matching, "the linked fact is the surviving one");
    assert!(
        !facts.iter().any(|(id, _)| id == &leftover),
        "the replaced goal fact must not survive convergence: {facts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_goal_fact_ranked_below_the_list_cap_is_still_the_athletes_own() -> Result<()> {
    // F22: ownership of an LLM-supplied goal_fact_id used to be decided by
    // listing the athlete's Goal facts and scanning. An athlete with more Goal
    // facts than the cap had legitimate ids below the cut read as foreign,
    // dropped, and silently replaced by a freshly minted fact. The check is a
    // point lookup, so rank cannot decide ownership.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);

    // The athlete's real goal fact, written first so every later fact outranks
    // it in the newest-first ordering the list read uses.
    let owned = seed_agnostic_goal_fact(
        &executor,
        tenant,
        user_id,
        "Big Red (gravel) on 2026-08-08 — priority A",
    )
    .await?;
    for i in 0..520 {
        executor
            .resources
            .repos()
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: tenant,
                user_id: &user_id.to_string(),
                coach_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: Some(Pillar::TrainingAndMovement),
                subject: "you",
                predicate: "want to",
                object: &format!("hit checkpoint {i}"),
                confidence: 0.8,
                source: FactSource::Conversation,
                valid_until: None,
                source_msg_id: None,
                embedding: None,
            })
            .await?;
    }

    let mut payload = full_plan_payload();
    payload["goal_fact_id"] = json!(owned);
    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            payload,
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);
    assert_eq!(
        save.result.expect("result")["goal_fact_id"].as_str(),
        Some(owned.as_str()),
        "the athlete's own goal fact must be linked, not replaced"
    );

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert_eq!(
        get.result.expect("result")["plan"]["goal_fact_id"].as_str(),
        Some(owned.as_str()),
        "the stored plan must carry the supplied fact id"
    );
    Ok(())
}

/// Six one-hour rides inside the six-week snapshot window: a baseline of
/// exactly one hour a week, so a planned week's verdict is arithmetic rather
/// than a guess.
async fn seed_one_hour_per_week_baseline(
    executor: &UniversalToolExecutor,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<()> {
    let tenant = TenantId::from_uuid(Uuid::parse_str(tenant_id)?);
    let rides: Vec<Activity> = (0..6)
        .map(|i| {
            ActivityBuilder::new(
                format!("baseline-{i}"),
                format!("baseline ride {i}"),
                SportType::Ride,
                Utc::now() - chrono::Duration::days(2 + i * 7),
                3_600,
                "strava",
            )
            .build()
        })
        .collect();
    executor
        .resources
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &rides)
        .await?;
    Ok(())
}

/// An outline plus two weeks sent NEWEST FIRST: a five-hour week ahead of the
/// one-hour week the athlete actually starts on.
fn plan_with_the_heavy_week_first() -> Value {
    json!({
        "coach_id": "endurance-coach",
        "outline": {
            "goal_race": {
                "name": "Big Red",
                "date": "2026-08-08",
                "discipline": "gravel",
                "priority": "A"
            },
            "strategy": "ease back in, then one big week before the taper"
        },
        "weeks": [
            {
                "week_start": "2026-07-20",
                "focus": "big week",
                "days": [
                    {"date": "2026-07-21", "sport": "gravel", "workout": "long endurance", "duration_min": 300, "intensity": "Z2"}
                ]
            },
            {
                "week_start": "2026-07-13",
                "focus": "ease back in",
                "days": [
                    {"date": "2026-07-14", "sport": "gravel", "workout": "easy hour", "duration_min": 60, "intensity": "Z2"}
                ]
            }
        ]
    })
}

/// The `notify` events a captured run emitted for `event`.
fn notify_events<'a>(events: &'a [CapturedEvent], event: &str) -> Vec<&'a CapturedEvent> {
    events
        .iter()
        .filter(|e| e.field("event").is_some_and(|v| v.contains(event)))
        .collect()
}

#[tokio::test]
async fn the_ramp_check_grades_the_earliest_week_not_the_first_in_the_payload() -> Result<()> {
    // F12: the check used to measure `weeks[0]`, the first element of the
    // payload array. Nothing sorts the weeks and the schema documents
    // single-week adjustment saves, so a plan sent newest-first was graded on
    // an arbitrary mid-plan week — here it would warn "planned opening week
    // exceeds recent weekly load" about a week five weeks in, while the week
    // the athlete actually starts on matches their baseline exactly.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    seed_one_hour_per_week_baseline(&executor, &tenant_id, user_id).await?;

    let (events, guard) = setup_capture();
    let save = executor
        .execute_tool(make_request(
            "save_training_plan",
            plan_with_the_heavy_week_first(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);

    assert!(save.success, "save failed: {:?}", save.error);
    assert!(
        notify_events(&captured, "training_plan.ramp_warning").is_empty(),
        "the opening week matches the baseline — no warning is owed: {captured:?}"
    );
    let checked = notify_events(&captured, "training_plan.ramp_checked");
    assert_eq!(
        checked.len(),
        1,
        "exactly one ramp verdict per save: {captured:?}"
    );
    let planned = checked[0].field("planned_hours").unwrap_or_default();
    assert!(
        planned.contains("1.0"),
        "the earliest week (1.0 h) must be the one measured, got {planned}"
    );
    assert!(
        checked[0]
            .field("baseline_hours")
            .is_some_and(|b| b.contains("1.0")),
        "the seeded baseline must be the comparison: {captured:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_week_only_adjustment_is_not_graded_as_an_opening_week() -> Result<()> {
    // F12: a save with no outline adjusts one week of a plan whose opening week
    // was measured when it was created. Grading it as an opening week reports a
    // ramp the athlete was never asked to make — a big mid-plan week is the
    // point of a big mid-plan week.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    seed_one_hour_per_week_baseline(&executor, &tenant_id, user_id).await?;

    executor
        .execute_tool(make_request(
            "save_training_plan",
            plan_with_the_heavy_week_first(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;

    let (events, guard) = setup_capture();
    let adjust = executor
        .execute_tool(make_request(
            "save_training_plan",
            json!({
                "coach_id": "endurance-coach",
                "weeks": [{
                    "week_start": "2026-07-20",
                    "focus": "big week",
                    "adjustment_reason": "feeling strong — keeping the volume",
                    "days": [
                        {"date": "2026-07-21", "sport": "gravel", "workout": "long endurance", "duration_min": 300, "intensity": "Z2"},
                        {"date": "2026-07-23", "sport": "mtb", "workout": "long endurance", "duration_min": 300, "intensity": "Z2"}
                    ]
                }]
            }),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);

    assert!(adjust.success, "adjust failed: {:?}", adjust.error);
    let result = adjust.result.expect("adjust result");
    assert_eq!(result["weeks_saved"], 1);
    assert_eq!(result["weeks"][0]["superseded"], true);
    for verdict in [
        "training_plan.ramp_warning",
        "training_plan.ramp_checked",
        "training_plan.ramp_unmeasurable",
    ] {
        assert!(
            notify_events(&captured, verdict).is_empty(),
            "a week-only adjustment must emit no {verdict}: {captured:?}"
        );
    }

    // The adjustment itself still landed.
    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    let fetched = get.result.expect("get result");
    let weeks = fetched["weeks"].as_array().expect("weeks");
    assert_eq!(weeks.len(), 2, "still one active row per week");
    assert_eq!(weeks[1]["days"][1]["duration_min"], 300);
    assert_eq!(weeks[0]["focus"], "ease back in", "week 1 untouched");
    Ok(())
}

#[tokio::test]
async fn a_date_outside_the_calendar_domain_is_rejected_with_no_write() -> Result<()> {
    // F17: `%Y` accepts a signed, unlimited-digit year and NaiveDate's Display
    // re-emits it, so "+262142-12-31" round-trips through the format check.
    // Stored, it panics the renderers on every later turn — a plan is
    // re-rendered into the system prompt for the rest of the athlete's chat
    // history. Every date the payload carries is bounded at save time.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    for (pointer, value) in [
        ("/outline/goal_race/date", "+262142-12-31"),
        ("/outline/blocks/0/start", "+262142-01-05"),
        ("/weeks/0/week_start", "+262142-07-13"),
        ("/weeks/0/days/0/date", "+262142-07-13"),
        ("/outline/goal_race/date", "0999-08-08"),
    ] {
        let mut payload = full_plan_payload();
        *payload
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer} exists in the fixture")) = json!(value);
        let message = outcome_text(
            &executor,
            make_request("save_training_plan", payload, user_id, Some(&tenant_id)),
        )
        .await;
        assert!(
            message.contains("is outside 1000-9999"),
            "the rejection of {pointer} must name the calendar bound it broke, \
             not a shape it satisfies: {message}"
        );
        assert!(
            message.contains(value),
            "the rejection of {pointer} must name the offending date: {message}"
        );

        let get = executor
            .execute_tool(make_request(
                "get_training_plan",
                json!({"coach_id": "endurance-coach"}),
                user_id,
                Some(&tenant_id),
            ))
            .await?;
        assert!(
            get.result.expect("get result")["plan"].is_null(),
            "a plan carrying an out-of-range {pointer} must persist nothing"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_hallucinated_week_count_is_rejected_with_no_write() -> Result<()> {
    // F25: every other collection in the payload is capped. An unbounded weeks
    // array turns one hallucinated call into thousands of statements in a
    // single transaction, and a plan that is re-rendered into every later
    // prompt.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let mut week_start = chrono::NaiveDate::from_ymd_opt(2026, 7, 13).expect("valid date");
    let mut weeks = Vec::new();
    for _ in 0..105 {
        weeks.push(json!({
            "week_start": week_start.format("%Y-%m-%d").to_string(),
            "focus": "endurance",
            "days": [{
                "date": week_start.format("%Y-%m-%d").to_string(),
                "sport": "gravel",
                "workout": "endurance",
                "duration_min": 90,
                "intensity": "Z2"
            }]
        }));
        week_start += chrono::Duration::days(7);
    }
    let mut payload = full_plan_payload();
    payload["weeks"] = Value::Array(weeks);

    let message = outcome_text(
        &executor,
        make_request("save_training_plan", payload, user_id, Some(&tenant_id)),
    )
    .await;
    assert!(
        message.contains("too many weeks"),
        "the rejection must say the week count is the problem: {message}"
    );

    let get = executor
        .execute_tool(make_request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        get.result.expect("get result")["plan"].is_null(),
        "an over-long weeks array must persist nothing"
    );
    Ok(())
}

#[tokio::test]
async fn save_refuses_mid_walk_even_with_no_conversation_in_scope() -> Result<()> {
    // F27: the withhold used to key on the conversation the chat pipeline puts
    // in a task-local. A `tools/call` arriving on the `/mcp` endpoint has none,
    // so the guard was skipped there and a plan could be written mid-interview
    // — on the very surface the server-side refusal exists to cover. The flow
    // is resolved from the athlete when no conversation is in scope.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let repos = executor.resources.repos();

    let conversation = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "profile walk",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await?;
    repos
        .chat
        .set_conversation_onboarding_state(
            &conversation.id,
            Some(&OnboardingState::start_now_column(GuidedFlow::Pillars)),
            tenant,
        )
        .await?;

    // No conversation id in scope — the MCP-direct shape.
    let refused = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await;
    let error = refused
        .expect_err("an MCP-direct save must be refused during the walk")
        .to_string();
    assert!(
        error.contains("profile walk"),
        "refusal must tell the model why, got: {error}"
    );
    // The payload names its coach, and with no conversation in scope that
    // argument is the slug the plan would be saved under — so that is the slug
    // the "nothing was written" check has to look at.
    assert!(
        repos
            .training_plans
            .get_active_plan(&tenant_id, &user_id.to_string(), Some("endurance-coach"))
            .await?
            .is_none(),
        "the refused save must not have written a plan"
    );

    // A finished interview is not an active one: the marker the release
    // directive leaves behind must not keep refusing saves.
    repos
        .chat
        .set_conversation_onboarding_state(
            &conversation.id,
            Some(
                &OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Pillars)
                    .completed(Utc::now().to_rfc3339())
                    .to_column()?,
            ),
            tenant,
        )
        .await?;
    let saved = executor
        .execute_tool(make_request(
            "save_training_plan",
            full_plan_payload(),
            user_id,
            Some(&tenant_id),
        ))
        .await?;
    assert!(
        saved.success,
        "a completed interview must not withhold the tool, got: {:?}",
        saved.error
    );
    assert_eq!(
        saved.result.expect("result")["goal_race"],
        "Big Red on 2026-08-08"
    );
    Ok(())
}

#[tokio::test]
async fn advertised_schema_is_the_schema_the_tool_accepts() -> Result<()> {
    // The invariant nobody was testing: every field name save_training_plan
    // advertises is a field its deserializer knows, and every field the
    // deserializer requires is advertised. Fill the published schema in and
    // serde must never answer "missing field" or "unknown field".
    //
    // Scoped to field *names* on purpose. The generated values are generic
    // placeholders, so a value-level complaint (a `priority` of "placeholder"
    // is not one of A/B/C) is expected and is not schema drift; a name-level
    // complaint always is. Renaming a payload field without updating the
    // schema — the drift that would silently recreate this whole outage —
    // reds this test.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let schema = SaveTrainingPlanTool.definition().input_schema;
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("advertised properties");
    let payload = json!({
        "outline": filled_from_schema(props.get("outline").expect("outline advertised")),
        "weeks": filled_from_schema(props.get("weeks").expect("weeks advertised")),
    });

    let message = outcome_text(
        &executor,
        make_request("save_training_plan", payload, user_id, Some(&tenant_id)),
    )
    .await;
    for drift in ["missing field", "unknown field"] {
        assert!(
            !message.contains(drift),
            "advertised schema and payload struct disagree on field names ({drift}): {message}"
        );
    }
    Ok(())
}

/// Build a value matching an advertised JSON-Schema node, so a test can fill
/// in the schema the tool publishes without hand-copying its field names.
fn filled_from_schema(node: &Value) -> Value {
    match node.get("type").and_then(Value::as_str) {
        Some("object") => {
            let mut map = serde_json::Map::new();
            if let Some(props) = node.get("properties").and_then(Value::as_object) {
                for (name, child) in props {
                    map.insert(name.clone(), filled_from_schema(child));
                }
            }
            Value::Object(map)
        }
        Some("array") => node
            .get("items")
            .map_or_else(|| json!([]), |item| json!([filled_from_schema(item)])),
        Some("integer") => json!(1),
        Some("number") => json!(1.0),
        Some("boolean") => json!(false),
        _ => json!("placeholder"),
    }
}

#[tokio::test]
async fn rendered_tool_catalog_exposes_the_nested_field_names() {
    // End-to-end guard on the 2026-07-28 root cause. On copilot_headless there
    // is no native function calling, so this catalog text is the ONLY schema
    // the model ever sees. It rendered `outline (object)` / `weeks (array)`
    // and stopped, hiding every inner field — 24 of 24 live saves rejected.
    let definition = SaveTrainingPlanTool.definition();
    let catalog = generate_tool_catalog(&[FunctionDeclaration {
        name: definition.name,
        description: definition.description,
        parameters: Some(definition.input_schema),
    }]);

    for field in [
        "week_start",
        "goal_race",
        "discipline",
        "priority",
        "sport",
        "workout",
        "duration_min",
        "intensity",
        "phase",
    ] {
        assert!(
            catalog.contains(&format!("`{field}`")),
            "the model must see `{field}` in the catalog:\n{catalog}"
        );
    }
    // The format hints that make those names fillable.
    assert!(catalog.contains("YYYY-MM-DD"), "catalog:\n{catalog}");
    assert!(
        catalog.contains("rest | base | build | peak | taper"),
        "catalog:\n{catalog}"
    );
    // Arrays of objects announce themselves as such, and nesting is legible.
    assert!(catalog.contains("(array of object)"), "catalog:\n{catalog}");
    assert!(catalog.contains("  - `"), "catalog:\n{catalog}");
}
