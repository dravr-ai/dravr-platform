// ABOUTME: Integration tests for inbound emoji reactions writing the shared per-message feedback
// ABOUTME: Covers the write, the clear, the unmapped no-op, the Meta gate, and the group-room trap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An emoji on a coach reply is the messaging surface's thumb. These tests
//! drive the real ingress — repository lookup, ownership rule, feedback write —
//! and assert the row the web and mobile thumbs would have written.

mod common;

use std::sync::Arc;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use http::HeaderMap;
use pierre_core::models::messaging::{ChannelType, InboundReaction, ReactionAction};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_database::repositories::{
    CreateSessionParams, InsertMessageParams, MessagingRepository,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::messaging_ingress::reactions::{
    apply_reactions, channel_delivers_reactions, rating_for_emoji,
};
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::channels::messenger::MessengerChannel;
use pierre_messaging::channels::whatsapp::WhatsAppChannel;
use serde_json::json;
use uuid::Uuid;

/// One athlete, their conversation, the assistant message the coach wrote, and
/// the channel message that delivered it.
struct Delivered {
    resources: Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: String,
    conversation_id: String,
    assistant_message_id: String,
    /// Channel-native id of the message the athlete sees, the one a reaction
    /// will quote.
    channel_message_id: String,
    /// The athlete's own channel id — the reactor a rating is accepted from.
    channel_user_id: String,
    /// The chat the session is bound to.
    channel_conversation_id: String,
}

/// Build an athlete whose coach reply has been delivered over Telegram, with
/// the outbound row stamped the way `persist_outbound_message` stamps it.
async fn deliver_a_reply(chat_id: &str, channel_user_id: &str) -> Delivered {
    let resources = create_test_server_resources().await.unwrap();
    let (user_uuid, user) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id;
    let user_id = user_uuid.to_string();

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(&user_id, tenant_id, "Messaging: telegram", "", None, None)
        .await
        .unwrap();
    let conversation_id = conversation.id.clone();

    let assistant = resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "assistant",
            content: "Your form is trending up — hold the volume this week.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let session_id = Uuid::new_v4().to_string();
    db.create_session(&CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: "telegram",
        channel_user_id,
        channel_conversation_id: Some(chat_id),
        pierre_conversation_id: Some(&conversation_id),
    })
    .await
    .unwrap();

    let channel_message_id = format!("tg-{}", Uuid::new_v4());
    db.insert_message(&InsertMessageParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        session_id: &session_id,
        direction: "outbound",
        channel_type: "telegram",
        channel_message_id: &channel_message_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: Some("Your form is trending up — hold the volume this week."),
        correlation_id: &Uuid::new_v4().to_string(),
        raw_payload: None,
        chat_message_id: Some(&assistant.id),
    })
    .await
    .unwrap();

    Delivered {
        resources,
        tenant_id,
        user_id,
        conversation_id,
        assistant_message_id: assistant.id,
        channel_message_id,
        channel_user_id: channel_user_id.to_owned(),
        channel_conversation_id: chat_id.to_owned(),
    }
}

fn reaction(
    delivered: &Delivered,
    reactor_id: &str,
    emoji: &str,
    action: ReactionAction,
) -> InboundReaction {
    InboundReaction {
        channel_type: ChannelType::Telegram,
        channel_message_id: delivered.channel_message_id.clone(),
        reactor_id: reactor_id.to_owned(),
        emoji: emoji.to_owned(),
        action,
        conversation_id: Some(delivered.channel_conversation_id.clone()),
        timestamp: Utc::now(),
        raw_payload: json!({}),
    }
}

/// The athlete's own feedback rows on the conversation, as the clients read them.
async fn feedback(delivered: &Delivered) -> Vec<(String, String)> {
    delivered
        .resources
        .common
        .repos
        .chat
        .get_conversation_feedback(
            &delivered.conversation_id,
            &delivered.user_id,
            delivered.tenant_id,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|record| (record.message_id, record.rating))
        .collect()
}

#[tokio::test]
async fn thumbs_up_reaction_records_an_up_rating_on_the_assistant_message() {
    let delivered = deliver_a_reply("-100777", "athlete-42").await;

    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            &delivered.channel_user_id,
            "👍",
            ReactionAction::Added,
        )],
    )
    .await;

    let rows = feedback(&delivered).await;
    assert_eq!(rows.len(), 1, "exactly one feedback row: {rows:?}");
    assert_eq!(
        rows[0],
        (delivered.assistant_message_id.clone(), "up".to_owned()),
        "the reaction rates the assistant message the channel message delivered"
    );
}

#[tokio::test]
async fn a_negative_reaction_records_a_down_rating() {
    let delivered = deliver_a_reply("-100778", "athlete-43").await;

    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            &delivered.channel_user_id,
            "👎",
            ReactionAction::Added,
        )],
    )
    .await;

    let rows = feedback(&delivered).await;
    assert_eq!(rows.len(), 1, "exactly one feedback row: {rows:?}");
    assert_eq!(rows[0].1, "down");
}

#[tokio::test]
async fn removing_the_reaction_clears_the_rating() {
    let delivered = deliver_a_reply("-100779", "athlete-44").await;

    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            &delivered.channel_user_id,
            "👍",
            ReactionAction::Added,
        )],
    )
    .await;
    assert_eq!(feedback(&delivered).await.len(), 1, "rating landed first");

    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            &delivered.channel_user_id,
            "👍",
            ReactionAction::Removed,
        )],
    )
    .await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "taking the reaction back leaves no rating behind"
    );
}

#[tokio::test]
async fn swapping_the_emoji_in_one_update_ends_on_the_new_rating() {
    let delivered = deliver_a_reply("-100780", "athlete-45").await;

    // Telegram reports the whole reaction set before and after, so a swap
    // arrives as one addition and one removal in the same update.
    apply_reactions(
        &delivered.resources,
        &[
            reaction(
                &delivered,
                &delivered.channel_user_id,
                "👎",
                ReactionAction::Added,
            ),
            reaction(
                &delivered,
                &delivered.channel_user_id,
                "👍",
                ReactionAction::Removed,
            ),
        ],
    )
    .await;

    let rows = feedback(&delivered).await;
    assert_eq!(rows.len(), 1, "the swap leaves one rating: {rows:?}");
    assert_eq!(
        rows[0].1, "down",
        "the athlete ends on the emoji they chose"
    );
}

#[tokio::test]
async fn a_reaction_on_an_unknown_channel_message_records_nothing() {
    let delivered = deliver_a_reply("-100781", "athlete-46").await;

    let mut orphan = reaction(
        &delivered,
        &delivered.channel_user_id,
        "👍",
        ReactionAction::Added,
    );
    orphan.channel_message_id = "tg-a-message-this-platform-never-sent".to_owned();

    // A no-op, not an error: reactions arrive unsolicited on messages that may
    // predate the mapping entirely.
    apply_reactions(&delivered.resources, &[orphan]).await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "an unmapped channel message rates nothing"
    );
}

#[tokio::test]
async fn an_emoji_that_is_not_a_rating_records_nothing() {
    let delivered = deliver_a_reply("-100782", "athlete-47").await;

    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            &delivered.channel_user_id,
            "🤔",
            ReactionAction::Added,
        )],
    )
    .await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "a thinking face says nothing about the reply's quality"
    );
}

#[tokio::test]
async fn a_group_member_who_is_not_the_athlete_cannot_rate_as_the_athlete() {
    let delivered = deliver_a_reply("-100783", "athlete-48").await;

    // Same room, same coach reply, a different member's thumb.
    apply_reactions(
        &delivered.resources,
        &[reaction(
            &delivered,
            "bystander-99",
            "👍",
            ReactionAction::Added,
        )],
    )
    .await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "a bystander's applause is not the athlete's rating"
    );
}

#[tokio::test]
async fn a_reaction_from_another_chat_does_not_rate_this_conversation() {
    let delivered = deliver_a_reply("-100784", "athlete-49").await;

    // Telegram message ids are unique per chat, so the same id in a different
    // chat is a different message.
    let mut elsewhere = reaction(
        &delivered,
        &delivered.channel_user_id,
        "👍",
        ReactionAction::Added,
    );
    elsewhere.conversation_id = Some("-100999".to_owned());

    apply_reactions(&delivered.resources, &[elsewhere]).await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "the chat id narrows the match, so a collision elsewhere rates nothing"
    );
}

#[test]
fn only_the_three_reaction_delivering_channels_reach_the_mapper() {
    for channel in [
        ChannelType::Telegram,
        ChannelType::Slack,
        ChannelType::Discord,
    ] {
        assert!(
            channel_delivers_reactions(channel),
            "{channel} delivers reaction events, so the ingress must parse them"
        );
    }
    for channel in [ChannelType::WhatsApp, ChannelType::Messenger] {
        assert!(
            !channel_delivers_reactions(channel),
            "{channel} delivers no reaction event; the ingress must not ask for one"
        );
    }
}

#[tokio::test]
async fn a_meta_channel_parses_no_reaction_even_from_a_reaction_shaped_body() {
    // The gate above keeps these bodies away from the parser; this is the
    // second lock — the adapters themselves surface nothing to map.
    let body = serde_json::to_vec(&json!({
        "message_reaction": {
            "chat": { "id": -100_777 },
            "message_id": 42,
            "user": { "id": 7 },
            "new_reaction": [{ "type": "emoji", "emoji": "👍" }]
        }
    }))
    .unwrap();

    let whatsapp = WhatsAppChannel::new("secret".to_owned());
    assert!(
        whatsapp
            .receive_reactions(&HeaderMap::new(), &body)
            .await
            .unwrap()
            .is_empty(),
        "WhatsApp surfaces no inbound reaction"
    );

    let messenger = MessengerChannel::new("secret".to_owned());
    assert!(
        messenger
            .receive_reactions(&HeaderMap::new(), &body)
            .await
            .unwrap()
            .is_empty(),
        "Messenger surfaces no inbound reaction"
    );
}

#[test]
fn slack_reaction_names_and_variation_selectors_map_to_the_same_ratings() {
    // Telegram sends the character, Slack sends its own name, and a client may
    // append a variation selector to either.
    assert_eq!(rating_for_emoji("👍"), Some("up"));
    assert_eq!(rating_for_emoji("thumbsup"), Some("up"));
    assert_eq!(rating_for_emoji("❤\u{fe0f}"), Some("up"));
    assert_eq!(rating_for_emoji("👎"), Some("down"));
    assert_eq!(rating_for_emoji("thumbsdown"), Some("down"));
    assert_eq!(rating_for_emoji("🤔"), None);
}
