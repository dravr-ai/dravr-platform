// ABOUTME: Unit tests for slack_socket helpers (parse_allowed_bot_ids, parse_socket_payload)
// ABOUTME: Pierre-side coverage that runs on every push, complementing the nightly real-Slack e2e
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
#![allow(missing_docs)]

use pierre_core::models::messaging::ChannelType;
use pierre_mcp_server::services::slack_socket::{parse_allowed_bot_ids, parse_socket_payload};
use serde_json::json;

#[test]
fn parse_allowed_bot_ids_empty_string() {
    assert!(parse_allowed_bot_ids("").is_empty());
}

#[test]
fn parse_allowed_bot_ids_single() {
    let ids = parse_allowed_bot_ids("B0ABC123");
    assert_eq!(ids, vec!["B0ABC123".to_owned()]);
}

#[test]
fn parse_allowed_bot_ids_multiple_clean() {
    let ids = parse_allowed_bot_ids("B0ABC123,B0DEF456,B0GHI789");
    assert_eq!(
        ids,
        vec![
            "B0ABC123".to_owned(),
            "B0DEF456".to_owned(),
            "B0GHI789".to_owned(),
        ]
    );
}

#[test]
fn parse_allowed_bot_ids_trims_whitespace() {
    let ids = parse_allowed_bot_ids(" B0ABC123 , B0DEF456 ,  B0GHI789  ");
    assert_eq!(
        ids,
        vec![
            "B0ABC123".to_owned(),
            "B0DEF456".to_owned(),
            "B0GHI789".to_owned(),
        ]
    );
}

#[test]
fn parse_allowed_bot_ids_drops_empty_entries() {
    // Trailing/leading commas, double commas — operators copy-paste, this
    // happens. Drop them silently rather than emit empty bot IDs that
    // would never match any real Slack message.
    let ids = parse_allowed_bot_ids(",B0ABC123,,B0DEF456,");
    assert_eq!(ids, vec!["B0ABC123".to_owned(), "B0DEF456".to_owned()]);
}

#[test]
fn parse_allowed_bot_ids_only_commas_yields_empty() {
    assert!(parse_allowed_bot_ids(",,,").is_empty());
    assert!(parse_allowed_bot_ids("  ,  ,  ").is_empty());
}

/// Slack Events API envelope shape that Socket Mode delivers under the
/// `payload` field of an `events_api` frame. canot's parser expects this
/// exact JSON structure on the wire.
fn slack_event_payload(user_id: &str, text: &str, channel_type: &str) -> serde_json::Value {
    json!({
        "type": "event_callback",
        "team_id": "T0TESTTEAM",
        "event": {
            "type": "message",
            "user": user_id,
            "text": text,
            "channel": "C0TESTCHAN",
            "channel_type": channel_type,
            "ts": "1700000000.000100",
        },
    })
}

fn slack_bot_event_payload(bot_id: &str, text: &str) -> serde_json::Value {
    json!({
        "type": "event_callback",
        "team_id": "T0TESTTEAM",
        "event": {
            "type": "message",
            "bot_id": bot_id,
            "user": "U0BOTUSER",
            "text": text,
            "channel": "C0TESTCHAN",
            "channel_type": "channel",
            "ts": "1700000000.000200",
        },
    })
}

#[tokio::test]
async fn parse_socket_payload_human_message_passes_through() {
    let payload = slack_event_payload("U0HUMAN1", "what's a tempo run", "channel");
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("parser must accept a well-formed human message payload");
    assert_eq!(messages.len(), 1);
    let msg = &messages[0];
    assert!(matches!(msg.channel_type, ChannelType::Slack));
    assert_eq!(msg.sender_id, "U0HUMAN1");
    assert_eq!(msg.channel_message_id, "1700000000.000100");
}

#[tokio::test]
async fn parse_socket_payload_drops_unlisted_bot_messages() {
    // Default canot loop-prevention behaviour: any frame carrying a
    // bot_id is dropped unless the bot is explicitly allow-listed.
    let payload = slack_bot_event_payload("B0UNLISTED", "scope refusal probe");
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("parse_inbound returns Ok with an empty Vec, not an error");
    assert!(
        messages.is_empty(),
        "unlisted bot must be dropped by canot's parser; got {messages:?}"
    );
}

#[tokio::test]
async fn parse_socket_payload_accepts_allow_listed_bot() {
    // Same payload as above but the bot ID matches our allow-list.
    let payload = slack_bot_event_payload("B0QADRIVER", "scope refusal probe");
    let messages = parse_socket_payload(&payload, &["B0QADRIVER".to_owned()])
        .await
        .expect("allow-listed bot payload must parse");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].channel_message_id, "1700000000.000200");
}

#[tokio::test]
async fn parse_socket_payload_im_marks_direct_message_flag() {
    let payload = slack_event_payload("U0HUMAN1", "private question", "im");
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("im channel_type must parse");
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].is_direct_message,
        "channel_type=im must populate IncomingMessage.is_direct_message"
    );
}

#[tokio::test]
async fn parse_socket_payload_channel_type_is_not_direct_message() {
    let payload = slack_event_payload("U0HUMAN1", "public ask", "channel");
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("channel channel_type must parse");
    assert_eq!(messages.len(), 1);
    assert!(
        !messages[0].is_direct_message,
        "channel_type=channel must NOT populate IncomingMessage.is_direct_message"
    );
}

#[tokio::test]
async fn parse_socket_payload_url_verification_yields_empty() {
    // Slack sends url_verification challenges through the same envelope
    // surface; canot's parser returns an empty messages list rather than
    // erroring, leaving the handshake response to a higher layer.
    let payload = json!({
        "type": "url_verification",
        "challenge": "abc123",
    });
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("url_verification payload must parse to empty Vec");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn parse_socket_payload_non_message_event_yields_empty() {
    // Non-message events (typing indicators, reactions, …) shouldn't
    // surface as user messages in the pipeline.
    let payload = json!({
        "type": "event_callback",
        "team_id": "T0TESTTEAM",
        "event": {
            "type": "reaction_added",
            "user": "U0HUMAN1",
            "item": {"type": "message", "ts": "1700000000.000100"},
        },
    });
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("reaction_added must parse to empty Vec");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn parse_socket_payload_invalid_json_returns_none() {
    // Practically unreachable since the payload arrives as a parsed
    // Value, but guards against a future caller passing something
    // un-serializable. serde_json::Value::Null serializes fine, so we
    // cover the "valid Value but invalid downstream" path: an event
    // missing the expected `event` field still parses to an empty Vec.
    let payload = json!({"unexpected": "shape"});
    let messages = parse_socket_payload(&payload, &[])
        .await
        .expect("non-event payload should parse to empty Vec");
    assert!(messages.is_empty());
}
