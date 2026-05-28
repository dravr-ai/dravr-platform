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
use pierre_core::billing::{BillingProvider, WebhookPayload};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{SubscriptionStatus, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_routes_billing::dispatch_billing_event;
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
    let tenant_id = TenantId::new();
    let now = Utc::now();

    let user = User {
        id: user_id,
        email: format!("stripe+{user_id}@example.com"),
        display_name: Some("Stripe Webhook Test".to_owned()),
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
        timezone: None,
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
    dispatch_billing_event(&auth_repos, &usage_repos, STRIPE_PROVIDER, &envelope.event)
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
