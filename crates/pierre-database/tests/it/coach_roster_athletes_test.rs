// ABOUTME: Pins list_athletes_coached_by against whichever backend DATABASE_URL names
// ABOUTME: A left member, an archived group and a detached coach each drop off the roster
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `list_athletes_coached_by` is the roster a coach's reads are scoped to.
//! Its edge cases were pinned only by the chat pipeline's inline tests, which
//! run on `SQLite` alone; `create_test_db` opens whichever `DATABASE_URL`
//! names, so this suite holds the same query to `PostgreSQL` too.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::groups::UpdateGroupRequest;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use pierre_test_support::delegation::{add_group_member, create_coached_group, create_group_agent};
use pierre_test_support::garmin::create_athlete;
use uuid::Uuid;

/// A coach of three groups: A (active; `m1` live, `m2` left), B (archived;
/// `m3` still a live member row), C (active; `m1` again). Plus a user who
/// coaches nothing.
struct Seeded {
    db: Database,
    tenant: TenantId,
    coach: Uuid,
    stranger: Uuid,
    group_a: Uuid,
    group_c: Uuid,
    m1: Uuid,
    m2: Uuid,
    m3: Uuid,
}

async fn user(repos: &RepositoryRegistry) -> Uuid {
    create_athlete(repos).await.unwrap().0
}

async fn seeded() -> Seeded {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (owner, tenant) = create_athlete(repos).await.unwrap();
    let coach = user(repos).await;
    let m1 = user(repos).await;
    let m2 = user(repos).await;
    let m3 = user(repos).await;
    let stranger = user(repos).await;

    let agent_id = create_group_agent(repos, owner, tenant).await.unwrap();
    let mut groups = Vec::new();
    for name in ["A", "B", "C"] {
        groups.push(
            create_coached_group(repos, owner, tenant, &agent_id, name, coach)
                .await
                .unwrap(),
        );
    }
    let (group_a, group_b, group_c) = (groups[0], groups[1], groups[2]);

    add_group_member(repos, group_a, m1, tenant).await.unwrap();
    add_group_member(repos, group_a, m2, tenant).await.unwrap();
    add_group_member(repos, group_b, m3, tenant).await.unwrap();
    add_group_member(repos, group_c, m1, tenant).await.unwrap();

    assert!(
        repos
            .groups
            .remove_member(&group_a.to_string(), m2)
            .await
            .unwrap(),
        "m2's membership of group A is removed"
    );
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
                peer_data_sharing: None,
                respond_mode: None,
                digest_mode: None,
                is_active: Some(false),
            },
        )
        .await
        .unwrap()
        .expect("group B is updated under its tenant");
    assert!(
        repos
            .groups
            .get_member(&group_b.to_string(), m3)
            .await
            .unwrap()
            .is_some_and(|m| m.left_at.is_none()),
        "archiving group B leaves m3's membership row live"
    );

    Seeded {
        db,
        tenant,
        coach,
        stranger,
        group_a,
        group_c,
        m1,
        m2,
        m3,
    }
}

#[tokio::test]
async fn the_roster_is_the_live_members_of_active_coached_groups() {
    let s = seeded().await;
    let got =
        s.db.repositories()
            .groups
            .list_athletes_coached_by(s.coach)
            .await
            .unwrap();

    assert_eq!(
        got,
        vec![s.m1],
        "m1 once though a member of two coached groups; m2 left; m3's group is archived"
    );
    assert!(!got.contains(&s.m2), "m2 left group A: {got:?}");
    assert!(!got.contains(&s.m3), "group B is archived: {got:?}");
    assert!(
        !got.contains(&s.stranger),
        "a user in no group is on no roster: {got:?}"
    );
}

#[tokio::test]
async fn a_user_who_coaches_no_group_has_an_empty_roster() {
    let s = seeded().await;
    let repos = s.db.repositories();

    let got = repos
        .groups
        .list_athletes_coached_by(s.stranger)
        .await
        .unwrap();
    assert!(got.is_empty(), "the stranger coaches nothing, got {got:?}");

    let got = repos.groups.list_athletes_coached_by(s.m1).await.unwrap();
    assert!(
        got.is_empty(),
        "being a member of a group is not coaching it, got {got:?}"
    );
}

#[tokio::test]
async fn detaching_the_coach_takes_the_group_off_their_roster() {
    let s = seeded().await;
    let repos = s.db.repositories();

    assert!(
        repos
            .groups
            .set_group_coach_user(&s.group_a.to_string(), None, s.tenant)
            .await
            .unwrap(),
        "group A's coach is detached"
    );
    // m1 is still coached through group C, so A's detachment shows only once
    // C goes too.
    assert_eq!(
        repos
            .groups
            .list_athletes_coached_by(s.coach)
            .await
            .unwrap(),
        vec![s.m1],
        "m1 is still coached through group C"
    );

    assert!(
        repos
            .groups
            .set_group_coach_user(&s.group_c.to_string(), None, s.tenant)
            .await
            .unwrap(),
        "group C's coach is detached"
    );
    let got = repos
        .groups
        .list_athletes_coached_by(s.coach)
        .await
        .unwrap();
    assert!(
        got.is_empty(),
        "a coach detached from every group coaches nobody, got {got:?}"
    );
}
