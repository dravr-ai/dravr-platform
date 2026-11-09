// ABOUTME: Tests for get_group_member_activities — consent-gated peer activity fetch (Fix B).
// ABOUTME: Covers consent grant, consent denial, kill-switch, unknown member, tenant isolation, and the room pin.
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

#[cfg(feature = "tools-groups")]
mod peer_fetch_tests {
    use crate::common::create_test_server_resources;
    use chrono::{Duration, Utc};
    use dravr_tronc::mcp::schema::ToolResponse;
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupInviteKind, GroupMember,
        GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::Transport;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::context::{CONVERSATION_ID, TURN_DIRECT_MESSAGE};
    use pierre_tool_runtime::implementations::groups::GetGroupMemberActivitiesTool;
    use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
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

    async fn seed_agent(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) -> Uuid {
        resources
            .common
            .repos
            .agents
            .create_system_agent(
                user_id,
                tenant_id,
                &CreateSystemAgentRequest {
                    title: "Peer Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: AgentVisibility::Global,
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn create_group(
        resources: &ServerContext,
        tenant_id: TenantId,
        agent_id: Uuid,
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
            agent_id: agent_id.to_string(),
            owner_id,
            coach_user_id: None,
            peer_data_sharing,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::Off,
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
                coach_sharing_consent: consent,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// A member's two sharing decisions, set independently.
    struct MemberSharing {
        group_id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
        peers: bool,
        coach: bool,
    }

    async fn add_member_sharing(resources: &ServerContext, sharing: MemberSharing) {
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: sharing.group_id,
                user_id: sharing.user_id,
                tenant_id: sharing.tenant_id.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: sharing.peers,
                coach_sharing_consent: sharing.coach,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// A Strava connection and one recent ride for `user_id` under their own
    /// tenant.
    async fn seed_ride(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(user_id, &tenant_id, "strava", &[recent_ride("peer-ride-1")])
            .await
            .unwrap();
    }

    fn first_activity_id(payload: &Value) -> Option<&str> {
        payload
            .get("activities")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str)
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

    /// A conversation `user_id` owns in `tenant_id`, bound to `group_id`'s
    /// room when one is given and private to them otherwise.
    async fn conversation(
        resources: &ServerContext,
        user_id: Uuid,
        tenant_id: TenantId,
        group_id: Option<Uuid>,
    ) -> String {
        let group = group_id.map(|g| g.to_string());
        resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "thread",
                "gemini-2.0-flash",
                None,
                group.as_deref(),
            )
            .await
            .unwrap()
            .id
    }

    /// The tool's answer to a web turn in `conversation_id`: the route calls
    /// a turn a direct message when its conversation carries no group.
    async fn read_in(
        runtime: &Arc<dyn ToolRuntime>,
        caller: (Uuid, TenantId),
        conversation_id: &str,
        args: Value,
    ) -> Value {
        let (user, tenant) = caller;
        let is_direct_message = runtime
            .repos()
            .chat
            .get_conversation(conversation_id, &user.to_string(), tenant)
            .await
            .unwrap()
            .is_none_or(|conversation| conversation.group_id.is_none());
        let ctx = tool_context(user, tenant);
        let response = TURN_DIRECT_MESSAGE
            .scope(
                Some(is_direct_message),
                CONVERSATION_ID.scope(
                    Some(conversation_id.to_owned()),
                    GetGroupMemberActivitiesTool.execute(runtime, &ctx, args),
                ),
            )
            .await;
        structured(&response)
    }

    /// The tool's answer when a messaging turn in `conversation_id` dispatches
    /// it. The executor is built inside the turn's direct-message scope, as
    /// the chat pipeline builds it, and runs outside it, as a loopback call
    /// does: what reaches the tool is what the executor carried.
    async fn dispatch_from_turn(
        runtime: &Arc<dyn ToolRuntime>,
        caller: (Uuid, TenantId),
        conversation_id: &str,
        is_direct_message: bool,
        member: &str,
    ) -> Value {
        let (user, tenant) = caller;
        let executor = TURN_DIRECT_MESSAGE.sync_scope(Some(is_direct_message), || {
            UniversalToolExecutor::new(Arc::clone(runtime))
                .with_scopes(OAuthScope::self_grant())
                .with_transport(Transport::Messaging)
                .with_conversation_id(conversation_id.to_owned())
                .with_conversation_tenant(tenant.as_uuid())
        });
        let response = executor
            .execute_tool(UniversalRequest {
                tool_name: "get_group_member_activities".to_owned(),
                parameters: json!({ "member": member }),
                user_id: user.to_string(),
                protocol: "chat".to_owned(),
                tenant_id: Some(tenant.to_string()),
            })
            .await
            .expect("the executor dispatches the tool");
        response
            .result
            .expect("the tool answers with structured content")
    }

    /// A consenting peer in a shared group whose Strava connection + activities
    /// live in their OWN tenant (distinct from the requester's) are returned —
    /// proving both the consent grant and cross-tenant fetch.
    #[tokio::test]
    async fn consenting_peer_activities_returned_under_peer_tenant() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let agent = seed_agent(&resources, requester, host_tenant).await;

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

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;

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

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;

        // peer_data_sharing = false, but the member consented.
        let gid = create_group(&resources, host_tenant, agent, requester, false).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
    /// `ok(count: 0)` taught the agent the peer had not trained; the reply
    /// must now be an error naming the fetch failure.
    #[tokio::test]
    async fn peer_fetch_outage_is_an_error_not_an_empty_week() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philtool").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let agent = seed_agent(&resources, requester, host_tenant).await;

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

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;
        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let agent = seed_agent(&resources, requester, host_tenant).await;

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
        let room = create_group(&resources, host_tenant, agent, requester, true).await;
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
        let elsewhere = create_group(&resources, host_tenant, agent, requester, true).await;
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
    /// prompt view lists the roster that shares with them — so their fetch
    /// resolves the same names. The coach reads by the member's coach consent
    /// (ADR-002: joining grants it), which neither the member's peer consent
    /// nor the group's peer switch governs; revoking it refuses the coach.
    #[tokio::test]
    async fn the_attached_human_coach_reads_by_coach_consent_alone() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = seed_user(&resources, "ownertool").await;
        let host_tenant = create_tenant_owned_by(&resources, owner).await;
        let agent_persona = seed_agent(&resources, owner, host_tenant).await;

        let human_coach = seed_user(&resources, "coachtool").await;
        let coach_tenant = create_tenant_owned_by(&resources, human_coach).await;

        let peer = seed_user(&resources, "raphtool").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        seed_ride(&resources, peer, peer_tenant).await;

        // Peer sharing is off at both levels: the switch and the member.
        let gid = create_group(&resources, host_tenant, agent_persona, owner, false).await;
        add_member(&resources, gid, owner, host_tenant, GroupRole::Owner, false).await;
        add_member_sharing(
            &resources,
            MemberSharing {
                group_id: gid,
                user_id: peer,
                tenant_id: peer_tenant,
                peers: false,
                coach: true,
            },
        )
        .await;
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
            first_activity_id(&payload),
            Some("peer-ride-1"),
            "the attached coach reads the member's ride: {payload}"
        );

        // The member revokes coach sharing: the coach is refused, with the
        // way back named.
        assert!(resources
            .common
            .repos
            .groups
            .update_coach_sharing_consent(&gid.to_string(), peer, false)
            .await
            .unwrap());
        let refused = structured(
            &GetGroupMemberActivitiesTool
                .execute(&runtime, &ctx, json!({ "member": "raphtool" }))
                .await,
        );
        assert_eq!(
            refused.get("reason").and_then(Value::as_str),
            Some("no_coach_consent"),
            "a revoked coach consent refuses the coach: {refused}"
        );
        assert!(refused.get("activities").is_none());
    }

    /// Joining through an invite grants the coach's read with no consent step,
    /// and leaves the member's peers shut out: the coach reads the athlete the
    /// moment they join, while a fellow member is refused even with the
    /// group's peer switch on.
    #[tokio::test]
    async fn joining_shares_with_the_coach_and_not_with_peers() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = seed_user(&resources, "ownerjoin").await;
        let host_tenant = create_tenant_owned_by(&resources, owner).await;
        let agent_persona = seed_agent(&resources, owner, host_tenant).await;

        let human_coach = seed_user(&resources, "coachjoin").await;
        let coach_tenant = create_tenant_owned_by(&resources, human_coach).await;

        let athlete = seed_user(&resources, "raphjoin").await;
        let athlete_tenant = create_tenant_owned_by(&resources, athlete).await;
        seed_ride(&resources, athlete, athlete_tenant).await;

        let gid = create_group(&resources, host_tenant, agent_persona, owner, true).await;
        add_member(&resources, gid, owner, host_tenant, GroupRole::Owner, false).await;
        assert!(resources
            .common
            .repos
            .groups
            .set_group_coach_user(&gid.to_string(), Some(human_coach), host_tenant)
            .await
            .unwrap());
        let invite = resources
            .group_service()
            .create_invite(gid, owner, host_tenant, None, None, GroupInviteKind::Member)
            .await
            .unwrap();
        let joined = resources
            .group_service()
            .join_group(&invite.code, athlete, host_tenant)
            .await
            .unwrap();
        assert!(joined.coach_sharing_consent, "joining grants coach sharing");
        assert!(!joined.peer_sharing_consent, "peer sharing stays opt-in");

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let coach_read = structured(
            &GetGroupMemberActivitiesTool
                .execute(
                    &runtime,
                    &tool_context(human_coach, coach_tenant),
                    json!({ "member": "raphjoin" }),
                )
                .await,
        );
        assert_eq!(
            first_activity_id(&coach_read),
            Some("peer-ride-1"),
            "the coach reads the athlete right after the join: {coach_read}"
        );

        let peer_read = structured(
            &GetGroupMemberActivitiesTool
                .execute(
                    &runtime,
                    &tool_context(owner, host_tenant),
                    json!({ "member": "raphjoin" }),
                )
                .await,
        );
        assert_eq!(
            peer_read.get("reason").and_then(Value::as_str),
            Some("no_consent"),
            "a fellow member still needs the athlete's peer consent: {peer_read}"
        );
        assert!(peer_read.get("activities").is_none());
    }

    /// carnet#1420: a coach of two groups, asking in a one-athlete room about
    /// the other group's athlete. Every member of the room reads the reply,
    /// and that athlete shared with their own group's coach, not with this
    /// room — so the room pins the scan to its own group, whatever `group_id`
    /// the model passes, and the athlete resolves as no match there. The
    /// coach's own conversation still reads them across groups.
    #[tokio::test]
    async fn a_room_never_reads_another_groups_athlete() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_user(&resources, "coachpin").await;
        let coach_tenant = create_tenant_owned_by(&resources, coach).await;
        let agent_persona = seed_agent(&resources, coach, coach_tenant).await;

        let ana = seed_user(&resources, "anapin").await;
        let ana_tenant = create_tenant_owned_by(&resources, ana).await;
        seed_ride(&resources, ana, ana_tenant).await;
        let bea = seed_user(&resources, "beapin").await;
        let bea_tenant = create_tenant_owned_by(&resources, bea).await;
        seed_ride(&resources, bea, bea_tenant).await;

        let mut groups = Vec::new();
        for (athlete, tenant) in [(ana, ana_tenant), (bea, bea_tenant)] {
            let gid = create_group(&resources, coach_tenant, agent_persona, coach, true).await;
            add_member(&resources, gid, athlete, tenant, GroupRole::Member, true).await;
            assert!(
                resources
                    .common
                    .repos
                    .groups
                    .set_group_coach_user(&gid.to_string(), Some(coach), coach_tenant)
                    .await
                    .unwrap(),
                "test setup bug: the coach was not attached to the group just created"
            );
            groups.push(gid);
        }
        let (anas_group, beas_group) = (groups[0], groups[1]);

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let caller = (coach, coach_tenant);
        let anas_room = conversation(&resources, coach, coach_tenant, Some(anas_group)).await;

        for args in [
            json!({ "member": "beapin" }),
            json!({ "member": "beapin", "group_id": beas_group.to_string() }),
        ] {
            let refused = read_in(&runtime, caller, &anas_room, args.clone()).await;
            assert_eq!(
                refused.get("reason").and_then(Value::as_str),
                Some("no_match"),
                "another group's athlete is no match in this room, for {args}: {refused}"
            );
            assert!(
                refused.get("activities").is_none(),
                "no rows reach the room for {args}: {refused}"
            );
            let said = refused["error"].as_str().unwrap();
            assert!(
                said.contains("that group's room") && said.contains("thread about that athlete"),
                "the refusal says where that athlete is read: {said}"
            );
        }

        // Both athletes' seeded rides share one id, so the member the answer
        // names is what says whose sessions came back.
        let own_athlete =
            read_in(&runtime, caller, &anas_room, json!({ "member": "anapin" })).await;
        assert_eq!(
            own_athlete.get("member").and_then(Value::as_str),
            Some("anapin Athlete"),
            "the room's own athlete is the one read in it: {own_athlete}"
        );
        assert_eq!(
            first_activity_id(&own_athlete),
            Some("peer-ride-1"),
            "the room's own athlete is still read in it: {own_athlete}"
        );

        let direct_chat = conversation(&resources, coach, coach_tenant, None).await;
        let across = read_in(
            &runtime,
            caller,
            &direct_chat,
            json!({ "member": "beapin" }),
        )
        .await;
        assert_eq!(
            across.get("member").and_then(Value::as_str),
            Some("beapin Athlete"),
            "the coach's own conversation reads the other group's athlete: {across}"
        );
        assert_eq!(
            first_activity_id(&across),
            Some("peer-ride-1"),
            "the coach's own conversation reads every athlete they coach: {across}"
        );
    }

    /// The same pin holds for a member who is not a coach: in their own
    /// group's room they never read a peer who shared only with another group
    /// they belong to, though their own conversation does.
    #[tokio::test]
    async fn a_member_of_two_groups_never_reads_one_groups_peer_in_the_others_room() {
        let resources = create_test_server_resources().await.unwrap();
        let ana = seed_user(&resources, "anaroom").await;
        let ana_tenant = create_tenant_owned_by(&resources, ana).await;
        let agent_persona = seed_agent(&resources, ana, ana_tenant).await;
        let bea = seed_user(&resources, "bearoom").await;
        let bea_tenant = create_tenant_owned_by(&resources, bea).await;
        seed_ride(&resources, bea, bea_tenant).await;

        let anas_room_group = create_group(&resources, ana_tenant, agent_persona, ana, true).await;
        add_member(
            &resources,
            anas_room_group,
            ana,
            ana_tenant,
            GroupRole::Owner,
            false,
        )
        .await;
        let shared = create_group(&resources, ana_tenant, agent_persona, ana, true).await;
        add_member(&resources, shared, ana, ana_tenant, GroupRole::Owner, false).await;
        add_member(&resources, shared, bea, bea_tenant, GroupRole::Member, true).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let caller = (ana, ana_tenant);

        let room = conversation(&resources, ana, ana_tenant, Some(anas_room_group)).await;
        let refused = read_in(&runtime, caller, &room, json!({ "member": "bearoom" })).await;
        assert_eq!(
            refused.get("reason").and_then(Value::as_str),
            Some("no_match"),
            "a peer of another group is no match in this room: {refused}"
        );
        assert!(
            refused.get("activities").is_none(),
            "no rows of another group's peer reach this room: {refused}"
        );

        let own = conversation(&resources, ana, ana_tenant, None).await;
        let read = read_in(&runtime, caller, &own, json!({ "member": "bearoom" })).await;
        assert_eq!(
            first_activity_id(&read),
            Some("peer-ride-1"),
            "the member's own conversation reads the peer who shared with them: {read}"
        );
    }

    /// A conversation the caller is no participant of may be any room, so
    /// nothing is scanned and no peer's sessions are published into it.
    #[tokio::test]
    async fn an_unresolvable_conversation_reads_no_one() {
        let resources = create_test_server_resources().await.unwrap();
        let requester = seed_user(&resources, "philunres").await;
        let host_tenant = create_tenant_owned_by(&resources, requester).await;
        let agent = seed_agent(&resources, requester, host_tenant).await;
        let peer = seed_user(&resources, "raphunres").await;
        let peer_tenant = create_tenant_owned_by(&resources, peer).await;
        seed_ride(&resources, peer, peer_tenant).await;

        let gid = create_group(&resources, host_tenant, agent, requester, true).await;
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

        let stranger = seed_user(&resources, "strangerunres").await;
        let stranger_tenant = create_tenant_owned_by(&resources, stranger).await;
        let theirs = conversation(&resources, stranger, stranger_tenant, None).await;

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let caller = (requester, host_tenant);
        let refused = read_in(&runtime, caller, &theirs, json!({ "member": "raphunres" })).await;
        assert_eq!(
            refused.get("reason").and_then(Value::as_str),
            Some("room_unresolved"),
            "a conversation the caller is no participant of is refused, not scanned: {refused}"
        );
        assert!(
            refused.get("activities").is_none(),
            "no rows reach a conversation that did not resolve: {refused}"
        );
    }

    /// carnet#1421: a messaging group chat whose group binding failed keeps
    /// no group on its conversation, so the row alone reads as the coach's
    /// private chat. The turn knows other people read the reply, and the
    /// executor carries that to the tool: the coach reads no athlete of
    /// either group they coach there, while the same conversation on a
    /// direct-message turn reads both.
    #[tokio::test]
    async fn an_ungrouped_shared_chat_reads_no_one() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_user(&resources, "coachshared").await;
        let coach_tenant = create_tenant_owned_by(&resources, coach).await;
        let agent_persona = seed_agent(&resources, coach, coach_tenant).await;
        let athletes = ["anashared", "brunoshared"];

        // Each athlete in a group of their own, every one coached by `coach`.
        for label in athletes {
            let athlete = seed_user(&resources, label).await;
            let athlete_tenant = create_tenant_owned_by(&resources, athlete).await;
            seed_ride(&resources, athlete, athlete_tenant).await;
            let gid = create_group(&resources, coach_tenant, agent_persona, coach, true).await;
            add_member(
                &resources,
                gid,
                athlete,
                athlete_tenant,
                GroupRole::Member,
                true,
            )
            .await;
            assert!(
                resources
                    .common
                    .repos
                    .groups
                    .set_group_coach_user(&gid.to_string(), Some(coach), coach_tenant)
                    .await
                    .unwrap(),
                "test setup bug: the coach was not attached to the group just created"
            );
        }

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let chat = conversation(&resources, coach, coach_tenant, None).await;

        for label in athletes {
            let shared =
                dispatch_from_turn(&runtime, (coach, coach_tenant), &chat, false, label).await;
            assert_eq!(
                shared.get("reason").and_then(Value::as_str),
                Some("room_unresolved"),
                "a chat others read with no group bound to it is never treated as private, \
                 so {label} is not read there: {shared}"
            );
            assert!(
                shared.get("activities").is_none(),
                "none of {label}'s rows reach a shared chat: {shared}"
            );

            let direct =
                dispatch_from_turn(&runtime, (coach, coach_tenant), &chat, true, label).await;
            let display_name = format!("{label} Athlete");
            assert_eq!(
                direct.get("member").and_then(Value::as_str),
                Some(display_name.as_str()),
                "the same conversation on a direct-message turn reads coached {label}: {direct}"
            );
            assert_eq!(
                first_activity_id(&direct),
                Some("peer-ride-1"),
                "{label}'s ride comes back on the direct-message turn: {direct}"
            );
        }
    }
}
