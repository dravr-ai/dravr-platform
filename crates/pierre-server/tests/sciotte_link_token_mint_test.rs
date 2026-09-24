// ABOUTME: Tests for POST /api/channels/provider/sciotte/link-token — the pair it signs must be a real membership
// ABOUTME: A member (user, tenant) mints a token carrying that pair; a non-member pair is refused with 404
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The mint endpoint is service-to-service: the channel bot names a
//! `(user_id, tenant_id)` pair and gets back a signed hosted-login URL. The
//! login handler then stores the token's `tid` verbatim on the provider
//! session, so a pair that is not a `tenant_users` membership would bind a
//! provider session under a foreign tenant. These tests pin that only a
//! recorded membership mints, and that what is minted carries exactly the
//! pair named.

mod common;
mod helpers;

use std::sync::Arc;

use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::verify_link_token;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

const MINT_PATH: &str = "/api/channels/provider/sciotte/link-token";

/// Create a user with the given role owning a dedicated tenant, and return
/// `(user_id, tenant_id, "Bearer <jwt>")`.
async fn create_user_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> (Uuid, TenantId, String) {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();

    let mut user = User::new(email.to_owned(), password_hash, Some("Mint".to_owned()));
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user_id, tenant_id, format!("Bearer {token}"))
}

fn mint_body(user_id: Uuid, tenant_id: TenantId) -> Value {
    json!({
        "user_id": user_id.to_string(),
        "tenant_id": tenant_id.to_string(),
        "target": "strava",
        "channel": "telegram"
    })
}

#[tokio::test]
async fn a_member_pair_mints_a_token_carrying_that_pair() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, admin_auth) =
        create_user_with_role(&resources, "mint-admin@link.test", UserRole::Admin).await;
    let (athlete_id, athlete_tenant, _) =
        create_user_with_role(&resources, "mint-athlete@link.test", UserRole::User).await;

    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &admin_auth)
        .json(&mint_body(athlete_id, athlete_tenant))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();

    let token = body["token"]
        .as_str()
        .expect("the mint returns the raw token");
    assert!(
        body["url"]
            .as_str()
            .unwrap()
            .contains("/providers/sciotte/login?token="),
        "the hosted-login URL embeds the token: {body}"
    );

    let claims = verify_link_token(token, &resources.auth.admin_jwt_secret, "sciotte")
        .expect("the minted token verifies under the admin secret");
    assert_eq!(claims.sub, athlete_id.to_string());
    assert_eq!(
        claims.tid,
        athlete_tenant.to_string(),
        "the token binds the tenant the athlete is a member of"
    );
    assert_eq!(claims.tgt, "strava");
    assert_eq!(claims.channel, "telegram");
}

#[tokio::test]
async fn a_non_member_pair_is_refused_before_minting() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, admin_auth) =
        create_user_with_role(&resources, "mint-admin-2@link.test", UserRole::Admin).await;
    let (athlete_id, _, _) =
        create_user_with_role(&resources, "mint-athlete-2@link.test", UserRole::User).await;
    let (_, foreign_tenant, _) =
        create_user_with_role(&resources, "mint-other-owner@link.test", UserRole::User).await;

    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &admin_auth)
        .json(&mint_body(athlete_id, foreign_tenant))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(
        response.status(),
        404,
        "a pair with no tenant_users row must not mint"
    );
    let body: Value = response.json();
    assert_eq!(body["code"], "ResourceNotFound", "{body}");
    assert!(
        body.get("token").is_none() && body.get("url").is_none(),
        "no token may leave the server for a non-member pair: {body}"
    );

    // An unknown user is the same refusal: there is no membership to sign.
    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &admin_auth)
        .json(&mint_body(Uuid::new_v4(), foreign_tenant))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn a_non_admin_caller_cannot_mint_even_for_a_member_pair() {
    let resources = create_test_server_resources().await.unwrap();
    let (athlete_id, athlete_tenant, athlete_auth) =
        create_user_with_role(&resources, "mint-self@link.test", UserRole::User).await;

    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &athlete_auth)
        .json(&mint_body(athlete_id, athlete_tenant))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(body["message"], "Admin privileges required", "{body}");
}

#[tokio::test]
async fn a_trainingpeaks_target_mints_and_an_unknown_one_names_every_target() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, admin_auth) =
        create_user_with_role(&resources, "mint-admin-tp@link.test", UserRole::Admin).await;
    let (athlete_id, athlete_tenant, _) =
        create_user_with_role(&resources, "mint-athlete-tp@link.test", UserRole::User).await;

    let mut body = mint_body(athlete_id, athlete_tenant);
    body["target"] = json!("trainingpeaks");
    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &admin_auth)
        .json(&body)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 200);
    let minted: Value = response.json();
    let claims = verify_link_token(
        minted["token"].as_str().unwrap(),
        &resources.auth.admin_jwt_secret,
        "sciotte",
    )
    .expect("the minted token verifies under the admin secret");
    assert_eq!(
        claims.tgt, "trainingpeaks",
        "the hosted login must open the TrainingPeaks scraper, not the Strava default"
    );

    body["target"] = json!("polar");
    let response = AxumTestRequest::post(MINT_PATH)
        .header("authorization", &admin_auth)
        .json(&body)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 400);
    let refusal: Value = response.json();
    let message = refusal["message"].as_str().unwrap_or_default();
    for target in ["strava", "garmin", "trainingpeaks"] {
        assert!(
            message.contains(target),
            "the refusal must name every hosted-login target ({target}): {refusal}"
        );
    }
}
