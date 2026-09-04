// ABOUTME: Unit tests for pierre_runtime_context::tenant::resolve_tenant
// ABOUTME: Covers happy path, mismatch rejection, fallback, and the Required/Optional split

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for `pierre_runtime_context::tenant::resolve_tenant`.
//!
//! The canonical tenant resolver replaced ~10 ad-hoc
//! `TenantId::from_uuid(user_id)` fallbacks across the route layer. These five
//! tests pin its observable contract so a future refactor can't quietly
//! reintroduce the user-id fabrication that masked the "user has no
//! tenant" failure mode.

use anyhow::Result;
use pierre_auth::auth::{AuthMethod, AuthResult};
use pierre_auth::rate_limiting::UnifiedRateLimitInfo;
use pierre_core::errors::ErrorCode;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_runtime_context::{resolve_tenant, MiddlewareCtx, TenantMode};
use uuid::Uuid;

mod common;

fn auth_for(user_id: Uuid, active_tenant_id: Option<Uuid>) -> AuthResult {
    AuthResult {
        session_id: None,
        // This fixture is about tenant resolution, not the scope gate; a
        // first-party credential's grant keeps it representative.
        scopes: OAuthScope::self_grant(),
        user_id,
        auth_method: AuthMethod::JwtToken {
            tier: "starter".to_owned(),
        },
        rate_limit: UnifiedRateLimitInfo {
            is_rate_limited: false,
            limit: Some(1000),
            remaining: Some(1000),
            reset_at: None,
            tier: "starter".to_owned(),
            auth_method: "jwt".to_owned(),
        },
        active_tenant_id,
    }
}

/// 1/5: `active_tenant_id` claim that matches a real membership resolves to
/// the claimed tenant.
#[tokio::test]
async fn active_tenant_id_verified_membership_returns_claimed() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (user_id, _user) =
        common::create_test_user_with_email(&resources.coach.database, "claim-ok@pierre.test")
            .await?;

    let tenants = resources.repos().tenants.list_for_user(user_id).await?;
    let claimed = tenants
        .first()
        .map(|t| t.id.as_uuid())
        .expect("create_test_user_with_email seeds at least one tenant");

    let auth = auth_for(user_id, Some(claimed));
    let resolved = resolve_tenant(&resources, &auth, TenantMode::Required).await?;

    assert_eq!(
        resolved.map(|t| t.as_uuid()),
        Some(claimed),
        "verified membership must return the claimed tenant id"
    );
    Ok(())
}

/// 2/5: `active_tenant_id` claim that doesn't match any membership must fail
/// closed with `AppError::Auth*` — never silently downgrade to first-tenant
/// or fabricate a tenant.
#[tokio::test]
async fn active_tenant_id_mismatch_returns_auth_error() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (user_id, _user) = common::create_test_user_with_email(
        &resources.coach.database,
        "claim-mismatch@pierre.test",
    )
    .await?;

    let forged = Uuid::new_v4();
    let auth = auth_for(user_id, Some(forged));
    let err = resolve_tenant(&resources, &auth, TenantMode::Required)
        .await
        .expect_err("forged active_tenant_id must be rejected");

    assert_eq!(
        err.code,
        ErrorCode::AuthInvalid,
        "expected ErrorCode::AuthInvalid, got {err:?}"
    );
    Ok(())
}

/// 3/5: No `active_tenant_id` claim falls back to the user's first listed
/// tenant (single-tenant accounts + tokens without the claim).
#[tokio::test]
async fn no_claim_falls_back_to_first_tenant() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (user_id, _user) =
        common::create_test_user_with_email(&resources.coach.database, "no-claim@pierre.test")
            .await?;

    let expected = resources
        .repos()
        .tenants
        .list_for_user(user_id)
        .await?
        .first()
        .map(|t| t.id.as_uuid())
        .expect("create_test_user_with_email seeds at least one tenant");

    let auth = auth_for(user_id, None);
    let resolved = resolve_tenant(&resources, &auth, TenantMode::Required).await?;

    assert_eq!(
        resolved.map(|t| t.as_uuid()),
        Some(expected),
        "missing claim must fall back to the user's first tenant"
    );
    Ok(())
}

/// 4/5: Empty tenant list + `Required` must return `AppError::Auth*`. This
/// is the case the old `TenantId::from_uuid(user_id)` fallback silently masked.
#[tokio::test]
async fn empty_list_required_errors() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let orphan_user_id = Uuid::new_v4();

    let auth = auth_for(orphan_user_id, None);
    let err = resolve_tenant(&resources, &auth, TenantMode::Required)
        .await
        .expect_err("user with no tenants under Required must error");

    assert_eq!(
        err.code,
        ErrorCode::AuthInvalid,
        "expected ErrorCode::AuthInvalid, got {err:?}"
    );
    Ok(())
}

/// 5/5: Empty tenant list + `Optional` resolves to `Ok(None)` so onboarding
/// routes can serve users who haven't been assigned a tenant yet.
#[tokio::test]
async fn empty_list_optional_returns_none() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let orphan_user_id = Uuid::new_v4();

    let auth = auth_for(orphan_user_id, None);
    let resolved = resolve_tenant(&resources, &auth, TenantMode::Optional).await?;

    assert!(
        resolved.is_none(),
        "Optional with no tenants must return None, got {resolved:?}"
    );
    Ok(())
}
