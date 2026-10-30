// ABOUTME: Integration tests for the 60 g/h single-transporter ceiling save_training_plan holds a fuelled day to
// ABOUTME: Above it a day states its carbohydrate source or names products whose labels state a glucose:fructose ratio (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
use dravr_cageux::periodization::MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H;
use pierre_core::models::{FuelFacts, FuelForm};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::fuel_fixtures;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp),
    ))
}

async fn create_athlete(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("fuel_ceiling_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
    }
}

/// Save `plan`; `Ok(())` when it is accepted, `Err(text)` with what the
/// refusal says otherwise.
async fn save(
    executor: &UniversalToolExecutor,
    plan: Value,
    athlete: Uuid,
    tenant: &str,
) -> Result<(), String> {
    match executor
        .execute_tool(request("save_training_plan", plan, athlete, tenant))
        .await
    {
        Err(error) => Err(error.to_string()),
        Ok(response) if response.success => Ok(()),
        Ok(response) => Err(format!("{:?} {:?}", response.error, response.result)),
    }
}

/// Add catalogue product `product` to the athlete's list.
async fn carry(
    executor: &UniversalToolExecutor,
    product: Uuid,
    athlete: Uuid,
    tenant: &str,
) -> Result<()> {
    let response = executor
        .execute_tool(request(
            "add_fuel_item",
            json!({"product_id": product.to_string()}),
            athlete,
            tenant,
        ))
        .await?;
    assert!(
        response.success,
        "add_fuel_item failed: {:?}",
        response.error
    );
    Ok(())
}

/// A one-week plan whose long ride is fuelled at `carbs_g_per_h`, with an
/// optional `carb_source` and the products it names.
fn long_ride(carbs_g_per_h: f32, carb_source: Option<&str>, products: &[Uuid]) -> Value {
    let mut fueling = json!({"carbs_g_per_h": carbs_g_per_h, "fluid_ml_per_h": 700.0});
    if let Some(source) = carb_source {
        fueling["carb_source"] = json!(source);
    }
    let ids: Vec<String> = products.iter().map(Uuid::to_string).collect();
    json!({
        "agent_id": "endurance-coach",
        "outline": {
            "goal_race": {"name": "Big Red", "date": "2026-08-08", "discipline": "gravel", "priority": "A"},
            "strategy": "race-specific fuelling practice"
        },
        "weeks": [{
            "week_start": "2026-07-13",
            "focus": "fuelling practice",
            "days": [
                {"date": "2026-07-13", "sport": "rest", "workout": "off"},
                {
                    "date": "2026-07-15",
                    "sport": "gravel",
                    "workout": "long endurance, practise race fuelling",
                    "duration_min": 240,
                    "intensity": "Z2",
                    "fueling": fueling,
                    "fuel_product_ids": ids,
                }
            ]
        }]
    })
}

/// A gel label carrying carbohydrate but stating no glucose:fructose ratio,
/// or stating glucose alone (ratio 0).
fn single_source_gel(ratio: Option<f64>) -> FuelFacts {
    FuelFacts {
        glucose_fructose_ratio: ratio,
        ..fuel_fixtures::gel_facts()
    }
}

/// An electrolyte capsule: no carbohydrate on its label.
fn capsule() -> FuelFacts {
    FuelFacts {
        serving_label: Some("1 capsule (1 g)".to_owned()),
        serving_g: Some(1.0),
        sodium_mg: Some(215.0),
        ..FuelFacts::default()
    }
}

#[tokio::test]
async fn the_ceiling_is_sixty_and_a_rate_above_it_states_its_source() -> Result<()> {
    assert!((MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H - 60.0).abs() < f32::EPSILON);
    let executor = create_executor().await?;
    let (athlete, tenant) = create_athlete(&executor).await?;

    // Exactly 60 g/h is a single-source rate and needs nothing more.
    save(&executor, long_ride(60.0, None, &[]), athlete, &tenant)
        .await
        .expect("60 g/h needs no stated source");

    // Above it, a day naming no product must say where the second
    // transportable carbohydrate comes from; a blank source says nothing.
    for source in [None, Some("   ")] {
        let refusal = save(&executor, long_ride(90.0, source, &[]), athlete, &tenant)
            .await
            .expect_err("90 g/h with no stated source is refused");
        assert!(
            refusal.contains("one carbohydrate source cannot deliver"),
            "{refusal}"
        );
        assert!(refusal.contains("carb_source"), "{refusal}");
    }
    save(
        &executor,
        long_ride(90.0, Some("glucose:fructose 1:0.8"), &[]),
        athlete,
        &tenant,
    )
    .await
    .expect("90 g/h with a stated source is saved");
    Ok(())
}

#[tokio::test]
async fn above_the_ceiling_every_named_carb_product_states_its_ratio() -> Result<()> {
    let executor = create_executor().await?;
    let (athlete, tenant) = create_athlete(&executor).await?;
    let repos = executor.resources.repos();
    let blended = fuel_fixtures::approved_gel(repos, athlete, "Blend").await;
    let no_ratio =
        fuel_fixtures::approved_gel_with(repos, athlete, "Plain", single_source_gel(None)).await;
    let glucose_only =
        fuel_fixtures::approved_gel_with(repos, athlete, "Dextro", single_source_gel(Some(0.0)))
            .await;
    let salt =
        fuel_fixtures::approved_product(repos, athlete, "Salt", FuelForm::Capsule, capsule()).await;
    for product in [blended, no_ratio, glucose_only, salt] {
        carry(&executor, product, athlete, &tenant).await?;
    }

    // A label with no ratio, or glucose alone, cannot back 90 g/h — even when
    // the day's carb_source claims a blend — and the refusal names it.
    for (single, brand) in [(no_ratio, "Plain"), (glucose_only, "Dextro")] {
        let refusal = save(
            &executor,
            long_ride(90.0, Some("glucose:fructose 1:0.8"), &[blended, single]),
            athlete,
            &tenant,
        )
        .await
        .expect_err("a single-source label under 90 g/h is refused");
        assert!(
            refusal.contains("state no glucose:fructose ratio"),
            "{refusal}"
        );
        assert!(refusal.contains(&single.to_string()), "{refusal}");
        assert!(refusal.contains(brand), "{refusal}");
        assert!(!refusal.contains(&blended.to_string()), "{refusal}");
    }

    // The same products are fine at the ceiling itself.
    save(
        &executor,
        long_ride(60.0, None, &[blended, no_ratio, glucose_only]),
        athlete,
        &tenant,
    )
    .await
    .expect("60 g/h is reachable from any carbohydrate source");

    // A blended gel backs 90 g/h, and a capsule with no carbohydrate neither
    // helps nor hurts.
    save(
        &executor,
        long_ride(90.0, None, &[blended, salt]),
        athlete,
        &tenant,
    )
    .await
    .expect("a blended label backs 90 g/h");
    Ok(())
}
