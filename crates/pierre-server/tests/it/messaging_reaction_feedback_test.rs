// ABOUTME: Integration tests for inbound emoji reactions writing the shared per-message feedback
// ABOUTME: Covers the write, the clear, the unmapped no-op, the Meta gate, the group-room trap, and the Discord Gateway stream
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An emoji on an agent reply is the messaging surface's thumb. These tests
//! drive the real ingress — repository lookup, ownership rule, feedback write —
//! and assert the row the web and mobile thumbs would have written.

use crate::common;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::channels::descriptor_for;
use dravr_canot::channels::discord::DiscordChannel;
use dravr_canot::channels::messenger::MessengerChannel;
use dravr_canot::channels::whatsapp::WhatsAppChannel;
use http::HeaderMap;
use pierre_core::models::messaging::{ChannelType, InboundReaction, ReactionAction};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_database::repositories::{
    CreateSessionParams, InsertMessageParams, MessagingRepository,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::messaging_ingress::reactions::{
    apply_reactions, channel_delivers_reactions, consume_reactions, rating_for_emoji,
};
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

/// One athlete, their conversation, the assistant message the agent wrote, and
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

/// Build an athlete whose agent reply has been delivered over Telegram, with
/// the outbound row stamped the way `persist_outbound_message` stamps it.
async fn deliver_a_reply(chat_id: &str, channel_user_id: &str) -> Delivered {
    deliver_a_reply_on("telegram", chat_id, channel_user_id).await
}

/// [`deliver_a_reply`] over any channel, named by its `channel_type` slug.
async fn deliver_a_reply_on(channel: &str, chat_id: &str, channel_user_id: &str) -> Delivered {
    let resources = create_test_server_resources().await.unwrap();
    let (user_uuid, user) = create_test_user(&resources.agent.database).await.unwrap();
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
        .create_conversation(
            &user_id,
            tenant_id,
            &format!("{channel} coach"),
            "",
            None,
            None,
        )
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
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
        .unwrap();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let session_id = Uuid::new_v4().to_string();
    db.create_session(&CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: channel,
        channel_user_id,
        channel_conversation_id: Some(chat_id),
        pierre_conversation_id: Some(&conversation_id),
    })
    .await
    .unwrap();

    let channel_message_id = format!("{channel}-{}", Uuid::new_v4());
    db.insert_message(&InsertMessageParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        session_id: &session_id,
        direction: "outbound",
        channel_type: channel,
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

    // Same room, same agent reply, a different member's thumb.
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

/// What canot's Gateway client forwards for a Discord reaction: the dispatch
/// frame Discord sends, decoded by canot's own parser.
async fn discord_gateway_reactions(
    delivered: &Delivered,
    event_name: &str,
    reactor_id: &str,
    emoji: &str,
) -> Vec<InboundReaction> {
    let frame = json!({
        "op": 0,
        "s": 31,
        "t": event_name,
        "d": {
            "user_id": reactor_id,
            "channel_id": delivered.channel_conversation_id,
            "message_id": delivered.channel_message_id,
            "guild_id": "103735883630395392",
            "emoji": {"id": null, "name": emoji},
        }
    });
    DiscordChannel::new("ab".repeat(32), "app-id".to_owned())
        .receive_reactions(&HeaderMap::new(), &serde_json::to_vec(&frame).unwrap())
        .await
        .unwrap()
}

/// Feed `reactions` through the stream consumer the Discord Gateway hands its
/// receiver to, and wait until it has applied every one.
async fn stream_reactions(delivered: &Delivered, reactions: Vec<InboundReaction>) {
    let (tx, rx) = mpsc::channel(8);
    for reaction in reactions {
        tx.send(reaction).await.unwrap();
    }
    drop(tx);
    consume_reactions(Arc::clone(&delivered.resources), "test gateway", rx).await;
}

#[tokio::test]
async fn a_discord_gateway_reaction_records_a_rating_on_the_assistant_message() {
    let delivered = deliver_a_reply_on("discord", "103735883630395392", "80351110224678912").await;

    let reactions = discord_gateway_reactions(
        &delivered,
        "MESSAGE_REACTION_ADD",
        &delivered.channel_user_id,
        "👍",
    )
    .await;
    assert_eq!(
        reactions.len(),
        1,
        "the dispatch frame carries one reaction"
    );
    assert_eq!(reactions[0].channel_type, ChannelType::Discord);
    stream_reactions(&delivered, reactions).await;

    assert_eq!(
        feedback(&delivered).await,
        vec![(delivered.assistant_message_id.clone(), "up".to_owned())],
        "a Discord thumb rates the reply exactly as a Telegram or Slack one does"
    );
}

#[tokio::test]
async fn a_discord_gateway_reaction_removal_clears_the_rating() {
    let delivered = deliver_a_reply_on("discord", "103735883630395393", "80351110224678913").await;

    let added = discord_gateway_reactions(
        &delivered,
        "MESSAGE_REACTION_ADD",
        &delivered.channel_user_id,
        "👎",
    )
    .await;
    assert_eq!(added.len(), 1, "the addition frame carries one reaction");
    stream_reactions(&delivered, added).await;
    assert_eq!(
        feedback(&delivered).await,
        vec![(delivered.assistant_message_id.clone(), "down".to_owned())],
        "rating landed first"
    );

    let removed = discord_gateway_reactions(
        &delivered,
        "MESSAGE_REACTION_REMOVE",
        &delivered.channel_user_id,
        "👎",
    )
    .await;
    assert_eq!(removed.len(), 1, "the removal frame carries one reaction");
    assert_eq!(removed[0].action, ReactionAction::Removed);
    stream_reactions(&delivered, removed).await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "taking the reaction back on Discord leaves no rating behind"
    );
}

#[tokio::test]
async fn a_discord_bystander_reaction_from_the_gateway_rates_nothing() {
    let delivered = deliver_a_reply_on("discord", "103735883630395394", "80351110224678914").await;

    let reactions =
        discord_gateway_reactions(&delivered, "MESSAGE_REACTION_ADD", "bystander-99", "👍").await;
    assert_eq!(reactions.len(), 1, "the bystander's reaction is decoded");
    assert_eq!(reactions[0].reactor_id, "bystander-99");
    stream_reactions(&delivered, reactions).await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "another member of the Discord channel is not the athlete"
    );
}

#[tokio::test]
async fn the_reaction_stream_drops_a_channel_that_declares_no_reactions() {
    // WhatsApp's reply is mapped exactly like any other, so only the
    // descriptor gate stands between this reaction and a rating.
    let delivered = deliver_a_reply_on("whatsapp", "chat-whatsapp", "athlete-50").await;

    let mut reaction = reaction(
        &delivered,
        &delivered.channel_user_id,
        "👍",
        ReactionAction::Added,
    );
    reaction.channel_type = ChannelType::WhatsApp;
    stream_reactions(&delivered, vec![reaction.clone()]).await;

    assert_eq!(
        feedback(&delivered).await,
        Vec::<(String, String)>::new(),
        "a channel whose API delivers no reaction event rates nothing"
    );

    // The same reaction applied past the gate does rate the reply, so the
    // empty result above is the stream's gate at work and not a lookup miss.
    apply_reactions(&delivered.resources, &[reaction]).await;
    assert_eq!(
        feedback(&delivered).await,
        vec![(delivered.assistant_message_id.clone(), "up".to_owned())],
        "without the gate the reaction resolves to the delivered reply"
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

#[test]
fn the_reaction_gate_reads_canots_descriptor_for_every_channel() {
    // canot's `descriptor_for` is the one ChannelType -> descriptor lookup;
    // the ingress gate must answer exactly what the descriptor declares.
    for channel in [
        ChannelType::Telegram,
        ChannelType::Slack,
        ChannelType::Discord,
        ChannelType::WhatsApp,
        ChannelType::Messenger,
    ] {
        let declared = descriptor_for(channel)
            .expect("client-messaging compiles every channel")
            .delivers_inbound_reactions();
        assert_eq!(
            channel_delivers_reactions(channel),
            declared,
            "{channel} reaction flag"
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
