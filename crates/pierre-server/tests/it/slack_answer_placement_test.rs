// ABOUTME: A Slack answer lands where its status placeholder would have, even when the placeholder post fails
// ABOUTME: Drives the real Slack webhook with the placeholder API refusing, then reads where the answer was sent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1490.
//!
//! On Slack the first part of every answer is the status placeholder, edited
//! in place, and it opens at the top level of the channel or DM the athlete
//! wrote in. The parts sent the ordinary way carried `reply_to` = the athlete's
//! message, which Slack's renderer turns into `thread_ts`: they opened a thread
//! under that message instead. When the placeholder post failed — one Slack
//! `chat.postMessage` timeout on the 2026-11-10 nightly eval — the whole answer
//! went into a thread the athlete never opened, and the eval reported no reply.

#[cfg(feature = "client-messaging")]
mod slack_answer_placement {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use chrono::Utc;
    use dravr_canot::channel::MessagingChannel;
    use hmac::{Hmac, Mac};
    use pierre_core::llm::LlmProvider;
    use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::channel_adapters::ChannelAdapterFactory;
    use serde_json::{json, Value};
    use sha2::Sha256;
    use tokio::net::TcpListener;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::drained_turn::{create_active_user, ParkedProvider};
    use crate::helpers::offline_channel::OfflineSendAdapters;

    const SIGNING_SECRET: &str = "slack_answer_placement_secret_1490";
    const SLACK_USER: &str = "U0PLACEMENT";
    const CHANNEL: &str = "C0PLACEMENT";
    const ATHLETE_TS: &str = "1794316793.587409";
    /// Distinctive enough that no other outbound (a connect card, a proposal)
    /// can match it.
    const ANSWER: &str = "Ton tempo de mardi était solide: 4:12/km tenu sur 25 minutes.";

    /// A Slack Web API whose `chat.postMessage` refuses every placeholder,
    /// recording each attempt so the test proves it took the fallback path.
    async fn refusing_slack_api() -> (String, Arc<Mutex<usize>>) {
        let attempts = Arc::new(Mutex::new(0_usize));
        let seen = Arc::clone(&attempts);
        let app = Router::new().route(
            "/chat.postMessage",
            post(move |Json(_body): Json<Value>| {
                let seen = Arc::clone(&seen);
                async move {
                    *seen.lock().unwrap() += 1;
                    Json(json!({ "ok": false, "error": "internal_error" }))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), attempts)
    }

    /// Sends captured offline; the status placeholder aimed at `status_base`.
    struct RefusedPlaceholderAdapters {
        offline: OfflineSendAdapters,
        status_base: String,
    }

    impl ChannelAdapterFactory for RefusedPlaceholderAdapters {
        fn build(
            &self,
            channel_type: ChannelType,
            config: &Value,
        ) -> Option<Arc<dyn MessagingChannel>> {
            self.offline.build(channel_type, config)
        }

        fn status_api_base(&self) -> Option<String> {
            Some(self.status_base.clone())
        }
    }

    fn slack_signature(timestamp: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn text_of(message: &OutgoingMessage) -> Option<&str> {
        match &message.content {
            MessageContent::Text { body } | MessageContent::RichText { body } => Some(body),
            MessageContent::Media { .. }
            | MessageContent::Location { .. }
            | MessageContent::Card { .. } => None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_placeholder_still_answers_in_the_channel_not_a_thread() {
        let provider = Arc::new(ParkedProvider::answering(ANSWER));
        provider.release();
        let llm: Arc<dyn LlmProvider> = Arc::<ParkedProvider>::clone(&provider);
        let resources = create_test_server_resources_with_chat_provider(llm)
            .await
            .unwrap();
        let (user_id, tenant_id) =
            create_active_user(&resources, "slack-placement@example.com").await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-slack-placement"),
            api_secret: None,
            webhook_secret: Some(SIGNING_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "slack",
            channel_user_id: SLACK_USER,
            display_name: Some("Placement"),
        })
        .await
        .unwrap();

        let (status_base, placeholder_attempts) = refusing_slack_api().await;
        let adapters = Arc::new(RefusedPlaceholderAdapters {
            offline: OfflineSendAdapters::default(),
            status_base,
        });
        let factory: Arc<dyn ChannelAdapterFactory> =
            Arc::<RefusedPlaceholderAdapters>::clone(&adapters);
        let router = MessagingRoutes::routes_with_adapters(Arc::clone(&resources), factory);

        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel_type": "channel",
                "user": SLACK_USER,
                "text": "Comment était mon tempo de mardi?",
                "channel": CHANNEL,
                "ts": ATHLETE_TS
            }
        })
        .to_string();
        let timestamp = Utc::now().timestamp().to_string();
        let response = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &slack_signature(&timestamp, &body))
            .text(&body)
            .send(router)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "the signed Slack webhook must be accepted before the turn can run"
        );

        let mut answer: Option<OutgoingMessage> = None;
        for _ in 0..300 {
            answer = adapters
                .offline
                .sends()
                .into_iter()
                .find(|sent| text_of(sent) == Some(ANSWER));
            if answer.is_some() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let answer = answer.unwrap_or_else(|| {
            panic!(
                "the answer never reached the channel adapter; sent: {:?}",
                adapters.offline.sends()
            )
        });

        assert!(
            *placeholder_attempts.lock().unwrap() >= 1,
            "the turn must have tried, and failed, to open its placeholder"
        );
        assert_eq!(
            answer.recipient_id, CHANNEL,
            "the answer goes to the channel"
        );
        assert_eq!(
            answer.reply_to, None,
            "an answer to a channel-level message must not open a thread under it"
        );
    }
}
