// ABOUTME: The dravr.ai website routes end to end — the docs magic link's send and its redemption
// ABOUTME: Asserts on the rows written and the account returned, and that no answer reveals whether an address has an account

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The website's Worker holds an admin token carrying only `manage_website`
//! and reaches two routes under `/admin/website/`. These tests drive them
//! through the admin-token mount and read the tables back, so a handler that
//! answered `ok` without writing (or wrote without checking) fails here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use anyhow::Result;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::{User, UserStatus};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_services::link_token::generate_link_token;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// The admin-token mount, as the composition root builds it.
fn website_api(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::routes(AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    }))
}

/// `"Bearer <jwt>"` for a fresh service token holding `permissions`.
async fn bearer(resources: &Arc<ServerContext>, permissions: Vec<AdminPermission>) -> String {
    let request = CreateAdminTokenRequest {
        service_name: "dravr_website".to_owned(),
        service_description: Some("seeded by website_members_test".to_owned()),
        permissions: Some(permissions),
        expires_in_days: Some(30),
        is_super_admin: false,
        tenant_id: None,
        operator_user_id: None,
    };
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", token.jwt_token)
}

async fn website_token(resources: &Arc<ServerContext>) -> String {
    bearer(resources, vec![AdminPermission::ManageWebsite]).await
}

/// Create a user in `status`; return it.
async fn account(resources: &Arc<ServerContext>, email: &str, status: UserStatus) -> User {
    let mut user = User::new(
        email.to_owned(),
        "not-a-real-hash".to_owned(),
        Some("Docs Reader".to_owned()),
    );
    user.user_status = status;
    resources.common.repos.users.create(&user).await.unwrap();
    user
}

async fn post(
    resources: &Arc<ServerContext>,
    token: &str,
    path: &str,
    body: &Value,
) -> (u16, Value) {
    let response = AxumTestRequest::post(path)
        .header("authorization", token)
        .json(body)
        .send(website_api(resources))
        .await;
    let status = response.status();
    let text = response.text();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, json)
}

const TOKEN_COUNT_SQL: &str = "SELECT COUNT(*) FROM website_sign_in_tokens WHERE user_id = $1";

async fn token_rows(resources: &Arc<ServerContext>, user_id: Uuid) -> i64 {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query_scalar(TOKEN_COUNT_SQL)
            .bind(user_id.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query_scalar(TOKEN_COUNT_SQL)
            .bind(user_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
    }
}

/// Store a sign-in token for `user_id` living `ttl_minutes`, the way the send
/// route does; return the token the mail would carry.
async fn mint_with_ttl(resources: &Arc<ServerContext>, user_id: Uuid, ttl_minutes: i64) -> String {
    let generated = generate_link_token();
    resources
        .common
        .repos
        .website_sign_in_tokens
        .store_token(
            user_id,
            &generated.selector,
            &generated.verifier_hash,
            ttl_minutes,
        )
        .await
        .unwrap();
    generated.token
}

/// A token with the send route's 15-minute lifetime.
async fn mint(resources: &Arc<ServerContext>, user_id: Uuid) -> String {
    mint_with_ttl(resources, user_id, 15).await
}

#[tokio::test]
#[serial]
async fn every_website_route_needs_manage_website() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let other = bearer(&resources, vec![AdminPermission::ManageConfiguration]).await;

    for (path, body) in [
        (
            "/admin/website/sign-in-link",
            json!({ "email": "nope@example.com", "next": "/docs", "lang": "en" }),
        ),
        (
            "/admin/website/sign-in-link/consume",
            json!({ "token": "a.b" }),
        ),
    ] {
        let (status, _) = post(&resources, &other, path, &body).await;
        assert_eq!(
            status, 403,
            "{path} must refuse a token without manage_website"
        );
    }
    Ok(())
}

#[tokio::test]
#[serial]
async fn the_website_token_reaches_nothing_beyond_the_sign_in_link() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;

    // The website keeps no signup list: no route answers under a waitlist path.
    for path in ["/admin/website/waitlist", "/admin/website/waitlist/import"] {
        let (status, _) = post(&resources, &token, path, &json!({})).await;
        assert_eq!(status, 404, "{path} is not a route");
    }
    // And the token opens no other admin surface.
    let (status, _) = post(
        &resources,
        &token,
        "/admin/pre-approved-emails",
        &json!({ "email": "nope@example.com" }),
    )
    .await;
    assert_eq!(status, 403, "manage_website does not pre-approve accounts");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_malformed_address_is_refused_and_mints_nothing() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let active = account(&resources, "wellformed@example.com", UserStatus::Active).await;

    for email in ["not-an-email", "a b@example.com", "@example.com"] {
        let (status, _) = post(
            &resources,
            &token,
            "/admin/website/sign-in-link",
            &json!({ "email": email, "next": "/docs", "lang": "en" }),
        )
        .await;
        assert_eq!(status, 400, "{email:?} must be refused");
    }
    assert_eq!(token_rows(&resources, active.id).await, 0);
    Ok(())
}

// ===========================================================================
// Sending the sign-in link
// ===========================================================================

#[tokio::test]
#[serial]
async fn only_an_active_account_earns_a_link_and_every_answer_is_the_same() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let active = account(&resources, "active@example.com", UserStatus::Active).await;
    let pending = account(&resources, "pending@example.com", UserStatus::Pending).await;

    let mut answers = Vec::new();
    for email in [
        "Active@Example.com",
        "pending@example.com",
        "nobody@example.com",
    ] {
        let (status, body) = post(
            &resources,
            &token,
            "/admin/website/sign-in-link",
            &json!({ "email": email, "next": "/fr/docs/connect-your-data", "lang": "fr" }),
        )
        .await;
        answers.push((status, body));
    }
    assert!(
        answers
            .iter()
            .all(|a| a == &(200, json!({ "status": "ok" }))),
        "active, pending and unknown addresses answer alike: {answers:?}"
    );
    assert_eq!(
        token_rows(&resources, active.id).await,
        1,
        "the active account got one link"
    );
    assert_eq!(
        token_rows(&resources, pending.id).await,
        0,
        "a pending account gets none"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_next_outside_the_docs_or_an_unknown_locale_is_refused() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let active = account(&resources, "redirect@example.com", UserStatus::Active).await;

    for next in [
        "https://evil.com",
        "//evil.com",
        "/docs/../admin",
        "/docs?x=1",
        "/docsx",
        "/docs/Upper",
        "/docs//x",
        "",
    ] {
        let (status, _) = post(
            &resources,
            &token,
            "/admin/website/sign-in-link",
            &json!({ "email": active.email, "next": next, "lang": "en" }),
        )
        .await;
        assert_eq!(status, 400, "next {next:?} must be refused");
    }
    let (status, _) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link",
        &json!({ "email": active.email, "next": "/docs", "lang": "de" }),
    )
    .await;
    assert_eq!(status, 400, "only en and fr");
    assert_eq!(
        token_rows(&resources, active.id).await,
        0,
        "no refused request minted a link"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_account_gets_at_most_five_links_an_hour() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let active = account(&resources, "budget@example.com", UserStatus::Active).await;

    for _ in 0..6 {
        let (status, body) = post(
            &resources,
            &token,
            "/admin/website/sign-in-link",
            &json!({ "email": active.email, "next": "/docs", "lang": "en" }),
        )
        .await;
        assert_eq!(
            (status, body),
            (200, json!({ "status": "ok" })),
            "the capped request answers the same"
        );
    }
    assert_eq!(
        token_rows(&resources, active.id).await,
        5,
        "the sixth request in the hour minted nothing"
    );
    Ok(())
}

// ===========================================================================
// Redeeming the sign-in link
// ===========================================================================

#[tokio::test]
#[serial]
async fn a_link_redeems_once_to_its_account() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let active = account(&resources, "redeem@example.com", UserStatus::Active).await;
    let link = mint(&resources, active.id).await;

    let (status, body) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link/consume",
        &json!({ "token": link }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "user_id": active.id.to_string(), "email": "redeem@example.com" })
    );

    let (status, again) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link/consume",
        &json!({ "token": link }),
    )
    .await;
    assert_eq!(status, 404, "a link works once: {again}");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_link_closes_when_its_account_is_suspended_and_garbage_answers_the_same() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let user = account(&resources, "suspended@example.com", UserStatus::Active).await;
    let link = mint(&resources, user.id).await;
    resources
        .common
        .repos
        .users
        .update_status(user.id, UserStatus::Suspended, None)
        .await?;

    let (suspended_status, suspended_body) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link/consume",
        &json!({ "token": link }),
    )
    .await;
    assert_eq!(
        suspended_status, 404,
        "a suspended account's link is closed"
    );

    for garbage in ["", "no-delimiter", "unknown.selector", "a.b"] {
        let (status, body) = post(
            &resources,
            &token,
            "/admin/website/sign-in-link/consume",
            &json!({ "token": garbage }),
        )
        .await;
        assert_eq!(status, 404, "{garbage:?}");
        assert_eq!(
            (&body["code"], &body["message"]),
            (&suspended_body["code"], &suspended_body["message"]),
            "every failure answers with the same code and message"
        );
    }
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_expired_link_answers_like_any_other_failure() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let user = account(&resources, "expired@example.com", UserStatus::Active).await;
    let expired = mint_with_ttl(&resources, user.id, -1).await;

    let (status, body) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link/consume",
        &json!({ "token": expired }),
    )
    .await;
    assert_eq!(status, 404, "an expired link is closed: {body}");

    let (_, garbage) = post(
        &resources,
        &token,
        "/admin/website/sign-in-link/consume",
        &json!({ "token": "unknown.selector" }),
    )
    .await;
    assert_eq!(
        (&body["code"], &body["message"]),
        (&garbage["code"], &garbage["message"]),
        "expiry is indistinguishable from an unknown link"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_wrong_guess_costs_an_attempt_and_five_lock_the_link() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let token = website_token(&resources).await;
    let consume = "/admin/website/sign-in-link/consume";

    // Four wrong verifiers leave the link live for its real holder.
    let survivor = account(&resources, "survivor@example.com", UserStatus::Active).await;
    let link = mint(&resources, survivor.id).await;
    let (selector, _) = link.split_once('.').unwrap();
    for _ in 0..4 {
        let (status, _) = post(
            &resources,
            &token,
            consume,
            &json!({ "token": format!("{selector}.wrong") }),
        )
        .await;
        assert_eq!(status, 404, "a wrong verifier is refused");
    }
    let (status, body) = post(&resources, &token, consume, &json!({ "token": link })).await;
    assert_eq!(
        status, 200,
        "four wrong guesses do not spend the link: {body}"
    );
    assert_eq!(body["email"], "survivor@example.com");

    // The fifth wrong verifier locks it, even against the right one.
    let locked = account(&resources, "locked@example.com", UserStatus::Active).await;
    let link = mint(&resources, locked.id).await;
    let (selector, _) = link.split_once('.').unwrap();
    for _ in 0..5 {
        post(
            &resources,
            &token,
            consume,
            &json!({ "token": format!("{selector}.wrong") }),
        )
        .await;
    }
    let (status, body) = post(&resources, &token, consume, &json!({ "token": link })).await;
    assert_eq!(status, 404, "five wrong guesses lock the link: {body}");
    Ok(())
}
