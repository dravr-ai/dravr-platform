// ABOUTME: Integration tests for GroupService::inject_group_context — the privacy gate
// ABOUTME: Covers group resolution, per-member consent filter, kill switch, and disambiguation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `inject_group_context` is the single function that makes a coach prompt
//! group-aware. The per-member consent filter, the `peer_data_sharing` kill
//! switch, the multi-group disambiguation branch, and the no-group / unknown-
//! group early returns all live here and decide which peers' fitness data
//! reaches the LLM — a cross-member data-leak boundary. The strategy renderers
//! below it are unit-tested in `pierre-groups/tests/per_member_consent_gate_test.rs`;
//! these tests exercise the service-level wiring against a real repository.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "tools-groups")]
mod inject_tests {
    use crate::common::create_test_server_resources;
    use chrono::Utc;
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupMember, GroupRespondMode, GroupRole, MemberFitnessSnapshot,
        OvertrainingRiskLevel,
    };
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use std::collections::HashMap;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    const BASE_PROMPT: &str = "You are a running coach.";

    async fn seed_user(resources: &ServerContext, label: &str) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            format!("{label}-{}@test.com", Uuid::new_v4()),
            password_hash,
            Some(format!("{label} User")),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: "Inject Test Tenant".to_owned(),
            slug: format!("inject-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();

        (user_id, tenant_id)
    }

    /// Create a bare active user (no tenant) for use as an additional group
    /// member. Both `coaching_groups.owner_id` and
    /// `coaching_group_members.user_id` are FKs to `users(id)`, so every
    /// participant must reference a real user row.
    async fn seed_bare_user(resources: &ServerContext, label: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("{label}-{}@test.com", Uuid::new_v4()),
            password_hash,
            Some(format!("{label} Peer")),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    async fn seed_coach(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) -> Uuid {
        resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant_id,
                &CreateSystemCoachRequest {
                    title: "Inject Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn create_group(
        resources: &ServerContext,
        tenant_id: TenantId,
        coach_id: Uuid,
        owner_id: Uuid,
        name: &str,
        peer_data_sharing: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            coach_id: coach_id.to_string(),
            owner_id,
            coach_user_id: None,
            peer_data_sharing,
            respond_mode: GroupRespondMode::default(),
            max_members: 20,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await
            .unwrap();
        id
    }

    async fn add_member(
        resources: &ServerContext,
        group_id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
        role: GroupRole,
        consent: bool,
    ) {
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role,
                peer_sharing_consent: consent,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    fn snapshot(user_id: Uuid, name: &str) -> MemberFitnessSnapshot {
        MemberFitnessSnapshot {
            user_id,
            display_name: name.to_owned(),
            ctl: Some(50.0),
            atl: Some(45.0),
            tsb: Some(5.0),
            weekly_volume_km: 30.0,
            previous_week_volume_km: Some(28.0),
            weekly_activity_count: 4,
            weekly_duration_seconds: 10_800,
            primary_sport: Some("run".to_owned()),
            vdot: Some(50.0),
            overtraining_risk: OvertrainingRiskLevel::Low,
            days_since_last_activity: Some(1),
            last_activity_per_provider: HashMap::new(),
            recent_activities: vec![],
            needs_reauth_providers: vec![],
            computed_at: Utc::now(),
        }
    }

    /// No conversation group + the user belongs to no group with this coach →
    /// the base prompt is returned verbatim (fast early return, no injection).
    #[tokio::test]
    async fn inject_returns_base_prompt_when_user_in_no_group() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = seed_user(&resources, "solo").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id).await;

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                user_id,
                tenant_id,
                None,
                &[],
            )
            .await
            .unwrap();

        assert_eq!(
            out, BASE_PROMPT,
            "no group with this coach must leave the prompt untouched"
        );
    }

    /// A `conversation_group_id` that resolves to no group (stale/unknown id)
    /// also returns the base prompt unchanged — guards the `let Some(group)
    /// else` early return on the fast path.
    #[tokio::test]
    async fn inject_returns_base_when_conversation_group_not_found() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = seed_user(&resources, "ghost").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id).await;
        let unknown_group = Uuid::new_v4().to_string();

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                user_id,
                tenant_id,
                Some(&unknown_group),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(out, BASE_PROMPT);
    }

    /// User in two groups with the same coach (no conversation group selected)
    /// → the prompt is augmented with a disambiguation note naming both groups
    /// instead of guessing.
    #[tokio::test]
    async fn inject_appends_disambiguation_for_multiple_groups() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = seed_user(&resources, "multi").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id).await;

        let g1 = create_group(&resources, tenant_id, coach_id, user_id, "Trail Crew", true).await;
        let g2 = create_group(&resources, tenant_id, coach_id, user_id, "Track Club", true).await;
        add_member(&resources, g1, user_id, tenant_id, GroupRole::Member, true).await;
        add_member(&resources, g2, user_id, tenant_id, GroupRole::Member, true).await;

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                user_id,
                tenant_id,
                None,
                &[],
            )
            .await
            .unwrap();

        assert!(
            out.starts_with(BASE_PROMPT),
            "base prompt must be preserved"
        );
        assert!(
            out.contains("multiple groups"),
            "expected disambiguation note, got: {out}"
        );
        assert!(out.contains("Trail Crew") && out.contains("Track Club"));
    }

    /// `peer_data_sharing = true`: the requester's own snapshot plus consenting
    /// peers surface; a peer who has NOT set `peer_sharing_consent` is filtered
    /// out before any card is built.
    #[tokio::test]
    async fn inject_consenting_peer_visible_non_consenting_hidden() {
        let resources = create_test_server_resources().await.unwrap();
        let (requester, tenant_id) = seed_user(&resources, "req").await;
        let coach_id = seed_coach(&resources, requester, tenant_id).await;
        // The requester owns the group row but joins as a plain Member so they
        // get the individual-focus context view (which renders peer cards).
        // Peers are real users (members.user_id is a FK to users).
        let peer_yes = seed_bare_user(&resources, "peeryes").await;
        let peer_no = seed_bare_user(&resources, "peerno").await;

        let gid = create_group(
            &resources,
            tenant_id,
            coach_id,
            requester,
            "Tempo Squad",
            true,
        )
        .await;
        add_member(
            &resources,
            gid,
            requester,
            tenant_id,
            GroupRole::Member,
            false,
        )
        .await;
        add_member(
            &resources,
            gid,
            peer_yes,
            tenant_id,
            GroupRole::Member,
            true,
        )
        .await;
        add_member(
            &resources,
            gid,
            peer_no,
            tenant_id,
            GroupRole::Member,
            false,
        )
        .await;

        let snapshots = vec![
            snapshot(requester, "RequesterRunner"),
            snapshot(peer_yes, "PeerAlice"),
            snapshot(peer_no, "PeerBob"),
        ];

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                requester,
                tenant_id,
                Some(&gid.to_string()),
                &snapshots,
            )
            .await
            .unwrap();

        assert!(out.starts_with(BASE_PROMPT));
        assert!(
            out.contains("PeerAlice"),
            "consenting peer must surface, got: {out}"
        );
        assert!(
            !out.contains("PeerBob"),
            "non-consenting peer must be filtered out, got: {out}"
        );
    }

    /// A visible member whose provider connection died surfaces a "Connection alerts"
    /// section naming the dead provider, so the coach reports it instead of fabricating
    /// data for a disconnected source.
    #[tokio::test]
    async fn inject_surfaces_needs_reauth_for_visible_member() {
        let resources = create_test_server_resources().await.unwrap();
        let (requester, tenant_id) = seed_user(&resources, "reauthreq").await;
        let coach_id = seed_coach(&resources, requester, tenant_id).await;
        let peer = seed_bare_user(&resources, "reauthpeer").await;

        let gid = create_group(
            &resources,
            tenant_id,
            coach_id,
            requester,
            "Recovery Squad",
            true,
        )
        .await;
        add_member(
            &resources,
            gid,
            requester,
            tenant_id,
            GroupRole::Member,
            false,
        )
        .await;
        add_member(&resources, gid, peer, tenant_id, GroupRole::Member, true).await;

        let mut peer_snapshot = snapshot(peer, "PhilPeer");
        peer_snapshot.needs_reauth_providers = vec!["whoop".to_owned()];
        let snapshots = vec![snapshot(requester, "RequesterRunner"), peer_snapshot];

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                requester,
                tenant_id,
                Some(&gid.to_string()),
                &snapshots,
            )
            .await
            .unwrap();

        assert!(
            out.contains("Connection alerts"),
            "expected a connection-alerts section, got: {out}"
        );
        assert!(
            out.contains("PhilPeer needs to reconnect: whoop"),
            "alert must name the member and the dead provider, got: {out}"
        );
    }

    /// `peer_data_sharing = false` is the admin kill switch: even a peer who
    /// individually consented is hidden — only the requester's own data leaks.
    #[tokio::test]
    async fn inject_kill_switch_hides_all_peers_despite_consent() {
        let resources = create_test_server_resources().await.unwrap();
        let (requester, tenant_id) = seed_user(&resources, "kill").await;
        let coach_id = seed_coach(&resources, requester, tenant_id).await;
        let peer_yes = seed_bare_user(&resources, "peeryes").await;

        // Kill switch OFF (peer_data_sharing = false) despite the peer consenting.
        let gid = create_group(
            &resources,
            tenant_id,
            coach_id,
            requester,
            "Locked Group",
            false,
        )
        .await;
        add_member(
            &resources,
            gid,
            requester,
            tenant_id,
            GroupRole::Member,
            false,
        )
        .await;
        add_member(
            &resources,
            gid,
            peer_yes,
            tenant_id,
            GroupRole::Member,
            true,
        )
        .await;

        let snapshots = vec![
            snapshot(requester, "RequesterRunner"),
            snapshot(peer_yes, "PeerAlice"),
        ];

        let out = resources
            .group_service()
            .inject_group_context(
                BASE_PROMPT,
                &coach_id.to_string(),
                requester,
                tenant_id,
                Some(&gid.to_string()),
                &snapshots,
            )
            .await
            .unwrap();

        assert!(
            !out.contains("PeerAlice"),
            "kill switch must hide a consenting peer's data, got: {out}"
        );
    }
}
