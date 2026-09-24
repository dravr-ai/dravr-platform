// ABOUTME: The TrainingPeaks delegation seeder leaves one coach link waiting for the member it names
// ABOUTME: The coach account, the coached group, both group threads and the proposal, and a rerun that seeds nothing twice
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::groups::GroupRole;
use pierre_core::models::{DelegationStatus, ProviderAccountRole};
use pierre_seeders::trainingpeaks_delegation::{
    run, SeedArgs, ATHLETE_ID, ATHLETE_NAME, GROUP_NAME,
};

/// The backend TrainingPeaks' exposure notice guards.
const TP_NOTICE_BACKEND: &str = "sciotte_trainingpeaks";

/// TrainingPeaks' current exposure-notice version.
fn tp_terms_version() -> &'static str {
    provider_terms_version(TP_NOTICE_BACKEND).expect("TrainingPeaks carries a notice")
}

fn args() -> SeedArgs {
    SeedArgs {
        coach_email: "coach@seed.test".to_owned(),
        member_email: "member@seed.test".to_owned(),
        owner_email: "owner@seed.test".to_owned(),
        model: Some("seed-model".to_owned()),
    }
}

#[tokio::test]
async fn the_seeder_leaves_one_link_waiting_for_its_member() {
    let res = create_test_server_resources().await.unwrap();
    let database = &res.agent.database;
    let (coach, _, coach_tenant) =
        create_test_user_with_plan(database, "coach@seed.test", "professional")
            .await
            .unwrap();
    let (member, _, member_tenant) =
        create_test_user_with_plan(database, "member@seed.test", "professional")
            .await
            .unwrap();
    let (owner, _, _) = create_test_user_with_plan(database, "owner@seed.test", "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;

    run(args(), repos).await.unwrap();

    // The coach: a TrainingPeaks coach account, granted, notice accepted.
    let coach_user = repos.users.get_global(coach).await.unwrap().unwrap();
    assert!(coach_user.manages_roster);
    assert_eq!(
        repos
            .users
            .provider_terms_version(coach, TP_NOTICE_BACKEND)
            .await
            .unwrap()
            .as_deref(),
        Some(tp_terms_version())
    );
    let coach_rows = repos
        .provider_connections
        .get_for_user(coach, Some(coach_tenant))
        .await
        .unwrap();
    let coach_row = coach_rows
        .iter()
        .find(|row| row.provider == SCIOTTE_TRAININGPEAKS)
        .expect("the coach's TrainingPeaks connection");
    assert_eq!(coach_row.account_role, Some(ProviderAccountRole::Coach));
    assert!(repos
        .oauth_tokens
        .get_token(coach, coach_tenant, SCIOTTE_TRAININGPEAKS)
        .await
        .unwrap()
        .is_some());

    // The group: coached by the coach, the owner and the member in it.
    let groups = repos.groups.list_groups_coached_by(coach).await.unwrap();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.name, GROUP_NAME);
    assert_eq!(group.owner_id, owner);
    let members = repos
        .groups
        .list_members(&group.id.to_string())
        .await
        .unwrap();
    let mut roles: Vec<(uuid::Uuid, GroupRole)> =
        members.iter().map(|m| (m.user_id, m.role)).collect();
    roles.sort_by_key(|(id, _)| *id);
    let mut expected = vec![(owner, GroupRole::Owner), (member, GroupRole::Member)];
    expected.sort_by_key(|(id, _)| *id);
    assert_eq!(roles, expected);

    // The member's group thread and the proposal naming them.
    let page = repos
        .chat
        .list_conversations(&member.to_string(), member_tenant, 50, 0)
        .await
        .unwrap();
    let group_id = group.id.to_string();
    assert_eq!(
        page.items
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id.as_str()))
            .count(),
        1
    );
    // The coach's own thread in the group, which is their way into Group info.
    let coach_page = repos
        .chat
        .list_conversations(&coach.to_string(), coach_tenant, 50, 0)
        .await
        .unwrap();
    assert_eq!(
        coach_page
            .items
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id.as_str()))
            .count(),
        1
    );
    let links = repos
        .delegated_connections
        .list_live_for_member(member)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.status, DelegationStatus::Proposed);
    assert_eq!(link.group_id, group.id);
    assert_eq!(link.coach_user_id, coach);
    assert_eq!(link.provider, SCIOTTE_TRAININGPEAKS);
    assert_eq!(link.provider_athlete_id, ATHLETE_ID);
    assert_eq!(link.provider_athlete_name.as_deref(), Some(ATHLETE_NAME));

    assert!(
        page.items
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id.as_str()))
            .all(|c| c.model == "seed-model"),
        "the thread runs on the model the seeder was given"
    );

    // A rerun finds the group and seeds nothing twice.
    run(args(), repos).await.unwrap();
    assert_eq!(
        repos
            .groups
            .list_groups_coached_by(coach)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repos
            .delegated_connections
            .list_live_for_member(member)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// With no model resolved the seeder refuses before writing anything, and
/// names both ways to give it one.
#[tokio::test]
async fn the_seeder_refuses_without_a_model_and_writes_nothing() {
    let res = create_test_server_resources().await.unwrap();
    let database = &res.agent.database;
    let (coach, _, _) = create_test_user_with_plan(database, "coach@seed.test", "professional")
        .await
        .unwrap();
    for email in ["member@seed.test", "owner@seed.test"] {
        create_test_user_with_plan(database, email, "professional")
            .await
            .unwrap();
    }
    let repos = &res.common.repos;

    let error = run(
        SeedArgs {
            model: None,
            ..args()
        },
        repos,
    )
    .await
    .expect_err("no model to create the threads with");
    assert_eq!(error.code, ErrorCode::ConfigError, "{error}");
    assert!(error.message.contains("pass --model"), "{error}");
    assert!(repos
        .groups
        .list_groups_coached_by(coach)
        .await
        .unwrap()
        .is_empty());
    let coach_user = repos.users.get_global(coach).await.unwrap().unwrap();
    assert!(!coach_user.manages_roster);
}
