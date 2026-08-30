// ABOUTME: Tests for get_group_member_activities — consent-gated peer activity fetch (Fix B).
// ABOUTME: Covers consent grant, consent denial, kill-switch, unknown member, and tenant isolation.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_group_member_activities` is the only path that reads a *peer's* data in
//! a group chat (the single-user `get_activities` always runs as the requester).
//! Its authorization is security-critical: it must return data only when the
//! requester and target share a group, the target set `peer_sharing_consent`,
//! and the group's `peer_data_sharing` kill-switch is on — and it must fetch
//! under the target's OWN connection tenant, never the requester's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "tools-groups")]
mod peer_fetch_tests {
    use crate::common::create_test_server_resources;
    use chrono::{Duration, Utc};
    use dravr_tronc::mcp::schema::ToolResponse;
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::implementations::groups::GetGroupMemberActivitiesTool;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    async fn seed_user(resources: &ServerContext, label: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("{label}-{}@test.com", Uuid::new_v4()),
            password_hash,
            Some(format!("{label} Athlete")),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    async fn create_tenant_owned_by(resources: &ServerContext, owner_id: Uuid) -> TenantId {
        let tenant_id = TenantId::generate();
        let now = Utc::now();
        let tenant = Tenant {
            id: tenant_id,
            name: "Peer Tenant".to_owned(),
            slug: format!("peer-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: owner_id,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();
        tenant_id
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
                    title: "Peer Coach".to_owned(),
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
        peer_data_sharing: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: "Tempo Squad".to_owned(),
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

    fn recent_ride(id: &str) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Utc::now() - Duration::days(2),
            7_200,
            "strava".to_owned(),
        )
        .distance_meters(60_000.0)
        .build()
    }

    fn tool_context(user_id: Uuid, tenant_id: TenantId) -> ToolContext {
        ToolContext::new()
            .with_user(user_id.to_string())
            .with_tenant(tenant_id.to_string())
            .with_auth_method("jwt_bearer")
    }

    fn structured(response: &ToolResponse) -> Value {
        response
            .structured_content
            .clone()
            .expect("tool result carries structured content")
    }

    /// A consenting peer in a shared group whose Strava connection + activities
    /// live in their OWN tenant (distinct from the requester's) are returned —
    /// proving both the consent grant and cross-tenant fetch.
    #[tokio::test]
    async fn consenting_peer_activities_returned_under_peer_tenant() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(peer, peer_tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(peer, &peer_tenant, "strava", &[recent_ride("peer-ride-1")])
            .await
            .unwrap();

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, true).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
            .await;

        let payload = structured(&response);
        assert!(
            payload.get("error").is_none(),
            "consenting peer must not be refused, got: {payload}"
        );
        let activities = payload
            .get("activities")
            .and_then(Value::as_array)
            .expect("activities array present");
        assert_eq!(
            activities.len(),
            1,
            "peer's one cached ride must be returned"
        );
        assert_eq!(
            activities[0].get("id").and_then(Value::as_str),
            Some("peer-ride-1"),
            "must return the PEER's activity (fetched under the peer's tenant)"
        );
    }

    /// A peer who has not consented is refused with a clear message — no data.
    #[tokio::test]
    async fn non_consenting_peer_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(peer, peer_tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(peer, &peer_tenant, "strava", &[recent_ride("peer-ride-1")])
            .await
            .unwrap();

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        // consent = false
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, false).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
            .await;

        let payload = structured(&response);
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .expect("non-consenting peer must be refused with an error");
        assert!(
            err.contains("shared their data"),
            "error should explain consent is missing, got: {err}"
        );
        assert!(payload.get("activities").is_none(), "no data on refusal");
    }

    /// The group-level kill switch (`peer_data_sharing = false`) refuses even a
    /// peer who individually consented.
    #[tokio::test]
    async fn kill_switch_refuses_despite_consent() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;

        // peer_data_sharing = false, but the member consented.
        let gid = create_group(&resources, host_tenant, coach, requester, false).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, true).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
            .await;

        let payload = structured(&response);
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .expect("kill switch must refuse");
        assert!(
            err.contains("disabled"),
            "error should explain sharing is disabled, got: {err}"
        );
    }

    /// A name that matches no member of any of the requester's groups is refused
    /// (also covers the not-co-members case — the peer never shares a group).
    #[tokio::test]
    async fn unknown_member_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "nobody-here" }))
            .await;

        let payload = structured(&response);
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .expect("unknown member must be refused");
        assert!(
            err.contains("No group member matching"),
            "error should explain the member was not found, got: {err}"
        );
    }

    /// Querying your own name used to fall through to the generic "no member
    /// matching" error — a dead end no retry can escape (the 2026-08-22
    /// fallback thrash retried name variants against errors carrying no
    /// gradient). A self-lookup must name the actual mistake and the tool
    /// that serves it.
    #[tokio::test]
    async fn self_query_is_redirected_to_get_activities() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "philtool" }))
            .await;

        let payload = structured(&response);
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .expect("a self-query must be refused with a redirect");
        assert!(
            err.contains("is you") && err.contains("get_activities"),
            "the error must say the query matched the requester and name the \
             tool for their own data, got: {err}"
        );
    }

    /// A consenting peer whose every provider fetch fails (no token, empty
    /// cache) is an OUTAGE, not an empty week. The old unconditional
    /// `ok(count: 0)` taught the coach the peer had not trained; the reply
    /// must now be an error naming the fetch failure.
    #[tokio::test]
    async fn peer_fetch_outage_is_an_error_not_an_empty_week() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        // A connection with no OAuth token and nothing in the activity cache:
        // the live fetch fails to authenticate and the stale-cache fallback is
        // empty, so every fetch for this peer returns None.
        resources
            .common
            .repos
            .provider_connections
            .register_connection(peer, peer_tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, true).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let response = GetGroupMemberActivitiesTool
            .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
            .await;

        let payload = structured(&response);
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                panic!("an all-connections-failed fetch must be an error, got: {payload}")
            });
        assert!(
            err.contains("did not respond"),
            "the error must present the outage as temporary, got: {err}"
        );
        assert!(
            payload.get("count").is_none(),
            "no activity count may be fabricated for a failed fetch, got: {payload}"
        );
    }
    /// The tool's refusals carry a structured `reason`, so a caller can tell
    /// a consent decline (relayable to the athlete) from a resolution failure
    /// (never relayed) without parsing English.
    #[tokio::test]
    async fn refusals_carry_a_structured_reason() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;
        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;

        let gid = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            gid,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, false).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);
        let cases = [
            (json!({ "member": "raphtool" }), "no_consent"),
            (json!({ "member": "nobody-here" }), "no_match"),
            (json!({ "member": "philtool" }), "self"),
        ];
        for (args, expected) in cases {
            let response = GetGroupMemberActivitiesTool
                .execute(&runtime, &ctx, args.clone())
                .await;
            let payload = structured(&response);
            assert_eq!(
                payload.get("reason").and_then(Value::as_str),
                Some(expected),
                "reason for {args}: {payload}"
            );
        }
    }

    /// `group_id` pins the consent read to THAT group's membership row: a peer
    /// who consented in another shared group is still refused in the room
    /// where they did not.
    #[tokio::test]
    async fn a_group_pin_uses_that_groups_consent_not_one_granted_elsewhere() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let coach = seed_coach(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(peer, peer_tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(peer, &peer_tenant, "strava", &[recent_ride("peer-ride-1")])
            .await
            .unwrap();

        // The room: peer has NOT consented here.
        let room = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            room,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(
            &resources,
            room,
            peer,
            peer_tenant,
            GroupRole::Member,
            false,
        )
        .await;
        // Another shared group where the peer DID consent.
        let elsewhere = create_group(&resources, host_tenant, coach, requester, true).await;
        add_member(
            &resources,
            elsewhere,
            requester,
            host_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        add_member(
            &resources,
            elsewhere,
            peer,
            peer_tenant,
            GroupRole::Member,
            true,
        )
        .await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(requester, host_tenant);

        let pinned = structured(
            &GetGroupMemberActivitiesTool
                .execute(
                    &runtime,
                    &ctx,
                    json!({ "member": "raphtool", "group_id": room.to_string() }),
                )
                .await,
        );
        assert_eq!(
            pinned.get("reason").and_then(Value::as_str),
            Some("no_consent"),
            "pinned to the room, the room's consent decides: {pinned}"
        );
        assert!(pinned.get("activities").is_none());

        let unpinned = structured(
            &GetGroupMemberActivitiesTool
                .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
                .await,
        );
        assert_eq!(
            unpinned
                .get("activities")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1),
            "unpinned, the most permissive shared group serves: {unpinned}"
        );
    }

    /// The group's attached human coach has no membership row, yet their
    /// prompt view lists the consenting roster — so their fetch resolves the
    /// same names, under the same consent gate.
    #[tokio::test]
    async fn the_attached_human_coach_can_fetch_a_consenting_member() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = seed_user(&resources, "ownertool").await;
        let host_tenant = create_tenant_owned_by(&resources, owner).await;
        let coach_persona = seed_coach(&resources, owner, host_tenant).await;

        let human_coach = seed_user(&resources, "coachtool").await;
        let coach_tenant = create_tenant_owned_by(&resources, human_coach).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(peer, peer_tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(peer, &peer_tenant, "strava", &[recent_ride("peer-ride-1")])
            .await
            .unwrap();

        let gid = create_group(&resources, host_tenant, coach_persona, owner, true).await;
        add_member(&resources, gid, owner, host_tenant, GroupRole::Owner, false).await;
        add_member(&resources, gid, peer, peer_tenant, GroupRole::Member, true).await;
        assert!(resources
            .common
            .repos
            .groups
            .set_group_coach_user(&gid.to_string(), Some(human_coach), host_tenant)
            .await
            .unwrap());

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ctx = tool_context(human_coach, coach_tenant);
        let payload = structured(
            &GetGroupMemberActivitiesTool
                .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
                .await,
        );
        assert_eq!(
            payload
                .get("activities")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| a.get("id"))
                .and_then(Value::as_str),
            Some("peer-ride-1"),
            "the attached coach reads the consenting member's ride: {payload}"
        );

        // The same gate still holds for the coach: consent withdrawn, no data.
        assert!(resources
            .common
            .repos
            .groups
            .update_peer_sharing_consent(&gid.to_string(), peer, false)
            .await
            .unwrap());
        let refused = structured(
            &GetGroupMemberActivitiesTool
                .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
                .await,
        );
        assert_eq!(
            refused.get("reason").and_then(Value::as_str),
            Some("no_consent"),
            "the coach's read is gated by the member's consent too: {refused}"
        );
    }
}
