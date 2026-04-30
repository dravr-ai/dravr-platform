// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Integration test for the billing webhook → DB → tier mutation pipeline
// ABOUTME: Drives dispatch_billing_event with synthetic BillingEvents (no provider HTTP layer involved)

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use chrono::Utc;
use pierre_core::billing::{BillingEvent, SubscriptionEventPayload};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{SubscriptionStatus, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_mcp_server::routes::billing::dispatch_billing_event;
use uuid::Uuid;

const TEST_PROVIDER: &str = "dummy";

fn upsert_payload(
    tenant_id: TenantId,
    user_id: Uuid,
    customer_id: &str,
    subscription_id: Option<&str>,
    plan_tier: &str,
    status: &str,
) -> SubscriptionEventPayload {
    SubscriptionEventPayload {
        provider_customer_id: customer_id.to_owned(),
        provider_subscription_id: subscription_id.map(str::to_owned),
        tenant_id: tenant_id.to_string(),
        user_id: user_id.to_string(),
        plan_tier: plan_tier.to_owned(),
        status: status.to_owned(),
        current_period_start: Some(Utc::now()),
        current_period_end: Some(Utc::now() + chrono::Duration::days(30)),
        cancel_at_period_end: false,
        canceled_at: None,
        trial_end: None,
        metadata: None,
    }
}

async fn seed_starter_user_and_tenant(db: &Database) -> (Uuid, TenantId) {
    let repos = db.repositories();
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::new();
    let now = Utc::now();

    let user = User {
        id: user_id,
        email: format!("phase5b+{user_id}@example.com"),
        display_name: Some("Phase 5B Test".to_owned()),
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(user_id),
        approved_at: Some(now),
        created_at: now,
        last_active: now,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "en".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
    };
    repos.users.create(&user).await.unwrap();

    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {tenant_id}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    (user_id, tenant_id)
}

#[tokio::test]
async fn subscription_upserted_flips_tier_and_plan() {
    let db = create_test_db().await.unwrap();
    let (user_id, tenant_id) = seed_starter_user_and_tenant(&db).await;
    let repos = db.repositories();

    // Sanity: user starts as Starter, tenant as starter.
    let user_before = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_before.tier, UserTier::Starter);
    let tenant_before = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(tenant_before.plan, "starter");

    let customer_id = format!("cus_test_{}", Uuid::new_v4().simple());
    let subscription_id = format!("sub_test_{}", Uuid::new_v4().simple());
    let event = BillingEvent::SubscriptionUpserted(Box::new(upsert_payload(
        tenant_id,
        user_id,
        &customer_id,
        Some(&subscription_id),
        "professional",
        "active",
    )));

    dispatch_billing_event(&repos, TEST_PROVIDER, &event)
        .await
        .expect("dispatch should succeed");

    let user_after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_after.tier, UserTier::Professional);

    let tenant_after = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(tenant_after.plan, "professional");

    let by_user = repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .expect("subscription persisted");
    assert_eq!(by_user.status, SubscriptionStatus::Active);
    assert_eq!(by_user.plan_tier, UserTier::Professional);
    assert_eq!(by_user.provider, TEST_PROVIDER);
    assert_eq!(by_user.provider_customer_id, customer_id);
    assert_eq!(
        by_user.provider_subscription_id.as_deref(),
        Some(subscription_id.as_str())
    );

    let by_provider_sub = repos
        .subscriptions
        .get_subscription_by_provider_subscription_id(TEST_PROVIDER, &subscription_id)
        .await
        .unwrap();
    assert!(by_provider_sub.is_some());
}

#[tokio::test]
async fn subscription_canceled_downgrades_to_starter() {
    let db = create_test_db().await.unwrap();
    let (user_id, tenant_id) = seed_starter_user_and_tenant(&db).await;
    let repos = db.repositories();

    repos
        .users
        .set_tier(user_id, UserTier::Professional)
        .await
        .unwrap();
    repos
        .tenants
        .set_plan(tenant_id, "professional")
        .await
        .unwrap();

    let customer_id = format!("cus_test_{}", Uuid::new_v4().simple());
    let subscription_id = format!("sub_test_{}", Uuid::new_v4().simple());

    // Seed an active subscription so the cancel path has a row to update.
    let upsert = BillingEvent::SubscriptionUpserted(Box::new(upsert_payload(
        tenant_id,
        user_id,
        &customer_id,
        Some(&subscription_id),
        "professional",
        "active",
    )));
    dispatch_billing_event(&repos, TEST_PROVIDER, &upsert)
        .await
        .unwrap();

    let cancel = BillingEvent::SubscriptionCanceled {
        provider_subscription_id: subscription_id.clone(),
        canceled_at: Some(Utc::now()),
    };

    dispatch_billing_event(&repos, TEST_PROVIDER, &cancel)
        .await
        .expect("dispatch should succeed");

    let user_after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_after.tier, UserTier::Starter);
    let tenant_after = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(tenant_after.plan, "starter");

    let row = repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SubscriptionStatus::Canceled);
    assert!(row.canceled_at.is_some());
}

#[tokio::test]
async fn payment_failed_flips_status_to_past_due() {
    let db = create_test_db().await.unwrap();
    let (user_id, tenant_id) = seed_starter_user_and_tenant(&db).await;
    let repos = db.repositories();

    let customer_id = format!("cus_test_{}", Uuid::new_v4().simple());
    let subscription_id = format!("sub_test_{}", Uuid::new_v4().simple());

    let upsert = BillingEvent::SubscriptionUpserted(Box::new(upsert_payload(
        tenant_id,
        user_id,
        &customer_id,
        Some(&subscription_id),
        "professional",
        "active",
    )));
    dispatch_billing_event(&repos, TEST_PROVIDER, &upsert)
        .await
        .unwrap();

    let payment_failed = BillingEvent::PaymentFailed {
        provider_subscription_id: subscription_id.clone(),
    };
    dispatch_billing_event(&repos, TEST_PROVIDER, &payment_failed)
        .await
        .expect("payment_failed dispatch should succeed");

    let row = repos
        .subscriptions
        .get_subscription_by_provider_subscription_id(TEST_PROVIDER, &subscription_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SubscriptionStatus::PastDue);

    // Tier downgrade is *not* applied here — that is the dunning cron's
    // responsibility once the grace period elapses (provider-specific,
    // wired in dravr-stripe / dravr-revenuecat / etc.).
    let user_after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_after.tier, UserTier::Professional);
}

#[tokio::test]
async fn idempotency_skips_duplicate_event_id() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    assert!(!repos
        .subscriptions
        .is_billing_event_processed(TEST_PROVIDER, "evt_x")
        .await
        .unwrap());
    repos
        .subscriptions
        .mark_billing_event_processed(TEST_PROVIDER, "evt_x", "subscription.upserted")
        .await
        .unwrap();
    assert!(repos
        .subscriptions
        .is_billing_event_processed(TEST_PROVIDER, "evt_x")
        .await
        .unwrap());

    // Calling mark again is a no-op (INSERT OR IGNORE / ON CONFLICT DO NOTHING).
    repos
        .subscriptions
        .mark_billing_event_processed(TEST_PROVIDER, "evt_x", "subscription.upserted")
        .await
        .unwrap();
}

#[tokio::test]
async fn ignored_event_is_a_noop() {
    let db = create_test_db().await.unwrap();
    let (user_id, _tenant_id) = seed_starter_user_and_tenant(&db).await;
    let repos = db.repositories();

    dispatch_billing_event(&repos, TEST_PROVIDER, &BillingEvent::Ignored)
        .await
        .expect("ignored events should dispatch cleanly");

    // No subscription row, no tier flip.
    assert!(repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .is_none());
    let user_after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_after.tier, UserTier::Starter);
}
