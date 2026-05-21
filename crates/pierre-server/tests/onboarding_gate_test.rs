// ABOUTME: Integration tests for the onboarding gate that refuses messaging until a fitness provider is connected
// ABOUTME: Exercises services::onboarding_gate + GET /api/me/onboarding-status against a real SQLite repo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Integration tests for the onboarding-gate behavior.
//!
//! These guard the contract that a freshly registered user — one with zero
//! rows in `provider_connections` — gets a structured `NoProviderConnected`
//! refusal everywhere the LLM coach is reachable, and an unblock the moment
//! they register their first connection. Without this gate the model
//! hallucinates specifics from the user's message (`feedback_group_card_renders_only_what_llm_sees`).

use std::sync::Arc;

use pierre_core::errors::ErrorCode;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::repositories::ProviderConnectionRepository;
use pierre_mcp_server::services::onboarding_gate;

mod common;

/// New user with no provider connections must be rejected by the gate, and
/// the rejection must carry the structured `NoProviderConnected` code so the
/// REST layer renders a 403 and the frontend can redirect to onboarding.
#[tokio::test]
async fn require_connected_provider_rejects_user_with_no_connections() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();

    let err = onboarding_gate::require_connected_provider(&repos.provider_connections, user_id)
        .await
        .expect_err("user with zero providers must be refused");

    assert_eq!(err.code, ErrorCode::NoProviderConnected);
    assert_eq!(err.http_status(), 403);

    let details = err.details.as_ref().expect("details payload");
    assert_eq!(
        details.get("action").and_then(|v| v.as_str()),
        Some("connect_provider"),
        "frontend reacts on details.action without string-matching the message"
    );
}

/// Once any provider connection exists, the gate must let the user through —
/// even with a `Synthetic` connection type. This mirrors how the OAuth
/// callback registers a row and unblocks chat immediately.
#[tokio::test]
async fn require_connected_provider_passes_once_a_connection_exists() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();

    onboarding_gate::require_connected_provider(&repos.provider_connections, user_id)
        .await
        .expect("connected user must pass the gate");
}

/// `user_has_connected_provider` returns the boolean for the
/// `GET /api/me/onboarding-status` endpoint. Verifying both sides of the flip
/// here means the endpoint can stay a one-line wrapper without a separate
/// test of its own.
#[tokio::test]
async fn user_has_connected_provider_reflects_table_state() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();

    let repo: Arc<dyn ProviderConnectionRepository> = repos.provider_connections.clone();

    assert!(
        !onboarding_gate::user_has_connected_provider(&repo, user_id)
            .await
            .unwrap(),
        "fresh user has no connections"
    );

    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    assert!(
        onboarding_gate::user_has_connected_provider(&repo, user_id)
            .await
            .unwrap(),
        "registered connection flips the flag"
    );
}
