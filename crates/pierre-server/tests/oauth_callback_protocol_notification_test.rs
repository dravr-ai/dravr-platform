// ABOUTME: Integration test for OAuth-completion notifications reaching MCP protocol streams
// ABOUTME: Exercises the production auth_routes_context().sync_notifier -> SseManager -> protocol stream seam
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "transport-sse")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_email, get_shared_test_jwks};
use pierre_core::models::OAuthNotification;
use pierre_mcp_server::sse::protocol::McpProtocolStreamFactory;
use pierre_runtime_context::SseCtx;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

fn oauth_notification(user_id: Uuid, provider: &str) -> OAuthNotification {
    OAuthNotification {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider: provider.to_owned(),
        success: true,
        message: format!("{provider} connected successfully"),
        expires_at: None,
        created_at: chrono::Utc::now(),
        read_at: None,
    }
}

/// The OAuth callback handler delivers its completion notification through
/// `AuthRoutesContext.sync_notifier`, which production wires to the same
/// `SseManager` that owns MCP protocol streams (`auth_routes_context()` hands
/// out `self.sse.sse_manager`). This proves that seam end to end: a
/// notification dispatched through the auth context's `sync_notifier` — exactly
/// what the callback handler holds — reaches a protocol stream registered on
/// the server's real `SseManager`, scoped to the owning user.
#[tokio::test]
async fn oauth_notification_via_auth_context_reaches_protocol_stream() -> anyhow::Result<()> {
    let resources = create_test_server_resources().await?;

    // The server's real SseManager — the very Arc that auth_routes_context()
    // returns as sync_notifier. Install the protocol factory so streams can register.
    let sse_manager = resources.sse.sse_manager.clone();
    let _ = sse_manager.install_protocol_factory(Arc::new(McpProtocolStreamFactory {
        resources: Arc::clone(&resources),
    }));

    let (user_id, user) =
        create_test_user_with_email(&resources.coach.database, "oauth-cb-single@example.com")
            .await?;
    let jwks = get_shared_test_jwks();
    let token = resources.auth.auth_manager.generate_token(&user, &jwks)?;

    let ctx: Arc<dyn SseCtx> = Arc::clone(&resources) as _;
    let mut receiver = sse_manager
        .register_protocol_stream(
            "cb_session".to_owned(),
            Some(format!("Bearer {token}")),
            ctx,
        )
        .await?;

    // Dispatch through the SAME object the OAuth callback handler uses on success.
    let auth_ctx = resources.auth_routes_context();
    auth_ctx
        .sync_notifier
        .send_oauth_notification_to_protocol_streams(
            user_id,
            &oauth_notification(user_id, "strava"),
        )
        .await?;

    let message = timeout(Duration::from_millis(500), receiver.recv()).await??;
    assert!(message.contains("strava"), "payload was: {message}");

    sse_manager.unregister_protocol_stream("cb_session");
    Ok(())
}

/// Multi-tenant guard at the production seam: a notification dispatched through
/// `auth_routes_context().sync_notifier` for one user must never reach another
/// user's protocol stream.
#[tokio::test]
async fn oauth_notification_via_auth_context_is_user_scoped() -> anyhow::Result<()> {
    let resources = create_test_server_resources().await?;
    let sse_manager = resources.sse.sse_manager.clone();
    let _ = sse_manager.install_protocol_factory(Arc::new(McpProtocolStreamFactory {
        resources: Arc::clone(&resources),
    }));

    let (user1_id, user1) =
        create_test_user_with_email(&resources.coach.database, "oauth-cb-iso-1@example.com")
            .await?;
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.coach.database, "oauth-cb-iso-2@example.com")
            .await?;
    let jwks = get_shared_test_jwks();
    let token1 = resources.auth.auth_manager.generate_token(&user1, &jwks)?;
    let token2 = resources.auth.auth_manager.generate_token(&user2, &jwks)?;

    let ctx: Arc<dyn SseCtx> = Arc::clone(&resources) as _;
    let mut recv1 = sse_manager
        .register_protocol_stream(
            "u1".to_owned(),
            Some(format!("Bearer {token1}")),
            ctx.clone(),
        )
        .await?;
    let mut recv2 = sse_manager
        .register_protocol_stream("u2".to_owned(), Some(format!("Bearer {token2}")), ctx)
        .await?;

    let auth_ctx = resources.auth_routes_context();
    auth_ctx
        .sync_notifier
        .send_oauth_notification_to_protocol_streams(
            user1_id,
            &oauth_notification(user1_id, "fitbit"),
        )
        .await?;

    let msg1 = timeout(Duration::from_millis(500), recv1.recv()).await??;
    assert!(msg1.contains("fitbit"), "payload was: {msg1}");

    let leaked = timeout(Duration::from_millis(150), recv2.recv()).await;
    assert!(
        leaked.is_err(),
        "user2 must not receive user1's OAuth notification"
    );

    sse_manager.unregister_protocol_stream("u1");
    sse_manager.unregister_protocol_stream("u2");
    Ok(())
}
