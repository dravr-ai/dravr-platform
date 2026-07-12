// ABOUTME: Integration tests for get/save_training_plan MCP tools + prompt injection
// ABOUTME: Content-asserting: real plan payloads through the executor, goal-fact write-back, prompt text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::TenantId;
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
        "BASE PROMPT".to_owned(),
    )
    .await;
    assert_eq!(untouched, "BASE PROMPT");
    Ok(())
}
