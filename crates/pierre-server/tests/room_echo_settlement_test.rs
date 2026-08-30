// ABOUTME: A shared room that keeps a slash-command echo must be told where the answer went
// ABOUTME: Pins both settlement branches — echo deleted stays silent, echo surviving speaks once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::similar_names)]

//! `/plan` typed in a Telegram group produced nothing visible, twice on
//! 2026-08-29. Both turns were answered — into the caller's DM, deliberately,
//! because a room can hold several athletes and one member's plan is not the
//! room's business. What was missing was any word about that, because the echo
//! delete is best-effort and the bot is not an admin in most rooms.
//!
//! The settlement decides what the room is left seeing, and it was covered only
//! by locale-string tests: nothing asserted that a room which keeps the echo
//! gets the notice. That gap is why a production turn could not be told apart
//! from a silent one.

mod common;

// The channel fakes live in a helpers subdir rather than a top-level test
// binary, and are pulled in by path so this test shares the same copy as the
// notifier render tests.
#[path = "helpers/messaging_fixtures.rs"]
// Only the channel fake is needed here; the fixture's db/seed re-exports go
// unused in this binary, which is not a reason to keep a second copy of it.
#[allow(unused_imports)]
mod messaging_fixtures;

#[cfg(feature = "client-messaging")]
mod room_echo_tests {
    use crate::common::create_test_server_resources;
    use crate::messaging_fixtures::CapturingChannel;
    use async_trait::async_trait;
    use chrono::Utc;
    use http::HeaderMap;
    use pierre_contremaitre::messaging_strings::KEY_SLASH_ANSWERED_PRIVATELY;
    use pierre_core::models::messaging::{
        ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage,
        MessageContent, OutgoingMessage,
    };
    use pierre_core::models::TenantId;
    use pierre_database::backends::{MessagingRepository, UpsertChannelConfigParams};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::services::messaging_ingress::room_echo::{
        settle_room_echo, RoomEchoSettlement,
    };
    use pierre_messaging::channel::MessagingChannel;
    use pierre_messaging::error::MessagingResult;
    use pierre_messaging::turn::ConversationTurnId;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    const ROOM: &str = "-5284201188";
    const ECHO_ID: &str = "4471";

    /// A channel that really removes the message, as Telegram does when the bot
    /// is an admin with `can_delete_messages`. Records each deletion so the test
    /// can prove one was attempted rather than skipped.
    struct DeletingChannel {
        deleted: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl MessagingChannel for DeletingChannel {
        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        fn verify_signature(&self, _headers: &HeaderMap, _body: &[u8]) -> MessagingResult<()> {
            Ok(())
        }

        async fn receive(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> MessagingResult<Vec<IncomingMessage>> {
            Ok(Vec::new())
        }

        fn render(&self, _msg: &OutgoingMessage) -> MessagingResult<Value> {
            Ok(Value::Null)
        }

        async fn send(
            &self,
            msg: &OutgoingMessage,
            _config: &ChannelConfig,
        ) -> MessagingResult<DeliveryReceipt> {
            Ok(DeliveryReceipt {
                message_id: "deleting-channel".to_owned(),
                channel_message_id: None,
                status: DeliveryStatus::Sent,
                timestamp: Utc::now(),
                turn_id: msg.turn_id,
            })
        }

        async fn send_raw(
            &self,
            _payload: &Value,
            turn_id: ConversationTurnId,
            _config: &ChannelConfig,
        ) -> MessagingResult<DeliveryReceipt> {
            Ok(DeliveryReceipt {
                message_id: "deleting-channel".to_owned(),
                channel_message_id: None,
                status: DeliveryStatus::Sent,
                timestamp: Utc::now(),
                turn_id,
            })
        }

        async fn delete_message(
            &self,
            conversation_id: &str,
            channel_message_id: &str,
            _config: &ChannelConfig,
        ) -> MessagingResult<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((conversation_id.to_owned(), channel_message_id.to_owned()));
            Ok(())
        }
    }

    async fn tenant_with_telegram_config(resources: &ServerContext) -> TenantId {
        let tenant_id = TenantId::generate();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some("room_echo_secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:ROOM_ECHO_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
        tenant_id
    }

    fn settlement<'a>(
        resources: &'a ServerContext,
        tenant_id: TenantId,
        adapter: &'a Arc<dyn MessagingChannel>,
        user_id: &'a str,
    ) -> RoomEchoSettlement<'a> {
        RoomEchoSettlement {
            resources,
            db: &*resources.common.repos.messaging,
            tenant_id,
            channel: "telegram",
            channel_type: ChannelType::Telegram,
            adapter,
            room_id: ROOM,
            channel_message_id: ECHO_ID,
            user_id,
            sender_id: "6606309489",
        }
    }

    /// A channel that cannot delete leaves the command on screen, so the room is
    /// owed one line saying where the answer went.
    ///
    /// `CapturingChannel` overrides no deletion, exactly like the `WhatsApp` and
    /// Messenger adapters. Until dravr-canot v0.4.26 the trait default returned
    /// `Ok(())` for those, which the settlement read as "the echo is gone" — so
    /// it stayed silent over a command still sitting in the room. This test
    /// fails on that default.
    #[tokio::test]
    async fn a_room_that_keeps_the_echo_is_told_where_the_answer_went() {
        let resources = create_test_server_resources().await.unwrap();
        let tenant_id = tenant_with_telegram_config(&resources).await;
        let adapter: Arc<dyn MessagingChannel> = Arc::new(CapturingChannel::default());
        let user_id = Uuid::new_v4().to_string();

        let notice = settle_room_echo(settlement(&resources, tenant_id, &adapter, &user_id))
            .await
            .expect("a room still showing the command must be told where the answer went");

        assert_eq!(
            notice.recipient_id, ROOM,
            "the notice belongs in the room, not in the caller's DM"
        );

        let MessageContent::Text { body } = &notice.content else {
            panic!("expected a plain-text notice, got {:?}", notice.content);
        };
        assert!(!body.is_empty(), "the notice must carry real text");

        // The exact string the registry serves, so a locale that silently
        // resolved to nothing cannot pass as a delivered notice.
        let expected = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_SLASH_ANSWERED_PRIVATELY, "fr");
        assert_eq!(body, &expected);

        // It says where to look, never any part of what was said.
        assert!(
            !body.contains("/plan"),
            "the notice must not echo the command it is standing in for: {body}"
        );
    }

    /// With the echo gone the room shows nothing and is owed nothing — a notice
    /// there would be noise about a message no member can see.
    #[tokio::test]
    async fn a_room_whose_echo_was_deleted_stays_silent() {
        let resources = create_test_server_resources().await.unwrap();
        let tenant_id = tenant_with_telegram_config(&resources).await;
        let deleting = Arc::new(DeletingChannel {
            deleted: Mutex::new(Vec::new()),
        });
        let adapter: Arc<dyn MessagingChannel> = deleting.clone();
        let user_id = Uuid::new_v4().to_string();

        let notice = settle_room_echo(settlement(&resources, tenant_id, &adapter, &user_id)).await;

        assert!(
            notice.is_none(),
            "nothing is owed to a room whose command echo is gone"
        );

        let deleted = deleting.deleted.lock().unwrap();
        assert_eq!(
            deleted.len(),
            1,
            "the settlement must actually attempt the deletion"
        );
        assert_eq!(deleted[0], (ROOM.to_owned(), ECHO_ID.to_owned()));
    }
}
