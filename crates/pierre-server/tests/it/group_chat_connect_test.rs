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
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::messaging::MessageContent;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::{json, Value};
    use serial_test::serial;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources_with_chat_provider,
        create_test_server_resources_with_chat_provider_on_adapters,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::command_e2e::{
        CommandE2e, Member, RoomE2e, RouterLlm, WebhookOutcome, BOT_USERNAME,
    };
    use crate::helpers::drained_turn::wait_for_turns_to_finish;
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

    /// Enrol `m` as a member who shares their training with the group.
    async fn enrol_sharing(e2e: &CommandE2e, group: &CoachingGroup, m: &Member) {
        enrol(e2e, group, m, GroupRole::Member).await;
        e2e.resources
            .common
            .repos
            .groups
            .update_peer_sharing_consent(&group.id.to_string(), m.user_id, true)
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

    /// Wait until every reply the bot spawned has been delivered: a chat's
    /// replies go out from a detached task, after the webhook answered.
    async fn delivered(e2e: &CommandE2e) {
        assert!(
            wait_for_turns_to_finish(&e2e.resources, Duration::from_secs(10)).await,
            "bug: the bot's replies were still being delivered 10 s after the webhook answered"
        );
    }

    /// Sends the bot made into the chat.
    fn chat_sends(e2e: &CommandE2e) -> Vec<String> {
        e2e.adapters
            .sends()
            .into_iter()
            .filter(|s| s.recipient_id == CHAT.to_string())
            .map(|s| match s.content {
                // The ingress answers in plain text; the platform's own posts
                // (the group chat poster) go out as rich text, escaped.
                MessageContent::Text { body } | MessageContent::RichText { body } => {
                    body.replace('\\', "")
                }
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
        delivered(&e2e).await;

        let connected = reread(&e2e, group.id).await;
        assert_eq!(
            connected.channel_type.as_deref(),
            Some("telegram"),
            "bug: the web group is bound to the Telegram chat"
        );
        assert_eq!(
            connected.channel_chat_id.as_deref(),
            Some(CHAT.to_string().as_str()),
            "bug: the web group is bound to the chat the code was redeemed in"
        );
        let connection = connected
            .chat_connection
            .expect("connected from the web app");
        assert_eq!(
            connection.channel_tenant_id, e2e.bot_tenant,
            "bug: the connection records the bot tenant that serves the chat"
        );
        assert_eq!(
            connection.chat_title.as_deref(),
            Some("Command E2e Group"),
            "bug: the chat's title is kept for the web panel"
        );
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
        assert_eq!(
            reread(&e2e, group.id).await.channel_type,
            None,
            "bug: a member who neither owns nor coaches the group connects nothing"
        );

        say(&e2e, &coach, &format!("/group link {code}")).await;
        assert_eq!(
            chat_group(&e2e).await.map(|g| g.id),
            Some(group.id),
            "bug: the owner's unspent code connects the chat"
        );

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
        delivered(&e2e).await;
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
        delivered(&e2e).await;
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
        assert_eq!(
            record.group_id,
            Some(group.id.to_string()),
            "bug: a member's room conversation is bound to the web group"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_chat_already_bound_to_a_group_its_owner_runs_moves_to_the_web_group() {
        let e2e = Arc::clone(&start().await);
        let coach = e2e.linked_member(true).await;
        let old = RoomE2e::bind_room(Arc::clone(&e2e), CHAT, GroupRespondMode::All, &coach).await;
        // The owner already talks in the chat-created group's room.
        e2e.llm.set_turn_replies(&["OK."]);
        say(&e2e, &coach, &format!("@{BOT_USERNAME} hi")).await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "bug: the owner's turn in the chat-created room reaches the agent"
        );
        let before = e2e
            .conversation_id(&coach, e2e.bot_tenant, &CHAT.to_string())
            .await
            .expect("the owner has a room session");
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        let code = mint(&e2e, &group, &coach).await;

        say(&e2e, &coach, &format!("/start {code}")).await;

        assert_eq!(
            chat_group(&e2e).await.map(|g| g.id),
            Some(group.id),
            "bug: the chat moves to the web group when its owner connects it"
        );
        let left = reread(&e2e, old.group_id).await;
        assert_eq!(
            left.channel_type, None,
            "bug: the old group loses only its chat"
        );
        assert!(left.is_active, "bug: the old group and its history stay");

        // Their next turn runs in a conversation of the web group, never in
        // the one written with the old group's context.
        say(&e2e, &coach, &format!("@{BOT_USERNAME} and now?")).await;
        assert!(
            e2e.wait_llm_turns(2).await,
            "bug: the owner's turn in the moved chat reaches the agent"
        );
        let after = e2e
            .conversation_id(&coach, e2e.bot_tenant, &CHAT.to_string())
            .await
            .expect("the owner still has a room session");
        assert_ne!(
            after, before,
            "bug: the session leaves the old group's conversation"
        );
        let record = e2e
            .resources
            .common
            .repos
            .chat
            .get_conversation(&after, &coach.user_id.to_string(), e2e.bot_tenant)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.group_id,
            Some(group.id.to_string()),
            "bug: the new conversation carries the web group's context"
        );
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

        assert_eq!(
            chat_group(&e2e).await.map(|g| g.id),
            Some(old.group_id),
            "bug: a chat bound to a group someone else owns stays with that group"
        );
        assert_eq!(
            reread(&e2e, group.id).await.channel_type,
            None,
            "bug: the refused web group stays bound to no chat"
        );
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
    async fn a_disconnected_chat_goes_silent_and_creates_no_group() {
        let e2e = start().await;
        let coach = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        let code = mint(&e2e, &group, &coach).await;
        say(&e2e, &coach, &format!("/start {code}")).await;
        e2e.llm.set_turn_replies(&["OK."]);
        say(&e2e, &coach, &format!("@{BOT_USERNAME} hi")).await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "bug: the owner's turn in the connected chat reaches the agent"
        );

        let connected = reread(&e2e, group.id).await;
        let released = e2e
            .resources
            .group_service()
            .disconnect_chat(&connected, coach.user_id)
            .await
            .unwrap()
            .expect("the group had a chat");
        assert_eq!(
            released.channel_tenant_id, e2e.bot_tenant,
            "bug: the released binding names the bot tenant that serves the chat"
        );
        // Let the connected turn's follow-through settle before counting.
        delivered(&e2e).await;
        let sends_before = chat_sends(&e2e).len();
        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);

        // Silent for everyone, the owner included: nothing stored, no turn.
        let question = format!("@{BOT_USERNAME} still there?");
        let outcome = say(&e2e, &coach, &question).await;
        assert_eq!(
            outcome.messages_stored(),
            0,
            "bug: nothing is dispatched in a disconnected chat"
        );
        delivered(&e2e).await;
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_before,
            "bug: the agent answers nothing in a disconnected chat"
        );
        assert_eq!(
            e2e.count_inbound_with_body(&question).await,
            0,
            "bug: a disconnected chat's messages are not stored"
        );
        assert_eq!(
            chat_sends(&e2e).len(),
            sends_before,
            "bug: the bot posts nothing into a disconnected chat"
        );
        assert!(
            chat_group(&e2e).await.is_none(),
            "bug: no group is auto-created for a disconnected chat"
        );

        // A connect code still works there, so the chat can be connected again.
        let again = mint(&e2e, &reread(&e2e, group.id).await, &coach).await;
        say(&e2e, &coach, &format!("/start {again}")).await;
        assert_eq!(
            chat_group(&e2e).await.map(|g| g.id),
            Some(group.id),
            "bug: a disconnected chat can be connected again"
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
        assert_eq!(status, StatusCode::OK, "bug: the owner reads the panel");
        assert_eq!(
            body["connected"],
            json!(false),
            "bug: a fresh web group has no chat"
        );
        assert_eq!(
            body["can_manage"],
            json!(true),
            "bug: the owner may connect the group"
        );

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
        assert_eq!(status, StatusCode::OK, "bug: a member reads the panel");
        assert_eq!(
            body["connected"],
            json!(true),
            "bug: the panel reads the chat connected"
        );
        assert_eq!(
            body["channel"],
            json!("telegram"),
            "bug: the panel names the chat app"
        );
        assert_eq!(
            body["chat_title"],
            json!("Command E2e Group"),
            "bug: the panel names the chat"
        );
        assert_eq!(
            body["can_manage"],
            json!(false),
            "bug: a plain member may not change the connection"
        );

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
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "bug: a plain member may not disconnect"
        );
        let (status, _) = rest(&e2e, "DELETE", &group, &coach_token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "bug: the owner disconnects");
        assert!(
            chat_group(&e2e).await.is_none(),
            "bug: after the disconnect the chat finds no group"
        );
        let (status, _) = rest(&e2e, "DELETE", &group, &coach_token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nothing left to disconnect");
    }

    /// A test whose platform posts (the group chat poster) land in the same
    /// captured adapters as the webhook's replies.
    async fn start_capturing() -> Arc<CommandE2e> {
        let llm = RouterLlm::new();
        let adapters = Arc::new(OfflineSendAdapters::default());
        let resources = create_test_server_resources_with_chat_provider_on_adapters(
            Arc::<RouterLlm>::clone(&llm),
            Arc::<OfflineSendAdapters>::clone(&adapters),
        )
        .await
        .unwrap();
        CommandE2e::start_on(resources, llm, adapters).await
    }

    /// Post `text` in `m`'s own app thread of `group`'s room.
    async fn post_in_app(e2e: &CommandE2e, group: &CoachingGroup, m: &Member, text: &str) {
        let token = token_for(e2e, m).await;
        let opened = AxumTestRequest::post(&format!("/api/chat/groups/{}/room", group.id))
            .header("authorization", &format!("Bearer {token}"))
            .send(ChatRoutes::routes(Arc::clone(&e2e.resources)))
            .await;
        let room: Value = opened.json();
        let room_id = room["id"].as_str().expect("the room opens").to_owned();
        let sent = AxumTestRequest::post(&format!("/api/chat/conversations/{room_id}/messages"))
            .header("authorization", &format!("Bearer {token}"))
            .json(&json!({ "content": text }))
            .send(ChatRoutes::routes(Arc::clone(&e2e.resources)))
            .await;
        assert!(
            sent.status_code().is_success(),
            "the app post is accepted: {}",
            sent.status_code()
        );
    }

    /// Wait until the chat holds a send containing `needle`, or give up.
    async fn wait_chat_send(e2e: &CommandE2e, needle: &str) -> bool {
        for _ in 0..50 {
            if chat_sends(e2e).iter().any(|body| body.contains(needle)) {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn an_app_post_and_its_reply_reach_the_chat_only_when_the_room_may_read_them() {
        let e2e = start_capturing().await;
        let coach = e2e.linked_member(true).await;
        let alice = e2e.linked_member(true).await;
        let bob = e2e.linked_member(true).await;
        let group = web_group(&e2e, &coach, "Les Rouleurs").await;
        enrol_sharing(&e2e, &group, &alice).await;
        enrol(&e2e, &group, &bob, GroupRole::Member).await;
        let code = mint(&e2e, &group, &coach).await;
        say(&e2e, &coach, &format!("/start {code}")).await;
        delivered(&e2e).await;

        // Alice shares with the group: her post crosses attributed, and the
        // agent's reply follows it.
        e2e.llm
            .set_turn_replies(&["Departure at 7, see you there."]);
        post_in_app(&e2e, &group, &alice, "Ride at 7 tomorrow?").await;
        assert!(
            wait_chat_send(&e2e, ": Ride at 7 tomorrow?").await,
            "the post reaches the chat, attributed: {:?}",
            chat_sends(&e2e)
        );
        assert!(
            wait_chat_send(&e2e, "Departure at 7, see you there.").await,
            "the reply follows it: {:?}",
            chat_sends(&e2e)
        );
        assert!(
            chat_sends(&e2e)
                .iter()
                .all(|body| !body.contains("@example.com")),
            "no email address reaches the chat"
        );

        // Bob does not share: the room cannot read him in the app, so the
        // chat does not either.
        let before = chat_sends(&e2e).len();
        e2e.llm.set_turn_replies(&["Bob's private answer."]);
        post_in_app(&e2e, &group, &bob, "My knee hurts").await;
        assert!(e2e.wait_llm_turns(2).await, "Bob's turn ran in the app");
        sleep(Duration::from_millis(500)).await;
        assert_eq!(
            chat_sends(&e2e).len(),
            before,
            "nothing of Bob's crosses: {:?}",
            chat_sends(&e2e)
        );

        // A turn that came from the chat is answered there once, by the
        // ingress, and never relayed a second time.
        let before = chat_sends(&e2e).len();
        e2e.llm.set_turn_replies(&["Answered in the chat."]);
        say(&e2e, &alice, &format!("@{BOT_USERNAME} and the route?")).await;
        assert!(
            wait_chat_send(&e2e, "Answered in the chat.").await,
            "bug: the chat's own turn is answered in the chat"
        );
        sleep(Duration::from_millis(500)).await;
        let echoes = chat_sends(&e2e)[before..]
            .iter()
            .filter(|body| {
                body.contains("Answered in the chat.") || body.contains("and the route?")
            })
            .count();
        assert_eq!(
            echoes,
            1,
            "the chat's own turn crosses once: {:?}",
            chat_sends(&e2e)
        );
    }
}
