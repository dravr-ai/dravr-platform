// ABOUTME: A human coach's citation roster is the live members of the active groups they coach
// ABOUTME: Runs on whichever driver the test factory selects, so the same assertions prove SQLite and Postgres
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `coach_roster_scope` feeds the tenant-isolation conformance check: a coach
//! persona's reply may cite only athletes the coach coaches. The roster is
//! read from the groups whose `coach_user_id` is the coach, so each way a
//! member stops counting — leaving, the group archived, the coach detached —
//! is asserted here against real rows.

mod common;

use chrono::Utc;
use common::{create_test_database, create_test_user_with_email, create_test_user_with_plan};
use pierre_chat_pipeline::stages::persona_conformance::coach_roster_scope;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use std::sync::Arc;
use uuid::Uuid;

/// The lowercased last four characters of a uuid: the identity a citation
/// carries and `RosterScope::allows` compares.
fn last4(id: Uuid) -> String {
    let text = id.simple().to_string();
    text[text.len() - 4..].to_owned()
}

async fn create_agent(repos: &RepositoryRegistry, owner: Uuid, tenant: TenantId) -> String {
    repos
        .agents
        .create(
            owner,
            tenant,
            &CreateAgentRequest {
                title: "Group Agent".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

async fn create_group(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    owner: Uuid,
    agent_id: &str,
    name: &str,
    coach: Uuid,
) -> Uuid {
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
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
        .unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach), tenant)
        .await
        .unwrap());
    group.id
}

async fn add_member(repos: &RepositoryRegistry, group: Uuid, user: Uuid, tenant: TenantId) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group,
            user_id: user,
            tenant_id: tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

struct Seeded {
    db: Arc<Database>,
    tenant: TenantId,
    coach: Uuid,
    stranger: Uuid,
    group_a: Uuid,
    m1: Uuid,
    m2: Uuid,
    m3: Uuid,
}

/// A coach of three groups: A (active; m1 live, m2 left), B (archived; m3
/// still a live member row), C (active; m1 again). Plus a user who coaches
/// nothing.
async fn seeded() -> Seeded {
    let db = create_test_database().await.unwrap();
    let (owner, _, tenant) = create_test_user_with_plan(&db, "owner@roster.test", "professional")
        .await
        .unwrap();
    let (coach, _) = create_test_user_with_email(&db, "coach@roster.test")
        .await
        .unwrap();
    let (m1, _) = create_test_user_with_email(&db, "m1@roster.test")
        .await
        .unwrap();
    let (m2, _) = create_test_user_with_email(&db, "m2@roster.test")
        .await
        .unwrap();
    let (m3, _) = create_test_user_with_email(&db, "m3@roster.test")
        .await
        .unwrap();
    let (stranger, _) = create_test_user_with_email(&db, "stranger@roster.test")
        .await
        .unwrap();

    let repos = db.repositories();
    let agent_id = create_agent(&repos, owner, tenant).await;
    let group_a = create_group(&repos, tenant, owner, &agent_id, "A", coach).await;
    let group_b = create_group(&repos, tenant, owner, &agent_id, "B", coach).await;
    let group_c = create_group(&repos, tenant, owner, &agent_id, "C", coach).await;

    add_member(&repos, group_a, m1, tenant).await;
    add_member(&repos, group_a, m2, tenant).await;
    add_member(&repos, group_b, m3, tenant).await;
    add_member(&repos, group_c, m1, tenant).await;

    assert!(repos
        .groups
        .remove_member(&group_a.to_string(), m2)
        .await
        .unwrap());
    // Archived by flag alone, so m3's membership row stays live: the group's
    // own `is_active` is what must exclude it.
    repos
        .groups
        .update_group(
            &group_b.to_string(),
            tenant,
            &UpdateGroupRequest {
                name: None,
                description: None,
                agent_id: None,
                max_members: None,
                peer_data_sharing: None,
                respond_mode: None,
                digest_mode: None,
                is_active: Some(false),
            },
        )
        .await
        .unwrap()
        .expect("group B is updated under its tenant");
    assert!(repos
        .groups
        .get_member(&group_b.to_string(), m3)
        .await
        .unwrap()
        .is_some_and(|m| m.left_at.is_none()));

    Seeded {
        db,
        tenant,
        coach,
        stranger,
        group_a,
        m1,
        m2,
        m3,
    }
}

#[tokio::test]
async fn the_roster_is_the_live_members_of_active_coached_groups() {
    let s = seeded().await;
    let repos = s.db.repositories();

    assert_eq!(
        repos
            .groups
            .list_athletes_coached_by(s.coach)
            .await
            .unwrap(),
        vec![s.m1],
        "m1 once though a member of two coached groups; m2 left; m3's group is archived"
    );

    let scope = coach_roster_scope(repos.groups.as_ref(), s.coach)
        .await
        .unwrap();
    assert!(scope.allows(&last4(s.m1)));
    assert!(
        scope.allows(&last4(s.m1).to_uppercase()),
        "a citation's case does not matter"
    );
    assert!(!scope.allows(&last4(s.m2)), "m2 left group A");
    assert!(!scope.allows(&last4(s.m3)), "group B is archived");
    assert!(!scope.allows(&last4(s.stranger)));
    assert!(!scope.is_empty());
}

#[tokio::test]
async fn a_user_who_coaches_no_group_has_an_empty_roster() {
    let s = seeded().await;
    let repos = s.db.repositories();

    assert!(repos
        .groups
        .list_athletes_coached_by(s.stranger)
        .await
        .unwrap()
        .is_empty());
    assert!(coach_roster_scope(repos.groups.as_ref(), s.stranger)
        .await
        .unwrap()
        .is_empty());
    assert!(
        coach_roster_scope(repos.groups.as_ref(), s.m1)
            .await
            .unwrap()
            .is_empty(),
        "being a member of a group is not coaching it"
    );
}

#[tokio::test]
async fn detaching_the_coach_takes_the_group_off_their_roster() {
    let s = seeded().await;
    let repos = s.db.repositories();

    assert!(repos
        .groups
        .set_group_coach_user(&s.group_a.to_string(), None, s.tenant)
        .await
        .unwrap());

    // m1 is still coached through group C, so only A's detachment is visible
    // once C goes too.
    assert_eq!(
        repos
            .groups
            .list_athletes_coached_by(s.coach)
            .await
            .unwrap(),
        vec![s.m1]
    );
    let groups = repos.groups.list_groups_coached_by(s.coach).await.unwrap();
    assert_eq!(groups.len(), 1, "only group C is still coached");
    assert!(repos
        .groups
        .set_group_coach_user(&groups[0].id.to_string(), None, s.tenant)
        .await
        .unwrap());

    assert!(repos
        .groups
        .list_athletes_coached_by(s.coach)
        .await
        .unwrap()
        .is_empty());
    assert!(coach_roster_scope(repos.groups.as_ref(), s.coach)
        .await
        .unwrap()
        .is_empty());
}
