// ABOUTME: The three-zone cuts as admin config — defaults are TidCuts::default(), a tenant override moves the verdict
// ABOUTME: A write or reset that would leave a pair out of order for any tenant is refused naming the slot

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The compliance rail places every percent-of-threshold and RPE band with a
//! `TidCuts` it resolves per tenant from the `tid_cuts.*` parameters. These
//! tests pin the three things that makes true: the catalogue's defaults are
//! the researched cuts, an override reaches the verdict an agent reads, and
//! no write path can store a set that does not build.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use pierre_config::admin_types::{
    ConfigDataType, ConfigScope, ResetConfigRequest, UpdateConfigRequest, UpdateConfigResponse,
};
use pierre_config::tid_cuts::{
    build_tid_cuts, TidCutsConfigError, CATEGORY, HEART_RATE_BELOW_LT1_MAX_KEY,
    HEART_RATE_BETWEEN_MAX_KEY, MAX_PERCENT_CUT, MAX_RPE_CUT, SLOTS,
};
use pierre_core::models::periodization::{BandCuts, TidCuts, TidCutsError};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_mcp_server::config::admin::service::{AdminConfigService, UpdateConfigContext};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::ConfigLookupScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

mod common;

// ============================================================================
// Catalogue
// ============================================================================

#[tokio::test]
async fn the_catalogue_defaults_are_the_researched_cuts() -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;

    let mut resolved = HashMap::new();
    for slot in &SLOTS {
        for key in [slot.below_lt1_max_key, slot.between_max_key] {
            let value = svc
                .get_value(key, ConfigLookupScope::global())
                .await?
                .unwrap_or_else(|| panic!("{key} is registered"));
            resolved.insert(key, value.as_i64().expect("an integer cut"));
        }
    }
    assert_eq!(resolved.len(), 10, "five slots, two ceilings each");

    let built = build_tid_cuts(|key| resolved.get(key).copied())?;
    assert_eq!(built, TidCuts::default());
    // The researched values themselves, so a change to either side is seen.
    assert_eq!(resolved[HEART_RATE_BELOW_LT1_MAX_KEY], 89);
    assert_eq!(resolved[HEART_RATE_BETWEEN_MAX_KEY], 100);
    assert_eq!(resolved["tid_cuts.ftp.below_lt1_max"], 75);
    assert_eq!(resolved["tid_cuts.ftp.between_max"], 105);
    assert_eq!(resolved["tid_cuts.pace.below_lt1_max"], 88);
    assert_eq!(resolved["tid_cuts.pace.between_max"], 102);
    assert_eq!(resolved["tid_cuts.unstated.below_lt1_max"], 75);
    assert_eq!(resolved["tid_cuts.unstated.between_max"], 105);
    assert_eq!(resolved["tid_cuts.rpe.below_lt1_max"], 4);
    assert_eq!(resolved["tid_cuts.rpe.between_max"], 6);
    Ok(())
}

#[tokio::test]
async fn every_cut_surfaces_in_the_admin_catalog_on_its_slots_scale() -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let catalog = svc.get_catalog(ConfigLookupScope::global()).await?;

    let category = catalog
        .categories
        .iter()
        .find(|c| c.name == CATEGORY)
        .expect("the training_zones category is listed");
    let cuts: Vec<_> = category
        .parameters
        .iter()
        .filter(|p| p.key.starts_with("tid_cuts."))
        .collect();
    assert_eq!(
        cuts.len(),
        10,
        "{:?}",
        cuts.iter().map(|p| &p.key).collect::<Vec<_>>()
    );

    for slot in &SLOTS {
        for key in [slot.below_lt1_max_key, slot.between_max_key] {
            let parameter = cuts
                .iter()
                .find(|p| p.key == key)
                .unwrap_or_else(|| panic!("{key} is in the catalog"));
            assert_eq!(parameter.data_type, ConfigDataType::Integer);
            assert!(parameter.is_runtime_configurable);
            assert!(
                parameter
                    .description
                    .contains("Three-Zone Cuts by Threshold — Power, Heart Rate and Pace"),
                "{key} cites the vault note: {}",
                parameter.description
            );
            let range = parameter.valid_range.as_ref().expect("range-bounded");
            assert_eq!(range.min.as_i64(), Some(1));
            assert_eq!(range.max.as_i64(), Some(slot.max));
        }
    }

    // The ranges are exactly the scales `TidCuts::new` accepts: the ceiling
    // builds, one past it does not.
    let at = |max: i64| u16::try_from(max).unwrap();
    let default = TidCuts::default();
    let percent_top = BandCuts::new(1, at(MAX_PERCENT_CUT))?;
    assert!(TidCuts::new(
        percent_top,
        default.heart_rate(),
        default.pace(),
        default.unstated(),
        default.rpe()
    )
    .is_ok());
    let percent_over = BandCuts::new(1, at(MAX_PERCENT_CUT) + 1)?;
    assert!(TidCuts::new(
        percent_over,
        default.heart_rate(),
        default.pace(),
        default.unstated(),
        default.rpe()
    )
    .is_err());
    let rpe_top = BandCuts::new(1, at(MAX_RPE_CUT))?;
    assert!(TidCuts::new(
        default.ftp(),
        default.heart_rate(),
        default.pace(),
        default.unstated(),
        rpe_top
    )
    .is_ok());
    let rpe_over = BandCuts::new(1, at(MAX_RPE_CUT) + 1)?;
    assert!(TidCuts::new(
        default.ftp(),
        default.heart_rate(),
        default.pace(),
        default.unstated(),
        rpe_over
    )
    .is_err());
    Ok(())
}

#[test]
fn a_set_that_does_not_build_is_refused_naming_its_slot() {
    let default = TidCuts::default();
    let researched: HashMap<&str, i64> = SLOTS
        .iter()
        .flat_map(|slot| {
            let pair = slot.cuts(&default);
            [
                (slot.below_lt1_max_key, i64::from(pair.below_lt1_max())),
                (slot.between_max_key, i64::from(pair.between_max())),
            ]
        })
        .collect();

    let out_of_order = build_tid_cuts(|key| {
        if key == "tid_cuts.pace.below_lt1_max" {
            Some(110)
        } else {
            researched.get(key).copied()
        }
    });
    assert_eq!(
        out_of_order,
        Err(TidCutsConfigError::Invalid(TidCutsError::OutOfOrder {
            slot: "pace",
            below_lt1_max: 110,
            between_max: 102,
        }))
    );

    let off_scale = build_tid_cuts(|key| {
        if key == "tid_cuts.rpe.between_max" {
            Some(12)
        } else {
            researched.get(key).copied()
        }
    });
    assert_eq!(
        off_scale,
        Err(TidCutsConfigError::Invalid(TidCutsError::OutOfRange {
            slot: "rpe",
            field: "between_max",
            value: 12,
            min: 1,
            max: 10,
        }))
    );

    let missing = build_tid_cuts(|key| {
        (key != HEART_RATE_BETWEEN_MAX_KEY)
            .then(|| researched.get(key).copied())
            .flatten()
    });
    assert_eq!(
        missing,
        Err(TidCutsConfigError::Missing {
            key: HEART_RATE_BETWEEN_MAX_KEY
        })
    );
}

// ============================================================================
// Write-time refusal
// ============================================================================

fn context<'a>(admin_id: &'a str, scope: ConfigScope<'a>) -> UpdateConfigContext<'a> {
    UpdateConfigContext {
        admin_user_id: admin_id,
        admin_email: "tid-cuts-admin@example.com",
        scope,
        ip_address: None,
        user_agent: None,
    }
}

async fn write(
    svc: &AdminConfigService,
    admin_id: &str,
    scope: ConfigScope<'_>,
    parameters: &[(&str, i64)],
) -> Result<UpdateConfigResponse> {
    let parameters = parameters
        .iter()
        .map(|(key, value)| ((*key).to_owned(), json!(value)))
        .collect();
    Ok(svc
        .update_config(
            &UpdateConfigRequest {
                parameters,
                reason: Some("carnet#547 test".to_owned()),
            },
            context(admin_id, scope),
        )
        .await?)
}

/// An admin user, and the tenant it owns: override rows reference both.
async fn admin_and_tenant(db: &Database) -> Result<(String, String)> {
    let (admin_id, _) = common::create_test_user(db).await?;
    let tenants = db.repositories().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == admin_id)
        .ok_or_else(|| anyhow::anyhow!("the admin owns a tenant"))?;
    Ok((admin_id.to_string(), tenant.id.to_string()))
}

#[tokio::test]
async fn a_tenant_override_that_leaves_no_middle_zone_is_refused_naming_the_slot() -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let (admin, tenant_id) = admin_and_tenant(&db).await?;
    let tenant = tenant_id.as_str();

    let refused = write(
        &svc,
        &admin,
        ConfigScope::Tenant(tenant),
        &[(HEART_RATE_BELOW_LT1_MAX_KEY, 100)],
    )
    .await?;
    assert!(!refused.success);
    assert_eq!(refused.updated_count, 0);
    assert_eq!(refused.validation_errors.len(), 1);
    assert_eq!(
        refused.validation_errors[0].parameter,
        HEART_RATE_BELOW_LT1_MAX_KEY
    );
    assert_eq!(
        refused.validation_errors[0].message,
        "tid_cuts.heart_rate.below_lt1_max (100) must stay below \
         tid_cuts.heart_rate.between_max (100)"
    );
    let stored = svc
        .get_value(
            HEART_RATE_BELOW_LT1_MAX_KEY,
            ConfigLookupScope::tenant(tenant),
        )
        .await?;
    assert_eq!(stored, Some(json!(89)), "nothing was written");

    let off_scale = write(
        &svc,
        &admin,
        ConfigScope::Tenant(tenant),
        &[("tid_cuts.rpe.between_max", 11)],
    )
    .await?;
    assert!(!off_scale.success);
    assert_eq!(
        off_scale.validation_errors[0].parameter,
        "tid_cuts.rpe.between_max"
    );
    Ok(())
}

#[tokio::test]
async fn a_system_wide_write_is_refused_when_a_tenant_would_be_left_without_a_middle_zone(
) -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let (admin, tenant_id) = admin_and_tenant(&db).await?;
    let tenant = tenant_id.as_str();

    // 95 orders against the system-wide 100 the tenant reads.
    let tenant_row = write(
        &svc,
        &admin,
        ConfigScope::Tenant(tenant),
        &[(HEART_RATE_BELOW_LT1_MAX_KEY, 95)],
    )
    .await?;
    assert!(tenant_row.success, "{:?}", tenant_row.validation_errors);

    // 92 orders system-wide (89 < 92) but not for the tenant (95 >= 92).
    let refused = write(
        &svc,
        &admin,
        ConfigScope::Global,
        &[(HEART_RATE_BETWEEN_MAX_KEY, 92)],
    )
    .await?;
    assert!(!refused.success);
    assert_eq!(refused.validation_errors.len(), 1);
    assert_eq!(
        refused.validation_errors[0].message,
        format!(
            "tid_cuts.heart_rate.below_lt1_max (95) must stay below \
             tid_cuts.heart_rate.between_max (92) for tenant {tenant}, which overrides one of them"
        )
    );
    assert_eq!(
        svc.get_value(HEART_RATE_BETWEEN_MAX_KEY, ConfigLookupScope::global())
            .await?,
        Some(json!(100)),
        "nothing was written"
    );

    // One that orders for the tenant too lands.
    let accepted = write(
        &svc,
        &admin,
        ConfigScope::Global,
        &[(HEART_RATE_BETWEEN_MAX_KEY, 97)],
    )
    .await?;
    assert!(accepted.success, "{:?}", accepted.validation_errors);
    Ok(())
}

#[tokio::test]
async fn a_system_wide_reset_is_refused_when_a_tenant_would_be_left_without_a_middle_zone(
) -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let (admin, tenant_id) = admin_and_tenant(&db).await?;
    let tenant = tenant_id.as_str();

    let lowered = write(
        &svc,
        &admin,
        ConfigScope::Global,
        &[(HEART_RATE_BELOW_LT1_MAX_KEY, 60)],
    )
    .await?;
    assert!(lowered.success, "{:?}", lowered.validation_errors);
    // The tenant's 70 orders above the system-wide 60 it reads.
    let tenant_row = write(
        &svc,
        &admin,
        ConfigScope::Tenant(tenant),
        &[(HEART_RATE_BETWEEN_MAX_KEY, 70)],
    )
    .await?;
    assert!(tenant_row.success, "{:?}", tenant_row.validation_errors);

    // Resetting the system-wide 60 hands the tenant the default 89.
    let refused = svc
        .reset_config(
            &ResetConfigRequest {
                category: Some(CATEGORY.to_owned()),
                keys: Some(vec![HEART_RATE_BELOW_LT1_MAX_KEY.to_owned()]),
                reason: None,
            },
            context(&admin, ConfigScope::Global),
        )
        .await;
    let message = refused.expect_err("the reset is refused").to_string();
    assert!(
        message.contains(&format!(
            "tid_cuts.heart_rate.below_lt1_max (89) must stay below \
             tid_cuts.heart_rate.between_max (70) for tenant {tenant} after this reset"
        )),
        "{message}"
    );
    assert_eq!(
        svc.get_value(HEART_RATE_BELOW_LT1_MAX_KEY, ConfigLookupScope::global())
            .await?,
        Some(json!(60)),
        "nothing was reset"
    );
    Ok(())
}

// ============================================================================
// The verdict an agent reads
// ============================================================================

/// The executor, and the server context behind it, whose admin config
/// service writes the overrides the tool then reads.
async fn create_executor() -> Result<(Arc<UniversalToolExecutor>, Arc<ServerContext>)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let executor = Arc::new(
        UniversalToolExecutor::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>)
            .with_scopes(OAuthScope::self_grant()),
    );
    Ok((executor, resources))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("tid_cuts_{}@example.com", Uuid::new_v4());
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
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn day_in(week: &str, offset: i64) -> String {
    let start = chrono::NaiveDate::parse_from_str(week, "%Y-%m-%d").expect("a week start");
    (start + chrono::Duration::days(offset))
        .format("%Y-%m-%d")
        .to_string()
}

/// A steady run at 85–87 % of threshold heart rate: midpoint 86, below LT1 on
/// the researched cut of 89, between the thresholds on a cut of 83.
fn heart_rate_day(date: &str) -> Value {
    json!({"date": date, "sport": "run", "workout": "steady aerobic run",
           "duration_min": 60, "intensity": "85-87% LTHR"})
}

/// A week the phase wants almost all below LT1.
async fn save_easy_week(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    week: &str,
) -> Result<()> {
    let payload = json!({
        "coach_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Autumn 10k", "date": "2027-03-14", "discipline": "run_10k", "priority": "A" },
            "strategy": "aerobic base",
            "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                { "kind": "base", "start": week, "weeks": 6, "intent": "all easy",
                  "target_hours": 3.0,
                  "tid_target": {
                      "z1": {"min": 0.9, "max": 1.0},
                      "z2": {"min": 0.0, "max": 0.1},
                      "z3": {"min": 0.0, "max": 0.1}
                  } }
            ]
        },
        "weeks": [{ "week_start": week, "focus": "base week", "phase_index": 0, "days": [
            heart_rate_day(&day_in(week, 1)),
            heart_rate_day(&day_in(week, 3)),
            heart_rate_day(&day_in(week, 5)),
        ] }]
    });
    let saved = executor
        .execute_tool(request("save_training_plan", payload, user_id, tenant_id))
        .await?;
    assert!(saved.success, "the plan saves: {:?}", saved.error);
    Ok(())
}

/// The week's time-in-zone outcome, as `get_training_plan` hands it an agent.
async fn tid_outcome(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    week: &str,
) -> Result<String> {
    let asked = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach", "include_state": true}),
            user_id,
            tenant_id,
        ))
        .await?;
    let plan = asked.result.expect("a plan");
    let compliance = &plan["state"]["compliance_weeks"][0];
    assert_eq!(compliance["week_start"], week, "{plan}");
    Ok(compliance["tid"]
        .as_str()
        .unwrap_or_else(|| panic!("tid is reported: {compliance}"))
        .to_owned())
}

#[tokio::test]
async fn a_tenant_override_moves_the_verdict_for_that_tenant_only() -> Result<()> {
    let (executor, resources) = create_executor().await?;
    let (user_a, tenant_a) = create_test_user(&executor).await?;
    let (user_b, tenant_b) = create_test_user(&executor).await?;
    assert_ne!(tenant_a, tenant_b);
    let week = (chrono::Utc::now().date_naive() + chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();

    save_easy_week(&executor, user_a, &tenant_a, &week).await?;
    save_easy_week(&executor, user_b, &tenant_b, &week).await?;
    assert_eq!(
        tid_outcome(&executor, user_a, &tenant_a, &week).await?,
        "within"
    );
    assert_eq!(
        tid_outcome(&executor, user_b, &tenant_b, &week).await?,
        "within"
    );

    let svc = resources
        .agent
        .admin_config
        .as_ref()
        .expect("admin config is wired");
    let written = write(
        svc,
        &user_a.to_string(),
        ConfigScope::Tenant(&tenant_a),
        &[(HEART_RATE_BELOW_LT1_MAX_KEY, 83)],
    )
    .await?;
    assert!(written.success, "{:?}", written.validation_errors);

    // On a cut of 83 the 86 % midpoint sits between the thresholds, so the
    // week the phase wanted easy is all middle.
    assert_eq!(
        tid_outcome(&executor, user_a, &tenant_a, &week).await?,
        "off"
    );
    assert_eq!(
        tid_outcome(&executor, user_b, &tenant_b, &week).await?,
        "within",
        "tenant B still reads the researched cut"
    );
    Ok(())
}
