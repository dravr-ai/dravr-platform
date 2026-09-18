// ABOUTME: Tests enforce_conversation_quota — the one cap check the REST create and /reset share
// ABOUTME: Pins the refusal at the cap, the 0-means-unlimited value, and the default when the lookup fails
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_config::constants::usage_quotas::{
    DEFAULT_MAX_ACTIVE_CONVERSATIONS, UNLIMITED_CONVERSATIONS,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use pierre_services::conversation_forge::{
    enforce_conversation_quota, max_active_conversations, MAX_ACTIVE_CONVERSATIONS_KEY,
};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Test double for the admin catalogue: answers one pinned value for the
/// conversation cap, or fails the read outright, so the helper's fallback
/// can be exercised without a catalogue row. Real catalogue resolution is
/// covered by `admin_config_scopes_test`.
struct PinnedCap(Result<Option<i64>, ()>);

#[async_trait]
impl AdminConfigLookup for PinnedCap {
    async fn get_value(
        &self,
        key: &str,
        _scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<Value>> {
        assert_eq!(key, MAX_ACTIVE_CONVERSATIONS_KEY);
        match self.0 {
            Ok(cap) => Ok(cap.map(Value::from)),
            Err(()) => Err(AppError::internal("catalogue unreachable")),
        }
    }

    /// The pin is the only row this double knows, so it is its override too.
    async fn get_override_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<Value>> {
        self.get_value(key, scope).await
    }
}

async fn athlete_with_threads(n: usize) -> (Arc<ServerContext>, String, TenantId) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("quota-{}@dravr.test", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let user = user_id.to_string();
    for i in 0..n {
        resources
            .common
            .repos
            .chat
            .create_conversation(&user, tenant_id, &format!("Thread {i}"), "m", None, None)
            .await
            .unwrap();
    }
    (resources, user, tenant_id)
}

#[tokio::test]
async fn the_cap_refuses_the_thread_that_would_exceed_it() {
    let (resources, user, tenant) = athlete_with_threads(2).await;
    let repos = &resources.common.repos;
    let cap = PinnedCap(Ok(Some(2)));

    let err = enforce_conversation_quota(repos, Some(&cap), &user, tenant)
        .await
        .expect_err("two owned threads against a cap of two must refuse a third");
    assert_eq!(err.code, ErrorCode::QuotaExceeded);
    let details = err.details.expect("a quota refusal carries its numbers");
    assert_eq!(details["limit_type"], "max_active_conversations");
    assert_eq!(details["current"], 2);
    assert_eq!(details["limit"], 2);
}

#[tokio::test]
async fn one_below_the_cap_is_allowed() {
    let (resources, user, tenant) = athlete_with_threads(1).await;
    enforce_conversation_quota(
        &resources.common.repos,
        Some(&PinnedCap(Ok(Some(2)))),
        &user,
        tenant,
    )
    .await
    .expect("one owned thread against a cap of two leaves room for one more");
}

#[tokio::test]
async fn zero_lifts_the_cap() {
    // More threads than the registered default allows, so this passes only
    // because the unlimited value skips the count — not because the count
    // happens to fit.
    let extra = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap() + 2;
    let (resources, user, tenant) = athlete_with_threads(extra).await;
    let cap = PinnedCap(Ok(Some(UNLIMITED_CONVERSATIONS)));
    assert_eq!(
        max_active_conversations(&cap, &user, tenant).await,
        UNLIMITED_CONVERSATIONS
    );
    enforce_conversation_quota(&resources.common.repos, Some(&cap), &user, tenant)
        .await
        .expect("an unlimited account is never refused");
}

#[tokio::test]
async fn a_failed_or_unregistered_lookup_falls_back_to_the_registered_default() {
    let (_, user, tenant) = athlete_with_threads(0).await;
    assert_eq!(
        max_active_conversations(&PinnedCap(Err(())), &user, tenant).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "a catalogue read error must degrade to the default, never to no cap"
    );
    assert_eq!(
        max_active_conversations(&PinnedCap(Ok(None)), &user, tenant).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "an unregistered key must degrade to the default"
    );
}
