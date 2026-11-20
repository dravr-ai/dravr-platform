// ABOUTME: A voice note or an uncaptioned photo draws the text-only reply and starts no agent turn
// ABOUTME: Drives Telegram and WhatsApp media updates through the real webhook, ingress and turn runner

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Nothing downloads, transcribes or analyses inbound media (carnet#1670), so a
//! message with nothing but media sent to the agent gets one short localized
//! reply saying the agent reads text, and it never reaches the model. A
//! captioned photo is answered from its caption. A photo posted into a room
//! for its people draws no reply at all.
//!
//! canot parses a Telegram photo as media and every other non-text type but a
//! location (voice, video, document, sticker) as its unsupported-type
//! placeholder, and neither reaches the model as text. On `WhatsApp` that
//! placeholder is mostly a reaction, so it is dropped before anything else
//! sees it: no link prompt, no session, no ledger row, no reply.

#[cfg(feature = "client-messaging")]
mod media_text_only {
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use hmac::Hmac;
    use pierre_contremaitre::messaging_strings::KEY_MEDIA_TEXT_ONLY;
    use pierre_core::models::groups::GroupRespondMode;
    use pierre_core::models::messaging::MessageContent;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::locale::resolve_user_locale;
    use serde_json::{json, Value};
    use serial_test::serial;
    use sha2::Sha256;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::command_e2e::{CommandE2e, Member, RoomE2e, RouterLlm, BOT_ID};
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// The caption the athlete wrote under a photo. Unique, so a coaching
    /// request that carries it is provably this photo's turn.
    const CAPTION: &str = "Voici mon profil de sortie longue de dimanche";
    /// The Telegram supergroup the room test binds.
    const ROOM_CHAT_ID: i64 = -100_167_000;
    /// The signing secret of the bot tenant's `WhatsApp` channel.
    const WA_SECRET: &str = "media_text_only_wa_secret";
    /// The business number the `WhatsApp` channel answers on.
    const WA_PHONE_NUMBER_ID: &str = "15550001670";
    /// The `WhatsApp` Business Account the channel belongs to.
    const WA_BUSINESS_ID: &str = "media_text_only_wa_business";

    async fn start() -> Arc<CommandE2e> {
        let llm = RouterLlm::new();
        let resources =
            create_test_server_resources_with_chat_provider(Arc::<RouterLlm>::clone(&llm))
                .await
                .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);
        e2e
    }

    async fn expected_reply(e2e: &CommandE2e, member: &Member) -> String {
        let locale =
            resolve_user_locale(e2e.resources.common.repos.users.as_ref(), member.user_id).await;
        e2e.resources
            .mcp
            .messaging_strings_registry
            .get(KEY_MEDIA_TEXT_ONLY, &locale)
    }

    async fn session_of(e2e: &CommandE2e, member: &Member) -> String {
        e2e.session_id(member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the message must forge the athlete's DM session")
    }

    /// Wait (≤30 s) until every delivery and turn the webhooks spawned has
    /// finished, so what the ledger and the offline adapters hold is final and
    /// an absence read after it is a real absence.
    async fn settle(e2e: &CommandE2e) {
        for _ in 0..1200 {
            if e2e.resources.common.turns.is_empty() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "{} spawned deliveries or turns still running after 30 s",
            e2e.resources.common.turns.len()
        );
    }

    /// The text of every message the offline adapters were handed, in order.
    fn sent_bodies(e2e: &CommandE2e) -> Vec<String> {
        e2e.adapters
            .sends()
            .into_iter()
            .filter_map(|message| match message.content {
                MessageContent::Text { body }
                | MessageContent::RichText { body }
                | MessageContent::Card { body, .. } => Some(body),
                MessageContent::Media { .. } | MessageContent::Location { .. } => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn an_uncaptioned_photo_gets_the_text_only_reply_and_no_turn() {
        let e2e = start().await;
        let member = e2e.linked_member(true).await;

        e2e.send_dm_content(
            &member,
            json!({ "photo": [{ "file_id": "small" }, { "file_id": "large" }] }),
        )
        .await;
        let session = session_of(&e2e, &member).await;
        e2e.wait_outbound_for_session(&session, 1).await;

        assert_eq!(
            e2e.outbound_bodies_for_session(&session).await,
            vec![expected_reply(&e2e, &member).await],
            "the athlete is told once that the agent reads text"
        );
        assert_eq!(
            e2e.inbound_rows_for_session(member.home_tenant, &session)
                .await,
            vec![("media".to_owned(), None)],
            "the photo is still ledgered, with no body to hand a model"
        );
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            0,
            "an uncaptioned photo must not start a coaching turn"
        );

        let rows = e2e
            .resources
            .common
            .repos
            .messaging
            .get_session_messages(&session, member.home_tenant, 100, 0)
            .await
            .unwrap();
        let turns: HashSet<&str> = rows
            .iter()
            .map(|row| row["correlation_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            rows.len(),
            2,
            "the ledger holds the photo and its one reply: {rows:?}"
        );
        assert_eq!(
            turns.len(),
            1,
            "the reply answers the photo's turn, so both rows share its id: {rows:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_voice_note_gets_the_text_only_reply_and_the_placeholder_never_reaches_the_agent() {
        let e2e = start().await;
        let member = e2e.linked_member(true).await;

        e2e.send_dm_content(
            &member,
            json!({ "voice": { "file_id": "v1", "duration": 4, "mime_type": "audio/ogg" } }),
        )
        .await;
        let session = session_of(&e2e, &member).await;
        e2e.wait_outbound_for_session(&session, 1).await;

        assert_eq!(
            e2e.outbound_bodies_for_session(&session).await,
            vec![expected_reply(&e2e, &member).await],
            "a Telegram voice note arrives as the placeholder and is told the agent reads text"
        );
        assert_eq!(
            e2e.inbound_rows_for_session(member.home_tenant, &session)
                .await,
            vec![("text".to_owned(), None)],
            "the placeholder is ledgered with no body: it is not the athlete's words"
        );
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            0,
            "the unsupported-type placeholder must not reach the model"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_text_only_reply_speaks_the_senders_language() {
        let e2e = start().await;
        let member = e2e.linked_member(true).await;
        e2e.resources
            .common
            .repos
            .users
            .update_locale(member.user_id, "de")
            .await
            .unwrap();
        let registry = &e2e.resources.mcp.messaging_strings_registry;
        let german = registry.get(KEY_MEDIA_TEXT_ONLY, "de");
        assert_ne!(
            german,
            registry.get(KEY_MEDIA_TEXT_ONLY, "fr"),
            "the German reply must differ from the default French one, or this test proves nothing"
        );

        e2e.send_dm_content(&member, json!({ "photo": [{ "file_id": "large" }] }))
            .await;
        let session = session_of(&e2e, &member).await;
        e2e.wait_outbound_for_session(&session, 1).await;

        assert_eq!(
            e2e.outbound_bodies_for_session(&session).await,
            vec![german],
            "the reply is written in the athlete's own language, not the default"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_captioned_photo_is_still_answered_from_its_caption() {
        let e2e = start().await;
        let member = e2e.linked_member(true).await;

        e2e.send_dm_content(
            &member,
            json!({ "photo": [{ "file_id": "large" }], "caption": CAPTION }),
        )
        .await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "a captioned photo must reach the LLM"
        );
        let session = session_of(&e2e, &member).await;
        e2e.wait_outbound_for_session(&session, 1).await;

        let requests = e2e.llm.seen_turn_requests.lock().unwrap().clone();
        assert!(
            !requests.is_empty(),
            "the caption's turn must have called the model"
        );
        assert!(
            requests.iter().all(|request| request.contains(CAPTION)),
            "the turn is the caption's: {requests:?}"
        );
        let reply = expected_reply(&e2e, &member).await;
        assert!(
            !e2e.outbound_bodies_for_session(&session)
                .await
                .contains(&reply),
            "a captioned photo is not told the agent reads text only"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_room_photo_draws_the_reply_only_when_it_addresses_the_bot() {
        let e2e = start().await;
        let owner = e2e.linked_member(true).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            ROOM_CHAT_ID,
            GroupRespondMode::All,
            &owner,
        )
        .await;
        let reply = expected_reply(&e2e, &owner).await;

        // A room that answers every message still leaves a photo posted for
        // its people alone.
        room.send_room_content(&owner, json!({ "photo": [{ "file_id": "room" }] }))
            .await;
        settle(&e2e).await;
        let session = room
            .session_id(&owner)
            .await
            .expect("the photo must forge the owner's room session");
        assert_eq!(
            e2e.inbound_rows_for_session(e2e.bot_tenant, &session).await,
            vec![("media".to_owned(), None)],
            "the room photo is ledgered under the room's tenant"
        );
        assert_eq!(
            e2e.outbound_count_containing(&reply).await,
            0,
            "a photo posted for the room's people must not be told the agent reads text"
        );

        // The control: the same bare photo as a reply to the bot is for the agent.
        room.send_room_content(
            &owner,
            json!({
                "photo": [{ "file_id": "room-reply" }],
                "reply_to_message": {
                    "message_id": 1,
                    "from": { "id": BOT_ID, "is_bot": true, "first_name": "Dravr" },
                    "text": "Solid week so far."
                }
            }),
        )
        .await;
        e2e.wait_outbound_containing(&reply, 1).await;
        settle(&e2e).await;
        assert_eq!(
            e2e.outbound_count_containing(&reply).await,
            1,
            "only the photo that addressed the bot is told the agent reads text"
        );
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            0,
            "neither room photo may start a coaching turn"
        );
    }

    async fn configure_whatsapp(e2e: &CommandE2e) {
        let db: &dyn MessagingRepository = &*e2e.resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: e2e.bot_tenant,
            channel_type: "whatsapp",
            api_key: Some("media_text_only_wa_token"),
            api_secret: None,
            webhook_secret: Some(WA_SECRET),
            verify_token: None,
            account_id: Some(WA_BUSINESS_ID),
            phone_number: Some(WA_PHONE_NUMBER_ID),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    }

    /// Post a signed `WhatsApp` Cloud API webhook carrying one message from
    /// `sender`, whose `type` and typed object are `message`, through the
    /// fixture's offline adapters. Returns the webhook's acknowledgement.
    async fn post_whatsapp(e2e: &CommandE2e, sender: &str, wamid: &str, message: Value) -> Value {
        use hmac::Mac;

        let mut entry = json!({ "from": sender, "id": wamid, "timestamp": "1700000000" });
        let fields = message.as_object().expect("message is a JSON object");
        for (key, value) in fields {
            entry[key] = value.clone();
        }
        let payload = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": WA_BUSINESS_ID,
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15550001670",
                            "phone_number_id": WA_PHONE_NUMBER_ID
                        },
                        "messages": [entry]
                    },
                    "field": "messages"
                }]
            }]
        });
        let mut mac = Hmac::<Sha256>::new_from_slice(WA_SECRET.as_bytes()).unwrap();
        mac.update(&serde_json::to_vec(&payload).unwrap());
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        let router = MessagingRoutes::routes_with_adapters(
            Arc::clone(&e2e.resources),
            Arc::<OfflineSendAdapters>::clone(&e2e.adapters),
        );
        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &signature)
            .json(&payload)
            .send(router)
            .await;
        let status = resp.status_code();
        let body: Value = resp.json();
        assert_eq!(
            status,
            StatusCode::OK,
            "the WhatsApp webhook must accept a signed update: {body}"
        );
        body
    }

    /// A `WhatsApp` reaction: a thumbs-up on an earlier message.
    fn reaction() -> Value {
        json!({
            "type": "reaction",
            "reaction": { "message_id": "wamid.earlier", "emoji": "\u{1f44d}" }
        })
    }

    /// A `WhatsApp` photo with no caption.
    fn bare_photo() -> Value {
        json!({
            "type": "image",
            "image": { "id": "wa-media-1", "mime_type": "image/jpeg" }
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_whatsapp_reaction_from_an_unlinked_sender_draws_no_link_prompt() {
        let e2e = start().await;
        configure_whatsapp(&e2e).await;
        let sender = "15557771670";

        let ack = post_whatsapp(&e2e, sender, "wamid.reaction-unlinked", reaction()).await;
        assert_eq!(
            ack["messages_received"], 1,
            "canot must parse the reaction into one inbound message: {ack}"
        );
        assert_eq!(
            ack["messages_stored"], 0,
            "a reaction must not be stored: {ack}"
        );
        settle(&e2e).await;
        assert_eq!(
            sent_bodies(&e2e),
            Vec::<String>::new(),
            "a reaction from an unlinked sender must not draw a link prompt"
        );

        // The control: the same unlinked sender's photo does draw the prompt,
        // so the empty capture above is the drop and not a dead adapter.
        post_whatsapp(&e2e, sender, "wamid.photo-unlinked", bare_photo()).await;
        settle(&e2e).await;
        assert!(
            !sent_bodies(&e2e).is_empty(),
            "an unlinked sender's photo must draw the link prompt"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_linked_athletes_whatsapp_reaction_leaves_no_trace_and_a_bare_photo_is_answered() {
        let e2e = start().await;
        configure_whatsapp(&e2e).await;
        let member = e2e.linked_member(true).await;
        let number = "15557771671";
        let db: &dyn MessagingRepository = &*e2e.resources.common.repos.messaging;
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: e2e.bot_tenant,
            user_id: &member.user_id.to_string(),
            channel_type: "whatsapp",
            channel_user_id: number,
            display_name: Some("Media Text Only Member"),
        })
        .await
        .unwrap();

        let ack = post_whatsapp(&e2e, number, "wamid.reaction-linked", reaction()).await;
        assert_eq!(
            ack["messages_stored"], 0,
            "a linked athlete's reaction must not be stored: {ack}"
        );
        settle(&e2e).await;
        let no_session = db
            .get_session_by_channel_identity(member.home_tenant, "whatsapp", number, None)
            .await
            .unwrap();
        assert!(
            no_session.is_none(),
            "a reaction must not open a session: {no_session:?}"
        );
        assert_eq!(
            sent_bodies(&e2e),
            Vec::<String>::new(),
            "a reaction must not be answered"
        );

        let ack = post_whatsapp(&e2e, number, "wamid.photo-linked", bare_photo()).await;
        assert_eq!(ack["messages_stored"], 1, "the photo must be stored: {ack}");
        settle(&e2e).await;
        let reply = expected_reply(&e2e, &member).await;
        assert_eq!(
            sent_bodies(&e2e),
            vec![reply.clone()],
            "the bare photo, and only it, is told the agent reads text"
        );
        let session = db
            .get_session_by_channel_identity(member.home_tenant, "whatsapp", number, None)
            .await
            .unwrap()
            .and_then(|s| s["id"].as_str().map(str::to_owned))
            .expect("the photo must open the athlete's WhatsApp session");
        assert_eq!(
            e2e.inbound_rows_for_session(member.home_tenant, &session)
                .await,
            vec![("media".to_owned(), None)],
            "the ledger holds the photo and nothing of the reaction"
        );
        assert_eq!(
            e2e.outbound_bodies_for_session(&session).await,
            vec![reply],
            "the reply is ledgered on the athlete's session"
        );
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            0,
            "neither the reaction nor the photo may start a coaching turn"
        );
    }
}
