// ABOUTME: carnet#1463 — `user reset --delete-owned-groups` archives the groups the account owns, then resets
// ABOUTME: Members released, invites off, history kept; a group it only coaches still refuses and nothing is archived

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /admin/users/{id}/reset` with `delete_owned_groups`, against the
//! real handler and a real database. Each active group the account owns is
//! archived through the owner's own delete before the reset runs; a group it
//! coaches for someone else still refuses, before anything is archived.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::Utc;
use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::agents::AgentVisibility;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupInvite, GroupInviteKind, GroupRespondMode,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_routes_admin::handlers::types::ResetAccountRequest;
use pierre_routes_admin::handlers::user_removal::handle_reset_user;
use pierre_routes_admin::handlers::users::handle_get_user;
use pierre_routes_admin::AdminApiContext;
use serde_json::json;
use uuid::Uuid;

use crate::admin_user_removal_test::{
    admin_context, blockers_of, body_json, join_group, manage_users_token, resources, seed_agent,
    seed_onboarded_user, seed_user, super_admin_token, RevokeStub,
};
use crate::admin_user_reset_test::count_for;

/// One account's group memberships, released or live.
const MEMBERSHIPS: &str =
    "SELECT COUNT(*) AS n FROM coaching_group_members WHERE CAST(user_id AS TEXT) = $1";

/// One account's memberships an archive released.
const RELEASED_MEMBERSHIPS: &str = "SELECT COUNT(*) AS n FROM coaching_group_members \
     WHERE CAST(user_id AS TEXT) = $1 AND left_at IS NOT NULL";

async fn reset_deleting_owned_groups(
    context: Arc<AdminApiContext>,
    token: ValidatedAdminToken,
    user_id: Uuid,
) -> AppResult<Response> {
    handle_reset_user(
        State(context),
        Extension(token),
        Path(user_id.to_string()),
        Json(ResetAccountRequest {
            reason: Some("starting over".to_owned()),
            delete_owned_groups: true,
        }),
    )
    .await
}

/// An active group `owner` owns, run by `agent_id` (an agent someone else
/// authored, so the agent itself never refuses the reset), optionally coached
/// by `coach`.
async fn seed_group_on(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant_id: TenantId,
    name: &str,
    agent_id: String,
    coach: Option<Uuid>,
) -> Uuid {
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id,
                owner_id: owner,
                coach_user_id: coach,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

/// The reset archives the group the account owns — members released, the
/// group row and its room kept — names it, and then wipes the account.
#[tokio::test]
async fn delete_owned_groups_archives_each_owned_group_then_resets() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-groups-owner").await;
    let (member_id, member_tenant, _) = seed_user(repos, "reset-groups-member").await;
    let agent = seed_agent(
        repos,
        member_id,
        member_tenant,
        "Shared hills coach",
        AgentVisibility::Tenant,
    )
    .await;
    let group_id = seed_group_on(repos, user_id, tenant_id, "Hill club", agent, None).await;
    join_group(repos, group_id, member_id, member_tenant).await;

    let response = reset_deleting_owned_groups(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["groups_archived"],
        json!([{ "group_id": group_id.to_string(), "name": "Hill club" }]),
        "{body}"
    );

    let group = repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("a group another member used is kept, archived");
    assert!(!group.is_active, "bug: the owned group is still active");
    assert!(
        repos
            .groups
            .get_member(&group_id.to_string(), member_id)
            .await
            .unwrap()
            .is_none(),
        "bug: the member is still live in the archived group"
    );
    assert_eq!(
        count_for(&resources, RELEASED_MEMBERSHIPS, &member_id.to_string()).await,
        1,
        "bug: the member's row was not kept, released, in the archived group"
    );
    assert_eq!(
        count_for(&resources, MEMBERSHIPS, &user_id.to_string()).await,
        0,
        "bug: the reset kept the account's own membership"
    );
    assert!(
        repos.users.get_global(user_id).await.unwrap().is_some(),
        "the account itself stays"
    );
}

/// A group the account coaches for someone else is not the account's to
/// delete: the reset still refuses, and the group it owns is left active.
#[tokio::test]
async fn a_group_the_account_only_coaches_still_refuses_and_nothing_is_archived() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-groups-coach").await;
    let (other_id, other_tenant, _) = seed_user(repos, "reset-groups-other-owner").await;
    let agent = seed_agent(
        repos,
        other_id,
        other_tenant,
        "Other owner coach",
        AgentVisibility::Tenant,
    )
    .await;
    let own = seed_group_on(repos, user_id, tenant_id, "Own squad", agent.clone(), None).await;
    seed_group_on(
        repos,
        other_id,
        other_tenant,
        "Their squad",
        agent,
        Some(user_id),
    )
    .await;

    let response = reset_deleting_owned_groups(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![(
            "coaches_coaching_group".to_owned(),
            "Their squad".to_owned()
        )],
        "{body}"
    );
    assert_eq!(body["data"]["groups_archived"], json!([]), "{body}");
    assert!(
        repos
            .groups
            .get_group(&own.to_string(), tenant_id)
            .await
            .unwrap()
            .unwrap()
            .is_active,
        "bug: a refused reset archived the group the account owns"
    );
}

/// The user read previews both answers, so `pierre-cli user reset
/// --delete-owned-groups` can show what it would delete before `--yes`.
#[tokio::test]
async fn the_user_read_previews_the_reset_with_owned_groups_deleted() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-groups-read").await;
    let (other_id, other_tenant, _) = seed_user(repos, "reset-groups-read-other").await;
    let agent = seed_agent(
        repos,
        other_id,
        other_tenant,
        "Read coach",
        AgentVisibility::Tenant,
    )
    .await;
    let group_id = seed_group_on(repos, user_id, tenant_id, "Read squad", agent, None).await;

    let response = handle_get_user(
        State(admin_context(&resources, &stub.url)),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
    )
    .await
    .expect("get user handler");
    let body = body_json(response.into_response()).await;
    assert_eq!(
        body["data"]["reset_blockers"],
        json!([{ "kind": "owns_coaching_group", "detail": "Read squad" }]),
        "{body}"
    );
    assert_eq!(
        body["data"]["reset_blockers_with_owned_groups_deleted"],
        json!([]),
        "{body}"
    );
    assert_eq!(
        body["data"]["owned_active_groups"],
        json!([{ "id": group_id.to_string(), "name": "Read squad" }]),
        "{body}"
    );
}

/// An invite the account made to a group someone else owns is not the
/// account's to delete either: the reset still refuses over it, and the
/// group the account owns is left active.
#[tokio::test]
async fn an_invite_to_another_owners_group_still_refuses_and_nothing_is_archived() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-groups-inviter").await;
    let (other_id, other_tenant, _) = seed_user(repos, "reset-groups-invite-owner").await;
    let agent = seed_agent(
        repos,
        other_id,
        other_tenant,
        "Invite owner coach",
        AgentVisibility::Tenant,
    )
    .await;
    let own = seed_group_on(repos, user_id, tenant_id, "Own pack", agent.clone(), None).await;
    let theirs = seed_group_on(repos, other_id, other_tenant, "Their pack", agent, None).await;
    repos
        .groups
        .create_invite(&GroupInvite {
            id: Uuid::new_v4(),
            group_id: theirs,
            tenant_id: other_tenant.to_string(),
            code: Uuid::new_v4().simple().to_string()[..8].to_owned(),
            kind: GroupInviteKind::Member,
            created_by: user_id,
            expires_at: None,
            max_uses: None,
            use_count: 0,
            is_active: true,
            approves_signup: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = reset_deleting_owned_groups(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![("created_group_invite".to_owned(), "Their pack".to_owned())],
        "{body}"
    );
    assert_eq!(body["data"]["groups_archived"], json!([]), "{body}");
    assert!(
        repos
            .groups
            .get_group(&own.to_string(), tenant_id)
            .await
            .unwrap()
            .unwrap()
            .is_active,
        "bug: a refused reset archived the group the account owns"
    );
}

/// Asked to delete the owned groups on a server with no group service wired,
/// the reset refuses before anything is touched rather than leave them.
#[tokio::test]
async fn delete_owned_groups_without_a_group_service_refuses_and_touches_nothing() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-groups-unwired").await;
    let (other_id, other_tenant, _) = seed_user(repos, "reset-groups-unwired-other").await;
    let agent = seed_agent(
        repos,
        other_id,
        other_tenant,
        "Unwired coach",
        AgentVisibility::Tenant,
    )
    .await;
    let own = seed_group_on(repos, user_id, tenant_id, "Unwired squad", agent, None).await;
    let mut context = Arc::try_unwrap(admin_context(&resources, &stub.url))
        .unwrap_or_else(|_| panic!("bug: the fresh admin context is shared"));
    context.group_service = None;

    let error = reset_deleting_owned_groups(Arc::new(context), super_admin_token(), user_id)
        .await
        .expect_err("no group service is wired");
    assert_eq!(error.code, ErrorCode::ResourceUnavailable, "{error}");
    assert!(
        repos
            .groups
            .get_group(&own.to_string(), tenant_id)
            .await
            .unwrap()
            .unwrap()
            .is_active,
        "bug: an unwired reset archived the group the account owns"
    );
    assert!(
        repos.users.get_global(user_id).await.unwrap().is_some(),
        "the account itself stays"
    );
}
