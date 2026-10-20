// ABOUTME: End-to-end test of StripeProvider.parse_webhook -> dispatch_billing_event -> tier flip
// ABOUTME: Signs a real Stripe webhook payload, verifies the adapter normalizes + persists the upsert
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use std::collections::HashMap;

use chrono::Utc;
use hmac::{Hmac, Mac};
use pierre_core::billing::stripe::{StripePriceConfig, StripeProvider};
use pierre_core::billing::{BillingProvider, EventEnvelope, WebhookPayload};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{SubscriptionStatus, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_routes_billing::{dispatch_billing_event, EventOutcome};
use pierre_test_support::db::create_test_db;
use sha2::Sha256;
use uuid::Uuid;

const WEBHOOK_SECRET: &str = "whsec_integration_test";
const STRIPE_PROVIDER: &str = "stripe";

/// Build a `StripeProvider` with a dummy client + price config. No real Stripe
/// HTTP calls are made — only `parse_webhook` (offline signature + parse) is
/// exercised here.
fn provider() -> StripeProvider {
    let client = dravr_stripe::StripeClient::new("sk_test_dummy", WEBHOOK_SECRET);
    let prices = StripePriceConfig {
        professional: "price_pro".to_owned(),
        enterprise: "price_ent".to_owned(),
        professional_extra_athlete: "price_athlete".to_owned(),
    };
    StripeProvider::new(client, prices)
}

/// Sign a webhook body the way Stripe does: HMAC-SHA256 over "{t}.{body}".
fn signed_header(body: &[u8]) -> String {
    let ts = Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("t={ts},v1={sig}")
}

async fn seed_starter_user_and_tenant(db: &Database) -> (Uuid, TenantId) {
    let repos = db.repositories();
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::generate();
    let now = Utc::now();

    let user = User {
        id: user_id,
        email: format!("stripe+{user_id}@example.com"),
        display_name: Some("Stripe Webhook Test".to_owned()),
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        strava_token: None,
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
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
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
async fn stripe_subscription_created_webhook_flips_tier() {
    let db = create_test_db().await.unwrap();
    let (user_id, tenant_id) = seed_starter_user_and_tenant(&db).await;
    let repos = db.repositories();

    let customer_id = format!("cus_{}", Uuid::new_v4().simple());
    let subscription_id = format!("sub_{}", Uuid::new_v4().simple());
    let event_id = format!("evt_{}", Uuid::new_v4().simple());

    // A realistic Stripe customer.subscription.created envelope carrying the
    // tenant/user/tier metadata the platform attaches at checkout.
    let body = serde_json::to_vec(&serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.created",
        "created": 1_700_000_100_i64,
        "data": { "object": {
            "id": subscription_id,
            "customer": customer_id,
            "status": "active",
            "current_period_start": 1_700_000_000_i64,
            "current_period_end": 1_702_592_000_i64,
            "cancel_at_period_end": false,
            "metadata": {
                "tenant_id": tenant_id.to_string(),
                "user_id": user_id.to_string(),
                "plan_tier": "professional"
            }
        }}
    }))
    .unwrap();

    let header = signed_header(&body);
    let mut headers = HashMap::new();
    headers.insert("stripe-signature".to_owned(), header);

    // Adapter verifies the signature and normalizes to an EventEnvelope.
    let envelope = provider()
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body: &body,
        })
        .await
        .expect("valid signed webhook should parse");

    assert_eq!(envelope.event_id, event_id);
    assert_eq!(envelope.event_type, "customer.subscription.created");

    // Dispatch through the same pipeline the route uses.
    let auth_repos = repos.auth_repos();
    let usage_repos = repos.usage_repos();
    dispatch_billing_event(
        &auth_repos,
        &usage_repos,
        STRIPE_PROVIDER,
        envelope.occurred_at,
        &envelope.event,
    )
    .await
    .expect("dispatch should succeed");

    // Tier + plan flipped.
    let user_after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user_after.tier, UserTier::Professional);
    let tenant_after = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(tenant_after.plan, "professional");

    // Subscription row persisted under the stripe provider.
    let row = repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .expect("subscription persisted");
    assert_eq!(row.status, SubscriptionStatus::Active);
    assert_eq!(row.plan_tier, UserTier::Professional);
    assert_eq!(row.provider, STRIPE_PROVIDER);
    assert_eq!(row.provider_customer_id, customer_id);
    assert_eq!(
        row.provider_subscription_id.as_deref(),
        Some(subscription_id.as_str())
    );
}

#[tokio::test]
async fn stripe_webhook_with_bad_signature_is_rejected() {
    let body = br#"{"id":"evt_x","type":"customer.subscription.created","data":{"object":{}}}"#;
    let ts = Utc::now().timestamp();
    let mut headers = HashMap::new();
    headers.insert("stripe-signature".to_owned(), format!("t={ts},v1=deadbeef"));

    let result = provider()
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body,
        })
        .await;
    assert!(result.is_err(), "forged signature must be rejected");
}

#[tokio::test]
async fn stripe_webhook_missing_signature_header_is_rejected() {
    let body = br#"{"id":"evt_y","type":"customer.subscription.created","data":{"object":{}}}"#;
    let headers = HashMap::new();

    let result = provider()
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body,
        })
        .await;
    assert!(result.is_err(), "missing signature header must be rejected");
}

/// A signed Stripe subscription event for `subscription_id`, created at
/// `created` (Unix seconds), parsed the way the webhook route parses it.
async fn deliver_subscription_event(
    event_type: &str,
    status: &str,
    created: i64,
    (customer_id, subscription_id): (&str, &str),
    (user_id, tenant_id): (Uuid, TenantId),
) -> EventEnvelope {
    let body = serde_json::to_vec(&serde_json::json!({
        "id": format!("evt_{}", Uuid::new_v4().simple()),
        "type": event_type,
        "created": created,
        "data": { "object": {
            "id": subscription_id,
            "customer": customer_id,
            "status": status,
            "current_period_start": 1_700_000_000_i64,
            "current_period_end": 1_702_592_000_i64,
            "cancel_at_period_end": status == "active",
            "canceled_at": (status == "canceled").then_some(created),
            "metadata": {
                "tenant_id": tenant_id.to_string(),
                "user_id": user_id.to_string(),
                "plan_tier": "professional"
            }
        }}
    }))
    .unwrap();
    let headers = HashMap::from([("stripe-signature".to_owned(), signed_header(&body))]);
    provider()
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body: &body,
        })
        .await
        .expect("a signed event parses")
}

// carnet#993: Stripe neither orders deliveries nor stops retrying, so an
// update created before the deletion can land after it. It must not bring the
// canceled subscription, or its paid tier, back.
#[tokio::test]
async fn a_late_update_never_revives_a_canceled_subscription() {
    let db = create_test_db().await.unwrap();
    let identity = seed_starter_user_and_tenant(&db).await;
    let (user_id, tenant_id) = identity;
    let repos = db.repositories();
    let auth_repos = repos.auth_repos();
    let usage_repos = repos.usage_repos();
    let customer_id = format!("cus_{}", Uuid::new_v4().simple());
    let subscription_id = format!("sub_{}", Uuid::new_v4().simple());
    let ids = (customer_id.as_str(), subscription_id.as_str());
    let (t0, t1, t2) = (1_700_000_000_i64, 1_700_000_500_i64, 1_700_001_000_i64);

    for (event_type, status, created) in [
        ("customer.subscription.created", "active", t0),
        ("customer.subscription.deleted", "canceled", t2),
    ] {
        let envelope = deliver_subscription_event(event_type, status, created, ids, identity).await;
        let outcome = dispatch_billing_event(
            &auth_repos,
            &usage_repos,
            STRIPE_PROVIDER,
            envelope.occurred_at,
            &envelope.event,
        )
        .await
        .unwrap();
        assert_eq!(outcome, EventOutcome::Applied, "{event_type}");
    }

    // The retried update: a different evt_ id, created before the deletion.
    let late =
        deliver_subscription_event("customer.subscription.updated", "active", t1, ids, identity)
            .await;
    let outcome = dispatch_billing_event(
        &auth_repos,
        &usage_repos,
        STRIPE_PROVIDER,
        late.occurred_at,
        &late.event,
    )
    .await
    .unwrap();
    assert_eq!(outcome, EventOutcome::Superseded);

    let row = repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SubscriptionStatus::Canceled);
    assert_eq!(row.last_event_at.map(|at| at.timestamp()), Some(t2));
    let user = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(
        user.tier,
        UserTier::Starter,
        "the paid tier stays withdrawn"
    );
    let tenant = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(tenant.plan, "starter");

    // An update for the same subscription created after the deletion still
    // cannot revive it: a deleted Stripe subscription stays deleted.
    let after = deliver_subscription_event(
        "customer.subscription.updated",
        "active",
        t2 + 60,
        ids,
        identity,
    )
    .await;
    let outcome = dispatch_billing_event(
        &auth_repos,
        &usage_repos,
        STRIPE_PROVIDER,
        after.occurred_at,
        &after.event,
    )
    .await
    .unwrap();
    assert_eq!(outcome, EventOutcome::Superseded);
}

// The canceled guard keys on the provider subscription: the same customer
// subscribing again is a new subscription, and it is applied.
#[tokio::test]
async fn a_new_subscription_for_a_canceled_customer_is_applied() {
    let db = create_test_db().await.unwrap();
    let identity = seed_starter_user_and_tenant(&db).await;
    let (user_id, _tenant_id) = identity;
    let repos = db.repositories();
    let auth_repos = repos.auth_repos();
    let usage_repos = repos.usage_repos();
    let customer_id = format!("cus_{}", Uuid::new_v4().simple());
    let first = format!("sub_{}", Uuid::new_v4().simple());
    let second = format!("sub_{}", Uuid::new_v4().simple());

    for (event_type, status, created, subscription_id) in [
        (
            "customer.subscription.created",
            "active",
            1_700_000_000_i64,
            &first,
        ),
        (
            "customer.subscription.deleted",
            "canceled",
            1_700_001_000_i64,
            &first,
        ),
        (
            "customer.subscription.created",
            "active",
            1_700_002_000_i64,
            &second,
        ),
    ] {
        let envelope = deliver_subscription_event(
            event_type,
            status,
            created,
            (customer_id.as_str(), subscription_id.as_str()),
            identity,
        )
        .await;
        let outcome = dispatch_billing_event(
            &auth_repos,
            &usage_repos,
            STRIPE_PROVIDER,
            envelope.occurred_at,
            &envelope.event,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            EventOutcome::Applied,
            "{event_type} {subscription_id}"
        );
    }

    let row = repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SubscriptionStatus::Active);
    assert_eq!(
        row.provider_subscription_id.as_deref(),
        Some(second.as_str())
    );
    let user = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(user.tier, UserTier::Professional);
}
