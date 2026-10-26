// ABOUTME: carnet#1161 — a coaching group its owner archived no longer blocks deleting the owner's account
// ABOUTME: The delete takes the group, its invites and its private agent along; a group someone could still lose blocks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A coach who deleted their only group in the app (which archives it) could
//! never delete their account: the blocker read named every group they owned
//! and every invite they wrote, archived or not. These tests drive the blocker
//! read and the one-transaction account deletion the operator and self-serve
//! paths share, against a real database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::Utc;
use pierre_core::errors::ErrorCode;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupInvite, GroupInviteKind, GroupMember,
    GroupRespondMode, GroupRole,
};
use pierre_core::models::{Tenant, TenantId, User, UserReferenceKind, UserStatus};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::account::delete_account;
use uuid::Uuid;

use crate::common::create_test_server_resources;

async fn resources() -> Arc<ServerContext> {
    create_test_server_resources().await.unwrap()
}

/// An active user with a tenant of their own.
async fn seed_user(repos: &RepositoryRegistry, label: &str) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// A group `owner` runs on a private agent they wrote, with one member invite
/// they created, as the coach onboarding step leaves it. Returns the group and
/// invite ids.
async fn seed_group_with_invite(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant_id: TenantId,
    name: &str,
) -> (Uuid, Uuid) {
    let agent_id = repos
        .agents
        .create_system_agent(
            owner,
            tenant_id,
            &CreateSystemAgentRequest {
                title: format!("{name} agent"),
                description: None,
                system_prompt: "You coach the group.".to_owned(),
                category: AgentCategory::Recovery,
                tags: vec![],
                visibility: AgentVisibility::Private,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    let group_id = repos
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
                coach_user_id: None,
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
        .id;
    let invite_id = repos
        .groups
        .create_invite(&GroupInvite {
            id: Uuid::new_v4(),
            group_id,
            tenant_id: tenant_id.to_string(),
            code: Uuid::new_v4().simple().to_string()[..8].to_owned(),
            kind: GroupInviteKind::Member,
            created_by: owner,
            expires_at: None,
            max_uses: None,
            use_count: 0,
            is_active: true,
            approves_signup: false,
            created_at: now,
        })
        .await
        .unwrap()
        .id;
    (group_id, invite_id)
}

async fn join_group(repos: &RepositoryRegistry, group_id: Uuid, user_id: Uuid, tenant: TenantId) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: false,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// The blockers the delete would refuse over, as `(kind, detail)`.
async fn blockers(repos: &RepositoryRegistry, user_id: Uuid) -> Vec<(String, String)> {
    repos
        .users
        .deletion_blockers(user_id)
        .await
        .unwrap()
        .into_iter()
        .map(|b| (b.kind.as_str().to_owned(), b.detail))
        .collect()
}

#[tokio::test]
async fn an_archived_group_nobody_joined_goes_with_its_owner() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (owner, tenant) = seed_user(repos, "coach").await;
    let (group_id, invite_id) =
        seed_group_with_invite(repos, owner, tenant, "Groupe E2E Camille").await;
    // The owner's own copy of the room goes with them and must not hold it back.
    repos
        .chat
        .create_conversation(
            &owner.to_string(),
            tenant,
            "Groupe E2E Camille",
            "model",
            None,
            Some(&group_id.to_string()),
        )
        .await
        .unwrap();
    assert!(
        repos
            .groups
            .delete_group(&group_id.to_string(), tenant)
            .await
            .unwrap(),
        "the owner's delete archives the group it just created"
    );

    assert_eq!(
        blockers(repos, owner).await,
        Vec::<(String, String)>::new(),
        "an archived group, its invite and its private agent block nothing"
    );
    let deletion = delete_account(repos, owner, None).await.unwrap();
    assert_eq!(
        deletion.removed_from("coaching_groups"),
        1,
        "the delete takes the archived group along and counts it"
    );
    assert!(
        repos.users.get_global(owner).await.unwrap().is_none(),
        "the account is gone once nothing blocks it"
    );
    assert!(
        repos
            .groups
            .get_invite_by_id(&invite_id.to_string())
            .await
            .unwrap()
            .is_none(),
        "the group's invite cascades with it"
    );
}

#[tokio::test]
async fn an_archived_group_a_former_member_still_talks_in_keeps_blocking() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (owner, tenant) = seed_user(repos, "coach").await;
    let (member, _) = seed_user(repos, "athlete").await;
    let (group_id, _) = seed_group_with_invite(repos, owner, tenant, "Tuesday Track").await;
    join_group(repos, group_id, member, tenant).await;
    repos
        .chat
        .create_conversation(
            &member.to_string(),
            tenant,
            "Tuesday Track",
            "model",
            None,
            Some(&group_id.to_string()),
        )
        .await
        .unwrap();
    assert!(
        repos
            .groups
            .delete_group(&group_id.to_string(), tenant)
            .await
            .unwrap(),
        "the owner's delete archives the group it just created"
    );

    let named = blockers(repos, owner).await;
    assert!(
        named.contains(&(
            UserReferenceKind::OwnsCoachingGroup.as_str().to_owned(),
            "Tuesday Track".to_owned()
        )),
        "the member's conversation still names the group: {named:?}"
    );
    let refused = delete_account(repos, owner, None).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ResourceLocked, "{refused}");
    assert!(
        repos.users.get_global(owner).await.unwrap().is_some(),
        "a refused delete leaves the account in place"
    );
}

#[tokio::test]
async fn a_live_group_still_blocks_its_owner() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (owner, tenant) = seed_user(repos, "coach").await;
    seed_group_with_invite(repos, owner, tenant, "Long Run Crew").await;

    let named = blockers(repos, owner).await;
    for kind in [
        UserReferenceKind::OwnsCoachingGroup,
        UserReferenceKind::CreatedGroupInvite,
        UserReferenceKind::AuthoredAgent,
    ] {
        assert!(
            named.iter().any(|(k, _)| k == kind.as_str()),
            "a live group keeps {} blocking: {named:?}",
            kind.as_str()
        );
    }
}
