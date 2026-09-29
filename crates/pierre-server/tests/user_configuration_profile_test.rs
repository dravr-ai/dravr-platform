// ABOUTME: update_user_configuration applies the named profile template; get_user_configuration reports it
// ABOUTME: Pins the template's values as base parameters, overrides on top, and the active profile by name

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `update_user_configuration` recorded a profile's name and applied none of
//! its values, and `get_user_configuration` reported `custom` for any athlete
//! with a saved document. A named template's values are now the base
//! parameters of the configuration, the athlete's overrides sit on top, and
//! the active profile is the template's own name.

use anyhow::Result;
use pierre_core::config::profiles::{ConfigProfile, FitnessLevel, ProfileTemplates};
use pierre_core::models::SportType;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<Uuid> {
    let email = format!("config_profile_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    Ok(user_id)
}

fn request(tool: &str, params: Value, user_id: Uuid) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: None,
    }
}

async fn update(executor: &UniversalToolExecutor, user_id: Uuid, params: Value) -> Result<Value> {
    let response = executor
        .execute_tool(request("update_user_configuration", params, user_id))
        .await?;
    assert!(response.success, "update: {:?}", response.error);
    Ok(response.result.expect("a payload"))
}

async fn read(executor: &UniversalToolExecutor, user_id: Uuid) -> Result<Value> {
    let response = executor
        .execute_tool(request("get_user_configuration", json!({}), user_id))
        .await?;
    assert!(response.success, "get: {:?}", response.error);
    Ok(response.result.expect("a payload"))
}

async fn refusal(executor: &UniversalToolExecutor, user_id: Uuid, params: Value) -> String {
    match executor
        .execute_tool(request("update_user_configuration", params, user_id))
        .await
    {
        Err(e) => e.to_string(),
        Ok(response) => {
            assert!(
                !response.success,
                "expected a refusal: {:?}",
                response.result
            );
            response.error.unwrap_or_default()
        }
    }
}

#[tokio::test]
async fn a_named_template_is_applied_and_reported_as_the_active_profile() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;

    let updated = update(
        &executor,
        user_id,
        json!({"profile": "Elite Athlete", "parameters": {"threshold_multiplier": 1.3}}),
    )
    .await?;
    assert_eq!(updated["changes_applied"], json!(2));
    assert_eq!(
        updated["updated_configuration"]["active_profile"],
        json!("elite")
    );

    let configuration = read(&executor, user_id).await?;
    assert_eq!(configuration["active_profile"], json!("elite"));
    let document = &configuration["configuration"];
    assert_eq!(
        document["profile"],
        json!({"type": "Elite", "performance_factor": 1.15, "recovery_sensitivity": 1.2}),
        "the template itself, with its values"
    );
    assert_eq!(
        document["profile_parameters"],
        json!({"performance_standards": 1.15, "recovery_sensitivity": 1.2, "threshold_multiplier": 1.15}),
        "the values the template applies"
    );
    assert_eq!(
        document["session_overrides"],
        json!({"threshold_multiplier": 1.3})
    );
    assert_eq!(
        document["effective_parameters"],
        json!({"performance_standards": 1.15, "recovery_sensitivity": 1.2, "threshold_multiplier": 1.3}),
        "the override sits on top of the template's value"
    );
    Ok(())
}

#[tokio::test]
async fn overrides_alone_leave_the_default_profile_active_not_custom() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;

    update(
        &executor,
        user_id,
        json!({"parameters": {"heart_rate.anaerobic_threshold": 88.0}}),
    )
    .await?;
    let configuration = read(&executor, user_id).await?;
    assert_eq!(configuration["active_profile"], json!("default"));
    assert_eq!(
        configuration["configuration"]["profile"],
        json!({"type": "Default"})
    );
    assert_eq!(
        configuration["configuration"]["effective_parameters"],
        json!({"heart_rate.anaerobic_threshold": 88.0})
    );
    Ok(())
}

#[tokio::test]
async fn a_later_update_keeps_the_applied_template_until_another_is_named() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;

    update(&executor, user_id, json!({"profile": "beginner"})).await?;
    update(
        &executor,
        user_id,
        json!({"parameters": {"insights.min_confidence": 0.6}}),
    )
    .await?;
    let configuration = read(&executor, user_id).await?;
    assert_eq!(configuration["active_profile"], json!("beginner"));
    assert_eq!(
        configuration["configuration"]["profile_parameters"]["threshold_multiplier"],
        json!(0.85)
    );

    // The template is found by its profile name as well as its listed one.
    update(&executor, user_id, json!({"profile": "sport_cycling"})).await?;
    let configuration = read(&executor, user_id).await?;
    assert_eq!(configuration["active_profile"], json!("sport_cycling"));
    assert_eq!(
        configuration["configuration"]["profile_parameters"]["power_weight_importance"],
        json!(1.2)
    );
    assert_eq!(
        configuration["configuration"]["session_overrides"],
        json!({"insights.min_confidence": 0.6}),
        "switching template keeps the athlete's overrides"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_template_is_refused_with_the_ones_on_offer() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;

    let error = refusal(&executor, user_id, json!({"profile": "endurance"})).await;
    assert!(
        error.contains("no configuration profile is called 'endurance'")
            && error.contains("Elite Athlete (elite)"),
        "{error}"
    );
    assert!(
        executor
            .resources
            .repos()
            .profiles
            .get_configuration(&user_id.to_string())
            .await?
            .is_none(),
        "a refused call saves nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_saved_name_no_template_answers_to_reads_as_the_default_profile() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;
    // The shape the tool wrote when it recorded a name and applied nothing.
    executor
        .resources
        .repos()
        .profiles
        .save_configuration(
            &user_id.to_string(),
            &json!({
                "active_profile": "custom",
                "profile": {"name": "custom", "sport_type": "general", "training_focus": "custom"},
                "session_overrides": {"pace.easy_zone_low": 0.6},
                "last_modified": "2026-09-01T00:00:00+00:00"
            })
            .to_string(),
        )
        .await?;

    let configuration = read(&executor, user_id).await?;
    assert_eq!(configuration["active_profile"], json!("default"));
    assert_eq!(
        configuration["configuration"]["session_overrides"],
        json!({"pace.easy_zone_low": 0.6})
    );
    assert_eq!(
        configuration["configuration"]["last_modified"],
        json!("2026-09-01T00:00:00+00:00")
    );
    Ok(())
}

#[tokio::test]
async fn physiological_measurements_are_refused_and_nothing_is_saved() -> Result<()> {
    let executor = create_executor().await?;
    let user_id = create_test_user(&executor).await?;

    let error = refusal(
        &executor,
        user_id,
        json!({"profile": "elite", "parameters": {"threshold_hr": 170, "pace.easy_zone_low": 0.6}}),
    )
    .await;
    assert!(
        error.contains("threshold_hr belongs to the athlete's physiological profile")
            && error.contains("set_physiology"),
        "{error}"
    );
    assert!(executor
        .resources
        .repos()
        .profiles
        .get_configuration(&user_id.to_string())
        .await?
        .is_none());
    Ok(())
}

#[test]
fn every_template_answers_to_its_listed_name_and_its_profile_name() {
    for (listed, profile) in ProfileTemplates::all() {
        assert_eq!(ProfileTemplates::get(&listed), Some(profile.clone()));
        assert_eq!(
            ProfileTemplates::get(&profile.name()),
            Some(profile.clone())
        );
        assert_eq!(
            ProfileTemplates::get(&listed.to_uppercase()),
            Some(profile.clone())
        );
    }
    assert_eq!(ProfileTemplates::get("custom"), None);
    assert_eq!(
        ProfileTemplates::get("default"),
        Some(ConfigProfile::Default)
    );
}

/// The migration that creates a physiological profile writes the two enum
/// columns as literal serde text; this pins that text to the serde form.
#[test]
fn the_migrated_profile_defaults_are_the_serde_form_of_the_model_defaults() {
    assert_eq!(
        serde_json::to_string(&FitnessLevel::Recreational).unwrap(),
        "\"Recreational\""
    );
    assert_eq!(serde_json::to_string(&SportType::Run).unwrap(), "\"run\"");
}
