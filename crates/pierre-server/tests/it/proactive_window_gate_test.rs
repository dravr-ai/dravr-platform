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

use axum::http::StatusCode;
use dravr_canot::channel::MessagingChannel;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::usage::JwtUsage;
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::InsertMessageParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_services::admin_ops;
use pierre_tool_runtime::runtime::{BackfillCompletion, BackfillNotifier, ReauthRetry};
use serde_json::{json, Value};
use tokio::time;
use uuid::Uuid;

use crate::check_in_post_test::backdate_session;
use crate::common::create_test_server_resources;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::date_golden::restamp;
use crate::helpers::drained_turn::{
    compute_whatsapp_sig, setup_whatsapp_config, WA_BUSINESS_ID, WA_PHONE_NUMBER_ID,
};
use crate::helpers::notify_capture::{capture_notify, named, only};
use crate::messaging_fixtures::{
    cached_activity, seed_activity_cache, seed_channel_link, seed_conversation, seed_session,
    seed_user, strings, CapturingChannel, FakeResolver,
};

/// The secret Meta signs the tenant's `WhatsApp` webhooks with.
const WEBHOOK_SECRET: &str = "window-gate-webhook-secret";
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
/// Move a chat row's instant, which the repository stamps with the wall clock.
const MOVE_MESSAGE: &str = "UPDATE chat_messages SET created_at = $1 WHERE CAST(id AS TEXT) = $2";

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
            (Timestamp::now() - Elapsed::days(30)).epoch_seconds(),
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

    /// Meta's status webhook reporting `wamid` failed with `code`, signed and
    /// posted to the tenant's `WhatsApp` webhook route as Meta posts it.
    async fn fail_late(&self, wamid: &str, code: i64) {
        let payload = failed_status_payload(wamid, code);
        let body = serde_json::to_vec(&payload).unwrap();
        let signature = compute_whatsapp_sig(WEBHOOK_SECRET, &body);
        let status = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &signature)
            .json(&payload)
            .send(MessagingRoutes::routes(Arc::clone(&self.resources)))
            .await
            .status_code();
        assert_eq!(status, StatusCode::OK, "webhooks always ack");
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
async fn whatsapp_dm(last_wrote: Elapsed) -> Dm {
    let resources = create_test_server_resources().await.unwrap();
    let db: &Database = &resources.agent.database;
    let (user, tenant) = seed_user(db).await;
    let owner = user.to_string();
    let conversation = seed_conversation(db, &owner, tenant).await;
    let session = seed_session(db, &owner, tenant, "whatsapp", PHONE, None, &conversation).await;
    seed_channel_link(db, &owner, tenant, "whatsapp", PHONE).await;
    setup_whatsapp_config(&*resources.common.repos.messaging, tenant, WEBHOOK_SECRET).await;
    backdate_session(db, &session, Timestamp::now() - last_wrote).await;
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

/// A Meta status webhook, in the shape Meta posts it, reporting `wamid` to the
/// athlete failed with error `code`.
fn failed_status_payload(wamid: &str, code: i64) -> Value {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": WA_BUSINESS_ID,
            "changes": [{
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+15550000003",
                        "phone_number_id": WA_PHONE_NUMBER_ID
                    },
                    "statuses": [{
                        "id": wamid,
                        "status": "failed",
                        "timestamp": "1782850000",
                        "recipient_id": PHONE,
                        "errors": [{ "code": code, "title": "Re-engagement message" }]
                    }]
                },
                "field": "messages"
            }]
        }]
    })
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
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(30)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(30)).await;
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
async fn a_closed_window_writes_the_reconnect_sentence_once_while_it_stands() {
    let dm = whatsapp_dm(Elapsed::hours(30)).await;
    let channel = whatsapp(&["wamid.never"]);
    let notifier = dm.notifier(&channel);
    let (events, _guard) = capture_notify();

    // Two automatic turns on the dead session ask in a row.
    for _ in 0..2 {
        notifier
            .push_provider_reauth(dm.user, dm.tenant, &dm.conversation, PROVIDER)
            .await;
    }
    let turns = dm.assistant_turns().await;
    assert_eq!(
        turns.len(),
        1,
        "the sentence still the latest turn is not written again: {turns:?}"
    );

    // The coach says something else; the sentence was written today.
    dm.resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: dm.tenant,
            conversation_id: &dm.conversation,
            user_id: &dm.user.to_string(),
            role: "assistant",
            content: "Your week looks balanced.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();
    notifier
        .push_provider_reauth(dm.user, dm.tenant, &dm.conversation, PROVIDER)
        .await;
    assert_eq!(
        dm.assistant_turns().await.len(),
        2,
        "a sentence written within the day is not written again"
    );
    assert_eq!(named(&events, HELD).len(), 3, "each hold is still reported");
    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "nothing goes to the closed chat"
    );

    // A day on, the sentence is no longer the latest turn and no longer stands.
    let written = dm
        .resources
        .common
        .repos
        .chat
        .get_messages(&dm.conversation, &dm.user.to_string(), dm.tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.role == "assistant")
        .expect("the sentence was written first");
    restamp(
        dm.db(),
        MOVE_MESSAGE,
        Timestamp::now() - Elapsed::hours(25),
        &written.id,
        None,
    )
    .await;
    notifier
        .push_provider_reauth(dm.user, dm.tenant, &dm.conversation, PROVIDER)
        .await;
    let turns = dm.assistant_turns().await;
    assert_eq!(
        turns.len(),
        3,
        "a sentence written more than a day ago is written again: {turns:?}"
    );
    assert_eq!(turns[2], turns[0], "the same sentence is written again");
    assert_eq!(named(&events, HELD).len(), 4, "that hold is reported too");
}

#[tokio::test]
async fn an_open_window_sends_the_reconnect_answer_to_whatsapp() {
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(30)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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

    // Meta delivering the same status again writes and reports nothing more.
    dm.fail_late("wamid.late-1", 131_047).await;
    assert_eq!(
        dm.assistant_turns().await.len(),
        1,
        "a repeated webhook does not write the notice twice"
    );
    assert_eq!(
        named(&events, HELD).len(),
        1,
        "a repeated webhook does not report the hold twice"
    );
}

#[tokio::test]
async fn two_racing_deliveries_of_one_131047_write_the_notice_once() {
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
    dm.warm_cache().await;
    let channel = whatsapp(&["wamid.race-1"]);
    dm.notifier(&channel)
        .push_backfill_complete(dm.completion())
        .await;
    let (events, _guard) = capture_notify();

    tokio::join!(
        dm.fail_late("wamid.race-1", 131_047),
        dm.fail_late("wamid.race-1", 131_047)
    );

    assert_eq!(
        dm.assistant_turns().await,
        text_of(&channel),
        "the delivery that wins the claim writes the one in-app copy"
    );
    assert_eq!(
        named(&events, HELD).len(),
        1,
        "only the winning delivery reports the hold"
    );
}

#[tokio::test]
async fn a_late_131047_reaches_the_app_of_an_athlete_past_their_request_budget() {
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
    dm.warm_cache().await;
    let channel = whatsapp(&["wamid.budget-1"]);
    dm.notifier(&channel)
        .push_backfill_complete(dm.completion())
        .await;
    let sent = text_of(&channel);
    assert_eq!(
        sent.len(),
        1,
        "inside the window the notice went to the chat"
    );
    // The athlete's month is spent: an override of one, and one turn counted.
    let repos = &dm.resources.common.repos;
    admin_ops::set_user_rate_limit_override(repos, dm.user, Some(1), None, None)
        .await
        .unwrap();
    repos
        .usage
        .record_jwt_usage(&JwtUsage {
            id: None,
            user_id: dm.user,
            timestamp: Timestamp::now(),
            endpoint: "messaging:whatsapp".to_owned(),
            method: "AUTH".to_owned(),
            status_code: 200,
            response_time_ms: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
            client_grant_id: None,
        })
        .await
        .unwrap();

    dm.fail_late("wamid.budget-1", 131_047).await;

    assert_eq!(
        dm.assistant_turns().await,
        sent,
        "a spent request budget refuses the athlete's turns, never a notice owed to them"
    );
}

#[tokio::test]
async fn a_late_131047_on_a_split_notice_writes_it_whole_on_its_opening_message() {
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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
    let dm = whatsapp_dm(Elapsed::hours(2)).await;
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
