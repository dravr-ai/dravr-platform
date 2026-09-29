// ABOUTME: The admin knob nutrition.protein_athlete_g_per_kg reaches calculate_daily_nutrition's athlete protein target
// ABOUTME: A tenant or per-user override moves the grams, other tenants keep the kernel's value, 1.2-2.0 g/kg/day is enforced
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The knob sat in the admin catalogue with nothing reading it: an operator
//! could move it and every athlete still got the kernel's compiled 1.8 g/kg.
//! These drive the tool itself, so a knob the calculator ignores fails here.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use pierre_config::admin_types::{ConfigScope, UpdateConfigRequest, UpdateConfigResponse};
use pierre_config::nutrition_params::PROTEIN_ATHLETE_G_PER_KG_KEY;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::config::admin::service::{AdminConfigService, UpdateConfigContext};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

/// Body mass the grams are computed from.
const WEIGHT_KG: f64 = 70.0;

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
}

async fn resources() -> Result<Arc<ServerContext>> {
    common::init_server_config();
    common::init_test_http_clients();
    common::create_test_server_resources().await
}

async fn athlete(resources: &ServerContext) -> Result<Athlete> {
    let email = format!("protein_{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter").await?;
    Ok(Athlete { user_id, tenant })
}

fn admin_config(resources: &ServerContext) -> &AdminConfigService {
    resources
        .agent
        .admin_config
        .as_ref()
        .expect("admin config is wired")
}

async fn set_protein(
    resources: &ServerContext,
    admin: &Athlete,
    scope: ConfigScope<'_>,
    g_per_kg: f64,
) -> Result<UpdateConfigResponse> {
    let admin_id = admin.user_id.to_string();
    Ok(admin_config(resources)
        .update_config(
            &UpdateConfigRequest {
                parameters: HashMap::from([(
                    PROTEIN_ATHLETE_G_PER_KG_KEY.to_owned(),
                    json!(g_per_kg),
                )]),
                reason: Some("athlete protein override test".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin_id,
                admin_email: "protein-admin@example.com",
                scope,
                ip_address: None,
                user_agent: None,
            },
        )
        .await?)
}

/// `calculate_daily_nutrition`'s protein target, in grams, for an athlete
/// at `activity_level` on `training_goal`.
async fn protein_g(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    activity_level: &str,
    training_goal: &str,
) -> Result<f64> {
    let shared: Arc<ServerContext> = Arc::clone(resources);
    let executor = UniversalToolExecutor::new(shared).with_scopes(OAuthScope::self_grant());
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "calculate_daily_nutrition".to_owned(),
            parameters: json!({
                "weight_kg": WEIGHT_KG,
                "height_cm": 178.0,
                "age": 34,
                "gender": "male",
                "activity_level": activity_level,
                "training_goal": training_goal
            }),
            user_id: athlete.user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await?;
    assert!(response.success, "the tool answers: {:?}", response.error);
    let result: Value = response.result.expect("a result");
    Ok(result["protein_g"]
        .as_f64()
        .unwrap_or_else(|| panic!("protein_g is a figure: {result:#}")))
}

fn assert_grams(actual: f64, g_per_kg: f64) {
    let expected = WEIGHT_KG * g_per_kg;
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected} g ({g_per_kg} g/kg × {WEIGHT_KG} kg), got {actual}"
    );
}

#[tokio::test]
async fn a_tenant_override_moves_the_athlete_protein_target_for_that_tenant_only() -> Result<()> {
    let resources = resources().await?;
    let a = athlete(&resources).await?;
    let b = athlete(&resources).await?;

    // The kernel's compiled target before anyone turns the knob.
    assert_grams(
        protein_g(&resources, &a, "very_active", "maintenance").await?,
        1.8,
    );

    let written = set_protein(
        &resources,
        &a,
        ConfigScope::Tenant(&a.tenant.to_string()),
        1.4,
    )
    .await?;
    assert!(written.success, "{:?}", written.validation_errors);

    assert_grams(
        protein_g(&resources, &a, "very_active", "maintenance").await?,
        1.4,
    );
    // A very or extra active athlete losing weight reads the same target.
    assert_grams(
        protein_g(&resources, &a, "extra_active", "weight_loss").await?,
        1.4,
    );
    // An endurance goal reads the kernel's endurance factor, not the knob.
    assert_grams(
        protein_g(&resources, &a, "very_active", "endurance_performance").await?,
        2.0,
    );
    // Tenant B never set one.
    assert_grams(
        protein_g(&resources, &b, "very_active", "maintenance").await?,
        1.8,
    );
    Ok(())
}

#[tokio::test]
async fn a_per_user_override_beats_the_tenants_and_other_targets_stay_put() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;

    let tenant = set_protein(
        &resources,
        &athlete,
        ConfigScope::Tenant(&athlete.tenant.to_string()),
        1.4,
    )
    .await?;
    assert!(tenant.success, "{:?}", tenant.validation_errors);
    let user = set_protein(
        &resources,
        &athlete,
        ConfigScope::User(&athlete.user_id.to_string()),
        2.0,
    )
    .await?;
    assert!(user.success, "{:?}", user.validation_errors);

    assert_grams(
        protein_g(&resources, &athlete, "extra_active", "maintenance").await?,
        2.0,
    );
    // The knob is the athlete target only: a moderately active athlete on a
    // maintenance goal keeps the kernel's moderate factor.
    assert_grams(
        protein_g(&resources, &athlete, "moderately_active", "maintenance").await?,
        1.3,
    );
    Ok(())
}

#[tokio::test]
async fn a_target_outside_the_position_statement_range_is_never_stored() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;

    for g_per_kg in [1.0, 2.5] {
        let refused = set_protein(
            &resources,
            &athlete,
            ConfigScope::Tenant(&athlete.tenant.to_string()),
            g_per_kg,
        )
        .await?;
        assert!(
            !refused.success,
            "{g_per_kg} g/kg/day is outside 1.2-2.0 and must be refused"
        );
    }
    assert_grams(
        protein_g(&resources, &athlete, "very_active", "maintenance").await?,
        1.8,
    );
    Ok(())
}
