// ABOUTME: Pins that POST /api/billing/{checkout,portal} take their identity from the bearer token, never the body
// ABOUTME: An anonymous caller gets 401; a body naming another user or tenant is ignored; the portal customer is the caller's own row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Until 2026-09-18 both handlers deserialized the provider input straight
//! off the wire and took no `AuthenticatedUser`, so an anonymous caller
//! could start a checkout attaching a paid tier to any tenant, and open the
//! hosted portal of any customer id it could guess (carnet#458). These
//! tests post through the real router with the `DummyProvider`, whose
//! deterministic URLs echo back exactly the ids the handler resolved, and
//! check both halves: the status, and whose identity reached the provider.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use chrono::{Duration, Utc};
use pierre_core::billing::dummy::DUMMY_PROVIDER_NAME;
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, User, UserTier};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;

/// One user on one tenant, the billing router over the shared resources,
/// and a bearer token whose `active_tenant_id` claim names that tenant.
struct Fixture {
    resources: Arc<ServerContext>,
    user: User,
    user_id: Uuid,
    tenant_id: TenantId,
    bearer: String,
}

impl Fixture {
    async fn new() -> Self {
        let resources = create_test_server_resources()
            .await
            .expect("server resources");
        let (user_id, user) = create_test_user(&resources.agent.database)
            .await
            .expect("test user");
        let tenant_id = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap()[0]
            .id;
        let token = generate_test_token(&resources, &user).await;
        Self {
            resources,
            user,
            user_id,
            tenant_id,
            bearer: format!("Bearer {token}"),
        }
    }

    fn router(&self) -> Router {
        pierre_routes_billing::billing_routes::<ServerContext>()
            .with_state(Arc::clone(&self.resources))
    }

    /// A bearer token for the same user that carries no tenant claim, so the
    /// handler must fall back to the user's first membership.
    fn bearer_without_tenant(&self) -> String {
        let token = self
            .resources
            .auth
            .auth_manager
            .generate_token_with_tenant(&self.user, &self.resources.auth.jwks_manager, None)
            .unwrap();
        format!("Bearer {token}")
    }

    /// Seed an entitled subscription row for the fixture user under the
    /// dummy provider and return the provider customer id it carries.
    async fn seed_subscription(&self) -> String {
        let customer_id = format!("cus_own_{}", Uuid::new_v4().simple());
        let now = Utc::now();
        let sub = Subscription {
            id: Uuid::new_v4(),
            tenant_id: self.tenant_id,
            user_id: self.user_id,
            provider: DUMMY_PROVIDER_NAME.to_owned(),
            provider_customer_id: customer_id.clone(),
            provider_subscription_id: Some(format!("sub_own_{}", Uuid::new_v4().simple())),
            status: SubscriptionStatus::Active,
            plan_tier: UserTier::Professional,
            current_period_start: Some(now),
            current_period_end: Some(now + Duration::days(30)),
            cancel_at_period_end: false,
            canceled_at: None,
            trial_end: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        };
        let stored = self
            .resources
            .common
            .repos
            .subscriptions
            .upsert_subscription(&sub)
            .await
            .unwrap();
        assert_eq!(
            stored.provider_customer_id, customer_id,
            "seeded row round-trips"
        );
        customer_id
    }
}

fn checkout_body() -> Value {
    json!({
        "tier": "professional",
        "success_url": "https://app.example/billing?upgrade=success",
        "cancel_url": "https://app.example/billing?upgrade=cancel"
    })
}

#[tokio::test]
async fn checkout_without_a_token_is_unauthorized() {
    let fx = Fixture::new().await;
    let resp = AxumTestRequest::post("/api/billing/checkout")
        .json(&checkout_body())
        .send(fx.router())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "an anonymous caller must not reach the billing provider"
    );
}

#[tokio::test]
async fn checkout_names_the_token_user_and_tenant() {
    let fx = Fixture::new().await;
    let resp = AxumTestRequest::post("/api/billing/checkout")
        .header("Authorization", &fx.bearer)
        .json(&checkout_body())
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);

    let body: Value = resp.json();
    let url = body["checkout_url"]
        .as_str()
        .expect("checkout_url is a string");
    assert!(
        url.starts_with("https://example.test/checkout?"),
        "dummy provider URL, got {url}"
    );
    assert!(
        url.contains("tier=professional"),
        "tier from the body: {url}"
    );
    assert!(
        url.contains(&format!("user={}", fx.user_id)),
        "user must be the token's subject: {url}"
    );
    assert!(
        url.contains(&format!("tenant={}", fx.tenant_id)),
        "tenant must be the token's active tenant: {url}"
    );
}

#[tokio::test]
async fn checkout_ignores_a_smuggled_user_and_tenant() {
    let fx = Fixture::new().await;
    let other_user = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();
    let mut body = checkout_body();
    body["user_id"] = json!(other_user.to_string());
    body["tenant_id"] = json!(other_tenant.to_string());

    let resp = AxumTestRequest::post("/api/billing/checkout")
        .header("Authorization", &fx.bearer)
        .json(&body)
        .send(fx.router())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::OK,
        "a stale client that still sends identity keys keeps working"
    );

    let body: Value = resp.json();
    let url = body["checkout_url"]
        .as_str()
        .expect("checkout_url is a string");
    assert!(
        url.contains(&format!("user={}", fx.user_id)),
        "the body's user_id must not override the token: {url}"
    );
    assert!(
        url.contains(&format!("tenant={}", fx.tenant_id)),
        "the body's tenant_id must not override the token: {url}"
    );
    assert!(
        !url.contains(&other_user.to_string()) && !url.contains(&other_tenant.to_string()),
        "no smuggled id may reach the provider: {url}"
    );
}

#[tokio::test]
async fn checkout_falls_back_to_the_users_first_tenant() {
    let fx = Fixture::new().await;
    let resp = AxumTestRequest::post("/api/billing/checkout")
        .header("Authorization", &fx.bearer_without_tenant())
        .json(&checkout_body())
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);

    let body: Value = resp.json();
    let url = body["checkout_url"]
        .as_str()
        .expect("checkout_url is a string");
    assert!(
        url.contains(&format!("tenant={}", fx.tenant_id)),
        "a token without a tenant claim resolves to the user's first membership: {url}"
    );
    assert!(
        url.contains(&format!("user={}", fx.user_id)),
        "user must still be the token's subject: {url}"
    );
}

#[tokio::test]
async fn portal_without_a_token_is_unauthorized() {
    let fx = Fixture::new().await;
    fx.seed_subscription().await;
    let resp = AxumTestRequest::post("/api/billing/portal")
        .json(&json!({ "return_url": "https://app.example/billing" }))
        .send(fx.router())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "an anonymous caller must not open any customer's portal"
    );
}

#[tokio::test]
async fn portal_without_a_subscription_row_is_not_found() {
    let fx = Fixture::new().await;
    let resp = AxumTestRequest::post("/api/billing/portal")
        .header("Authorization", &fx.bearer)
        .json(&json!({ "return_url": "https://app.example/billing" }))
        .send(fx.router())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NOT_FOUND,
        "no row means no customer to open a portal for"
    );
    // The handler's wording is sanitized off the wire; the code is what a
    // client can act on.
    let body: Value = resp.json();
    assert_eq!(
        body["code"], "ResourceNotFound",
        "the 404 is a missing-row error, got {body}"
    );
}

#[tokio::test]
async fn portal_opens_the_callers_own_customer() {
    let fx = Fixture::new().await;
    let customer_id = fx.seed_subscription().await;

    // A body naming another customer must not pick whose portal opens.
    let resp = AxumTestRequest::post("/api/billing/portal")
        .header("Authorization", &fx.bearer)
        .json(&json!({
            "return_url": "https://app.example/billing",
            "provider_customer_id": "cus_someone_else"
        }))
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);

    let body: Value = resp.json();
    let url = body["portal_url"].as_str().expect("portal_url is a string");
    assert_eq!(
        url,
        format!("https://example.test/portal?customer={customer_id}"),
        "the portal customer is the caller's own subscription row"
    );
}
