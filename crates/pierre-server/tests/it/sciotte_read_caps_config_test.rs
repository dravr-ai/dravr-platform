// ABOUTME: The sciotte session-read caps as admin config — every cap of every tier is in the catalog
// ABOUTME: An athlete's caps resolve by their tier, user over system-wide, and reach the limiter (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! JF on carnet#1661: "We must be able to change those caps using
//! pierre-cli." `pierre-cli config` reads and writes the admin catalog
//! through `AdminConfigService`, so these tests drive the same service: the
//! catalog lists every cap, a write reaches the athlete's resolved caps by
//! their tier, and the resolved caps are the ones the rate limiter counts
//! against.

use std::iter;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::http::StatusCode;
use chrono::Utc;
use pierre_config::admin_types::{ConfigDataType, ConfigScope, UpdateConfigRequest};
use pierre_core::admin::models::DEVICE_CLI_SERVICE_PREFIX;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{SessionReadCap, SessionReadCaps, UserTier, SESSION_READ_CAPS_CATEGORY};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::config::admin::service::{AdminConfigService, UpdateConfigContext};
use pierre_mcp_server::config::sciotte_read_caps::SciotteReadCaps;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::request_budget::{
    budget_period, session_read_cap, session_read_counter_key, ProviderRateLimiter, RequestBudget,
    SessionReadKind, SessionReadLimits, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use pierre_test_support::db::create_test_db;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::admin_config_device_token_test::{admin_token, router, user_with_role};
use crate::common;
use crate::helpers::axum_test::AxumTestRequest;

/// The service as the resolver reads it.
fn lookup(svc: &Arc<AdminConfigService>) -> Arc<dyn AdminConfigLookup> {
    Arc::<AdminConfigService>::clone(svc)
}

/// Write `key` = `value` at `scope`, as `pierre-cli config set` does.
async fn set(
    svc: &AdminConfigService,
    admin_id: &str,
    scope: ConfigScope<'_>,
    key: &str,
    value: u32,
) -> Result<()> {
    let response = svc
        .update_config(
            &UpdateConfigRequest {
                parameters: iter::once((key.to_owned(), json!(value))).collect(),
                reason: Some("carnet#1661 test".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: admin_id,
                admin_email: "sciotte-reads-admin@example.com",
                scope,
                ip_address: None,
                user_agent: None,
            },
        )
        .await?;
    assert!(
        response.success && response.validation_errors.is_empty(),
        "{key} = {value} was refused: {:?}",
        response.validation_errors
    );
    Ok(())
}

/// An athlete on `tier`.
async fn athlete(db: &Database, email: &str, tier: UserTier) -> Result<Uuid> {
    let (user_id, _) = common::create_test_user_with_email(db, email).await?;
    db.repositories().users.set_tier(user_id, tier).await?;
    Ok(user_id)
}

/// Every tier a cap has a parameter for.
const TIERS: [UserTier; 3] = [
    UserTier::Starter,
    UserTier::Professional,
    UserTier::Enterprise,
];

#[tokio::test]
async fn every_cap_of_every_tier_is_in_the_sciotte_reads_category() -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let catalog = svc.get_catalog(ConfigLookupScope::global()).await?;
    let category = catalog
        .categories
        .iter()
        .find(|c| c.name == SESSION_READ_CAPS_CATEGORY)
        .expect("the sciotte_reads category is listed, so pierre-cli config show can see it");
    assert_eq!(
        category.parameters.len(),
        TIERS.len() * SessionReadCap::ALL.len(),
        "bug: a cap of a tier is missing from the catalog, or listed twice: {:?}",
        category
            .parameters
            .iter()
            .map(|p| &p.key)
            .collect::<Vec<_>>()
    );
    for tier in &TIERS {
        let compiled = SessionReadCaps::for_tier(tier);
        for cap in SessionReadCap::ALL {
            let key = cap.config_key(tier);
            let parameter = category
                .parameters
                .iter()
                .find(|p| p.key == key)
                .unwrap_or_else(|| panic!("{key} is in the catalog"));
            assert_eq!(
                parameter.data_type,
                ConfigDataType::Integer,
                "bug: {key} is not an integer, so a typo would not be refused"
            );
            assert!(
                parameter.is_runtime_configurable,
                "bug: {key} cannot be changed at runtime by pierre-cli"
            );
            assert_eq!(
                parameter.default_value,
                json!(cap.of(&compiled)),
                "bug: {key} does not default to the compiled cap"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn an_athletes_caps_resolve_by_tier_user_over_system_wide() -> Result<()> {
    let db = create_test_db().await?;
    let svc = Arc::new(AdminConfigService::for_database(&db).await?);
    let (admin, _) = common::create_test_user(&db).await?;
    let admin = admin.to_string();
    let starter = athlete(&db, "starter-athlete@example.com", UserTier::Starter).await?;
    let pro = athlete(&db, "pro-athlete@example.com", UserTier::Professional).await?;
    let resolver = SciotteReadCaps::new(Some(lookup(&svc)), Arc::clone(&db.repositories().users));

    // Unset: each tier's compiled caps.
    assert_eq!(
        resolver.caps(pro).await.caps,
        SessionReadCaps::for_tier(&UserTier::Professional),
        "bug: with nothing set, a Professional athlete is not held to their tier's caps"
    );

    // A system-wide Starter override moves Starter athletes, not a
    // Professional one.
    let starter_day = SessionReadCap::AskedDaily.config_key(&UserTier::Starter);
    set(&svc, &admin, ConfigScope::Global, &starter_day, 90).await?;
    assert_eq!(
        resolver.caps(starter).await.caps.asked_daily,
        90,
        "bug: a system-wide Starter override did not reach a Starter athlete"
    );
    assert_eq!(
        resolver.caps(pro).await.caps.asked_daily,
        SessionReadCaps::for_tier(&UserTier::Professional).asked_daily,
        "a Starter key moved a Professional athlete"
    );

    // One athlete's own override wins over the system-wide one.
    let starter_id = starter.to_string();
    set(
        &svc,
        &admin,
        ConfigScope::User(&starter_id),
        &starter_day,
        12,
    )
    .await?;
    let own = resolver.caps(starter).await;
    assert_eq!(
        own.tier,
        UserTier::Starter,
        "bug: the athlete's tier was resolved wrong"
    );
    assert_eq!(
        own.caps.asked_daily, 12,
        "bug: the athlete's own override did not win over the system-wide one"
    );

    // A Professional key set for a Starter athlete is not read for them.
    let pro_burst = SessionReadCap::Background15Min.config_key(&UserTier::Professional);
    set(&svc, &admin, ConfigScope::User(&starter_id), &pro_burst, 3).await?;
    assert_eq!(
        resolver.caps(starter).await.caps.background_15min,
        SessionReadCaps::for_tier(&UserTier::Starter).background_15min,
        "bug: a Professional key set for a Starter athlete was read for them"
    );

    // With no admin config, the tier's compiled caps.
    let offline = SciotteReadCaps::new(None, Arc::clone(&db.repositories().users));
    assert_eq!(
        offline.caps(starter).await.caps,
        SessionReadCaps::for_tier(&UserTier::Starter),
        "bug: without admin config, the tier's compiled caps do not apply"
    );
    Ok(())
}

#[tokio::test]
async fn a_cap_set_through_the_admin_config_is_what_the_limiter_refuses_at() -> Result<()> {
    let db = create_test_db().await?;
    let svc = Arc::new(AdminConfigService::for_database(&db).await?);
    let (admin, _) = common::create_test_user(&db).await?;
    let athlete_id = athlete(&db, "capped-athlete@example.com", UserTier::Starter).await?;
    let repos = db.repositories();
    let limiter = Arc::new(ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    ));
    limiter.bind_session_read_limits(
        Arc::new(SciotteReadCaps::new(
            Some(lookup(&svc)),
            Arc::clone(&repos.users),
        )),
        Duration::ZERO,
    );
    let budget = RequestBudget::for_session_read(&limiter, "sciotte", athlete_id)
        .expect("the scraper's Strava session is counted");

    let day = SessionReadCap::AskedDaily.config_key(&UserTier::Starter);
    set(
        &svc,
        &admin.to_string(),
        ConfigScope::User(&athlete_id.to_string()),
        &day,
        2,
    )
    .await?;
    for read in 0..2 {
        assert!(
            budget.admit("sciotte").await.is_ok(),
            "read {read} is within the athlete's day of 2"
        );
    }
    let refused = budget
        .admit("sciotte")
        .await
        .expect_err("the third read passes the day of 2 set for the athlete");
    assert_eq!(
        refused.code,
        ErrorCode::ExternalRateLimited,
        "bug: a read past the athlete's day refuses with another code"
    );
    assert_eq!(
        session_read_cap(&refused),
        Some(SessionReadKind::Asked),
        "bug: the refusal does not name the asked cap"
    );
    let counted = repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &session_read_counter_key("sciotte", "asked", athlete_id),
            &budget_period(Utc::now(), ONE_DAY),
        )
        .await?;
    assert_eq!(counted.value, 2, "the refused read took no slot");

    // Raised again, the next read goes through with no restart.
    set(
        &svc,
        &admin.to_string(),
        ConfigScope::User(&athlete_id.to_string()),
        &day,
        3,
    )
    .await?;
    assert!(
        budget.admit("sciotte").await.is_ok(),
        "bug: a raised day did not admit the next read"
    );
    Ok(())
}

/// The server binds its limiter to the admin config at startup: through the
/// running server's own provider registry, a Professional athlete reads past
/// the Starter day, which an unbound limiter would refuse.
#[tokio::test]
async fn the_running_server_counts_session_reads_by_the_athletes_tier() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let data = resources.data();
    let db = data.database();
    let pro = athlete(db, "bound-pro@example.com", UserTier::Professional).await?;
    let key = session_read_counter_key("sciotte", "asked", pro);
    db.repositories()
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(Utc::now(), ONE_DAY),
            i64::from(SessionReadCaps::for_tier(&UserTier::Starter).asked_daily),
        )
        .await?;
    let budget = resources
        .data()
        .provider_registry()
        .session_read_budget("sciotte", pro)
        .expect("the server counts the scraper's Strava session");
    assert!(
        budget.admit("sciotte").await.is_ok(),
        "a Professional athlete was held to the Starter day: the limiter is not bound"
    );
    Ok(())
}

/// `pierre-cli config set <key> <value> [--user <id>]` as it goes on the wire:
/// `PUT /api/admin/config`, `?user_id=` for a `--user` write, the number the
/// admin typed sent as a JSON number (`parse_set_value`), under the admin
/// token `pierre-cli auth login` holds.
async fn cli_set(
    resources: &Arc<ServerContext>,
    token: &str,
    key: &str,
    value: u32,
    user: Option<Uuid>,
) -> Result<()> {
    let query = user.map_or_else(String::new, |user| format!("?user_id={user}"));
    let response = AxumTestRequest::put(&format!("/api/admin/config{query}"))
        .header("authorization", token)
        .json(&json!({
            "parameters": { key: value },
            "reason": "carnet#1661 via pierre-cli",
        }))
        .send(router(resources))
        .await;
    let status = response.status_code();
    let text = response.text();
    assert_eq!(
        status,
        StatusCode::OK,
        "pierre-cli config set {key} {value} was refused: {text}"
    );
    let body: Value = serde_json::from_str(&text)?;
    let data = body.get("data").unwrap_or(&body);
    assert_eq!(
        data["validation_errors"],
        json!([]),
        "{key} = {value}: {body}"
    );
    assert_eq!(
        data["requires_restart"], false,
        "{key} takes effect with no restart: {body}"
    );
    Ok(())
}

/// JF on carnet#1661: "We must be able to change those caps using
/// pierre-cli." Every one of the fifteen caps is written the way
/// `pierre-cli config set` writes it, through the admin config route the CLI
/// calls, and each is what an athlete of its tier is then held to. A `--user`
/// write reaches the running server's own limiter on that athlete's next
/// read, with no restart.
#[tokio::test]
async fn every_cap_is_changed_by_pierre_cli_config_set_with_no_restart() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let operator = user_with_role(&resources, UserRole::Admin).await;
    let token = admin_token(
        &resources,
        &format!("{DEVICE_CLI_SERVICE_PREFIX}{}", operator.id),
        true,
        Some(operator.id),
    )
    .await;

    // `pierre-cli config set sciotte_reads.<cap>.<tier> <n>`, all fifteen,
    // each a value of its own.
    let mut expected = Vec::new();
    for (step, tier) in (0_u32..).zip(&TIERS) {
        let mut caps = SessionReadCaps::for_tier(tier);
        for (index, cap) in (0_u32..).zip(SessionReadCap::ALL) {
            let value = 7 + 10 * step + index;
            cli_set(&resources, &token, &cap.config_key(tier), value, None).await?;
            cap.set(&mut caps, value);
        }
        expected.push((tier.clone(), caps));
    }
    let service = resources
        .agent
        .admin_config
        .clone()
        .expect("the test server runs the admin config");
    let users = Arc::clone(&resources.common.repos.users);
    let resolver = SciotteReadCaps::new(Some(lookup(&service)), Arc::clone(&users));
    let db = &resources.agent.database;
    for (tier, caps) in expected {
        let athlete_id = athlete(
            db,
            &format!("cli-{}@example.com", Uuid::new_v4()),
            tier.clone(),
        )
        .await?;
        let held = resolver.caps(athlete_id).await;
        assert_eq!(
            held.tier, tier,
            "bug: the athlete's tier was resolved wrong"
        );
        assert_eq!(
            held.caps, caps,
            "a {tier:?} athlete is held to the caps pierre-cli set for {tier:?}"
        );
    }

    // `pierre-cli config set sciotte_reads.asked_daily.starter 2 --user <id>`:
    // the running server's limiter refuses that athlete's third read.
    let athlete_id = athlete(
        db,
        &format!("cli-user-{}@example.com", Uuid::new_v4()),
        UserTier::Starter,
    )
    .await?;
    cli_set(
        &resources,
        &token,
        &SessionReadCap::AskedDaily.config_key(&UserTier::Starter),
        2,
        Some(athlete_id),
    )
    .await?;
    let budget = resources
        .data()
        .provider_registry()
        .session_read_budget("sciotte", athlete_id)
        .expect("the server counts the scraper's Strava session");
    for read in 0..2 {
        assert!(
            budget.admit("sciotte").await.is_ok(),
            "read {read} is within the day of 2 pierre-cli set"
        );
    }
    let refused = budget
        .admit("sciotte")
        .await
        .expect_err("the third read passes the day of 2 pierre-cli set for the athlete");
    assert_eq!(
        refused.code,
        ErrorCode::ExternalRateLimited,
        "bug: a read past the day pierre-cli set refuses with another code"
    );
    assert_eq!(
        session_read_cap(&refused),
        Some(SessionReadKind::Asked),
        "bug: the refusal does not name the asked cap"
    );
    Ok(())
}
