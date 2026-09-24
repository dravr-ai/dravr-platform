// ABOUTME: Repository-level tests for DelegatedConnectionRepository — every method, with the values each row decodes to
// ABOUTME: Runs on whichever driver the test factory selects, so the same assertions prove SQLite and Postgres
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `delegated_connections` round trips below the HTTP layer.
//!
//! A link ties an athlete on a coach's TrainingPeaks roster to a live member
//! of a group the coach coaches. These tests read back every column a link
//! carries, pin the live-row uniqueness the partial indexes give, and prove the
//! member's read path fails closed on each way the group relation can end —
//! so a decode or a join that differs between the two backends fails here.

mod common;

use chrono::{Duration, Utc};
use common::{create_test_database, create_test_user_with_plan};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::ErrorCode;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection, DelegationEndReason,
    DelegationStatus, TenantId,
};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use std::sync::Arc;
use uuid::Uuid;

const PROVIDER: &str = oauth_providers::SCIOTTE_TRAININGPEAKS;

struct Fixture {
    db: Arc<Database>,
    repos: RepositoryRegistry,
    owner: Uuid,
    owner_tenant: TenantId,
    coach: Uuid,
    coach_tenant: TenantId,
    agent_id: String,
}

/// A group owner (on a plan with groups), a human coach in their own tenant,
/// and an agent persona the groups run.
async fn fixture() -> Fixture {
    let db = create_test_database().await.unwrap();
    let (owner, _, owner_tenant) =
        create_test_user_with_plan(&db, "owner@delegation.test", "professional")
            .await
            .unwrap();
    let (coach, _, coach_tenant) =
        create_test_user_with_plan(&db, "coach@delegation.test", "starter")
            .await
            .unwrap();
    let repos = db.repositories();
    let agent_id = repos
        .agents
        .create(
            owner,
            owner_tenant,
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
        .to_string();
    Fixture {
        db,
        repos,
        owner,
        owner_tenant,
        coach,
        coach_tenant,
        agent_id,
    }
}

impl Fixture {
    /// An active group the fixture's coach coaches.
    async fn group(&self, name: &str) -> Uuid {
        let now = Utc::now();
        let group = self
            .repos
            .groups
            .create_group(
                self.owner_tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: self.owner_tenant.to_string(),
                    name: name.to_owned(),
                    description: None,
                    agent_id: self.agent_id.clone(),
                    owner_id: self.owner,
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
        assert!(self
            .repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(self.coach), self.owner_tenant)
            .await
            .unwrap());
        group.id
    }

    /// A user with a tenant of their own, joined to `group` as a member.
    async fn member(&self, email: &str, group: Uuid) -> (Uuid, TenantId) {
        let (user, _, tenant) = create_test_user_with_plan(&self.db, email, "starter")
            .await
            .unwrap();
        self.join(user, tenant, group).await;
        (user, tenant)
    }

    /// `user` joins `group` as a member.
    async fn join(&self, user: Uuid, tenant: TenantId, group: Uuid) {
        let now = Utc::now();
        self.repos
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

    fn proposal(&self, group: Uuid, member: Uuid, athlete: &str) -> DelegatedConnection {
        DelegatedConnection::propose(
            PROVIDER.to_owned(),
            group,
            self.coach,
            self.coach_tenant,
            member,
            athlete.to_owned(),
            Some(format!("Athlete {athlete}")),
        )
    }

    async fn proposed(&self, group: Uuid, member: Uuid, athlete: &str) -> DelegatedConnection {
        self.repos
            .delegated_connections
            .propose(&self.proposal(group, member, athlete))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet")
    }

    async fn confirmed(
        &self,
        group: Uuid,
        member: Uuid,
        member_tenant: TenantId,
        athlete: &str,
    ) -> DelegatedConnection {
        let link = self.proposed(group, member, athlete).await;
        self.repos
            .delegated_connections
            .confirm(link.id, member, member_tenant, Utc::now())
            .await
            .unwrap()
            .expect("the member confirms their own proposal")
    }
}

#[tokio::test]
async fn a_proposal_round_trips_every_column_to_both_participants() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, _) = f.member("m1@delegation.test", group).await;
    let (stranger, _) = f.member("stranger@delegation.test", group).await;
    let mut wanted = f.proposal(group, member, "900001");
    wanted.proposed_at = Utc::now() - Duration::minutes(5);
    let repo = &f.repos.delegated_connections;

    let stored = repo.propose(&wanted).await.unwrap().expect("stored");
    assert_eq!(stored.id, wanted.id);
    assert_eq!(stored.provider, PROVIDER);
    assert_eq!(stored.group_id, group);
    assert_eq!(stored.coach_user_id, f.coach);
    assert_eq!(stored.coach_tenant_id, f.coach_tenant);
    assert_eq!(stored.member_user_id, member);
    assert_eq!(stored.member_tenant_id, None);
    assert_eq!(stored.provider_athlete_id, "900001");
    assert_eq!(
        stored.provider_athlete_name.as_deref(),
        Some("Athlete 900001")
    );
    assert_eq!(stored.status, DelegationStatus::Proposed);
    assert_eq!(
        stored.proposed_at.timestamp_micros(),
        wanted.proposed_at.timestamp_micros(),
        "proposed_at survives the round trip to the microsecond"
    );
    assert_eq!(stored.confirmed_at, None);
    assert_eq!(stored.revoked_at, None);
    assert_eq!(stored.revoked_by, None);
    assert_eq!(stored.revoke_reason, None);

    for participant in [f.coach, member] {
        let read = repo
            .get_for_participant(wanted.id, group, participant)
            .await
            .unwrap()
            .expect("both participants read the link");
        assert_eq!(read, stored);
    }
    assert!(repo
        .get_for_participant(wanted.id, group, stranger)
        .await
        .unwrap()
        .is_none());
    let elsewhere = f.group("Elsewhere").await;
    assert!(repo
        .get_for_participant(wanted.id, elsewhere, f.coach)
        .await
        .unwrap()
        .is_none());

    let for_coach = repo
        .list_live_for_coach_in_group(group, f.coach)
        .await
        .unwrap();
    assert_eq!(for_coach, vec![stored.clone()]);
    let for_member = repo
        .list_live_for_member_in_group(group, member)
        .await
        .unwrap();
    assert_eq!(for_member, vec![stored.clone()]);
    assert_eq!(
        repo.list_live_for_member(member).await.unwrap(),
        vec![stored]
    );
    assert!(repo
        .list_live_for_member_in_group(group, stranger)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn one_live_link_per_member_in_a_group_and_per_coach_athlete() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (m1, _) = f.member("m1@delegation.test", group).await;
    let (m2, _) = f.member("m2@delegation.test", group).await;
    let repo = &f.repos.delegated_connections;
    f.proposed(group, m1, "900001").await;

    assert!(
        repo.propose(&f.proposal(group, m1, "900002"))
            .await
            .unwrap()
            .is_none(),
        "a second live link for the same member, group and provider"
    );
    assert!(
        repo.propose(&f.proposal(group, m2, "900001"))
            .await
            .unwrap()
            .is_none(),
        "the coach's athlete is already linked to someone"
    );
    assert!(repo
        .propose(&f.proposal(group, m2, "900002"))
        .await
        .unwrap()
        .is_some());

    let mut not_fresh = f.proposal(group, m2, "900003");
    not_fresh.status = DelegationStatus::Confirmed;
    let err = repo.propose(&not_fresh).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // Once ended, the pair is free again: the indexes cover live rows only.
    let live = repo.list_live_for_member_in_group(group, m1).await.unwrap();
    repo.end_one(
        live[0].id,
        f.coach,
        Some(f.coach),
        DelegationEndReason::Withdrawn,
        Utc::now(),
    )
    .await
    .unwrap()
    .expect("the coach withdraws a live proposal");
    assert!(repo
        .propose(&f.proposal(group, m1, "900001"))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn confirm_moves_only_the_members_own_proposal() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let other_group = f.group("Other").await;
    let (member, member_tenant) = f.member("m1@delegation.test", group).await;
    let (other, other_tenant) = f.member("m2@delegation.test", group).await;
    let now = Utc::now();
    f.repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: other_group,
            user_id: member,
            tenant_id: member_tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
    let repo = &f.repos.delegated_connections;
    let link = f.proposed(group, member, "900001").await;

    assert!(
        repo.confirm(link.id, other, other_tenant, now)
            .await
            .unwrap()
            .is_none(),
        "only the member the link names can confirm it"
    );
    let confirmed = repo
        .confirm(link.id, member, member_tenant, now)
        .await
        .unwrap()
        .expect("the member confirms");
    assert_eq!(confirmed.status, DelegationStatus::Confirmed);
    assert_eq!(confirmed.member_tenant_id, Some(member_tenant));
    assert_eq!(
        confirmed.confirmed_at.map(|t| t.timestamp_micros()),
        Some(now.timestamp_micros())
    );
    assert_eq!(confirmed.proposed_at, link.proposed_at);
    assert!(
        repo.confirm(link.id, member, member_tenant, now)
            .await
            .unwrap()
            .is_none(),
        "a confirmed link is not a proposal any more"
    );

    let second = f.proposed(other_group, member, "900002").await;
    let err = repo
        .confirm(second.id, member, member_tenant, now)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ResourceAlreadyExists);
    assert_eq!(
        err.details.as_deref().and_then(|d| d["reason"].as_str()),
        Some("already_linked")
    );
    let still = repo
        .get_for_participant(second.id, other_group, member)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, DelegationStatus::Proposed);

    let live = repo.list_live_for_member(member).await.unwrap();
    assert_eq!(
        live.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![confirmed.id, second.id],
        "the confirmed link first, then proposals"
    );
}

#[tokio::test]
async fn the_read_path_finds_a_confirmed_link_only_while_the_group_relation_holds() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;

    // Baseline, and the keys the read is scoped by.
    let group = f.group("Baseline").await;
    let (member, member_tenant) = f.member("base@delegation.test", group).await;
    let link = f.confirmed(group, member, member_tenant, "900001").await;
    assert_eq!(
        repo.find_active_for_member(member, member_tenant, PROVIDER)
            .await
            .unwrap(),
        Some(link)
    );
    assert!(
        repo.find_active_for_member(member, f.owner_tenant, PROVIDER)
            .await
            .unwrap()
            .is_none(),
        "another tenant of the member's"
    );
    assert!(repo
        .find_active_for_member(member, member_tenant, oauth_providers::STRAVA)
        .await
        .unwrap()
        .is_none());

    // The member left the group.
    let group = f.group("Left").await;
    let (member, member_tenant) = f.member("left@delegation.test", group).await;
    f.confirmed(group, member, member_tenant, "900002").await;
    assert!(f
        .repos
        .groups
        .remove_member(&group.to_string(), member)
        .await
        .unwrap());
    assert!(repo
        .find_active_for_member(member, member_tenant, PROVIDER)
        .await
        .unwrap()
        .is_none());

    // The group was archived.
    let group = f.group("Archived").await;
    let (member, member_tenant) = f.member("archived@delegation.test", group).await;
    f.confirmed(group, member, member_tenant, "900003").await;
    f.repos
        .groups
        .update_group(
            &group.to_string(),
            f.owner_tenant,
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
        .unwrap();
    assert!(repo
        .find_active_for_member(member, member_tenant, PROVIDER)
        .await
        .unwrap()
        .is_none());

    // The group's coach was replaced.
    let group = f.group("Replaced").await;
    let (member, member_tenant) = f.member("replaced@delegation.test", group).await;
    f.confirmed(group, member, member_tenant, "900004").await;
    assert!(f
        .repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(f.owner), f.owner_tenant)
        .await
        .unwrap());
    assert!(repo
        .find_active_for_member(member, member_tenant, PROVIDER)
        .await
        .unwrap()
        .is_none());

    // The link itself was revoked.
    let group = f.group("Revoked").await;
    let (member, member_tenant) = f.member("revoked@delegation.test", group).await;
    let link = f.confirmed(group, member, member_tenant, "900005").await;
    repo.end_one(
        link.id,
        member,
        Some(member),
        DelegationEndReason::RevokedByMember,
        Utc::now(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(repo
        .find_active_for_member(member, member_tenant, PROVIDER)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_member_is_shown_only_the_links_the_group_relation_backs() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;
    let squad = f.group("Squad").await;
    let (member, member_tenant) = f.member("backed@delegation.test", squad).await;
    let confirmed = f.confirmed(squad, member, member_tenant, "900001").await;
    let asked_in = f.group("Asked").await;
    f.join(member, member_tenant, asked_in).await;
    let asked = f.proposed(asked_in, member, "900002").await;

    // Three more proposals whose relation ends without any end recorded.
    let deactivated = f.group("Deactivated").await;
    let left = f.group("Left").await;
    let replaced = f.group("Replaced").await;
    for (group, athlete) in [
        (deactivated, "900003"),
        (left, "900004"),
        (replaced, "900005"),
    ] {
        f.join(member, member_tenant, group).await;
        f.proposed(group, member, athlete).await;
    }
    f.repos
        .groups
        .update_group(
            &deactivated.to_string(),
            f.owner_tenant,
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
        .unwrap();
    assert!(f
        .repos
        .groups
        .remove_member(&left.to_string(), member)
        .await
        .unwrap());
    assert!(f
        .repos
        .groups
        .set_group_coach_user(&replaced.to_string(), Some(f.owner), f.owner_tenant)
        .await
        .unwrap());

    assert_eq!(repo.list_live_for_member(member).await.unwrap().len(), 5);
    let backed: Vec<Uuid> = repo
        .list_backed_for_member(member, PROVIDER)
        .await
        .unwrap()
        .iter()
        .map(|link| link.id)
        .collect();
    assert_eq!(backed, [confirmed.id, asked.id], "the confirmed link first");
    assert!(repo
        .list_backed_for_member(member, oauth_providers::STRAVA)
        .await
        .unwrap()
        .is_empty());

    // The confirmed link the list leads with is the one the read path finds.
    assert_eq!(
        repo.find_active_for_member(member, member_tenant, PROVIDER)
            .await
            .unwrap()
            .map(|link| link.id),
        Some(confirmed.id)
    );
}

#[tokio::test]
async fn ends_return_exactly_the_rows_they_ended() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;
    let group = f.group("Squad").await;
    let (m1, t1) = f.member("m1@delegation.test", group).await;
    let (m2, _) = f.member("m2@delegation.test", group).await;
    let (m3, t3) = f.member("m3@delegation.test", group).await;
    let confirmed = f.confirmed(group, m1, t1, "900001").await;
    let proposed = f.proposed(group, m2, "900002").await;
    let now = Utc::now();

    // One participant ends one link; a stranger cannot.
    assert!(repo
        .end_one(
            proposed.id,
            m3,
            Some(m3),
            DelegationEndReason::Declined,
            now
        )
        .await
        .unwrap()
        .is_none());

    // A member leaving ends their links in that group, and only theirs.
    let ended = repo
        .end_for_group_member(group, m1, Some(m1), DelegationEndReason::MemberLeft, now)
        .await
        .unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, confirmed.id);
    assert_eq!(ended[0].status, DelegationStatus::Revoked);
    assert_eq!(
        ended[0].revoke_reason,
        Some(DelegationEndReason::MemberLeft)
    );
    assert_eq!(ended[0].revoked_by, Some(m1));
    assert_eq!(
        ended[0].revoked_at.map(|t| t.timestamp_micros()),
        Some(now.timestamp_micros())
    );
    assert_eq!(
        ended[0].confirmed_at, confirmed.confirmed_at,
        "a revoked row keeps when it had been confirmed"
    );
    assert_eq!(ended[0].member_tenant_id, Some(t1));
    assert!(
        repo.end_for_group_member(group, m1, None, DelegationEndReason::MemberLeft, now)
            .await
            .unwrap()
            .is_empty(),
        "a revoked row is terminal"
    );

    // Archiving ends every live link of the group.
    let other = f.confirmed(group, m3, t3, "900003").await;
    let ended = repo
        .end_for_group(group, None, DelegationEndReason::GroupArchived, now)
        .await
        .unwrap();
    let mut ids: Vec<Uuid> = ended.iter().map(|l| l.id).collect();
    ids.sort();
    let mut wanted = vec![proposed.id, other.id];
    wanted.sort();
    assert_eq!(ids, wanted);
    assert!(ended.iter().all(
        |l| l.revoke_reason == Some(DelegationEndReason::GroupArchived) && l.revoked_by.is_none()
    ));
    let ended_proposal = ended.iter().find(|l| l.id == proposed.id).unwrap();
    assert_eq!(
        ended_proposal.confirmed_at, None,
        "a proposal ended before confirming reads as never confirmed"
    );
    assert!(repo
        .list_live_for_coach_in_group(group, f.coach)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_coach_disconnect_ends_the_links_its_session_serves_and_no_others() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;
    let group = f.group("Squad").await;
    let (m1, t1) = f.member("m1@delegation.test", group).await;
    let (m2, _) = f.member("m2@delegation.test", group).await;
    let served = f.confirmed(group, m1, t1, "900001").await;
    // Proposed while the coach was active in another tenant: another session.
    let mut elsewhere = f.proposal(group, m2, "900002");
    elsewhere.coach_tenant_id = f.owner_tenant;
    let elsewhere = repo.propose(&elsewhere).await.unwrap().unwrap();

    let ended = repo
        .end_for_coach(
            f.coach,
            f.coach_tenant,
            PROVIDER,
            Some(f.coach),
            DelegationEndReason::CoachDisconnected,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, served.id);
    assert_eq!(
        ended[0].revoke_reason,
        Some(DelegationEndReason::CoachDisconnected)
    );
    assert_eq!(ended[0].confirmed_at, served.confirmed_at);
    assert_eq!(
        repo.list_live_for_coach_in_group(group, f.coach)
            .await
            .unwrap()
            .iter()
            .map(|l| l.id)
            .collect::<Vec<_>>(),
        vec![elsewhere.id]
    );
}

#[tokio::test]
async fn the_member_ends_their_confirmed_link_and_keeps_their_proposals() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;
    let group = f.group("Squad").await;
    let other_group = f.group("Other").await;
    let (member, tenant) = f.member("m1@delegation.test", group).await;
    let now = Utc::now();
    f.repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: other_group,
            user_id: member,
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
    let confirmed = f.confirmed(group, member, tenant, "900001").await;
    let pending = f.proposed(other_group, member, "900002").await;

    assert!(
        repo.end_confirmed_for_member(
            member,
            f.owner_tenant,
            PROVIDER,
            Some(member),
            DelegationEndReason::RevokedByMember,
            now,
        )
        .await
        .unwrap()
        .is_empty(),
        "scoped to the tenant the link was confirmed in"
    );
    let ended = repo
        .end_confirmed_for_member(
            member,
            tenant,
            PROVIDER,
            Some(member),
            DelegationEndReason::Superseded,
            now,
        )
        .await
        .unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, confirmed.id);
    assert_eq!(
        ended[0].revoke_reason,
        Some(DelegationEndReason::Superseded)
    );
    assert_eq!(
        repo.list_live_for_member(member)
            .await
            .unwrap()
            .iter()
            .map(|l| l.id)
            .collect::<Vec<_>>(),
        vec![pending.id]
    );
}

#[tokio::test]
async fn deleting_the_member_takes_their_links_with_them() {
    let f = fixture().await;
    let repo = &f.repos.delegated_connections;
    let group = f.group("Squad").await;
    let (member, tenant) = f.member("gone@delegation.test", group).await;
    let link = f.confirmed(group, member, tenant, "900001").await;

    f.repos.users.delete(member).await.unwrap();

    assert!(repo
        .get_for_participant(link.id, group, f.coach)
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .list_live_for_coach_in_group(group, f.coach)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removing_a_delegated_connection_never_removes_an_own_one() {
    let f = fixture().await;
    let connections = &f.repos.provider_connections;
    let (member, tenant) = (f.coach, f.coach_tenant);

    connections
        .register_connection(member, tenant, PROVIDER, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    assert!(
        !connections
            .remove_delegated_connection(member, tenant, PROVIDER)
            .await
            .unwrap(),
        "the member's own login is not a delegated row"
    );
    assert_eq!(
        connections
            .get_for_user(member, Some(tenant))
            .await
            .unwrap()
            .len(),
        1
    );

    connections
        .register_connection(
            member,
            tenant,
            PROVIDER,
            &ConnectionType::Delegated,
            Some(r#"{"delegated_connection_id":"x"}"#),
        )
        .await
        .unwrap();
    let rows = connections
        .get_for_user(member, Some(tenant))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].connection_type, ConnectionType::Delegated);
    assert!(connections
        .remove_delegated_connection(member, tenant, PROVIDER)
        .await
        .unwrap());
    assert!(connections
        .get_for_user(member, Some(tenant))
        .await
        .unwrap()
        .is_empty());
}
