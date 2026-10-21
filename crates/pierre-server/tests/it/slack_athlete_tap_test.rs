// ABOUTME: A Slack button standing for the athlete's words reaches their turn, never the ops approve/reject path
// ABOUTME: Drives the Slack interactivity route with a signed block_actions tap and reads what the ingress stored

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#829.
//!
//! Slack has one interactivity URL per app, and ours answered only two kinds
//! of tap: a `/command` postback and the ops channel's approve/reject. A
//! welcome starter (`ex:<slot>:<index>`) or an agent's number on the
//! onboarding proposal fell into the ops path, which refused it for not
//! coming from the ops channel — the athlete's tap went nowhere.
//!
//! Such a tap is now ingested through the messaging webhook path as the
//! message it spells: the same tenant resolution by signature, the same
//! channel-link gate, the same stored inbound row and turn a typed message
//! gets.

#[cfg(feature = "client-messaging")]
mod slack_athlete_tap {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use hmac::{Hmac, Mac};
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::channel_adapters::ChannelAdapterFactory;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use sha2::Sha256;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources_with_config, create_test_user_with_plan, test_server_config,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::offline_channel::OfflineSendAdapters;

    const SIGNING_SECRET: &str = "slack_athlete_tap_secret_829";
    const ACTIONS_PATH: &str = "/api/ops/slack/actions";
    const SLACK_USER: &str = "U0TAPPER";
    /// The athlete's DM with the bot (`D` prefix = IM).
    const DM_CHANNEL: &str = "D0TAPDM";

    /// A server whose `SLACK_SIGNING_SECRET` is [`SIGNING_SECRET`].
    async fn signing_server() -> Arc<ServerContext> {
        let mut config = test_server_config();
        config.webhooks.slack_signing_secret = Some(SecretString::from(SIGNING_SECRET));
        create_test_server_resources_with_config(config)
            .await
            .unwrap()
    }

    /// A linked athlete with a DM session, under a tenant whose Slack config
    /// signs with [`SIGNING_SECRET`] — what a tap needs to be anyone's turn.
    async fn linked_athlete(resources: &Arc<ServerContext>) {
        let (user_id, _user, tenant_id) = create_test_user_with_plan(
            &resources.agent.database,
            "slack-tap@test.com",
            "professional",
        )
        .await
        .unwrap();
        let user = user_id.to_string();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-slack-tap"),
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
            user_id: &user,
            channel_type: "slack",
            channel_user_id: SLACK_USER,
            display_name: Some("Tapper"),
        })
        .await
        .unwrap();
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(&user, tenant_id, "Slack DM", "test-model", None, None)
            .await
            .unwrap();
        db.create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &user,
            tenant_id,
            channel_type: "slack",
            channel_user_id: SLACK_USER,
            channel_conversation_id: None,
            pierre_conversation_id: Some(&conversation.id),
        })
        .await
        .unwrap();
    }

    /// A `block_actions` tap on `action_id`, form-encoded as Slack sends it.
    fn tap_body(action_id: &str, action_ts: &str) -> String {
        let payload = json!({
            "type": "block_actions",
            "user": { "id": SLACK_USER, "username": "tapper" },
            "channel": { "id": DM_CHANNEL, "name": "directmessage" },
            "message": { "ts": "1727440000.000100" },
            "actions": [{ "action_id": action_id, "action_ts": action_ts }],
        })
        .to_string();
        format!("payload={}", urlencoding::encode(&payload))
    }

    /// POST `body` to the interactivity route, signed under [`SIGNING_SECRET`].
    async fn post_tap(resources: &Arc<ServerContext>, body: &str) -> Value {
        let adapters: Arc<dyn ChannelAdapterFactory> = Arc::new(OfflineSendAdapters::default());
        let router = MessagingRoutes::routes_with_adapters(Arc::clone(resources), adapters);
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        let response = AxumTestRequest::post(ACTIONS_PATH)
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .text(body)
            .send(router)
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        response.json()
    }

    /// A starter's postback — an agent's example or a ranked use case that
    /// asks a question — and a pick from the proposal are each ingested as
    /// the athlete's message: stored like typed text, never refused as an ops
    /// action.
    #[tokio::test]
    async fn a_suggestion_or_a_pick_tapped_in_slack_is_the_athletes_message() {
        let resources = signing_server().await;
        linked_athlete(&resources).await;

        for (action_id, action_ts) in [
            ("ex:0:0", "1727440001.000001"),
            ("uc:0:connect", "1727440004.000004"),
            ("2", "1727440002.000002"),
        ] {
            let answer = post_tap(&resources, &tap_body(action_id, action_ts)).await;

            assert_eq!(answer["status"], "ok", "{action_id}: {answer}");
            assert_eq!(answer["messages_received"], 1, "{action_id}: {answer}");
            assert_eq!(
                answer["messages_stored"], 1,
                "{action_id} reaches the athlete's turn as their message: {answer}"
            );
        }
    }

    /// The ops path still answers what is its own: an approve tapped outside
    /// the ops channel is refused there, and nothing reaches a turn.
    #[tokio::test]
    async fn an_ops_action_stays_on_the_ops_path() {
        let resources = signing_server().await;
        linked_athlete(&resources).await;

        let approve = format!("approve_user:{}", Uuid::new_v4());
        let answer = post_tap(&resources, &tap_body(&approve, "1727440003.000003")).await;

        let text = answer["text"].as_str().unwrap_or_default();
        assert!(text.starts_with("Action failed:"), "{answer}");
        assert!(answer.get("messages_stored").is_none(), "{answer}");
    }
}
