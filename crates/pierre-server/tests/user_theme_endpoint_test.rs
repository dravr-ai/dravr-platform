// ABOUTME: Integration tests for PUT /api/user/theme and the chart theme it decides
// ABOUTME: Round-trips the pin through the route, then mints a chart URL from the stored value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The theme pin exists so a server-side render can paint in the athlete's own
//! scheme. These tests walk that whole path: the client writes the pin through
//! the route, the column holds it, and the messaging chart minter signs a token
//! carrying it — with an unpinned athlete still getting dark.

mod common;
mod helpers;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_chat_pipeline::RenderCapabilities;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ColorScheme, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::viz::VizToken;
use pierre_mcp_server::services::messaging_ingress::surface::messaging_render_profile;
use pierre_mcp_server::services::messaging_ingress::viz_delivery::{
    plan_media, target as viz_target, VizDelivery,
};
use pierre_routes_auth::AuthRoutes;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn setup() -> (Router, String, Uuid, Arc<ServerContext>) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let router = AuthRoutes::routes(resources.auth_routes_context());
    (router, format!("Bearer {token}"), user_id, resources)
}

/// Read back what the column holds for this athlete.
async fn stored_theme(resources: &Arc<ServerContext>, user_id: Uuid) -> Option<String> {
    resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("user row")
        .theme
}

/// Mint one messaging chart URL the way the scene publisher does, for an
/// athlete whose pin is `pinned`, and hand back the theme the signed token
/// carries.
fn minted_chart_theme(render: &RenderCapabilities, pinned: Option<&str>, secret: &str) -> String {
    let media = plan_media(
        &VizDelivery {
            target: viz_target(
                "conv-1".to_owned(),
                Uuid::new_v4().to_string(),
                TenantId::from_uuid(Uuid::new_v4()),
                "msg-1".to_owned(),
            ),
            stored_blocks: Some(r#"[{"title":"Weekly load"}]"#),
            render,
            locale: "en",
            theme: ColorScheme::resolve(pinned),
            base_url: "https://dravr.test",
            press_enabled: true,
        },
        secret,
    );
    assert_eq!(media.len(), 1, "one spec mints one chart URL");
    let raw = media[0]
        .url
        .strip_prefix("https://dravr.test/api/viz/")
        .and_then(|rest| rest.strip_suffix(".png"))
        .expect("minted URL is a signed viz token");
    VizToken::verify(raw, secret)
        .expect("the minted token verifies")
        .theme
}

#[tokio::test]
async fn put_theme_light_persists_the_pin() {
    let (router, auth, user_id, resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/theme")
        .header("authorization", &auth)
        .json(&json!({ "theme": "light" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        stored_theme(&resources, user_id).await.as_deref(),
        Some("light")
    );
}

#[tokio::test]
async fn put_theme_null_clears_the_pin() {
    let (router, auth, user_id, resources) = setup().await;

    let set = AxumTestRequest::put("/api/user/theme")
        .header("authorization", &auth)
        .json(&json!({ "theme": "dark" }))
        .send(router.clone())
        .await;
    assert_eq!(set.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        stored_theme(&resources, user_id).await.as_deref(),
        Some("dark")
    );

    let clear = AxumTestRequest::put("/api/user/theme")
        .header("authorization", &auth)
        .json(&json!({ "theme": null }))
        .send(router)
        .await;

    assert_eq!(clear.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        stored_theme(&resources, user_id).await,
        None,
        "JSON null clears the pin so clients follow the system again"
    );
}

#[tokio::test]
async fn put_theme_rejects_a_value_outside_the_enum() {
    let (router, auth, user_id, resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/theme")
        .header("authorization", &auth)
        .json(&json!({ "theme": "sepia" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        stored_theme(&resources, user_id).await,
        None,
        "a rejected value writes nothing"
    );
}

#[tokio::test]
async fn put_theme_without_a_token_is_unauthorised() {
    let (router, _auth, user_id, resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/theme")
        .json(&json!({ "theme": "light" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
    assert_eq!(stored_theme(&resources, user_id).await, None);
}

#[tokio::test]
async fn a_light_pinned_athlete_gets_a_light_chart_url_and_an_unpinned_one_gets_dark() {
    let (router, auth, user_id, resources) = setup().await;
    let render = messaging_render_profile(ChannelType::Telegram, "en").render;
    let secret = &resources.auth.admin_jwt_secret;

    // Unpinned is the starting state: the render has no client to ask, so dark.
    assert_eq!(
        stored_theme(&resources, user_id).await,
        None,
        "a fresh athlete has pinned nothing"
    );
    assert_eq!(
        minted_chart_theme(&render, None, secret),
        "dark",
        "an unpinned athlete's chart is minted dark"
    );

    let response = AxumTestRequest::put("/api/user/theme")
        .header("authorization", &auth)
        .json(&json!({ "theme": "light" }))
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::NO_CONTENT);

    let pinned = stored_theme(&resources, user_id).await;
    assert_eq!(pinned.as_deref(), Some("light"));
    assert_eq!(
        minted_chart_theme(&render, pinned.as_deref(), secret),
        "light",
        "the pin the athlete saved is the palette the chart is pressed in"
    );
}
