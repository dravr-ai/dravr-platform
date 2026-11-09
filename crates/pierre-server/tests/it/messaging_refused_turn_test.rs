// ABOUTME: Guards that a messaging turn the LLM health gate refuses is closed on the thread, not left awaiting
// ABOUTME: The question is on file; the interrupted notice row closes it, and the channel gets only the apology
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A refused channel turn on the shared personal thread.
//!
//! The health gate refuses a turn only once its question is stored
//! (carnet#1362), and an athlete's messaging DM writes into the same personal
//! thread the app reads (carnet#840). So a refusal on `WhatsApp` leaves a
//! question on that thread; without a row closing it, the app reads it as a
//! reply on its way for as long as the turn watchdog allows. This suite pins
//! the close: the question, then the interrupted notice, nothing awaiting —
//! and on the channel the one apology the dispatcher already sends.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod refused {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_chat_pipeline::turn_stop;
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
    use pierre_database::backends::MessagingRepository;
    use pierre_database::database::MessageRecord;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::messaging_ingress::turn_watchdog;
    use tokio::time;

    use crate::common::{
        create_test_server_resources_with_chat_provider_and_config, test_server_config,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::drained_turn::{
        compute_whatsapp_sig, create_active_user, link_channel, outbound_bodies,
        setup_whatsapp_config, wait_for_turns_to_finish, whatsapp_text_payload, HangingProvider,
    };

    /// A turn the health gate refuses is closed on the personal thread with
    /// the interrupted notice, and the channel hears only the apology.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_channel_turn_is_closed_on_the_thread() {
        let config = test_server_config();
        let ceiling = turn_watchdog(&config.timeouts);
        // The gate refuses before any provider call; a provider that never
        // answers makes a gate that let the turn through hang, not pass.
        let resources = create_test_server_resources_with_chat_provider_and_config(
            Arc::new(HangingProvider),
            config,
        )
        .await
        .unwrap();
        resources
            .common
            .llm_health
            .record_unhealthy("mock", "probe failed")
            .await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) = create_active_user(&resources, "refused-dm@example.com").await;
        let wa_secret = "wa_refused_turn_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;
        let sender_id = "15550009011";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let sent = "combien de watts sur ma dernière sortie?";
        let payload = whatsapp_text_payload(sender_id, "wamid.refused_001", sent);
        let body_bytes = serde_json::to_vec(&payload).unwrap();
        let sig = compute_whatsapp_sig(wa_secret, &body_bytes);
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let status = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &sig)
            .json(&payload)
            .send(router)
            .await
            .status_code();
        assert_eq!(status, StatusCode::OK, "webhooks always ack");
        assert!(
            wait_for_turns_to_finish(&resources, Duration::from_secs(30)).await,
            "the refused turn must end on its own"
        );

        let session = db
            .get_session_by_channel_identity(tenant_id, "whatsapp", sender_id, None)
            .await
            .unwrap()
            .expect("the DM has a session");
        let conversation_id = session["pierre_conversation_id"]
            .as_str()
            .expect("the session names its thread")
            .to_owned();
        let user = user_id.to_string();
        let chat = resources.common.repos.chat.as_ref();
        let mut rows: Vec<MessageRecord> = Vec::new();
        for _ in 0..50 {
            rows = turn_stop::recent_rows(chat, &conversation_id, &user, tenant_id)
                .await
                .unwrap();
            if rows.iter().any(|m| m.role == "assistant") {
                break;
            }
            time::sleep(Duration::from_millis(100)).await;
        }

        let questions: Vec<&MessageRecord> = rows.iter().filter(|m| m.role == "user").collect();
        assert_eq!(questions.len(), 1, "the question is on file once: {rows:?}");
        assert_eq!(questions[0].content, sent, "{rows:?}");
        let position = rows
            .iter()
            .position(|m| m.role == "user")
            .expect("question position");
        let closing = rows
            .get(position + 1)
            .unwrap_or_else(|| panic!("no row closes the refused question: {rows:?}"));
        assert_eq!(closing.role, "assistant", "{rows:?}");
        assert_eq!(
            closing.finish_reason.as_deref(),
            Some("interrupted"),
            "{rows:?}"
        );
        assert!(
            turn_stop::awaiting_question(&rows, &user, Utc::now(), ceiling).is_none(),
            "a refused turn is not a reply on its way: {rows:?}"
        );

        // The channel hears the dispatcher's apology once, never the notice
        // as a second message.
        let notice = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
        let bodies = outbound_bodies(db, tenant_id, sender_id).await;
        assert_eq!(bodies.len(), 1, "one reply on the channel: {bodies:?}");
        assert_ne!(
            bodies[0], notice,
            "the channel reply is the apology, not the thread's notice"
        );
    }
}
