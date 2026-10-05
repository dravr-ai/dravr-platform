// ABOUTME: Tests for get_roster_overview — the coach's consent-gated roster review from their own thread.
// ABOUTME: Covers the coach attachment, coach consent, sources, room pinning and the conformance roster.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_roster_overview` reads every athlete a coach holds at once, so its
//! scope is security-critical: the athletes are the live members of the
//! groups whose `coach_user_id` is the caller, each read only by their own
//! `coach_sharing_consent`, and a call surfaced from a group's room covers
//! that group alone — the reply there is read by every member (registre#748).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "tools-groups")]
mod roster_overview_tests {
    use crate::common::create_test_server_resources;
    use chrono::{Duration, Utc};
    use dravr_tronc::mcp::schema::ToolResponse;
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::context::CONVERSATION_ID;
    use pierre_tool_runtime::implementations::group_roster::GetRosterOverviewTool;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    async fn seed_user(resources: &ServerContext, name: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("{}-{}@test.com", name.to_lowercase(), Uuid::new_v4()),
            password_hash,
            Some(name.to_owned()),
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
            name: "Roster Tenant".to_owned(),
            slug: format!("roster-{tenant_id}"),
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
                    title: "Group Coach".to_owned(),
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

    /// A group owned by `owner`, with `coach` attached as its human coach.
    async fn create_group(
        resources: &ServerContext,
        tenant_id: TenantId,
        agent_id: Uuid,
        name: &str,
        owner_id: Uuid,
        coach: Option<Uuid>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            agent_id: agent_id.to_string(),
            owner_id,
            coach_user_id: coach,
            // The peer switch governs members, never the coach: off here so a
            // test that passes proves the coach read ignores it.
            peer_data_sharing: false,
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

    /// Add `user_id` as a member sharing with the coach or not; peer consent is
    /// always off, so only the coach consent can grant the read.
    async fn add_member(
        resources: &ServerContext,
        group_id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
        coach_consent: bool,
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
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: coach_consent,
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
            .upsert_activities(user_id, &tenant_id, "strava", &[recent_ride("ride-1")])
            .await
            .unwrap();
    }

    /// An athlete with their own tenant.
    async fn seed_athlete(resources: &ServerContext, name: &str) -> (Uuid, TenantId) {
        let user = seed_user(resources, name).await;
        let tenant = create_tenant_owned_by(resources, user).await;
        (user, tenant)
    }

    /// A coach with their own tenant and an agent their groups answer with.
    struct Coach {
        user: Uuid,
        tenant: TenantId,
        agent: Uuid,
    }

    async fn seed_coach(resources: &ServerContext, name: &str) -> Coach {
        let user = seed_user(resources, name).await;
        let tenant = create_tenant_owned_by(resources, user).await;
        let agent = seed_agent(resources, user, tenant).await;
        Coach {
            user,
            tenant,
            agent,
        }
    }

    async fn overview(resources: &Arc<ServerContext>, coach: &Coach, args: Value) -> Value {
        let runtime: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
        let ctx = ToolContext::new()
            .with_user(coach.user.to_string())
            .with_tenant(coach.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response: ToolResponse = GetRosterOverviewTool.execute(&runtime, &ctx, args).await;
        response
            .structured_content
            .expect("tool result carries structured content")
    }

    async fn overview_in(
        resources: &Arc<ServerContext>,
        coach: &Coach,
        conversation_id: String,
    ) -> Value {
        CONVERSATION_ID
            .scope(Some(conversation_id), overview(resources, coach, json!({})))
            .await
    }

    fn athlete<'a>(payload: &'a Value, name: &str) -> &'a Value {
        payload["athletes"]
            .as_array()
            .expect("athletes array")
            .iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from the roster: {payload:#}"))
    }

    fn names(payload: &Value) -> Vec<String> {
        let mut names: Vec<String> = payload["athletes"]
            .as_array()
            .expect("athletes array")
            .iter()
            .map(|a| a["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }

    /// Two groups, three athletes: one shared across both groups, one who
    /// revoked the coach's read, one sharing with no source. Each is listed
    /// once, with training only for the sharer.
    #[tokio::test]
    async fn the_coach_reviews_every_athlete_across_their_groups() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;

        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        seed_ride(&resources, alice, alice_tenant).await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        add_member(&resources, trail, alice, alice_tenant, true).await;

        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        seed_ride(&resources, bruno, bruno_tenant).await;
        add_member(&resources, trail, bruno, bruno_tenant, false).await;

        let (chloe, chloe_tenant) = seed_athlete(&resources, "Chloe").await;
        add_member(&resources, track, chloe, chloe_tenant, true).await;

        let payload = overview(&resources, &coach, json!({})).await;
        assert!(payload.get("error").is_none(), "refused: {payload:#}");
        assert_eq!(names(&payload), ["Alice", "Bruno", "Chloe"]);
        assert_eq!(payload["truncated"], false);
        assert_eq!(payload["groups"].as_array().unwrap().len(), 2);

        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "shared", "{alice:#}");
        assert_eq!(alice["groups"].as_array().unwrap().len(), 2);
        let training = &alice["training"];
        assert_eq!(
            training["recent_activities"].as_array().unwrap().len(),
            1,
            "the shared athlete's ride is listed: {alice:#}"
        );
        assert!(
            training["days_since_last_activity"].as_i64().is_some(),
            "{alice:#}"
        );

        let bruno = athlete(&payload, "Bruno");
        assert_eq!(bruno["status"], "no_coach_consent");
        assert!(
            bruno.get("training").is_none(),
            "a revoked athlete carries no training at all: {bruno:#}"
        );

        let chloe = athlete(&payload, "Chloe");
        assert_eq!(chloe["status"], "no_source");
        assert!(chloe.get("training").is_none());
    }

    /// An athlete in two of the coach's groups who withdrew the coach's read in
    /// one still shares through the other: each membership grants the same
    /// coach the same read, as `get_group_member_activities` resolves it.
    #[tokio::test]
    async fn sharing_through_one_group_is_enough() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        seed_ride(&resources, alice, alice_tenant).await;
        add_member(&resources, track, alice, alice_tenant, false).await;
        add_member(&resources, trail, alice, alice_tenant, true).await;

        let payload = overview(&resources, &coach, json!({})).await;
        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "shared", "{alice:#}");
        assert!(alice.get("training").is_some(), "{alice:#}");

        // Pinned to the group where she withdrew, the coach does not read her.
        let payload = overview(&resources, &coach, json!({ "group_id": track.to_string() })).await;
        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "no_coach_consent", "{alice:#}");
        assert!(alice.get("training").is_none(), "{alice:#}");
    }

    /// The roster the tool lists is exactly the one the tenant-isolation
    /// conformance stage allows a reply to cite, so no listed athlete is
    /// redacted from the reply that names them.
    #[tokio::test]
    async fn the_roster_is_the_set_conformance_allows() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let group = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let mut expected = Vec::new();
        for name in ["Alice", "Bruno"] {
            let (user, tenant) = seed_athlete(&resources, name).await;
            add_member(&resources, group, user, tenant, name == "Alice").await;
            expected.push(user);
        }
        // Someone who left is no longer the coach's athlete.
        let (gone, gone_tenant) = seed_athlete(&resources, "Gone").await;
        add_member(&resources, group, gone, gone_tenant, true).await;
        resources
            .common
            .repos
            .groups
            .remove_member(&group.to_string(), gone)
            .await
            .unwrap();

        let payload = overview(&resources, &coach, json!({})).await;
        assert_eq!(names(&payload), ["Alice", "Bruno"]);

        let mut allowed = resources
            .common
            .repos
            .groups
            .list_athletes_coached_by(coach.user)
            .await
            .unwrap();
        allowed.sort();
        expected.sort();
        assert_eq!(
            allowed, expected,
            "conformance allows exactly the listed athletes"
        );
    }

    /// Owning a group is not coaching it: the roster follows the coach
    /// attachment, like the conformance roster and the plan scope.
    #[tokio::test]
    async fn an_owner_who_is_not_the_groups_coach_has_no_roster() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = seed_coach(&resources, "Owner Omar").await;
        let group = create_group(
            &resources,
            owner.tenant,
            owner.agent,
            "Friends",
            owner.user,
            None,
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, group, alice, alice_tenant, true).await;

        let payload = overview(&resources, &owner, json!({})).await;
        assert_eq!(payload["reason"], "not_a_coach", "{payload:#}");
        assert!(payload.get("athletes").is_none());
    }

    /// The coach's own DM covers every group; a group's thread covers that
    /// group alone, because every member reads the reply there.
    #[tokio::test]
    async fn a_group_thread_is_pinned_to_its_group() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;

        let chat = &resources.common.repos.chat;
        let dm = chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "dm",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, dm.id).await;
        assert_eq!(names(&payload), ["Alice", "Bruno"], "{payload:#}");

        let track_thread = chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "track",
                "gemini-2.0-flash",
                None,
                Some(&track.to_string()),
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, track_thread.id).await;
        assert_eq!(
            names(&payload),
            ["Alice"],
            "Trail's athletes must not reach Track's room: {payload:#}"
        );
        assert_eq!(payload["groups"].as_array().unwrap().len(), 1);
    }

    /// In a room whose group the caller does not coach, the roster is refused.
    #[tokio::test]
    async fn a_room_the_caller_does_not_coach_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let mine = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, mine, alice, alice_tenant, true).await;

        let other = seed_coach(&resources, "Coach Other").await;
        let theirs = create_group(
            &resources,
            other.tenant,
            other.agent,
            "Club",
            other.user,
            Some(other.user),
        )
        .await;
        let club_thread = resources
            .common
            .repos
            .chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "club",
                "gemini-2.0-flash",
                None,
                Some(&theirs.to_string()),
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, club_thread.id).await;
        assert_eq!(payload["reason"], "not_this_groups_coach", "{payload:#}");
        assert!(payload.get("athletes").is_none());
    }

    /// A conversation the caller cannot resolve — a messaging room filed under
    /// the bot tenant — may be read by anyone, so it is refused, not guessed.
    #[tokio::test]
    async fn an_unresolvable_conversation_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let group = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, group, alice, alice_tenant, true).await;

        let bot = seed_user(&resources, "Dravr Bot").await;
        let bot_tenant = create_tenant_owned_by(&resources, bot).await;
        let room = resources
            .common
            .repos
            .chat
            .create_conversation(
                &bot.to_string(),
                bot_tenant,
                "room",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, room.id).await;
        assert_eq!(payload["reason"], "room_unresolved", "{payload:#}");
    }

    /// `group_id` narrows the review to one group, and a group the caller does
    /// not coach is refused rather than read.
    #[tokio::test]
    async fn the_group_argument_narrows_and_never_widens() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;

        let payload = overview(&resources, &coach, json!({ "group_id": trail.to_string() })).await;
        assert_eq!(names(&payload), ["Bruno"], "{payload:#}");

        let other = seed_coach(&resources, "Coach Other").await;
        let theirs = create_group(
            &resources,
            other.tenant,
            other.agent,
            "Club",
            other.user,
            Some(other.user),
        )
        .await;
        let payload = overview(
            &resources,
            &coach,
            json!({ "group_id": theirs.to_string() }),
        )
        .await;
        assert_eq!(payload["reason"], "not_your_group", "{payload:#}");
    }
}
