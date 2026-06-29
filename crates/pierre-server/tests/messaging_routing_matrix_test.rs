// ABOUTME: Cross-channel reply-routing matrix regression test for the backfill-completion push
// ABOUTME: Pins BOTH axes — every channel string AND every conversation-id state — for the 5df2c1706 silent-DM-drop class
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use std::sync::Arc;

use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_mcp_server::services::messaging_ingress::addressing::reply_recipient;
use pierre_messaging::channel::MessagingChannel;
use pierre_tool_runtime::runtime::BackfillNotifier;

// Shared messaging fixtures + channel fakes live in a helpers subdir (not a
// top-level test binary), pulled in via `#[path]` so this test and the sibling
// notifier route/render tests reuse one copy.
#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;
use messaging_fixtures::{
    create_test_db, seed_conversation, seed_session, seed_user, strings, CapturingChannel,
    FakeResolver,
};

/// Pure unit table-test of the consolidated reply-recipient rule.
///
/// `reply_recipient` is the single source of truth shared by the synchronous
/// dispatch paths and the backfill push. Pinning every cell of its fallback
/// contract here keeps a future edit from silently re-introducing the
/// empty-string drop class fixed in commit 5df2c1706.
#[test]
fn reply_recipient_falls_back_on_none_or_empty() {
    // Group/thread present: route to the conversation id verbatim.
    assert_eq!(reply_recipient(Some("chat_123"), "user_9"), "chat_123");
    // DM, NULL conversation id: fall back to the channel-native user id.
    assert_eq!(reply_recipient(None, "user_9"), "user_9");
    // DM, empty-string conversation id — the 5df2c1706 silent-drop class: an
    // empty id is not a valid recipient, so it falls back to the user id.
    assert_eq!(reply_recipient(Some(""), "user_9"), "user_9");
    // Whitespace is NON-empty: the impl filters only `is_empty()`, so a " "
    // string is returned as-is. Asserted to document the real contract — the
    // helper does NOT trim, and a future "trim then check" change would be a
    // behavior shift this cell would catch.
    assert_eq!(reply_recipient(Some(" "), "u"), " ");
}

/// Map a channel slug to its [`ChannelType`], mirroring the production
/// `ChannelType::from_str` parse the notifier performs in `resolve_route`.
/// Panics on an unhandled slug — a test programming error, not a runtime case.
fn channel_type_for(channel: &str) -> ChannelType {
    match channel {
        "telegram" => ChannelType::Telegram,
        "whatsapp" => ChannelType::WhatsApp,
        "slack" => ChannelType::Slack,
        "discord" => ChannelType::Discord,
        "messenger" => ChannelType::Messenger,
        other => panic!("unhandled test channel slug: {other}"),
    }
}

/// Drive the full backfill-completion push for one (channel, user, conversation
/// state) cell and return the channel-native id the single resulting notice was
/// addressed to.
///
/// Seeds a real user + conversation + messaging session on the given channel
/// (with `channel_conversation_id` set to the supplied state), wires the
/// notifier to a capturing fake adapter, fires `push_backfill_complete`, and
/// asserts exactly one text notice was sent before returning its `recipient_id`.
/// No activity cache is seeded, so the push degrades to the templated nudge —
/// the body is irrelevant here; only the resolved routing recipient is.
async fn route_recipient_for(
    channel: &str,
    channel_user_id: &str,
    channel_conversation_id: Option<&str>,
) -> String {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    seed_session(
        &db,
        &user_id,
        tenant_id,
        channel,
        channel_user_id,
        channel_conversation_id,
        &conversation_id,
    )
    .await;

    let capture = Arc::new(CapturingChannel::for_channel(channel_type_for(channel)));
    let resolver = Arc::new(FakeResolver::new(
        capture.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver);

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            1_700_000_000,
            7,
        )
        .await;

    let sent = capture.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "exactly one notice must be sent (channel={channel}, conversation_id={channel_conversation_id:?})"
    );
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text completion notice for channel={channel}");
    };
    assert!(
        !body.is_empty(),
        "notice body must not be empty for channel={channel}"
    );
    sent[0].recipient_id.clone()
}

/// Cross-channel reply-routing matrix: one run pins BOTH routing axes.
///
/// For every channel string (`telegram`/`whatsapp`/`slack`/`discord`/`messenger`)
/// and every conversation-id state (group present, DM with a NULL id, DM with an
/// empty-string id), the backfill-completion push must resolve the SAME recipient
/// rule via `reply_recipient`: the `channel_conversation_id` when it is a present,
/// non-empty value, else the channel-native `channel_user_id`.
///
/// This closes the "I only tested Telegram" gap: the routing fix from commit
/// 5df2c1706 lives in a single shared pipeline (`resolve_route` ->
/// `reply_recipient`), so validating it across ALL five channels and ALL three
/// conversation-id states in one run proves the shared fix holds everywhere — and
/// that a per-channel regression (or a regression of the empty-string DM cell)
/// can never slip through under the cover of a Telegram-only test. Channel-flavored
/// user/group ids make a wrong-routing bug (returning the user id for a group, or
/// vice versa) impossible to miss.
#[tokio::test]
async fn routing_matrix_all_channels_all_conversation_states() {
    // (channel slug, channel-native user id) — distinct, channel-flavored ids so
    // a mis-route is obvious in the assertion message.
    let channels = [
        ("telegram", "tg_user_1"),
        ("whatsapp", "14502244753"),
        ("slack", "U123"),
        ("discord", "D123"),
        ("messenger", "m_1"),
    ];

    for (channel, channel_user_id) in channels {
        // GROUP: a present conversation id routes the notice to that exact chat.
        let group_chat = format!("grp_{channel}");
        let group_recipient =
            route_recipient_for(channel, channel_user_id, Some(&group_chat)).await;
        assert_eq!(
            group_recipient, group_chat,
            "group push on {channel} must route to the conversation id"
        );

        // DM, NULL conversation id: a direct chat stores channel_conversation_id
        // as NULL (the group/DM split keys it), so the notice must fall back to
        // the channel-native user id — never silently dropped.
        let dm_null_recipient = route_recipient_for(channel, channel_user_id, None).await;
        assert_eq!(
            dm_null_recipient, channel_user_id,
            "DM push on {channel} with a NULL conversation id must route to the channel user id"
        );

        // DM, empty-string conversation id (the 5df2c1706 silent-drop class): an
        // empty id is not a valid recipient and must also fall back to the user id.
        let dm_empty_recipient = route_recipient_for(channel, channel_user_id, Some("")).await;
        assert_eq!(
            dm_empty_recipient, channel_user_id,
            "DM push on {channel} with an empty conversation id must route to the channel user id"
        );
    }
}
