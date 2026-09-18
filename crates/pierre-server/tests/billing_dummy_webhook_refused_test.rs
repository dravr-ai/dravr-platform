// ABOUTME: Pins that POST /webhooks/dummy refuses an unsigned body and leaves users.tier untouched
// ABOUTME: The receiver is mounted whenever Stripe is unwired, so an accepted body is an anonymous tier write
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The billing webhook receiver dispatches whatever event the active
//! provider hands back into `users.tier` and `tenants.plan`, keyed on the
//! ids the event names. `DummyProvider` is the active provider in every
//! deployment without Stripe secrets, nginx proxies `/webhooks/` publicly,
//! and it has no signing secret — so its `parse_webhook` must refuse every
//! body, or an anonymous caller sets any user's tier (carnet#454). This
//! test posts exactly that body through the real router and checks both
//! halves: the status, and the row the write would have landed on.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::models::UserTier;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::json;

use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;

#[tokio::test]
async fn unsigned_dummy_webhook_is_refused_and_writes_nothing() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = Arc::clone(&resources.common.repos);
    let tenant_id = repos.tenants.list_for_user(user_id).await.unwrap()[0].id;

    let before = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(before.tier, UserTier::Starter, "fixture starts on Starter");

    let router = pierre_routes_billing::billing_routes::<ServerContext>().with_state(resources);
    let resp = AxumTestRequest::post("/webhooks/dummy")
        .json(&json!({
            "id": "evt_forged_1",
            "type": "subscription.upserted",
            "data": {
                "provider_customer_id": "cus_forged",
                "provider_subscription_id": "sub_forged",
                "tenant_id": tenant_id.to_string(),
                "user_id": user_id.to_string(),
                "plan_tier": "enterprise",
                "status": "active"
            }
        }))
        .send(router)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "an unsigned body must not be applied"
    );

    let after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(
        after.tier,
        UserTier::Starter,
        "the forged upsert must not have moved the tier"
    );
    assert!(
        repos
            .subscriptions
            .get_subscription_by_user(user_id)
            .await
            .unwrap()
            .is_none(),
        "no subscription row may come from an unverified event"
    );
    assert!(
        !repos
            .subscriptions
            .is_billing_event_processed("dummy", "evt_forged_1")
            .await
            .unwrap(),
        "a refused event must not be recorded as processed"
    );
}
