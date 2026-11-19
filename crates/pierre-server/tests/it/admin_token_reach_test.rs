// ABOUTME: What a token caller reads and what a revoke of a rotated token answers, over both admin mounts
// ABOUTME: Pins the 409 that names a rotation's replacement, and that a token outside the caller's reach answers as a missing one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Two rules of the admin-token surface (carnet#1589), driven through the
//! token mount (`/admin/tokens`) and the console mount (`/api/admin/tokens`):
//!
//! - a rotated token has a live replacement, so revoking the old one ends
//!   nothing: it answers 409 and names the replacement by id, where a token
//!   revoked earlier keeps its idempotent 200;
//! - the list and the read of one token follow the reach the writes follow:
//!   a caller bound to one tenant sees that tenant's tokens, a caller bound
//!   to none without the super-admin flag sees the tokens bound to none, and
//!   the platform's callers see every token. A read, a revoke or a rotation
//!   outside reach answers exactly as a missing id does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_token_routes_test::{
    bearer, console, is_active, listed_ids, mint, mint_bound, post_as, quotable, session, token_api,
};
use crate::common;
use crate::helpers;

use std::sync::Arc;

use anyhow::Result;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, GeneratedAdminToken};
use pierre_core::models::TenantId;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use serial_test::serial;

/// `GET {path}` on the token mount as `caller`: the status and the answer,
/// fit to quote.
async fn get_as(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    path: &str,
) -> (u16, String) {
    let response = AxumTestRequest::get(path)
        .header("authorization", &bearer(caller))
        .send(token_api(resources))
        .await;
    quotable(response)
}

/// The ids `caller` is listed, revoked tokens included.
async fn listed_to(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
) -> Result<Vec<String>> {
    let (status, body) = get_as(resources, caller, "/admin/tokens?include_inactive=true").await;
    assert_eq!(status, 200, "{}: {body}", caller.service_name);
    Ok(listed_ids(&serde_json::from_str::<Value>(&body)?))
}

/// The replacement id a rotation of `token_id` by `caller` answers.
async fn rotate_as(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    token_id: &str,
) -> Result<String> {
    let path = format!("/admin/tokens/{token_id}/rotate");
    let (status, body) = post_as(resources, caller, &path, json!({})).await;
    assert_eq!(status, 200, "rotate: {body}");
    let rotated: Value = serde_json::from_str(&body)?;
    Ok(rotated["data"]["token_id"]
        .as_str()
        .expect("new token id")
        .to_owned())
}

/// Revoking a rotated token used to answer 200 while its replacement stayed
/// live: the caller read a credential as ended that had only changed id.
/// It answers 409 on both mounts, names the replacement by id and says to
/// revoke that one, and hands out no credential. The replacement is revoked
/// like any token, and a token revoked earlier, which names no replacement,
/// keeps its idempotent 200.
#[tokio::test]
#[serial]
async fn revoking_a_rotated_token_answers_409_and_names_its_replacement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-rotated@example.com", UserRole::Admin).await?;
    let root = mint(&resources, "root-desk", vec![], true).await?;
    let old = mint(
        &resources,
        "svc-rotated",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let replacement = rotate_as(&resources, &root, &old.token_id).await?;

    let on_the_console =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/revoke", old.token_id))
            .header("authorization", &admin)
            .json(&json!({}))
            .send(console(&resources))
            .await;
    let on_the_token_api = AxumTestRequest::post(&format!("/admin/tokens/{}/revoke", old.token_id))
        .header("authorization", &bearer(&root))
        .json(&json!({}))
        .send(token_api(&resources))
        .await;
    for response in [on_the_console, on_the_token_api] {
        let (status, body) = quotable(response);
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
    }
    assert!(
        is_active(&resources, &replacement).await?,
        "the refused revoke left the replacement live"
    );

    // The summary of the rotated token names its replacement too.
    let (status, body) = get_as(
        &resources,
        &root,
        &format!("/admin/tokens/{}", old.token_id),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let summary: Value = serde_json::from_str(&body)?;
    assert_eq!(
        summary["data"]["rotated_to_token_id"], replacement,
        "{body}"
    );

    // The replacement is revoked like any token, and revoking it again is
    // the idempotent answer of a token that names no replacement.
    let path = format!("/admin/tokens/{replacement}/revoke");
    for attempt in ["first", "second"] {
        let (status, body) = post_as(&resources, &root, &path, json!({})).await;
        assert_eq!(status, 200, "{attempt} revoke of the replacement: {body}");
    }
    assert!(!is_active(&resources, &replacement).await?);
    Ok(())
}

/// Rotating a rotated token answered 400 "A revoked admin token cannot be
/// rotated; mint a new one", which invited a second token beside the live
/// replacement (carnet#1589, security review). It answers as its revoke
/// does: 409 on both mounts, the replacement named by id, no credential
/// handed out and nothing minted.
#[tokio::test]
#[serial]
async fn rotating_a_rotated_token_answers_409_and_names_its_replacement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-rerotated@example.com", UserRole::Admin).await?;
    let root = mint(&resources, "root-desk", vec![], true).await?;
    let old = mint(
        &resources,
        "svc-rotated",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let replacement = rotate_as(&resources, &root, &old.token_id).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    let on_the_console =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", old.token_id))
            .header("authorization", &admin)
            .json(&json!({}))
            .send(console(&resources))
            .await;
    let on_the_token_api = AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", old.token_id))
        .header("authorization", &bearer(&root))
        .json(&json!({}))
        .send(token_api(&resources))
        .await;
    for response in [on_the_console, on_the_token_api] {
        let (status, body) = quotable(response);
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
    }
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

/// The 409 names a token, so it is answered only to a caller that reaches
/// the rotated one: a `ManageAdminTokens` holder bound to another tenant is
/// answered as for a missing token, which names no replacement.
#[tokio::test]
#[serial]
async fn a_caller_outside_reach_is_not_told_a_rotated_tokens_replacement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (own_tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let root = mint(&resources, "root-desk", vec![], true).await?;
    let bound = mint_bound(&resources, "club-desk", managing, own_tenant).await?;
    let elsewhere = mint_bound(
        &resources,
        "other-club",
        vec![AdminPermission::ListKeys],
        other_tenant,
    )
    .await?;
    let replacement = rotate_as(&resources, &root, &elsewhere.token_id).await?;

    let path = format!("/admin/tokens/{}/revoke", elsewhere.token_id);
    let (status, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(
        status, 404,
        "another tenant's token is out of reach: {body}"
    );
    assert!(
        !body.contains(&replacement),
        "a refusal outside reach names no replacement: {body}"
    );
    Ok(())
}

/// A refusal's status and body without the time it was answered at, so two
/// refusals compare by what they say.
fn undated((status, body): (u16, String)) -> (u16, Value) {
    let mut answer: Value = serde_json::from_str(&body).expect("a JSON answer");
    if let Some(fields) = answer.as_object_mut() {
        fields.remove("timestamp");
    }
    (status, answer)
}

/// An id no token has.
const NO_SUCH_TOKEN: &str = "admin_0000000000000000";

/// What a read of an id no token has answers `caller`, undated.
async fn missing_token_answer(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
) -> (u16, Value) {
    undated(get_as(resources, caller, &format!("/admin/tokens/{NO_SUCH_TOKEN}")).await)
}

/// What `act` ("revoke", "rotate") on `token_id` answers `caller`, undated.
async fn write_answer(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    token_id: &str,
    act: &str,
) -> (u16, Value) {
    let path = format!("/admin/tokens/{token_id}/{act}");
    undated(post_as(resources, caller, &path, json!({})).await)
}

/// The read of a token outside the caller's reach answered as a missing id
/// does, and the two writes still answered 403, in sentences that said the
/// token exists, where it is bound, and, checked first, whether it is a
/// super-admin token (carnet#1589, security review). A revoke and a
/// rotation outside reach now answer exactly as they answer an id no token
/// has, for a caller bound to one tenant and for one bound to none. The
/// token is left live and nothing is minted, as before.
#[tokio::test]
#[serial]
async fn a_write_outside_the_callers_reach_answers_as_a_missing_token_does() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (own_tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let lister = vec![AdminPermission::ListKeys];
    let bound = mint_bound(&resources, "club-desk", managing.clone(), own_tenant).await?;
    let unbound = mint(&resources, "platform-desk", managing, false).await?;
    let elsewhere = mint_bound(&resources, "other-club", lister.clone(), other_tenant).await?;
    let platform = mint(&resources, "platform-lister", lister.clone(), false).await?;
    let root = mint(&resources, "root-desk", lister, true).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    for act in ["revoke", "rotate"] {
        let missing = write_answer(&resources, &bound, NO_SUCH_TOKEN, act).await;
        assert_eq!(missing.0, 404, "{act}: {}", missing.1);
        for outside in [&elsewhere, &platform, &root] {
            let answer = write_answer(&resources, &bound, &outside.token_id, act).await;
            assert_eq!(
                answer, missing,
                "{act} of {} by a tenant-bound caller is answered as a missing token is",
                outside.service_name
            );
        }
        let missing = write_answer(&resources, &unbound, NO_SUCH_TOKEN, act).await;
        let answer = write_answer(&resources, &unbound, &elsewhere.token_id, act).await;
        assert_eq!(
            answer, missing,
            "{act} of a tenant's token by an unbound caller is answered as a missing token is"
        );
    }
    for outside in [&elsewhere, &platform, &root] {
        assert!(
            is_active(&resources, &outside.token_id).await?,
            "{} is left live",
            outside.service_name
        );
    }
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing was minted"
    );
    Ok(())
}

/// A `ManageAdminTokens` holder bound to one tenant listed and read the
/// summary of every token without the super-admin flag, of every tenant and
/// of the platform. It lists and reads its own tenant's tokens only: the
/// reach its revokes and rotations already kept. A read outside it answers
/// exactly as a missing id does, so the answer does not say the token
/// exists.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_lists_and_reads_only_its_tenants_tokens() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (own_tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let lister = vec![AdminPermission::ListKeys];
    let bound = mint_bound(&resources, "club-desk", managing, own_tenant).await?;
    let mine = mint_bound(&resources, "club-lister", lister.clone(), own_tenant).await?;
    let elsewhere = mint_bound(&resources, "other-club", lister.clone(), other_tenant).await?;
    let platform = mint(&resources, "platform-lister", lister.clone(), false).await?;
    let root = mint(&resources, "root-desk", lister, true).await?;

    let ids = listed_to(&resources, &bound).await?;
    assert!(
        ids.contains(&mine.token_id) && ids.contains(&bound.token_id),
        "its tenant's tokens are listed: {ids:?}"
    );
    for outside in [&elsewhere, &platform, &root] {
        assert!(
            !ids.contains(&outside.token_id),
            "{} is outside the tenant and must not be listed: {ids:?}",
            outside.service_name
        );
    }

    let (status, body) = get_as(
        &resources,
        &bound,
        &format!("/admin/tokens/{}", mine.token_id),
    )
    .await;
    assert_eq!(status, 200, "its tenant's token is read: {body}");
    let missing = missing_token_answer(&resources, &bound).await;
    assert_eq!(missing.0, 404, "{}", missing.1);
    // The super-admin token among them: bound to no tenant, it is outside
    // this caller's reach, and the answer does not say what kind it is.
    for outside in [&elsewhere, &platform, &root] {
        let path = format!("/admin/tokens/{}", outside.token_id);
        let answer = undated(get_as(&resources, &bound, &path).await);
        assert_eq!(
            answer, missing,
            "{} is read as a missing token is",
            outside.service_name
        );
    }

    // The platform's callers are the control: a super-admin token and an
    // admin's console session list every token.
    let ids = listed_to(&resources, &root).await?;
    let admin = session(&resources, "admin-reach@example.com", UserRole::Admin).await?;
    let body: Value = AxumTestRequest::get("/api/admin/tokens?include_inactive=true")
        .header("authorization", &admin)
        .send(console(&resources))
        .await
        .json();
    for listing in [ids, listed_ids(&body)] {
        for token in [&bound, &mine, &elsewhere, &platform, &root] {
            assert!(
                listing.contains(&token.token_id),
                "a platform caller lists {}: {listing:?}",
                token.service_name
            );
        }
    }
    let (status, body) = get_as(
        &resources,
        &root,
        &format!("/admin/tokens/{}", elsewhere.token_id),
    )
    .await;
    assert_eq!(status, 200, "a super-admin token reads any token: {body}");
    Ok(())
}

/// A `ManageAdminTokens` holder bound to no tenant, without the super-admin
/// flag, reads no tenant's data, and listed every tenant's tokens all the
/// same. It lists and reads the tokens bound to no tenant, the ones it
/// revokes and rotates, and a tenant's token is read as a missing one is.
#[tokio::test]
#[serial]
async fn an_unbound_token_lists_and_reads_only_tokens_bound_to_no_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let tenant = TenantId::generate();
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let lister = vec![AdminPermission::ListKeys];
    let unbound = mint(&resources, "platform-desk", managing, false).await?;
    let free = mint(&resources, "free-lister", lister.clone(), false).await?;
    let club = mint_bound(&resources, "club-lister", lister.clone(), tenant).await?;
    let root = mint(&resources, "root-desk", lister, true).await?;

    let ids = listed_to(&resources, &unbound).await?;
    assert!(
        ids.contains(&free.token_id) && ids.contains(&unbound.token_id),
        "the tokens bound to no tenant are listed: {ids:?}"
    );
    for outside in [&club, &root] {
        assert!(
            !ids.contains(&outside.token_id),
            "{} is outside its reach and must not be listed: {ids:?}",
            outside.service_name
        );
    }

    let (status, body) = get_as(
        &resources,
        &unbound,
        &format!("/admin/tokens/{}", free.token_id),
    )
    .await;
    assert_eq!(status, 200, "a token bound to no tenant is read: {body}");
    let missing = missing_token_answer(&resources, &unbound).await;
    assert_eq!(missing.0, 404, "{}", missing.1);
    let path = format!("/admin/tokens/{}", club.token_id);
    let answer = undated(get_as(&resources, &unbound, &path).await);
    assert_eq!(answer, missing, "a tenant's token is read as a missing one");
    Ok(())
}
