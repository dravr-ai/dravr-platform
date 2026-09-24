// ABOUTME: Every group lifecycle event and every TrainingPeaks disconnect ends the delegated links it takes away
// ABOUTME: Driven through the server's own GroupService and disconnect chokepoint, reading back each ended row and its release
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A delegated connection reads a member's TrainingPeaks workouts through
//! their group coach's session. It holds only while the group relation and
//! both sides' TrainingPeaks accounts hold, so each way one of them ends
//! must end the link too: the member leaving or being removed, the group
//! archived, the coach detached, the member or the coach disconnecting
//! TrainingPeaks.
//!
//! A confirmed link also left two traces — the member's `delegated`
//! provider connection and the workouts read through it — and ending it
//! releases both. A link only ever proposed left nothing to release. These
//! tests go through the `GroupService` the server wires and the disconnect
//! the app calls, so the wiring is under test as well as the store.

mod common;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::{
    ActivityBuilder, AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection,
    DelegationEndReason, DelegationStatus, SportType, TenantId, UserOAuthToken,
};
use pierre_groups::delegation::{DelegationStore, UnbackedLink};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::sciotte_remote::CoachedAthlete;
use pierre_services::delegated_connections::roster_cache_key;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use uuid::Uuid;

const PROVIDER: &str = oauth_providers::SCIOTTE_TRAININGPEAKS;

/// A group owner, a human coach, and the agent persona their groups run.
struct World {
    res: Arc<ServerContext>,
    owner: Uuid,
    owner_tenant: TenantId,
    coach: Uuid,
    coach_tenant: TenantId,
    agent_id: String,
}

async fn world() -> World {
    let res = create_test_server_resources().await.unwrap();
    let db = &res.agent.database;
    let (owner, _, owner_tenant) =
        create_test_user_with_plan(db, "owner@lifecycle.test", "professional")
            .await
            .unwrap();
    let (coach, _, coach_tenant) =
        create_test_user_with_plan(db, "coach@lifecycle.test", "starter")
            .await
            .unwrap();
    let agent_id = res
        .common
        .repos
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
    World {
        res,
        owner,
        owner_tenant,
        coach,
        coach_tenant,
        agent_id,
    }
}

impl World {
    /// An active group the world's coach coaches.
    async fn group(&self, name: &str) -> Uuid {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        let group = repos
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
        assert!(repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(self.coach), self.owner_tenant)
            .await
            .unwrap());
        group.id
    }

    /// A user with a tenant of their own, joined to `group` as a member.
    async fn member(&self, email: &str, group: Uuid) -> (Uuid, TenantId) {
        let (user, _, tenant) =
            create_test_user_with_plan(&self.res.agent.database, email, "starter")
                .await
                .unwrap();
        let now = Utc::now();
        self.res
            .common
            .repos
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
        (user, tenant)
    }

    async fn proposed(&self, group: Uuid, member: Uuid, athlete: &str) -> DelegatedConnection {
        self.res
            .common
            .repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                PROVIDER.to_owned(),
                group,
                self.coach,
                self.coach_tenant,
                member,
                athlete.to_owned(),
                Some(format!("Athlete {athlete}")),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet")
    }

    /// A confirmed link with the traces a confirm leaves: the member's
    /// delegated connection, and one workout read through it.
    async fn confirmed(
        &self,
        group: Uuid,
        member: Uuid,
        member_tenant: TenantId,
        athlete: &str,
    ) -> DelegatedConnection {
        let repos = &self.res.common.repos;
        let link = self.proposed(group, member, athlete).await;
        let link = repos
            .delegated_connections
            .confirm(link.id, member, member_tenant, Utc::now())
            .await
            .unwrap()
            .expect("the member confirms their own proposal");
        repos
            .provider_connections
            .register_connection(
                member,
                member_tenant,
                PROVIDER,
                &ConnectionType::Delegated,
                Some(&format!(r#"{{"delegated_connection_id":"{}"}}"#, link.id)),
            )
            .await
            .unwrap();
        let workout = ActivityBuilder::new(
            format!("{athlete}:7001"),
            "Tempo run".to_owned(),
            SportType::Run,
            Utc::now() - Duration::days(1),
            2_700,
            "sciotte".to_owned(),
        )
        .build();
        repos
            .activity_cache
            .upsert_activities(member, &member_tenant, PROVIDER, &[workout])
            .await
            .unwrap();
        assert!(self.has_delegated_connection(member, member_tenant).await);
        assert_eq!(self.cached_rows(member, member_tenant).await, 1);
        link
    }

    /// The coach's own TrainingPeaks session and connection, in their tenant.
    async fn coach_connected(&self) {
        let repos = &self.res.common.repos;
        let now = Utc::now();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: self.coach,
                tenant_id: self.coach_tenant.to_string(),
                provider: PROVIDER.to_owned(),
                access_token: r#"{"session_id":"coach-session","cookies":[]}"#.to_owned(),
                refresh_token: None,
                token_type: TOKEN_TYPE_SESSION.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: None,
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                self.coach,
                self.coach_tenant,
                PROVIDER,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }

    /// The link as it is stored now, read as its coach.
    async fn reread(&self, link: &DelegatedConnection) -> DelegatedConnection {
        self.res
            .common
            .repos
            .delegated_connections
            .get_for_participant(link.id, link.group_id, self.coach)
            .await
            .unwrap()
            .expect("an ended link stays for audit")
    }

    async fn has_delegated_connection(&self, member: Uuid, tenant: TenantId) -> bool {
        self.res
            .common
            .repos
            .provider_connections
            .get_for_user(member, Some(tenant))
            .await
            .unwrap()
            .iter()
            .any(|c| c.provider == PROVIDER && c.connection_type == ConnectionType::Delegated)
    }

    async fn cached_rows(&self, member: Uuid, tenant: TenantId) -> usize {
        self.res
            .common
            .repos
            .activity_cache
            .get_cached_activities(
                member,
                &tenant,
                Some(PROVIDER),
                Utc::now() - Duration::days(30),
                Utc::now(),
                100,
            )
            .await
            .unwrap()
            .len()
    }

    async fn disconnect(&self, user: Uuid, tenant: TenantId) {
        OAuthService::new(self.res.data(), self.res.common.config.clone())
            .disconnect_provider(
                user,
                oauth_providers::TRAININGPEAKS,
                Some(tenant.as_uuid()),
                DisconnectReason::Athlete,
            )
            .await
            .expect("a TrainingPeaks disconnect succeeds");
    }
}

/// Assert `link` ended for `reason` by `actor`, keeping when it had been
/// confirmed.
fn assert_ended(
    link: &DelegatedConnection,
    reason: DelegationEndReason,
    actor: Option<Uuid>,
    was_confirmed: bool,
) {
    assert_eq!(link.status, DelegationStatus::Revoked);
    assert_eq!(link.revoke_reason, Some(reason));
    assert_eq!(link.revoked_by, actor);
    assert!(link.revoked_at.is_some());
    assert_eq!(link.confirmed_at.is_some(), was_confirmed);
}

#[tokio::test]
async fn a_member_leaving_ends_their_link_and_releases_what_it_read() {
    let w = world().await;
    let group = w.group("Squad").await;
    let (leaver, leaver_tenant) = w.member("leaver@lifecycle.test", group).await;
    let (stayer, _) = w.member("stayer@lifecycle.test", group).await;
    let leaving = w.confirmed(group, leaver, leaver_tenant, "900001").await;
    let staying = w.proposed(group, stayer, "900002").await;

    let left = w
        .res
        .common
        .group_service
        .leave_group(&group.to_string(), leaver)
        .await
        .unwrap();
    assert!(left);

    assert_ended(
        &w.reread(&leaving).await,
        DelegationEndReason::MemberLeft,
        Some(leaver),
        true,
    );
    assert!(!w.has_delegated_connection(leaver, leaver_tenant).await);
    assert_eq!(w.cached_rows(leaver, leaver_tenant).await, 0);
    // Another member's link is not the leaver's to end.
    assert_eq!(w.reread(&staying).await.status, DelegationStatus::Proposed);
}

#[tokio::test]
async fn an_admin_removing_a_member_is_recorded_as_the_one_who_ended_it() {
    let w = world().await;
    let group = w.group("Squad").await;
    let (member, member_tenant) = w.member("removed@lifecycle.test", group).await;
    let link = w.confirmed(group, member, member_tenant, "900001").await;

    let removed = w
        .res
        .common
        .group_service
        .remove_member(&group.to_string(), member, w.owner)
        .await
        .unwrap();
    assert!(removed);

    assert_ended(
        &w.reread(&link).await,
        DelegationEndReason::MemberRemoved,
        Some(w.owner),
        true,
    );
    assert!(!w.has_delegated_connection(member, member_tenant).await);
    assert_eq!(w.cached_rows(member, member_tenant).await, 0);
}

#[tokio::test]
async fn detaching_the_coach_ends_every_link_in_that_group_only() {
    let w = world().await;
    let squad = w.group("Squad").await;
    let other = w.group("Other squad").await;
    let (m1, m1_tenant) = w.member("m1@lifecycle.test", squad).await;
    let (m2, _) = w.member("m2@lifecycle.test", squad).await;
    let (m3, m3_tenant) = w.member("m3@lifecycle.test", other).await;
    let confirmed = w.confirmed(squad, m1, m1_tenant, "900001").await;
    let proposed = w.proposed(squad, m2, "900002").await;
    let elsewhere = w.confirmed(other, m3, m3_tenant, "900003").await;

    // Writing the coach the group already has changes nothing, so nothing ends.
    assert!(w
        .res
        .common
        .group_service
        .set_group_coach(&other.to_string(), Some(w.coach), w.owner_tenant, w.owner)
        .await
        .unwrap());
    assert_eq!(
        w.reread(&elsewhere).await.status,
        DelegationStatus::Confirmed
    );

    assert!(w
        .res
        .common
        .group_service
        .set_group_coach(&squad.to_string(), None, w.owner_tenant, w.owner)
        .await
        .unwrap());

    assert_ended(
        &w.reread(&confirmed).await,
        DelegationEndReason::CoachDetached,
        Some(w.owner),
        true,
    );
    assert_ended(
        &w.reread(&proposed).await,
        DelegationEndReason::CoachDetached,
        Some(w.owner),
        false,
    );
    assert!(!w.has_delegated_connection(m1, m1_tenant).await);
    assert_eq!(w.cached_rows(m1, m1_tenant).await, 0);
    // The coach still coaches the other group, and that link stands.
    assert_eq!(
        w.reread(&elsewhere).await.status,
        DelegationStatus::Confirmed
    );
    assert!(w.has_delegated_connection(m3, m3_tenant).await);
    assert_eq!(w.cached_rows(m3, m3_tenant).await, 1);
}

#[tokio::test]
async fn archiving_the_group_ends_its_links() {
    let w = world().await;
    let group = w.group("Squad").await;
    let (member, member_tenant) = w.member("archived@lifecycle.test", group).await;
    let link = w.confirmed(group, member, member_tenant, "900001").await;

    let archived = w
        .res
        .common
        .group_service
        .delete_group(&group.to_string(), w.owner_tenant, w.owner)
        .await
        .unwrap();
    assert!(archived);

    assert_ended(
        &w.reread(&link).await,
        DelegationEndReason::GroupArchived,
        Some(w.owner),
        true,
    );
    assert!(!w.has_delegated_connection(member, member_tenant).await);
    assert_eq!(w.cached_rows(member, member_tenant).await, 0);
}

/// A group update that sets only its active flag, which leaves its members
/// in place.
fn active(is_active: bool) -> UpdateGroupRequest {
    UpdateGroupRequest {
        name: None,
        description: None,
        agent_id: None,
        max_members: None,
        peer_data_sharing: None,
        respond_mode: None,
        digest_mode: None,
        is_active: Some(is_active),
    }
}

#[tokio::test]
async fn deactivating_the_group_ends_its_links_and_reactivating_revives_none() {
    let w = world().await;
    let group = w.group("Squad").await;
    let (member, member_tenant) = w.member("deactivated@lifecycle.test", group).await;
    let link = w.confirmed(group, member, member_tenant, "900001").await;
    let (asked, _) = w.member("asked@lifecycle.test", group).await;
    let proposal = w.proposed(group, asked, "900002").await;
    let service = &w.res.common.group_service;

    let deactivated = service
        .update_group(
            &group.to_string(),
            w.owner_tenant,
            &active(false),
            Some(w.owner),
        )
        .await
        .unwrap()
        .expect("the owner's tenant holds the group");
    assert!(!deactivated.is_active);

    assert_ended(
        &w.reread(&link).await,
        DelegationEndReason::GroupArchived,
        Some(w.owner),
        true,
    );
    assert_ended(
        &w.reread(&proposal).await,
        DelegationEndReason::GroupArchived,
        Some(w.owner),
        false,
    );
    assert!(!w.has_delegated_connection(member, member_tenant).await);
    assert_eq!(w.cached_rows(member, member_tenant).await, 0);

    // Active again, the group reads nothing through the ended link: the
    // member's consent ended with it.
    let reactivated = service
        .update_group(
            &group.to_string(),
            w.owner_tenant,
            &active(true),
            Some(w.owner),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(reactivated.is_active);
    assert_eq!(w.reread(&link).await.status, DelegationStatus::Revoked);
    assert!(w
        .res
        .common
        .repos
        .delegated_connections
        .find_active_for_member(member, member_tenant, PROVIDER)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_link_its_group_no_longer_backs_ends_for_what_broke() {
    let w = world().await;
    let repos = &w.res.common.repos;
    let store = DelegationStore::new(repos);

    // Each relation broken by its bare write, as if no lifecycle hook ran.
    let archived = w.group("Archived").await;
    let (archived_member, archived_tenant) = w.member("a@lifecycle.test", archived).await;
    let archived_link = w
        .confirmed(archived, archived_member, archived_tenant, "900001")
        .await;
    repos
        .groups
        .update_group(&archived.to_string(), w.owner_tenant, &active(false))
        .await
        .unwrap()
        .unwrap();

    let replaced = w.group("Replaced").await;
    let (replaced_member, replaced_tenant) = w.member("r@lifecycle.test", replaced).await;
    let replaced_link = w
        .confirmed(replaced, replaced_member, replaced_tenant, "900002")
        .await;
    assert!(repos
        .groups
        .set_group_coach_user(&replaced.to_string(), Some(w.owner), w.owner_tenant)
        .await
        .unwrap());

    let left = w.group("Left").await;
    let (left_member, left_tenant) = w.member("l@lifecycle.test", left).await;
    let left_link = w.confirmed(left, left_member, left_tenant, "900003").await;
    assert!(repos
        .groups
        .remove_member(&left.to_string(), left_member)
        .await
        .unwrap());

    let standing = w.group("Standing").await;
    let (standing_member, standing_tenant) = w.member("s@lifecycle.test", standing).await;
    let standing_link = w
        .confirmed(standing, standing_member, standing_tenant, "900004")
        .await;

    for (member, tenant, link, reason) in [
        (
            archived_member,
            archived_tenant,
            &archived_link,
            DelegationEndReason::GroupArchived,
        ),
        (
            replaced_member,
            replaced_tenant,
            &replaced_link,
            DelegationEndReason::CoachDetached,
        ),
        (
            left_member,
            left_tenant,
            &left_link,
            DelegationEndReason::MemberLeft,
        ),
    ] {
        let outcome = store
            .end_unbacked_for_member(member, tenant, PROVIDER)
            .await
            .unwrap();
        assert!(
            matches!(&outcome, UnbackedLink::Ended(ended) if ended.id == link.id),
            "{outcome:?}"
        );
        assert_ended(&w.reread(link).await, reason, None, true);
        assert!(!w.has_delegated_connection(member, tenant).await);
        assert_eq!(w.cached_rows(member, tenant).await, 0);
        // Ended already: nothing is left to end.
        assert!(matches!(
            store
                .end_unbacked_for_member(member, tenant, PROVIDER)
                .await
                .unwrap(),
            UnbackedLink::Gone
        ));
    }

    // A link its relation still backs is left as it is.
    assert!(matches!(
        store
            .end_unbacked_for_member(standing_member, standing_tenant, PROVIDER)
            .await
            .unwrap(),
        UnbackedLink::Backed
    ));
    assert_eq!(
        w.reread(&standing_link).await.status,
        DelegationStatus::Confirmed
    );
    assert!(
        w.has_delegated_connection(standing_member, standing_tenant)
            .await
    );
}

#[tokio::test]
async fn a_member_disconnecting_trainingpeaks_unlinks() {
    let w = world().await;
    let group = w.group("Squad").await;
    let (member, member_tenant) = w.member("unlinks@lifecycle.test", group).await;
    let link = w.confirmed(group, member, member_tenant, "900001").await;

    w.disconnect(member, member_tenant).await;

    assert_ended(
        &w.reread(&link).await,
        DelegationEndReason::RevokedByMember,
        Some(member),
        true,
    );
    assert!(!w.has_delegated_connection(member, member_tenant).await);
    assert_eq!(w.cached_rows(member, member_tenant).await, 0);
}

#[tokio::test]
async fn a_coach_disconnecting_trainingpeaks_ends_every_link_their_session_served() {
    let w = world().await;
    w.coach_connected().await;
    let squad = w.group("Squad").await;
    let other = w.group("Other squad").await;
    let (m1, m1_tenant) = w.member("m1@lifecycle.test", squad).await;
    let (m2, _) = w.member("m2@lifecycle.test", other).await;
    let confirmed = w.confirmed(squad, m1, m1_tenant, "900001").await;
    let proposed = w.proposed(other, m2, "900002").await;
    let roster_key = roster_cache_key(w.coach, w.coach_tenant);
    let roster = vec![CoachedAthlete {
        id: "900001".to_owned(),
        display_name: Some("Athlete 900001".to_owned()),
    }];
    let cache = &w.res.common.cache;
    cache
        .set(&roster_key, &roster, StdDuration::from_mins(10))
        .await
        .unwrap();

    w.disconnect(w.coach, w.coach_tenant).await;

    assert!(
        cache
            .get::<Vec<CoachedAthlete>>(&roster_key)
            .await
            .unwrap()
            .is_none(),
        "the roster read through the gone session goes with it"
    );

    assert_ended(
        &w.reread(&confirmed).await,
        DelegationEndReason::CoachDisconnected,
        Some(w.coach),
        true,
    );
    assert_ended(
        &w.reread(&proposed).await,
        DelegationEndReason::CoachDisconnected,
        Some(w.coach),
        false,
    );
    assert!(!w.has_delegated_connection(m1, m1_tenant).await);
    assert_eq!(w.cached_rows(m1, m1_tenant).await, 0);
}
