// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Round-trip test for the SubscriptionsRepository (Phase 5 billing foundation)
// ABOUTME: Asserts upsert + read-back across every lookup method on the SQLite backend

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserTier};
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

const TEST_PROVIDER: &str = "dummy";

fn sample_subscription() -> Subscription {
    Subscription {
        id: Uuid::new_v4(),
        tenant_id: TenantId::new(),
        user_id: Uuid::new_v4(),
        provider: TEST_PROVIDER.to_owned(),
        provider_customer_id: format!("cus_test_{}", Uuid::new_v4().simple()),
        provider_subscription_id: Some(format!("sub_test_{}", Uuid::new_v4().simple())),
        status: SubscriptionStatus::Active,
        plan_tier: UserTier::Professional,
        current_period_start: Some(Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap()),
        current_period_end: Some(Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap()),
        cancel_at_period_end: false,
        canceled_at: None,
        trial_end: None,
        metadata: Some(serde_json::json!({"source": "checkout"})),
        created_at: Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap(),
    }
}

#[tokio::test]
async fn upsert_subscription_inserts_then_reads_back_via_every_lookup() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let sub = sample_subscription();

    let inserted = repos.subscriptions.upsert_subscription(&sub).await.unwrap();
    assert_eq!(inserted.id, sub.id);
    assert_eq!(inserted.provider, TEST_PROVIDER);
    assert_eq!(inserted.provider_customer_id, sub.provider_customer_id);
    assert_eq!(inserted.status, SubscriptionStatus::Active);
    assert_eq!(inserted.plan_tier, UserTier::Professional);
    assert!(inserted.is_entitled());

    let by_user = repos
        .subscriptions
        .get_subscription_by_user(sub.user_id)
        .await
        .unwrap()
        .expect("by-user lookup should hit");
    assert_eq!(by_user.id, sub.id);

    let by_tenant = repos
        .subscriptions
        .get_subscription_by_tenant(sub.tenant_id)
        .await
        .unwrap()
        .expect("by-tenant lookup should hit");
    assert_eq!(by_tenant.id, sub.id);

    let by_provider_sub = repos
        .subscriptions
        .get_subscription_by_provider_subscription_id(
            TEST_PROVIDER,
            sub.provider_subscription_id.as_deref().unwrap(),
        )
        .await
        .unwrap()
        .expect("by-provider-subscription-id lookup should hit");
    assert_eq!(by_provider_sub.id, sub.id);

    let by_provider_cus = repos
        .subscriptions
        .get_subscription_by_provider_customer_id(TEST_PROVIDER, &sub.provider_customer_id)
        .await
        .unwrap()
        .expect("by-provider-customer-id lookup should hit");
    assert_eq!(by_provider_cus.id, sub.id);

    let active = repos
        .subscriptions
        .list_subscriptions_by_status(SubscriptionStatus::Active)
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, sub.id);
}

#[tokio::test]
async fn upsert_subscription_updates_existing_row_on_status_change() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let mut sub = sample_subscription();
    repos.subscriptions.upsert_subscription(&sub).await.unwrap();

    sub.status = SubscriptionStatus::PastDue;
    sub.updated_at = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
    let updated = repos.subscriptions.upsert_subscription(&sub).await.unwrap();
    assert_eq!(updated.status, SubscriptionStatus::PastDue);
    assert!(!updated.is_entitled());

    // Only one row regardless of upsert count.
    let active_now = repos
        .subscriptions
        .list_subscriptions_by_status(SubscriptionStatus::Active)
        .await
        .unwrap();
    assert!(active_now.is_empty());

    let past_due = repos
        .subscriptions
        .list_subscriptions_by_status(SubscriptionStatus::PastDue)
        .await
        .unwrap();
    assert_eq!(past_due.len(), 1);
}

#[tokio::test]
async fn billing_event_idempotency_is_keyed_per_provider() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    assert!(!repos
        .subscriptions
        .is_billing_event_processed(TEST_PROVIDER, "evt_1")
        .await
        .unwrap());

    repos
        .subscriptions
        .mark_billing_event_processed(TEST_PROVIDER, "evt_1", "subscription.upserted")
        .await
        .unwrap();

    assert!(repos
        .subscriptions
        .is_billing_event_processed(TEST_PROVIDER, "evt_1")
        .await
        .unwrap());

    // The same event_id under a different provider is a different row.
    assert!(!repos
        .subscriptions
        .is_billing_event_processed("other_provider", "evt_1")
        .await
        .unwrap());

    // Re-marking the same (provider, event_id) is a no-op (idempotent).
    repos
        .subscriptions
        .mark_billing_event_processed(TEST_PROVIDER, "evt_1", "subscription.upserted")
        .await
        .unwrap();
}
