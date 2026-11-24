// ABOUTME: What a revoke or a rotation of a rotated admin token answers on the console mount
// ABOUTME: Pins the 409 that names a rotation's replacement, and the idempotent revoke of a token with none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A rotated token has a live replacement, so revoking or rotating the old
//! one ends nothing: it answers 409 and names the replacement by id, where a
//! token revoked earlier keeps its idempotent 200 (carnet#1589). Driven
//! through the console mount (`/api/admin/tokens`), behind an admin's
//! session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_token_routes_test::{console, is_active, mint, quotable, session};
use crate::common;
use crate::helpers;

use std::sync::Arc;

use anyhow::Result;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::AdminPermission;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use serial_test::serial;

/// `POST {path}` on the console mount as the session `admin`: the status and
/// the answer, fit to quote.
async fn post_on_the_console(
    resources: &Arc<ServerContext>,
    admin: &str,
    path: &str,
) -> (u16, String) {
    let response = AxumTestRequest::post(path)
        .header("authorization", admin)
        .json(&json!({}))
        .send(console(resources))
        .await;
    quotable(response)
}

/// The replacement id a rotation of `token_id` from the console answers.
async fn rotate_as(resources: &Arc<ServerContext>, admin: &str, token_id: &str) -> Result<String> {
    let path = format!("/api/admin/tokens/{token_id}/rotate");
    let (status, body) = post_on_the_console(resources, admin, &path).await;
    assert_eq!(status, 200, "rotate: {body}");
    let rotated: Value = serde_json::from_str(&body)?;
    Ok(rotated["data"]["token_id"]
        .as_str()
        .expect("new token id")
        .to_owned())
}

/// Revoking a rotated token used to answer 200 while its replacement stayed
/// live: the caller read a credential as ended that had only changed id.
/// It answers 409, names the replacement by id and says to
/// revoke that one, and hands out no credential. The replacement is revoked
/// like any token, and a token revoked earlier, which names no replacement,
/// keeps its idempotent 200.
#[tokio::test]
#[serial]
async fn revoking_a_rotated_token_answers_409_and_names_its_replacement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-rotated@example.com", UserRole::Admin).await?;
    let old = mint(
        &resources,
        "svc-rotated",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let replacement = rotate_as(&resources, &admin, &old.token_id).await?;

    let path = format!("/api/admin/tokens/{}/revoke", old.token_id);
    let (status, body) = post_on_the_console(&resources, &admin, &path).await;
    assert_eq!(status, 409, "a rotated token is not revoked again: {body}");
    let answer: Value = serde_json::from_str(&body)?;
    assert_eq!(answer["success"], false, "{body}");
    let message = answer["message"].as_str().expect("a message");
    assert!(
        message.contains(&replacement) && message.contains("revoke that one"),
        "the refusal names the replacement and what to do: {body}"
    );
    assert_eq!(answer["data"]["rotated_to_token_id"], replacement, "{body}");
    assert!(!body.contains("jwt_token"), "no credential is handed out");
    assert!(
        is_active(&resources, &replacement).await?,
        "the refused revoke left the replacement live"
    );

    // The replacement is revoked like any token, and revoking it again is
    // the idempotent answer of a token that names no replacement.
    let path = format!("/api/admin/tokens/{replacement}/revoke");
    for attempt in ["first", "second"] {
        let (status, body) = post_on_the_console(&resources, &admin, &path).await;
        assert_eq!(status, 200, "{attempt} revoke of the replacement: {body}");
    }
    assert!(!is_active(&resources, &replacement).await?);
    Ok(())
}

/// Rotating a rotated token answered 400 "A revoked admin token cannot be
/// rotated; mint a new one", which invited a second token beside the live
/// replacement (carnet#1589, security review). It answers as its revoke
/// does: 409, the replacement named by id, no credential
/// handed out and nothing minted.
#[tokio::test]
#[serial]
async fn rotating_a_rotated_token_answers_409_and_names_its_replacement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-rerotated@example.com", UserRole::Admin).await?;
    let old = mint(
        &resources,
        "svc-rotated",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let replacement = rotate_as(&resources, &admin, &old.token_id).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    let path = format!("/api/admin/tokens/{}/rotate", old.token_id);
    let (status, body) = post_on_the_console(&resources, &admin, &path).await;
    assert_eq!(status, 409, "a rotated token is not rotated again: {body}");
    let answer: Value = serde_json::from_str(&body)?;
    assert_eq!(answer["success"], false, "{body}");
    let message = answer["message"].as_str().expect("a message");
    assert!(
        message.contains(&replacement) && message.contains("rotate that one"),
        "the refusal names the replacement and what to do: {body}"
    );
    assert_eq!(answer["data"]["rotated_to_token_id"], replacement, "{body}");
    assert!(!body.contains("jwt_token"), "no credential is handed out");
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing is minted"
    );
    assert!(
        is_active(&resources, &replacement).await?,
        "the replacement stays live"
    );
    Ok(())
}
