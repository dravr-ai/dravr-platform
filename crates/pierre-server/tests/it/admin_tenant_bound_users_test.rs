// ABOUTME: What a ManageUsers token bound to one tenant reads and pre-approves, over the admin-token mount
// ABOUTME: Pins that its reads stay in its tenant, and that the allow-list and the sign-up codes, platform-wide, are not its own

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A `ManageUsers` token bound to one tenant already read one user by id
//! only inside that tenant, and approved by id only a user held within it
//! (carnet#1589). Two doors stayed open beside those:
//!
//! - the user list was read across every tenant;
//! - `POST /admin/pre-approved-emails` approved, on the spot and for every
//!   tenant, a pending sign-up of no tenant the token does not reach.
//!
//! The reads take the rule the read by id takes (a member of the token's
//! tenant, anyone else answered as a missing account is), and a standing
//! allow, which is platform-wide, is neither written nor removed by a
//! caller bound to one tenant. Nor is the allow-list read by it, and a
//! sign-up code, which lets in whoever redeems it, is neither listed,
//! minted nor revoked by it: the door the allow closed, one route over.
//! A super-admin token is the control: it behaves as before. A token bound
//! to no tenant without the super-admin flag reaches no tenant, so it reads
//! and pre-approves nobody (carnet#1589), and a tenant-bound token's read of
//! a member shows only what is held in its tenant.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_token_routes_test::{bearer, mint, mint_bound, quotable, token_api};
use crate::admin_user_removal_test::seed_user;
use crate::common;
use crate::helpers;

use std::sync::Arc;

use anyhow::Result;
use common::create_test_server_resources;
use dravr_tronc::dates::{Elapsed, Timestamp};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, GeneratedAdminToken};
use pierre_core::models::{ConnectionType, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::signup_codes::{self, MintRequest};
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// `GET {path}` on the token mount as `caller`: the status and the answer.
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

/// The ids of the users an answer lists under `data.users`.
fn listed_users(answer: &str) -> Vec<String> {
    let answer: Value = serde_json::from_str(answer).expect("a JSON answer");
    answer["data"]["users"]
        .as_array()
        .expect("data.users is an array")
        .iter()
        .filter_map(|user| user["id"].as_str().map(ToOwned::to_owned))
        .collect()
}

/// A sign-up still waiting for a decision, a member of no tenant yet.
async fn pending_sign_up(resources: &Arc<ServerContext>, email: &str) -> User {
    let mut user = User::new(email.to_owned(), "hash".to_owned(), Some(email.to_owned()));
    user.user_status = UserStatus::Pending;
    resources.common.repos.users.create(&user).await.unwrap();
    user
}

/// The stored status of `user_id`.
async fn status_of(resources: &Arc<ServerContext>, user_id: Uuid) -> UserStatus {
    resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("the account exists")
        .user_status
}

/// The addresses holding a standing allow.
async fn standing_allows(resources: &Arc<ServerContext>) -> Vec<String> {
    resources
        .common
        .repos
        .pre_approved_emails
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.email)
        .collect()
}

/// What a token bound to no tenant without the super-admin flag is answered.
const NO_TENANT_REFUSAL: &str =
    "Token has no tenant scope — super-admin required for cross-tenant access";

/// The three callers of this file: a `ManageUsers` token bound to `tenant`,
/// one bound to no tenant, and a super-admin token.
async fn callers(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
) -> Result<(
    GeneratedAdminToken,
    GeneratedAdminToken,
    GeneratedAdminToken,
)> {
    let manage_users = vec![AdminPermission::ManageUsers];
    let bound = mint_bound(resources, "club-desk", manage_users.clone(), tenant).await?;
    let unbound = mint(resources, "support-desk", manage_users, false).await?;
    let every_permission = AdminPermissions::super_admin().to_vec();
    let root = mint(resources, "platform-ops", every_permission, true).await?;
    Ok((bound, unbound, root))
}

/// The user list was read by a tenant-bound token across every tenant, while
/// the read of one user by id answered 404. It now stays inside the token's
/// tenant: it names its members only. The second tenant's user is the
/// control that the filter is the tenant and not the route, and the
/// super-admin token reads as before. The unbound token reads nothing.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_reads_only_the_users_of_its_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (inside, tenant, _) = seed_user(repos, "inside").await;
    let (outside, _, _) = seed_user(repos, "outside").await;
    let waiting = pending_sign_up(&resources, "waiting-read@example.com").await;
    let (bound, unbound, root) = callers(&resources, tenant).await?;
    let (inside, outside, waiting) = (
        inside.to_string(),
        outside.to_string(),
        waiting.id.to_string(),
    );

    for status in ["active", "all"] {
        let (code, body) =
            get_as(&resources, &bound, &format!("/admin/users?status={status}")).await;
        assert_eq!(code, 200, "{status}: {body}");
        let ids = listed_users(&body);
        assert!(
            ids.contains(&inside),
            "{status}: its member is listed: {ids:?}"
        );
        assert!(
            !ids.contains(&outside) && !ids.contains(&waiting),
            "{status}: no account outside its tenant is listed: {ids:?}"
        );
        assert!(
            !body.contains(&outside),
            "{status}: nor named anywhere in the answer, its cursor included: {body}"
        );
    }

    // A token bound to no tenant reaches no tenant: every read is refused.
    for path in [
        "/admin/users?status=active".to_owned(),
        format!("/admin/users/{outside}"),
    ] {
        let (code, body) = get_as(&resources, &unbound, &path).await;
        assert_eq!(code, 403, "{path}: {body}");
        assert!(body.contains(NO_TENANT_REFUSAL), "{path}: {body}");
        assert!(!body.contains(&outside), "{path}: names no account: {body}");
    }

    // The control: a super-admin token reads every tenant, as before.
    for caller in [&root] {
        let name = &caller.service_name;
        let (code, body) = get_as(&resources, caller, "/admin/users?status=active").await;
        assert_eq!(code, 200, "{name}: {body}");
        let ids = listed_users(&body);
        assert!(
            ids.contains(&inside) && ids.contains(&outside),
            "{name} lists every tenant's users: {ids:?}"
        );
    }
    Ok(())
}

/// `POST {path}` on the token mount as `caller`, with `body`.
async fn post_as(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    path: &str,
    body: Value,
) -> (u16, String) {
    let response = AxumTestRequest::post(path)
        .header("authorization", &bearer(caller))
        .json(&body)
        .send(token_api(resources))
        .await;
    quotable(response)
}

/// `DELETE {path}` on the token mount as `caller`.
async fn delete_as(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    path: &str,
) -> (u16, String) {
    let response = AxumTestRequest::delete(path)
        .header("authorization", &bearer(caller))
        .send(token_api(resources))
        .await;
    quotable(response)
}

/// A tenant-bound token approved a pending sign-up that is a member of no
/// tenant through its address: the allow route made no tenant check. A
/// standing allow is platform-wide, so a caller bound to one tenant neither
/// records one nor removes one (403, nothing written), for an address with a
/// pending account and for one with none. A token bound to no tenant allows
/// and removes nothing either; the super-admin token allows and removes as
/// before.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_neither_allows_nor_removes_a_pre_approval() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (_, tenant, _) = seed_user(repos, "inside").await;
    let waiting = pending_sign_up(&resources, "waiting-allow@example.com").await;
    let (bound, unbound, root) = callers(&resources, tenant).await?;
    let refusal = |act: &str| {
        format!(
            "Token is scoped to tenant {tenant}; a pre-approval is platform-wide, and it cannot {act} one"
        )
    };

    for email in [waiting.email.as_str(), "nobody-yet@example.com"] {
        let (code, body) = post_as(
            &resources,
            &bound,
            "/admin/pre-approved-emails",
            json!({ "email": email }),
        )
        .await;
        assert_eq!(
            code, 403,
            "{email}: a tenant-bound token allows no address: {body}"
        );
        assert!(body.contains(&refusal("allow")), "{email}: {body}");
    }
    assert_eq!(
        status_of(&resources, waiting.id).await,
        UserStatus::Pending,
        "the refused allow approved nobody"
    );
    assert!(
        standing_allows(&resources).await.is_empty(),
        "and recorded no standing allow"
    );

    // A standing allow a platform caller recorded is not its to remove.
    let (code, body) = post_as(
        &resources,
        &root,
        "/admin/pre-approved-emails",
        json!({ "email": "invited@example.com" }),
    )
    .await;
    assert_eq!(code, 200, "a super-admin token allows an address: {body}");
    let (code, body) = delete_as(
        &resources,
        &bound,
        "/admin/pre-approved-emails/invited@example.com",
    )
    .await;
    assert_eq!(code, 403, "a tenant-bound token removes no allow: {body}");
    assert!(body.contains(&refusal("remove")), "{body}");
    assert_eq!(
        standing_allows(&resources).await,
        vec!["invited@example.com".to_owned()],
        "the refused removal left the allow standing"
    );

    // A token bound to no tenant reaches no tenant: it neither approves the
    // sign-up through its address nor removes an allow.
    let (code, body) = post_as(
        &resources,
        &unbound,
        "/admin/pre-approved-emails",
        json!({ "email": waiting.email }),
    )
    .await;
    assert_eq!(code, 403, "an unbound token allows nothing: {body}");
    assert!(body.contains(NO_TENANT_REFUSAL), "{body}");
    assert_eq!(status_of(&resources, waiting.id).await, UserStatus::Pending);
    let (code, body) = delete_as(
        &resources,
        &unbound,
        "/admin/pre-approved-emails/invited@example.com",
    )
    .await;
    assert_eq!(code, 403, "an unbound token removes nothing: {body}");
    assert!(standing_allows(&resources)
        .await
        .contains(&"invited@example.com".to_owned()));

    // The controls: the super-admin token approves the sign-up through its
    // address and removes an allow.
    let (code, body) = post_as(
        &resources,
        &root,
        "/admin/pre-approved-emails",
        json!({ "email": waiting.email }),
    )
    .await;
    assert_eq!(code, 200, "a super-admin token allows: {body}");
    assert_eq!(status_of(&resources, waiting.id).await, UserStatus::Active);
    let (code, body) = delete_as(
        &resources,
        &root,
        "/admin/pre-approved-emails/invited@example.com",
    )
    .await;
    assert_eq!(code, 200, "a super-admin token removes: {body}");
    assert!(!standing_allows(&resources)
        .await
        .contains(&"invited@example.com".to_owned()));
    Ok(())
}

/// The allow closed one door on a tenant-bound token and left the ones
/// beside it open (carnet#1589, security review): the token read the
/// platform's allow-list, and listed, minted and revoked sign-up codes. A
/// redeemed code activates a pending account whatever tenant it is or will
/// be in, which is what the allow was refused for. Each is refused (403)
/// in the allow's words, nothing read and nothing written. A token bound to
/// no tenant does none of the four either; the super-admin token does all
/// four.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_reads_no_allow_list_and_touches_no_sign_up_code() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (_, tenant, _) = seed_user(repos, "inside").await;
    let (bound, unbound, root) = callers(&resources, tenant).await?;
    let refusal = |what: &str| format!("Token is scoped to tenant {tenant}; {what}");

    let (code, body) = post_as(
        &resources,
        &root,
        "/admin/pre-approved-emails",
        json!({ "email": "listed@example.com" }),
    )
    .await;
    assert_eq!(code, 200, "a super-admin token allows an address: {body}");
    let standing = signup_codes::mint(repos, None, &MintRequest::default()).await?;
    let standing_id = standing.record.id;

    let (code, body) = get_as(&resources, &bound, "/admin/pre-approved-emails").await;
    assert_eq!(
        code, 403,
        "a tenant-bound token reads no allow-list: {body}"
    );
    assert!(
        body.contains(&refusal(
            "the allow-list is platform-wide, and it cannot read it"
        )),
        "{body}"
    );
    assert!(!body.contains("listed@example.com"), "{body}");

    let (code, body) = get_as(&resources, &bound, "/admin/signup-codes").await;
    assert_eq!(
        code, 403,
        "a tenant-bound token lists no sign-up code: {body}"
    );
    assert!(
        body.contains(&refusal(
            "sign-up codes are platform-wide, and it cannot list them"
        )),
        "{body}"
    );
    assert!(!body.contains(&standing_id.to_string()), "{body}");

    let (code, body) = post_as(&resources, &bound, "/admin/signup-codes", json!({})).await;
    assert_eq!(
        code, 403,
        "a tenant-bound token mints no sign-up code: {body}"
    );
    assert!(
        body.contains(&refusal(
            "a sign-up code is platform-wide, and it cannot mint one"
        )),
        "{body}"
    );
    assert_eq!(
        signup_codes::list(repos).await?.len(),
        1,
        "the refused mint stored no code"
    );

    let path = format!("/admin/signup-codes/admin/{standing_id}");
    let (code, body) = delete_as(&resources, &bound, &path).await;
    assert_eq!(
        code, 403,
        "a tenant-bound token revokes no sign-up code: {body}"
    );
    assert!(
        body.contains(&refusal(
            "a sign-up code is platform-wide, and it cannot revoke one"
        )),
        "{body}"
    );
    assert!(
        signup_codes::is_live(repos, &standing.code).await?,
        "the refused revoke left the code live"
    );

    // A token bound to no tenant reaches no tenant: all four are refused.
    for (code, body) in [
        get_as(&resources, &unbound, "/admin/pre-approved-emails").await,
        get_as(&resources, &unbound, "/admin/signup-codes").await,
        post_as(&resources, &unbound, "/admin/signup-codes", json!({})).await,
        delete_as(&resources, &unbound, &path).await,
    ] {
        assert_eq!(code, 403, "{body}");
        assert!(body.contains(NO_TENANT_REFUSAL), "{body}");
        assert!(!body.contains("listed@example.com"), "{body}");
    }
    assert_eq!(signup_codes::list(repos).await?.len(), 1);
    assert!(signup_codes::is_live(repos, &standing.code).await?);

    // The control: the super-admin token reads, mints and revokes.
    let (code, body) = get_as(&resources, &root, "/admin/pre-approved-emails").await;
    assert_eq!(
        code, 200,
        "a super-admin token reads the allow-list: {body}"
    );
    assert!(body.contains("listed@example.com"), "{body}");
    let (code, body) = get_as(&resources, &root, "/admin/signup-codes").await;
    assert_eq!(code, 200, "a super-admin token lists the codes: {body}");
    assert!(body.contains(&standing_id.to_string()), "{body}");
    let (code, body) = post_as(&resources, &root, "/admin/signup-codes", json!({})).await;
    assert_eq!(code, 201, "a super-admin token mints a code: {body}");
    let (code, body) = delete_as(&resources, &root, &path).await;
    assert_eq!(code, 200, "a super-admin token revokes a code: {body}");
    assert!(!signup_codes::is_live(repos, &standing.code).await?);
    Ok(())
}

/// An active account created `age_minutes` ago, a member of no tenant yet,
/// so a listing has one order.
async fn account_aged(resources: &Arc<ServerContext>, label: &str, age_minutes: i64) -> Uuid {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    user.created_at = Timestamp::now() - Elapsed::minutes(age_minutes);
    resources.common.repos.users.create(&user).await.unwrap();
    user.id
}

/// The user list of a tenant-bound token was pinned on its first page only
/// (carnet#1589, test review): the pages after it are read by another
/// statement, with its own tenant filter. An account outside the tenant
/// sits between two members in the list's order, newest first, and the
/// token pages one account at a time: the second page is the older member,
/// never the outsider, and no page names it.
#[tokio::test]
#[serial]
async fn a_tenant_bound_tokens_user_list_stays_in_its_tenant_past_the_first_page() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (newest, tenant, _) = seed_user(repos, "newest").await;
    let outsider = account_aged(&resources, "outsider", 10).await;
    let oldest = account_aged(&resources, "oldest", 20).await;
    repos.users.update_tenant_id(oldest, tenant).await?;
    let (bound, _, _) = callers(&resources, tenant).await?;

    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let path = cursor.as_ref().map_or_else(
            || "/admin/users?status=all&limit=1".to_owned(),
            |cursor| format!("/admin/users?status=all&limit=1&cursor={cursor}"),
        );
        let (code, body) = get_as(&resources, &bound, &path).await;
        assert_eq!(code, 200, "{path}: {body}");
        assert!(
            !body.contains(&outsider.to_string()),
            "no page names the account outside the tenant: {body}"
        );
        let answer: Value = serde_json::from_str(&body)?;
        pages.push(listed_users(&body));
        assert!(pages.len() <= 3, "the listing ends: {pages:?}");
        match answer["data"]["next_cursor"].as_str() {
            Some(next) if answer["data"]["has_more"] == true => cursor = Some(next.to_owned()),
            _ => break,
        }
    }
    assert_eq!(
        pages.concat(),
        vec![newest.to_string(), oldest.to_string()],
        "the tenant's members, newest first, one page each: {pages:?}"
    );
    Ok(())
}

/// A tenant-bound token's read of a member showed the whole account: the
/// providers it holds in other tenants, the messaging identities linked
/// there, the groups it owns there, and the preview of a whole-account
/// reset across every tenant (carnet#1589). It shows what is held in the
/// token's tenant only, and no reset preview, a route the token cannot
/// take. The super-admin token's read of the same account is the control.
#[tokio::test]
#[serial]
async fn a_tenant_bound_tokens_read_of_a_member_shows_only_its_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    // A member of the token's tenant, owning a tenant of its own besides.
    let (_, tenant, _) = seed_user(repos, "inside").await;
    let (member, elsewhere, _) = seed_user(repos, "member").await;
    repos.users.update_tenant_id(member, tenant).await?;
    for (held_in, provider) in [(tenant, "strava"), (elsewhere, "whoop")] {
        repos
            .provider_connections
            .register_connection(member, held_in, provider, &ConnectionType::OAuth, None)
            .await?;
    }
    let (bound, _, root) = callers(&resources, tenant).await?;
    let path = format!("/admin/users/{member}");
    let providers = |body: &str| -> Vec<(String, String)> {
        let answer: Value = serde_json::from_str(body).expect("a JSON answer");
        let mut held: Vec<(String, String)> = answer["data"]["connected_providers"]
            .as_array()
            .expect("data.connected_providers")
            .iter()
            .map(|held| {
                (
                    held["provider"].as_str().unwrap_or_default().to_owned(),
                    held["tenant_id"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        held.sort();
        held
    };

    let (code, body) = get_as(&resources, &bound, &path).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(
        providers(&body),
        vec![("strava".to_owned(), tenant.to_string())],
        "only what the member holds in the token's tenant: {body}"
    );
    assert!(
        !body.contains(&elsewhere.to_string()) && !body.contains("whoop"),
        "nothing of the other tenant is named: {body}"
    );
    let answer: Value = serde_json::from_str(&body)?;
    assert!(
        answer["data"].get("reset_blockers").is_none()
            && answer["data"]
                .get("reset_blockers_with_owned_groups_deleted")
                .is_none(),
        "no preview of a reset the token cannot take: {body}"
    );

    let (code, body) = get_as(&resources, &root, &path).await;
    assert_eq!(code, 200, "{body}");
    let mut every = vec![
        ("strava".to_owned(), tenant.to_string()),
        ("whoop".to_owned(), elsewhere.to_string()),
    ];
    every.sort();
    assert_eq!(providers(&body), every, "the whole account: {body}");
    let answer: Value = serde_json::from_str(&body)?;
    assert!(answer["data"]["reset_blockers"].is_array(), "{body}");
    Ok(())
}
