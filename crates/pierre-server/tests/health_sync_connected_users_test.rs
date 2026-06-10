// ABOUTME: Regression test for list_connected_provider_users decoding a VARCHAR tenant_id on Postgres
// ABOUTME: A native-UUID decode of the VARCHAR column silently killed the entire scheduled health sync
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Connected-user listing for the scheduled health sync.
//!
//! `run_scheduled_sync_cycle` calls `list_connected_provider_users(provider)`
//! for every orchestrator-managed provider each cycle. `user_oauth_tokens.tenant_id`
//! is a VARCHAR column (the app models `UserOAuthToken.tenant_id` as `String`),
//! but the query decoded it as a native-UUID `TenantId` newtype. On Postgres that
//! threw "mismatched types ... UUID is not compatible with VARCHAR", the cycle
//! `continue`d past every provider, and NO health data (sleep, recovery, metrics)
//! was ever synced for anyone — silently. This pins that the listing succeeds.

mod common;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use pierre_core::models::UserOAuthToken;
use uuid::Uuid;

#[tokio::test]
async fn list_connected_provider_users_handles_varchar_tenant_id() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let repos = &resources.common.repos;

    let tenant = repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant");

    // Seed a Whoop token exactly as the app writes it: tenant_id is the
    // stringified UUID, stored in the VARCHAR column.
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.id.to_string(),
        provider: "whoop".to_owned(),
        access_token: "seed-access-token".to_owned(),
        refresh_token: Some("seed-refresh-token".to_owned()),
        token_type: "bearer".to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
        created_at: now,
        updated_at: now,
    };
    repos.oauth_tokens.upsert_token(&token).await.unwrap();

    // The exact call the scheduled health sync makes. Before the fix this errored
    // on Postgres (VARCHAR tenant_id decoded as a native-UUID TenantId), aborting
    // the whole sync cycle.
    let users = repos
        .sync_cursors
        .list_connected_provider_users("whoop")
        .await
        .expect("list_connected_provider_users must not error on a VARCHAR tenant_id");

    assert!(
        users.iter().any(|u| u.tenant_id == tenant.id),
        "the seeded whoop user must be listed for the scheduled sync; got {} user(s)",
        users.len()
    );
}
