// ABOUTME: Integration tests for get/save_training_plan MCP tools + prompt injection
// ABOUTME: Content-asserting: real plan payloads through the executor, goal-fact write-back, prompt text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

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
        .compose_dossier(TenantId::from(Uuid::parse_str(&tenant_id)?), user_id)
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
    let tenant = TenantId::from(Uuid::parse_str(tenant_id)?);
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
    let tenant = TenantId::from(Uuid::parse_str(&tenant_id)?);

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
    let tenant = TenantId::from(Uuid::parse_str(&tenant_id)?);
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
