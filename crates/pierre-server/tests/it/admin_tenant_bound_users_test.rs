// ABOUTME: What a ManageUsers token bound to one tenant reads and pre-approves, over the admin-token mount
// ABOUTME: Pins that its reads stay in its tenant, and that the allow-list and the sign-up codes, platform-wide, are not its own

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A `ManageUsers` token bound to one tenant already read one user by id
//! only inside that tenant, and approved by id only a user held within it
//! (carnet#1589). Two doors stayed open beside those:
//!
//! - the user list, the pending list, and one user's rate limits and
//!   activity were read across every tenant;
//! - `POST /admin/pre-approved-emails` approved, on the spot and for every
//!   tenant, the same pending sign-up the approval by id refused.
//!
//! The reads take the rule the read by id takes (a member of the token's
//! tenant, anyone else answered as a missing account is), and a standing
//! allow, which is platform-wide, is neither written nor removed by a
//! caller bound to one tenant. Nor is the allow-list read by it, and a
//! sign-up code, which lets in whoever redeems it, is neither listed,
//! minted nor revoked by it: the door the allow closed, one route over.
//! A token bound to no tenant and a super-admin token are the controls:
//! both behave as before.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_role_rules_test::reset_tokens_issued_for;
use crate::admin_token_routes_test::{bearer, mint, mint_bound, quotable, token_api};
use crate::admin_user_removal_test::seed_user;
use crate::common;
use crate::helpers;

use std::sync::Arc;

use anyhow::Result;
use chrono::{TimeDelta, Utc};
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, GeneratedAdminToken};
use pierre_core::models::{TenantId, User, UserStatus};
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

/// A refusal's status and body without the time it was answered at, so two
/// refusals compare by what they say.
fn undated((status, body): (u16, String)) -> (u16, Value) {
    let mut answer: Value = serde_json::from_str(&body).expect("a JSON answer");
    if let Some(fields) = answer.as_object_mut() {
        fields.remove("timestamp");
    }
    (status, answer)
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

/// The user list, the pending list, and one user's rate limits and activity
/// were read by a tenant-bound token across every tenant, while the read of
/// that user by id answered 404. Each now stays inside the token's tenant:
/// the lists name its members only, and the per-user reads answer another
/// tenant's user exactly as they answer a missing one. The second tenant's
/// user is the control that the filter is the tenant and not the route, and
/// the unbound and super-admin tokens read as before.
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
    let (code, body) = get_as(&resources, &bound, "/admin/pending-users").await;
    assert_eq!(code, 200, "{body}");
    assert!(
        !listed_users(&body).contains(&waiting),
        "a sign-up that is a member of no tenant is not its tenant's: {body}"
    );

    for read in ["rate-limit", "activity"] {
        let (code, body) =
            get_as(&resources, &bound, &format!("/admin/users/{inside}/{read}")).await;
        assert_eq!(code, 200, "{read} of its own member: {body}");
        let no_such_user = format!("/admin/users/{}/{read}", Uuid::new_v4());
        let missing = undated(get_as(&resources, &bound, &no_such_user).await);
        assert_eq!(missing.0, 404, "{read}: {}", missing.1);
        let another_tenants = format!("/admin/users/{outside}/{read}");
        let other = undated(get_as(&resources, &bound, &another_tenants).await);
        assert_eq!(
            other, missing,
            "{read}: another tenant's user is answered as a missing one"
        );
    }

    // The controls: a token bound to no tenant and a super-admin token read
    // every tenant, as before.
    for caller in [&unbound, &root] {
        let name = &caller.service_name;
        let (code, body) = get_as(&resources, caller, "/admin/users?status=active").await;
        assert_eq!(code, 200, "{name}: {body}");
        let ids = listed_users(&body);
        assert!(
            ids.contains(&inside) && ids.contains(&outside),
            "{name} lists every tenant's users: {ids:?}"
        );
        let (code, body) = get_as(&resources, caller, "/admin/pending-users").await;
        assert_eq!(code, 200, "{name}: {body}");
        assert!(listed_users(&body).contains(&waiting), "{name}: {body}");
        for read in ["rate-limit", "activity"] {
            let (code, body) = get_as(
                &resources,
                caller,
                &format!("/admin/users/{outside}/{read}"),
            )
            .await;
            assert_eq!(code, 200, "{name} reads {read} of any user: {body}");
        }
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

/// A tenant-bound token is refused the approval by id of a pending sign-up
/// that is a member of no tenant, and approved the same sign-up through its
/// address: the allow route made no tenant check. A standing allow is
/// platform-wide, so a caller bound to one tenant neither records one nor
/// removes one (403, nothing written), for an address with a pending
/// account and for one with none. The unbound and super-admin tokens allow
/// and remove as before.
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

    // The door it is already refused at: the approval by id.
    let path = format!("/admin/approve-user/{}", waiting.id);
    let (code, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(code, 403, "the approval by id is refused: {body}");

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

    // The controls: a token bound to no tenant approves the sign-up through
    // its address and removes an allow, as before.
    let (code, body) = post_as(
        &resources,
        &unbound,
        "/admin/pre-approved-emails",
        json!({ "email": waiting.email }),
    )
    .await;
    assert_eq!(code, 200, "an unbound token allows as before: {body}");
    assert_eq!(status_of(&resources, waiting.id).await, UserStatus::Active);
    let (code, body) = delete_as(
        &resources,
        &unbound,
        "/admin/pre-approved-emails/invited@example.com",
    )
    .await;
    assert_eq!(code, 200, "an unbound token removes as before: {body}");
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
/// no tenant does all four as before.
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

    // The control: a token bound to no tenant reads, mints and revokes.
    let (code, body) = get_as(&resources, &unbound, "/admin/pre-approved-emails").await;
    assert_eq!(code, 200, "an unbound token reads the allow-list: {body}");
    assert!(body.contains("listed@example.com"), "{body}");
    let (code, body) = get_as(&resources, &unbound, "/admin/signup-codes").await;
    assert_eq!(code, 200, "an unbound token lists the codes: {body}");
    assert!(body.contains(&standing_id.to_string()), "{body}");
    let (code, body) = post_as(&resources, &unbound, "/admin/signup-codes", json!({})).await;
    assert_eq!(code, 201, "an unbound token mints a code: {body}");
    let (code, body) = delete_as(&resources, &unbound, &path).await;
    assert_eq!(code, 200, "an unbound token revokes a code: {body}");
    assert!(!signup_codes::is_live(repos, &standing.code).await?);
    Ok(())
}

/// A password reset token signs whoever holds it in as the whole account,
/// in every tenant the account is in. The reset is a write, and it was
/// scoped by the rule of the reads: a member of the token's tenant. So a
/// tenant-bound token refused the suspension of a user who is also in
/// another tenant was handed that user's reset token (carnet#1589,
/// security review). The reset now takes the rule of the other writes: the
/// user is held entirely within the token's tenant, or it is refused (403)
/// in their words and no token is issued. A member held within the tenant
/// is reset as before, and so is the shared user by a token bound to no
/// tenant.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_resets_no_password_of_a_user_held_elsewhere_too() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (inside, tenant, _) = seed_user(repos, "inside").await;
    // A member of the token's tenant who owns a tenant of their own.
    let (shared, _, _) = seed_user(repos, "shared").await;
    repos.users.update_tenant_id(shared, tenant).await?;
    let (bound, unbound, _) = callers(&resources, tenant).await?;
    let held_elsewhere = format!(
        "Token is scoped to tenant {tenant}; the user belongs to or holds a provider in a tenant outside it"
    );

    // The write it is already refused: the suspension.
    let path = format!("/admin/suspend-user/{shared}");
    let (code, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(code, 403, "the suspension is refused: {body}");
    assert!(body.contains(&held_elsewhere), "{body}");

    let reset = format!("/admin/users/{shared}/reset-password");
    let (code, body) = post_as(&resources, &bound, &reset, json!({})).await;
    assert_eq!(
        code, 403,
        "a user held in another tenant too is not reset: {body}"
    );
    assert!(body.contains(&held_elsewhere), "{body}");
    assert!(
        !body.contains("reset_token"),
        "no token is handed out: {body}"
    );
    assert_eq!(
        reset_tokens_issued_for(&resources, shared).await,
        0,
        "and none was issued"
    );

    // Held entirely within its tenant: reset as before.
    let path = format!("/admin/users/{inside}/reset-password");
    let (code, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(code, 200, "its own member is reset: {body}");
    assert_eq!(reset_tokens_issued_for(&resources, inside).await, 1);

    // The control: a token bound to no tenant resets the shared user.
    let (code, body) = post_as(&resources, &unbound, &reset, json!({})).await;
    assert_eq!(code, 200, "an unbound token resets as before: {body}");
    assert_eq!(reset_tokens_issued_for(&resources, shared).await, 1);
    Ok(())
}

/// An active account created `age_minutes` ago, a member of no tenant yet,
/// so a listing has one order.
async fn account_aged(resources: &Arc<ServerContext>, label: &str, age_minutes: i64) -> Uuid {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    user.created_at = Utc::now() - TimeDelta::minutes(age_minutes);
    resources.common.repos.users.create(&user).await.unwrap();
    user.id
}

/// The pending list of a tenant-bound token was pinned only for what it
/// leaves out, so a list that named nobody passed (carnet#1589, test
/// review). A pending account that is a member of the token's tenant is
/// listed to it, beside the sign-up of no tenant that is not.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_is_listed_the_pending_members_of_its_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let (_, tenant, _) = seed_user(repos, "inside").await;
    let member = pending_sign_up(&resources, "pending-member@example.com").await;
    repos.users.update_tenant_id(member.id, tenant).await?;
    let waiting = pending_sign_up(&resources, "waiting-elsewhere@example.com").await;
    let (bound, _, _) = callers(&resources, tenant).await?;

    let (code, body) = get_as(&resources, &bound, "/admin/pending-users").await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(
        listed_users(&body),
        vec![member.id.to_string()],
        "its tenant's pending member, and not the sign-up of no tenant ({}): {body}",
        waiting.id
    );
    Ok(())
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
