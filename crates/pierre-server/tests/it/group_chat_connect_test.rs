// ABOUTME: Connecting a web coaching group to a Telegram group chat (carnet#1683), driven through the real webhook
// ABOUTME: The code connects once, only for the owner or coach; the chat then admits members only; D3 move/refuse; disconnect
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

// The topology every test here builds is the one production has: the bot
// serves its own tenant, while the web group lives under its owner's own
// tenant, so connecting must record the bot tenant or the chat's messages
// would never find the group (the tenant-mismatch design point).
#[cfg(feature = "client-messaging")]
mod group_chat_connect_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::{json, Value};
    use serial_test::serial;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::command_e2e::{
        CommandE2e, Member, RoomE2e, RouterLlm, WebhookOutcome, BOT_USERNAME,
    };
    use crate::helpers::offline_channel::OfflineSendAdapters;

    const CHAT: i64 = -1_009_683;

    async fn start() -> Arc<CommandE2e> {
        let llm = RouterLlm::new();
        let resources =
            create_test_server_resources_with_chat_provider(Arc::<RouterLlm>::clone(&llm))
                .await
                .unwrap();
        CommandE2e::start(resources, llm).await
    }

    /// A group made in the web app: under `owner`'s own tenant, bound to no
    /// chat, with `owner` enrolled as its owner.
    async fn web_group(e2e: &CommandE2e, owner: &Member, name: &str) -> CoachingGroup {
        let agent = e2e
            .seed_coach_agent(owner.user_id, owner.home_tenant, "Web Group Agent")
            .await;
        let now = Utc::now();
        let group = CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: owner.home_tenant.to_string(),
            name: name.to_owned(),
            description: None,
            agent_id: agent.id.to_string(),
            owner_id: owner.user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::All,
            digest_mode: GroupDigestMode::Off,
            max_members: 20,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            chat_connection: None,
            created_at: now,
            updated_at: now,
        };
        let created = e2e
            .resources
            .common
            .repos
            .groups
            .create_group(owner.home_tenant, &group)
            .await
            .unwrap();
        enrol(e2e, &created, owner, GroupRole::Owner).await;
        created
    }

    async fn enrol(e2e: &CommandE2e, group: &CoachingGroup, m: &Member, role: GroupRole) {
        let now = Utc::now();
        e2e.resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: m.user_id,
                tenant_id: group.tenant_id.clone(),
                role,
                peer_sharing_consent: false,
                coach_sharing_consent: true,
                coach_sharing_consent_at: Some(now),
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    async fn mint(e2e: &CommandE2e, group: &CoachingGroup, by: &Member) -> String {
        e2e.resources
            .group_service()
            .mint_chat_connect_code(group, by.user_id, "telegram", e2e.bot_tenant)
            .await
            .unwrap()
            .code
    }

    async fn reread(e2e: &CommandE2e, group_id: Uuid) -> CoachingGroup {
        e2e.resources
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), e2e.bot_tenant)
            .await
            .unwrap()
            .unwrap()
    }

    async fn chat_group(e2e: &CommandE2e) -> Option<CoachingGroup> {
        e2e.resources
            .group_service()
            .get_group_by_channel(e2e.bot_tenant, "telegram", &CHAT.to_string())
            .await
            .unwrap()
    }

    async fn say(e2e: &CommandE2e, m: &Member, text: &str) -> WebhookOutcome {
        e2e.send_group_text(m.channel_user_id.parse().unwrap(), CHAT, text)
            .await
    }

    /// Sends the bot made into the chat.
    fn chat_sends(e2e: &CommandE2e) -> Vec<String> {
        e2e.adapters
            .sends()
            .into_iter()
            .filter(|s| s.recipient_id == CHAT.to_string())
            .map(|s| match s.content {
                pierre_core::models::messaging::MessageContent::Text { body } => body,
                _ => String::new(),
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_add_to_group_start_connects_the_existing_web_group_under_the_bot_tenant() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        assert_ne!(
            coach.home_tenant, e2e.bot_tenant,
            "fixture must be cross-tenant"
        );
        let code = mint(&e2e, &group, &coach).await;

        say(&e2e, &coach, &format!("/start {code}")).await;

        let connected = reread(&e2e, group.id).await;
        assert_eq!(connected.channel_type.as_deref(), Some("telegram"));
        assert_eq!(
            connected.channel_chat_id.as_deref(),
            Some(CHAT.to_string().as_str())
        );
        let connection = connected
            .chat_connection
            .expect("connected from the web app");
        assert_eq!(connection.channel_tenant_id, e2e.bot_tenant);
        assert_eq!(connection.chat_title.as_deref(), Some("Command E2e Group"));
        assert_eq!(
            connected.tenant_id,
            coach.home_tenant.to_string(),
            "the group stays under its own tenant"
        );
        assert_eq!(
            chat_group(&e2e).await.map(|g| g.id),
            Some(group.id),
            "the chat's messages, which arrive under the bot tenant, find the web group"
        );
        assert!(
            chat_sends(&e2e)
                .iter()
                .any(|body| body.contains("Les Rouleurs")),
            "the chat is told it is connected: {:?}",
            chat_sends(&e2e)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_connect_code_connects_once_and_only_for_the_owner_or_coach() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let athlete = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        enrol(&e2e, &group, &athlete, GroupRole::Member).await;
        let code = mint(&e2e, &group, &coach).await;

        // A member who neither owns nor coaches the group cannot connect it,
        // and their attempt does not spend the code.
        say(&e2e, &athlete, &format!("/group link {code}")).await;
        assert_eq!(reread(&e2e, group.id).await.channel_type, None);

        say(&e2e, &coach, &format!("/group link {code}")).await;
        assert_eq!(chat_group(&e2e).await.map(|g| g.id), Some(group.id));

        // Spent: a second group cannot ride the same code into another chat.
        let other = web_group(&e2e, &coach, "Other Group").await;
        e2e.send_group_text(
            coach.channel_user_id.parse().unwrap(),
            CHAT - 1,
            &format!("/start {code}"),
        )
        .await;
        assert_eq!(
            reread(&e2e, other.id).await.channel_type,
            None,
            "a spent code connects nothing"
        );
        assert!(
            e2e.resources
                .group_service()
                .get_group_by_channel(e2e.bot_tenant, "telegram", &(CHAT - 1).to_string())
                .await
                .unwrap()
                .is_none_or(|g| g.id != group.id),
            "the spent code moved nothing to the second chat"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_connected_chat_admits_members_only_and_tells_a_stranger_once() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let athlete = e2e.linked_member(true).await;
        let stranger = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        enrol(&e2e, &group, &athlete, GroupRole::Member).await;
        let code = mint(&e2e, &group, &coach).await;
        say(&e2e, &coach, &format!("/start {code}")).await;
        let sends_before = chat_sends(&e2e).len();

        // A linked stranger: nothing stored, no session, no enrolment, no turn.
        let first = format!("@{BOT_USERNAME} what did the group ride?");
        let outcome = say(&e2e, &stranger, &first).await;
        assert_eq!(
            outcome.messages_stored(),
            0,
            "nothing is dispatched for a stranger"
        );
        say(&e2e, &stranger, &format!("@{BOT_USERNAME} hello again")).await;
        sleep(Duration::from_millis(300)).await;
        assert_eq!(
            e2e.count_inbound_with_body(&first).await,
            0,
            "a stranger's words are never stored"
        );
        assert!(
            e2e.session_id(&stranger, e2e.bot_tenant, &CHAT.to_string())
                .await
                .is_none(),
            "a stranger gets no session in the room"
        );
        assert!(
            e2e.resources
                .common
                .repos
                .groups
                .get_member(&group.id.to_string(), stranger.user_id)
                .await
                .unwrap()
                .is_none(),
            "writing in a connected chat enrols nobody"
        );
        assert_eq!(
            chat_sends(&e2e).len(),
            sends_before + 1,
            "the stranger is told how to join once, not once per message"
        );

        // A member is answered, in a conversation bound to the web group.
        e2e.llm.set_turn_replies(&["Belle semaine."]);
        say(&e2e, &athlete, &format!("@{BOT_USERNAME} how was my week?")).await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "a member's turn reaches the agent"
        );
        let conversation = e2e
            .conversation_id(&athlete, e2e.bot_tenant, &CHAT.to_string())
            .await
            .expect("the member has a room session");
        let record = e2e
            .resources
            .common
            .repos
            .chat
            .get_conversation(&conversation, &athlete.user_id.to_string(), e2e.bot_tenant)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.group_id, Some(group.id.to_string()));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_chat_already_bound_to_a_group_its_owner_runs_moves_to_the_web_group() {
        let e2e = Arc::clone(&start().await);
        let coach = e2e.linked_member(true).await;
        let old = RoomE2e::bind_room(Arc::clone(&e2e), CHAT, GroupRespondMode::All, &coach).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        let code = mint(&e2e, &group, &coach).await;

        say(&e2e, &coach, &format!("/start {code}")).await;

        assert_eq!(chat_group(&e2e).await.map(|g| g.id), Some(group.id));
        let left = reread(&e2e, old.group_id).await;
        assert_eq!(left.channel_type, None, "the old group loses only its chat");
        assert!(left.is_active, "the old group and its history stay");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_chat_bound_to_a_group_someone_else_owns_is_refused() {
        let e2e = Arc::clone(&start().await);
        let coach = e2e.linked_member(true).await;
        let first_sender = e2e.linked_member(true).await;
        let old =
            RoomE2e::bind_room(Arc::clone(&e2e), CHAT, GroupRespondMode::All, &first_sender).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        let code = mint(&e2e, &group, &coach).await;

        say(&e2e, &coach, &format!("/start {code}")).await;

        assert_eq!(chat_group(&e2e).await.map(|g| g.id), Some(old.group_id));
        assert_eq!(reread(&e2e, group.id).await.channel_type, None);
        assert!(
            e2e.resources
                .common
                .repos
                .groups
                .live_chat_connect_code(&code, "telegram", e2e.bot_tenant, Utc::now())
                .await
                .unwrap()
                .is_some(),
            "a refused connect leaves the code redeemable"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_disconnected_chat_stays_ungrouped_and_its_member_leaves_the_group_context() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        let code = mint(&e2e, &group, &coach).await;
        say(&e2e, &coach, &format!("/start {code}")).await;
        e2e.llm.set_turn_replies(&["OK."]);
        say(&e2e, &coach, &format!("@{BOT_USERNAME} hi")).await;
        assert!(e2e.wait_llm_turns(1).await);
        let grouped = e2e
            .conversation_id(&coach, e2e.bot_tenant, &CHAT.to_string())
            .await
            .unwrap();

        let connected = reread(&e2e, group.id).await;
        let released = e2e
            .resources
            .group_service()
            .disconnect_chat(&connected, coach.user_id)
            .await
            .unwrap()
            .expect("the group had a chat");
        assert_eq!(released.channel_tenant_id, e2e.bot_tenant);

        say(&e2e, &coach, &format!("@{BOT_USERNAME} still there?")).await;
        assert!(e2e.wait_llm_turns(2).await);
        assert!(
            chat_group(&e2e).await.is_none(),
            "no group is auto-created for the chat"
        );
        let now_conversation = e2e
            .conversation_id(&coach, e2e.bot_tenant, &CHAT.to_string())
            .await
            .unwrap();
        assert_ne!(
            now_conversation, grouped,
            "the session left the group's conversation"
        );
        let record = e2e
            .resources
            .common
            .repos
            .chat
            .get_conversation(
                &now_conversation,
                &coach.user_id.to_string(),
                e2e.bot_tenant,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.group_id, None,
            "the chat carries no group context any more"
        );
    }

    async fn token_for(e2e: &CommandE2e, m: &Member) -> String {
        let user = e2e
            .resources
            .common
            .repos
            .users
            .get_global(m.user_id)
            .await
            .unwrap()
            .unwrap();
        e2e.resources
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &user,
                &e2e.resources.auth.jwks_manager,
                Some(m.home_tenant.to_string()),
            )
            .unwrap()
    }

    async fn rest(
        e2e: &CommandE2e,
        method: &str,
        group: &CoachingGroup,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let router = MessagingRoutes::routes_with_adapters(
            Arc::clone(&e2e.resources),
            Arc::<OfflineSendAdapters>::clone(&e2e.adapters),
        );
        let path = format!("/api/groups/{}/chat-connection", group.id);
        let request = match method {
            "GET" => AxumTestRequest::get(&path),
            "POST" => AxumTestRequest::post(&path).json(&body.unwrap_or(Value::Null)),
            _ => AxumTestRequest::delete(&path),
        };
        let response = request
            .header("authorization", &format!("Bearer {token}"))
            .send(router)
            .await;
        let status = response.status_code();
        let text = response.text();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_group_panel_reads_and_disconnects_and_only_managers_may_change_it() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let athlete = e2e.linked_member(true).await;
        let outsider = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        enrol(&e2e, &group, &athlete, GroupRole::Member).await;
        let (coach_token, athlete_token, outsider_token) = (
            token_for(&e2e, &coach).await,
            token_for(&e2e, &athlete).await,
            token_for(&e2e, &outsider).await,
        );

        let (status, body) = rest(&e2e, "GET", &group, &coach_token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], json!(false));
        assert_eq!(body["can_manage"], json!(true));

        let (status, _) = rest(&e2e, "GET", &group, &outsider_token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a non-member learns nothing");

        let (status, _) = rest(
            &e2e,
            "POST",
            &group,
            &athlete_token,
            Some(json!({ "channel": "telegram" })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member cannot mint a code");

        let (status, _) = rest(
            &e2e,
            "POST",
            &group,
            &coach_token,
            Some(json!({ "channel": "whatsapp" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "WhatsApp is not built yet");

        let code = mint(&e2e, &group, &coach).await;
        say(&e2e, &coach, &format!("/start {code}")).await;

        let (status, body) = rest(&e2e, "GET", &group, &athlete_token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], json!(true));
        assert_eq!(body["channel"], json!("telegram"));
        assert_eq!(body["chat_title"], json!("Command E2e Group"));
        assert_eq!(body["can_manage"], json!(false));

        let (status, _) = rest(
            &e2e,
            "POST",
            &group,
            &coach_token,
            Some(json!({ "channel": "telegram" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "one chat app per group");

        let (status, _) = rest(&e2e, "DELETE", &group, &athlete_token, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = rest(&e2e, "DELETE", &group, &coach_token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(chat_group(&e2e).await.is_none());
        let (status, _) = rest(&e2e, "DELETE", &group, &coach_token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nothing left to disconnect");
    }
}
