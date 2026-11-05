// ABOUTME: The team replies to athlete feedback from its Slack card: the reply modal's view_submission on /api/ops/slack/actions
// ABOUTME: Pins that a reply from the feedback room lands in the athlete's thread as the team's, and one carried from elsewhere writes nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1392.
//!
//! A `feedback.recorded` card's Reply button opens a modal whose
//! `private_metadata` names the feedback and the room the card was tapped
//! in. Its submission arrives signed on the ops route as a `view_submission`;
//! the server re-checks that room against the `feedback.recorded` routing
//! rule before the reply is written to the athlete's thread.

#[cfg(feature = "client-messaging")]
mod slack_feedback_reply_action {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use chrono::Utc;
    use dravr_contremaitre::config::NOTIFY_ROUTING_YAML;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
    use pierre_core::models::{Tenant, TenantId, User};
    use pierre_database::repositories::{FeedbackAuthor, FeedbackWriter, NewFeedbackMessage};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use serial_test::serial;
    use sha2::Sha256;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources_with_config, test_server_config};
    use crate::helpers::axum_test::AxumTestRequest;

    const SIGNING_SECRET: &str = "slack_feedback_reply_secret_1392";
    const ACTIONS_PATH: &str = "/api/ops/slack/actions";
    const FEEDBACK_ROOM: &str = "C0FEEDBACKROOM";

    /// Route `feedback.recorded` to [`FEEDBACK_ROOM`] for one test, and put
    /// the pinned table back when it ends.
    ///
    /// These tests pin the room check, not which room the shipped routing
    /// names, so they load a table of their own into the process-wide
    /// provider the route reads.
    struct FeedbackRoom;

    impl FeedbackRoom {
        fn routed() -> Self {
            NOTIFY_ROUTING_PROVIDER
                .reload(&format!(
                    "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                     feedback.recorded:\n    channel: \"{FEEDBACK_ROOM}\"\n"
                ))
                .expect("the feedback routing table parses");
            Self
        }
    }

    impl Drop for FeedbackRoom {
        fn drop(&mut self) {
            NOTIFY_ROUTING_PROVIDER
                .reload(NOTIFY_ROUTING_YAML)
                .expect("the pinned routing rules parse");
        }
    }

    /// A server whose `SLACK_SIGNING_SECRET` is [`SIGNING_SECRET`].
    async fn signing_server() -> Arc<ServerContext> {
        let mut config = test_server_config();
        config.webhooks.slack_signing_secret = Some(SecretString::from(SIGNING_SECRET));
        create_test_server_resources_with_config(config)
            .await
            .unwrap()
    }

    /// An athlete owning a tenant, with one message in their feedback thread;
    /// the `thread:tenant:user:row` reference its card's buttons carry.
    async fn athlete_with_feedback(resources: &ServerContext) -> (TenantId, Uuid, String) {
        let user = User::new(
            format!("{}@feedback-reply.test", Uuid::new_v4()),
            "not-a-login-path".to_owned(),
            None,
        );
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        let tenant_id = TenantId::generate();
        resources
            .common
            .repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: "Feedback reply tenant".to_owned(),
                slug: format!("tenant-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        let message = resources
            .common
            .repos
            .feedback_messages
            .insert_feedback_message(&NewFeedbackMessage {
                tenant_id,
                user_id,
                writer: FeedbackWriter::Athlete,
                body: "The week view skips my indoor ride.",
                screen: None,
                app_version: None,
                locale: None,
                about: None,
                image: None,
                at: Utc::now(),
            })
            .await
            .unwrap();
        let reference = format!("thread:{tenant_id}:{user_id}:{}", message.id);
        (tenant_id, user_id, reference)
    }

    /// The reply modal sent with `reply` typed, for the feedback `reference`
    /// names, opened on a card in `channel_id`; signed as Slack signs it.
    /// The route's answer.
    async fn submit(
        resources: &Arc<ServerContext>,
        reference: &str,
        channel_id: &str,
        reply: &str,
    ) -> Value {
        // What the server wrote into the modal when the card's Reply was
        // tapped, which Slack hands back unchanged inside the signed request.
        let origin = json!({
            "feedback": reference,
            "channel_id": channel_id,
            "channel_name": "",
            "message_ts": "1730000000.000100",
            "card": [],
        });
        let payload = json!({
            "type": "view_submission",
            "user": { "id": "U0REPLIER", "username": "phil" },
            "view": {
                "callback_id": "feedback_reply",
                "private_metadata": origin.to_string(),
                "state": { "values": { "feedback_reply": { "body": {
                    "type": "plain_text_input", "value": reply } } } }
            }
        })
        .to_string();
        let body = format!("payload={}", urlencoding::encode(&payload));

        let timestamp = Utc::now().timestamp().to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        let response = AxumTestRequest::post(ACTIONS_PATH)
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .text(&body)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "bug: the route answers every signed submission with 200, a refusal included"
        );
        response.json()
    }

    /// The athlete's thread, as `(author, body)` oldest first.
    async fn thread(
        resources: &ServerContext,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> Vec<(FeedbackAuthor, String)> {
        resources
            .common
            .repos
            .feedback_messages
            .list_feedback_messages(tenant_id, user_id, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|message| (message.author, message.body))
            .collect()
    }

    /// The reason a refused submission keeps the modal open with.
    fn field_error(answer: &Value) -> &str {
        assert_eq!(
            answer["response_action"], "errors",
            "bug: a refused reply keeps the modal open: {answer}"
        );
        answer["errors"]["feedback_reply"]
            .as_str()
            .unwrap_or_else(|| panic!("a refusal names the reply field: {answer}"))
    }

    #[tokio::test]
    #[serial]
    async fn a_reply_sent_from_the_feedback_room_lands_in_the_athletes_thread_as_the_teams() {
        let resources = signing_server().await;
        let _room = FeedbackRoom::routed();
        let (tenant_id, user_id, reference) = athlete_with_feedback(&resources).await;

        let answer = submit(&resources, &reference, FEEDBACK_ROOM, "  Fixed tonight.  ").await;

        assert_eq!(
            answer,
            json!({ "response_action": "clear" }),
            "bug: a stored reply closes the modal"
        );
        assert_eq!(
            thread(&resources, tenant_id, user_id).await,
            [
                (
                    FeedbackAuthor::Athlete,
                    "The week view skips my indoor ride.".to_owned()
                ),
                (FeedbackAuthor::Team, "Fixed tonight.".to_owned()),
            ],
            "bug: the reply follows the athlete's message, trimmed, as the team's"
        );
        assert_eq!(
            resources
                .common
                .repos
                .feedback_messages
                .count_unread_team_replies(tenant_id, user_id)
                .await
                .unwrap(),
            1,
            "bug: the athlete has the reply waiting unread"
        );
    }

    #[tokio::test]
    #[serial]
    async fn a_reply_carried_from_another_room_or_naming_another_tenant_writes_nothing() {
        let resources = signing_server().await;
        let _room = FeedbackRoom::routed();
        let (tenant_id, user_id, reference) = athlete_with_feedback(&resources).await;

        for elsewhere in ["C0DEFAULT", "C0SOMEOTHERROOM"] {
            let answer = submit(&resources, &reference, elsewhere, "Hello").await;
            assert!(
                field_error(&answer).contains("ops channel"),
                "bug: a reply carried from {elsewhere} is refused: {answer}"
            );
        }

        // The athlete and row of the card, under a tenant they are not in.
        let forged = reference.replacen(&tenant_id.to_string(), &Uuid::new_v4().to_string(), 1);
        let answer = submit(&resources, &forged, FEEDBACK_ROOM, "Hello").await;
        assert!(
            field_error(&answer).contains("no longer exists"),
            "bug: feedback named under another tenant is never found: {answer}"
        );

        let answer = submit(&resources, &reference, FEEDBACK_ROOM, "   ").await;
        assert!(
            field_error(&answer).contains("cannot be empty"),
            "bug: an empty reply is refused: {answer}"
        );

        assert_eq!(
            thread(&resources, tenant_id, user_id).await.len(),
            1,
            "bug: a refused reply writes nothing to the athlete's thread"
        );
    }
}
