// ABOUTME: Backfill and reconnect notices pass the 24-hour window check-ins pass; WhatsApp's late 131047 reaches the app
// ABOUTME: Pins carnet#1672 — open window to WhatsApp, closed window to the thread plus messaging.proactive_held
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1672.
//!
//! A backfill-completion notice and a provider-reconnect notice are messages
//! the athlete did not just ask for, so on `WhatsApp` they pass Meta's
//! 24-hour window like a check-in: inside it they go to the chat, outside it
//! they are written into the conversation the app reads and
//! `messaging.proactive_held` says so. Meta reports an out-of-window refusal
//! only afterwards (error 131047 in a status webhook); a proactive notice that
//! fails that way is written into the conversation too, while a reply, and
//! every other error, stays as it was.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use dravr_canot::channel::MessagingChannel;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::InsertMessageParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::webhooks::parse_whatsapp_delivery_statuses;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_mcp_server::services::messaging_ingress::window_failure::{
    redeliver_in_app, WindowFailure,
};
use pierre_tool_runtime::runtime::{BackfillCompletion, BackfillNotifier, ReauthRetry};
use serde_json::json;
use tokio::time;
use uuid::Uuid;

use crate::check_in_post_test::backdate_session;
use crate::common::create_test_server_resources;
use crate::helpers::notify_capture::{capture_notify, named, only};
use crate::messaging_fixtures::{
    cached_activity, seed_activity_cache, seed_channel_link, seed_conversation, seed_session,
    seed_user, strings, CapturingChannel, FakeResolver,
};

/// The secret the notifier under test signs reconnect-link tokens with.
const LINK_SECRET: &str = "window-gate-link-secret";
/// The server root the notifier under test builds its links on.
const BASE_URL: &str = "https://window-gate.example";
/// The athlete's `WhatsApp` id.
const PHONE: &str = "15145550172";
/// A provider whose reconnect goes through the hosted sign-in.
const PROVIDER: &str = "sciotte_garmin";
/// The notify event every held or rerouted proactive message emits.
const HELD: &str = "messaging.proactive_held";

/// An athlete with a `WhatsApp` DM: its channel link, its session, and the
/// conversation the session writes into, which the app lists too.
struct Dm {
    resources: Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
    conversation: String,
    session: String,
}

impl Dm {
    fn db(&self) -> &Database {
        &self.resources.agent.database
    }

    /// A notifier whose channel adapter is `channel`.
    fn notifier(&self, channel: &Arc<CapturingChannel>) -> ServerBackfillNotifier {
        let adapter: Arc<dyn MessagingChannel> = channel.clone();
        ServerBackfillNotifier::with_resolver(
            Arc::clone(&self.resources.common.repos),
            strings(),
            Arc::new(FakeResolver::new(adapter)),
            Arc::from(LINK_SECRET),
            BASE_URL.to_owned(),
        )
    }

    /// Warm the activity cache as a finished backfill does.
    async fn warm_cache(&self) {
        seed_activity_cache(
            self.db(),
            self.user,
            self.tenant,
            &[cached_activity("a1", "Morning Trail Run", 2)],
        )
        .await;
    }

    /// The completion of a 30-day backfill of the warmed cache.
    fn completion(&self) -> BackfillCompletion<'_> {
        BackfillCompletion::backfill(
            self.user,
            self.tenant,
            &self.conversation,
            "strava",
            (Utc::now() - Duration::days(30)).timestamp(),
            1,
        )
    }

    /// Every assistant turn in the DM's conversation, oldest first.
    async fn assistant_turns(&self) -> Vec<String> {
        self.resources
            .common
            .repos
            .chat
            .get_messages(&self.conversation, &self.user.to_string(), self.tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| message.role == "assistant")
            .map(|message| message.content)
            .collect()
    }

    /// The window failure Meta's status webhook reports for `wamid`.
    async fn fail_late(&self, wamid: &str, code: i64) {
        let body = status_body(wamid, "failed", Some(code));
        let statuses = parse_whatsapp_delivery_statuses(body.as_bytes());
        let failure = statuses[0]
            .window_failure()
            .expect("a 131047 failure is a window failure");
        redeliver_in_app(&self.resources, self.tenant, failure).await;
    }

    /// Record an outbound ledger row on the DM's session, as a send does.
    async fn ledger_row(&self, wamid: &str, body: &str, turn: &str, purpose: Option<&str>) {
        let id = Uuid::new_v4().to_string();
        let inserted = self
            .resources
            .common
            .repos
            .messaging
            .insert_message(&InsertMessageParams {
                id: &id,
                tenant_id: self.tenant,
                session_id: &self.session,
                direction: "outbound",
                channel_type: "whatsapp",
                channel_message_id: wamid,
                sender_id: "pierre",
                content_type: "text",
                content_body: Some(body),
                correlation_id: turn,
                raw_payload: None,
                chat_message_id: None,
                proactive_purpose: purpose,
            })
            .await
            .unwrap();
        assert!(inserted, "a fresh ledger row inserts");
    }
}

/// A `WhatsApp` DM whose athlete last wrote `last_wrote` ago.
async fn whatsapp_dm(last_wrote: Duration) -> Dm {
    let resources = create_test_server_resources().await.unwrap();
    let db: &Database = &resources.agent.database;
    let (user, tenant) = seed_user(db).await;
    let owner = user.to_string();
    let conversation = seed_conversation(db, &owner, tenant).await;
    let session = seed_session(db, &owner, tenant, "whatsapp", PHONE, None, &conversation).await;
    seed_channel_link(db, &owner, tenant, "whatsapp", PHONE).await;
    backdate_session(db, &session, Utc::now() - last_wrote).await;
    Dm {
        resources,
        user,
        tenant,
        conversation,
        session,
    }
}

/// A `WhatsApp` channel that answers each send with the next of `wamids`.
fn whatsapp(wamids: &[&str]) -> Arc<CapturingChannel> {
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    channel.stub_receipt_ids(wamids.iter().map(|id| Some((*id).to_owned())));
    channel
}

/// A Meta status webhook body reporting `status` (and `code`) for `wamid`.
fn status_body(wamid: &str, status: &str, code: Option<i64>) -> String {
    let mut entry = json!({ "id": wamid, "status": status, "recipient_id": PHONE });
    if let Some(code) = code {
        entry["errors"] = json!([{ "code": code, "title": "Re-engagement message" }]);
    }
    json!({ "entry": [{ "changes": [{ "value": { "statuses": [entry] } }] }] }).to_string()
}

fn text_of(channel: &CapturingChannel) -> Vec<String> {
    channel
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|message| match &message.content {
            MessageContent::Text { body } => body.clone(),
            other => panic!("a notice is plain text: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn an_open_window_sends_the_backfill_notice_to_whatsapp_and_ledgers_it() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    dm.warm_cache().await;
    let channel = whatsapp(&["wamid.notice-1"]);
    let (events, _guard) = capture_notify();

    dm.notifier(&channel)
        .push_backfill_complete(dm.completion())
        .await;

    let sent = text_of(&channel);
    assert_eq!(
        sent.len(),
        1,
        "the open window lets the notice go to the chat"
    );
    assert!(sent[0].contains("Morning Trail Run"), "{}", sent[0]);
    assert_eq!(
        channel.sent.lock().unwrap()[0].recipient_id,
        PHONE,
        "the notice is addressed to the athlete's DM"
    );
    assert!(
        dm.assistant_turns().await.is_empty(),
        "nothing is written in the app"
    );
    assert!(named(&events, HELD).is_empty(), "nothing was held");

    let notice = dm
        .resources
        .common
        .repos
        .messaging
        .find_proactive_notice(dm.tenant, "whatsapp", "wamid.notice-1")
        .await
        .unwrap()
        .expect("the sent message is on the notice ledger");
    assert_eq!(
        notice.purpose, "backfill_complete",
        "the row names the notice"
    );
    assert_eq!(notice.user_id, dm.user.to_string(), "the session's athlete");
    assert_eq!(
        notice.conversation_id.as_deref(),
        Some(dm.conversation.as_str()),
        "the thread an in-app copy goes to"
    );
    assert_eq!(notice.parts.len(), 1, "one message was sent");
    assert_eq!(
        notice.parts[0].content_body.as_deref(),
        Some(sent[0].as_str()),
        "the row holds what the message carried"
    );
    assert_eq!(
        notice.parts[0].chat_message_id, None,
        "a templated list is not stored"
    );
}

#[tokio::test]
async fn a_closed_window_writes_the_backfill_notice_in_the_app_and_holds_it() {
    let dm = whatsapp_dm(Duration::hours(30)).await;
    dm.warm_cache().await;
    let channel = whatsapp(&["wamid.never"]);
    let (events, _guard) = capture_notify();

    dm.notifier(&channel)
        .push_backfill_complete(dm.completion())
        .await;

    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "Meta would refuse a free-form send outside the window, so none is attempted"
    );
    let turns = dm.assistant_turns().await;
    assert_eq!(
        turns.len(),
        1,
        "the notice is written into the conversation"
    );
    assert!(turns[0].contains("Morning Trail Run"), "{}", turns[0]);

    let held = only(&events, HELD);
    assert_eq!(held.field("purpose"), "backfill_complete", "what was held");
    assert_eq!(held.field("reason"), "window_closed", "why it was held");
    assert_eq!(held.field("channel"), "whatsapp", "where it was held");
    assert_eq!(
        held.field("tenant_id"),
        dm.tenant.to_string(),
        "whose it was"
    );
}

#[tokio::test]
async fn a_closed_window_answers_the_reconnect_in_the_app_without_a_link() {
    let dm = whatsapp_dm(Duration::hours(30)).await;
    let channel = whatsapp(&["wamid.never"]);
    let (events, _guard) = capture_notify();

    dm.notifier(&channel)
        .push_provider_reauth(dm.user, dm.tenant, &dm.conversation, PROVIDER)
        .await;

    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "nothing goes to the closed chat"
    );
    let turns = dm.assistant_turns().await;
    assert_eq!(
        turns.len(),
        1,
        "the reconnect sentence is written into the conversation"
    );
    assert!(
        turns[0].contains("Garmin"),
        "the provider is named: {}",
        turns[0]
    );
    assert!(
        !turns[0].contains(BASE_URL),
        "no one-time link is minted for a message that never travels: {}",
        turns[0]
    );
    let held = only(&events, HELD);
    assert_eq!(held.field("purpose"), "provider_reauth", "what was held");
    assert_eq!(held.field("reason"), "window_closed", "why it was held");
}

#[tokio::test]
async fn an_open_window_sends_the_reconnect_answer_to_whatsapp() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    let channel = whatsapp(&["wamid.reauth-1"]);
    let (events, _guard) = capture_notify();

    dm.notifier(&channel)
        .push_provider_reauth(dm.user, dm.tenant, &dm.conversation, PROVIDER)
        .await;

    let sent = text_of(&channel);
    assert_eq!(
        sent.len(),
        1,
        "the open window lets the nudge go to the chat"
    );
    assert!(
        sent[0].contains(BASE_URL),
        "the chat copy carries the link: {}",
        sent[0]
    );
    assert!(
        dm.assistant_turns().await.is_empty(),
        "nothing is written in the app"
    );
    assert!(named(&events, HELD).is_empty(), "nothing was held");
    let notice = dm
        .resources
        .common
        .repos
        .messaging
        .find_proactive_notice(dm.tenant, "whatsapp", "wamid.reauth-1")
        .await
        .unwrap()
        .expect("the nudge is on the notice ledger");
    assert_eq!(
        notice.purpose, "provider_reauth",
        "the row names the notice"
    );
}

#[tokio::test]
async fn a_linked_chat_whose_window_closed_gets_no_reconnect_notice() {
    let dm = whatsapp_dm(Duration::hours(30)).await;
    let channel = whatsapp(&["wamid.never"]);
    let (events, _guard) = capture_notify();

    let reached = dm
        .notifier(&channel)
        .push_reauth_to_linked_channels(dm.user, dm.tenant, PROVIDER, ReauthRetry::WhileFlagged)
        .await;

    assert_eq!(reached, 0, "a held chat is not counted as reached");
    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "nothing goes to the closed chat"
    );
    let queued = dm
        .resources
        .common
        .repos
        .messaging
        .claim_due_outbound(100, 60_000)
        .await
        .unwrap();
    assert!(
        queued.is_empty(),
        "an out-of-window send is not queued: {queued:?}"
    );
    let held = only(&events, HELD);
    assert_eq!(held.field("purpose"), "provider_reauth", "what was held");
    assert_eq!(held.field("reason"), "window_closed", "why it was held");
}

#[tokio::test]
async fn a_linked_chat_inside_its_window_gets_the_reconnect_notice_ledgered() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    let channel = whatsapp(&["wamid.linked-1"]);

    let reached = dm
        .notifier(&channel)
        .push_reauth_to_linked_channels(dm.user, dm.tenant, PROVIDER, ReauthRetry::WhileFlagged)
        .await;

    assert_eq!(reached, 1, "the open chat is reached");
    assert_eq!(
        channel.sent.lock().unwrap()[0].recipient_id,
        PHONE,
        "the notice is addressed to the linked chat"
    );
    let notice = dm
        .resources
        .common
        .repos
        .messaging
        .find_proactive_notice(dm.tenant, "whatsapp", "wamid.linked-1")
        .await
        .unwrap()
        .expect("the linked chat's notice is on the ledger");
    assert_eq!(
        notice.purpose, "provider_reauth",
        "the row names the notice"
    );
}

#[tokio::test]
async fn a_late_131047_writes_the_notice_in_the_app_once_and_holds_it() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    dm.warm_cache().await;
    let channel = whatsapp(&["wamid.late-1"]);
    dm.notifier(&channel)
        .push_backfill_complete(dm.completion())
        .await;
    let sent = text_of(&channel);
    assert_eq!(
        sent.len(),
        1,
        "inside the window the notice went to the chat"
    );
    let (events, _guard) = capture_notify();

    dm.fail_late("wamid.late-1", 131_047).await;

    let turns = dm.assistant_turns().await;
    assert_eq!(
        turns, sent,
        "the notice Meta failed is written into the conversation"
    );
    let held = only(&events, HELD);
    assert_eq!(held.field("purpose"), "backfill_complete", "what was held");
    assert_eq!(
        held.field("reason"),
        "window_closed_on_delivery",
        "why it was held"
    );
    assert_eq!(held.field("channel"), "whatsapp", "where it was held");

    // Meta delivering the same status again writes nothing more.
    dm.fail_late("wamid.late-1", 131_047).await;
    assert_eq!(
        dm.assistant_turns().await.len(),
        1,
        "a repeated webhook does not write the notice twice"
    );
}

#[tokio::test]
async fn a_late_131047_on_a_split_notice_writes_it_whole_on_its_opening_message() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    let turn = Uuid::new_v4().to_string();
    dm.ledger_row("wamid.part-1", "First half.", &turn, Some("check_in"))
        .await;
    // Distinct instants, so the send order is the ledger's order.
    time::sleep(StdDuration::from_millis(5)).await;
    dm.ledger_row("wamid.part-2", "Second half.", &turn, Some("check_in"))
        .await;
    let (events, _guard) = capture_notify();

    // Each message fails on its own; the later one arrives first.
    dm.fail_late("wamid.part-2", 131_047).await;
    assert!(
        dm.assistant_turns().await.is_empty(),
        "a later part waits for the opening one"
    );
    assert!(
        named(&events, HELD).is_empty(),
        "nothing is held for a later part"
    );

    dm.fail_late("wamid.part-1", 131_047).await;
    assert_eq!(
        dm.assistant_turns().await,
        vec!["First half.\n\nSecond half.".to_owned()],
        "the whole notice is written once, in order"
    );
    assert_eq!(
        only(&events, HELD).field("purpose"),
        "check_in",
        "what was held"
    );
}

#[tokio::test]
async fn a_late_131047_on_a_reply_is_left_as_logged() {
    let dm = whatsapp_dm(Duration::hours(2)).await;
    let turn = Uuid::new_v4().to_string();
    dm.ledger_row("wamid.reply-1", "Here is your week.", &turn, None)
        .await;
    let (events, _guard) = capture_notify();

    dm.fail_late("wamid.reply-1", 131_047).await;

    assert!(
        dm.assistant_turns().await.is_empty(),
        "a reply is not a proactive notice"
    );
    assert!(
        named(&events, HELD).is_empty(),
        "nothing is held for a reply"
    );
}

#[test]
fn only_a_failed_131047_is_a_window_failure() {
    let parse = |status: &str, code: Option<i64>| {
        parse_whatsapp_delivery_statuses(status_body("wamid.x", status, code).as_bytes())
    };
    let window: Vec<_> = parse("failed", Some(131_047))
        .iter()
        .filter_map(|s| {
            s.window_failure().map(
                |WindowFailure {
                     recipient_id,
                     message_id,
                 }| { (recipient_id.to_owned(), message_id.to_owned()) },
            )
        })
        .collect();
    assert_eq!(
        window,
        vec![(PHONE.to_owned(), "wamid.x".to_owned())],
        "a failed 131047 names its recipient and message"
    );

    // Another failure (here: the number is not on WhatsApp) stays a warning.
    assert!(
        parse("failed", Some(131_026))[0].window_failure().is_none(),
        "another error code is no window failure"
    );
    assert!(
        parse("failed", None)[0].window_failure().is_none(),
        "a failure with no code is no window failure"
    );
    assert!(
        parse("delivered", None)[0].window_failure().is_none(),
        "a delivered message is no failure"
    );
}
