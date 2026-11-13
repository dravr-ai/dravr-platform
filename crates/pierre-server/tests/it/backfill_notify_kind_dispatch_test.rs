// ABOUTME: A finished background read answers by kind: a live read the athlete asked past gets a one-line note
// ABOUTME: Pins the overtaken check against the conversation's newest user message and which question a live read re-asks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A live read that outran its turn answers the athlete's message once it
//! lands. When they have asked something newer since, re-answering the old
//! question would talk past them, so the conversation gets one line saying
//! the data landed instead (B5). A backfill keeps answering the newest
//! question, as it always has.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use pierre_contremaitre::messaging_strings::{KEY_HISTORY_WALK_COMPLETE, KEY_LIVE_READ_OVERTAKEN};
use pierre_core::models::messaging::MessageContent;
use pierre_core::models::{AddMessageParams, TenantId, COMMAND_FINISH_REASON};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::BackfillJobKind;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_mcp_server::services::platform_post::{PlatformPost, PlatformPostRequest, PostedReply};
use pierre_tool_runtime::runtime::{BackfillCompletion, BackfillNotifier, HistoryWalkCompletion};
use uuid::Uuid;

use crate::messaging_fixtures;
use messaging_fixtures::{
    cached_activity, create_test_db, seed_activity_cache, seed_conversation, seed_session,
    seed_user, strings, CapturingChannel, FakeResolver,
};

const LINK_SECRET: &str = "kind-dispatch-link-secret";
const BASE_URL: &str = "https://kind-dispatch.example";

/// A platform post that records the question it was asked and withholds its
/// answer, so the notifier falls back to the list it renders itself.
#[derive(Default)]
struct RecordingPost {
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl PlatformPost for RecordingPost {
    async fn post(&self, req: PlatformPostRequest<'_>) -> Option<PostedReply> {
        self.asked.lock().unwrap().push(req.prompt.to_owned());
        None
    }
}

struct Thread {
    repos: Arc<RepositoryRegistry>,
    user: Uuid,
    tenant: TenantId,
    conversation: String,
    channel: Arc<CapturingChannel>,
    post: Arc<RecordingPost>,
    notifier: ServerBackfillNotifier,
}

async fn add(thread: &Thread, role: &str, content: &str, finish_reason: Option<&str>) -> String {
    thread
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: thread.tenant,
            conversation_id: &thread.conversation,
            user_id: &thread.user.to_string(),
            role,
            content,
            token_count: None,
            finish_reason,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap()
        .id
}

/// A Telegram thread for an `en` athlete, with a warmed window to list.
async fn telegram_thread() -> Thread {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user, tenant) = seed_user(&db).await;
    repos.users.update_locale(user, "en").await.unwrap();
    let conversation = seed_conversation(&db, &user.to_string(), tenant).await;
    seed_session(
        &db,
        &user.to_string(),
        tenant,
        "telegram",
        "tg_user_b5",
        Some("tg_chat_b5"),
        &conversation,
    )
    .await;
    seed_activity_cache(&db, user, tenant, &[cached_activity("a-1", "Tempo run", 2)]).await;
    let channel = Arc::new(CapturingChannel::default());
    let post = Arc::new(RecordingPost::default());
    let notifier = ServerBackfillNotifier::with_resolver_and_platform_post(
        Arc::clone(&repos),
        strings(),
        Arc::new(FakeResolver::new(channel.clone())),
        post.clone(),
        Arc::from(LINK_SECRET),
        BASE_URL.to_owned(),
    );
    Thread {
        repos,
        user,
        tenant,
        conversation,
        channel,
        post,
        notifier,
    }
}

fn live_read<'a>(thread: &'a Thread, asked: Option<&'a str>) -> BackfillCompletion<'a> {
    BackfillCompletion {
        kind: BackfillJobKind::LiveRead,
        asked_message_id: asked,
        ..BackfillCompletion::backfill(
            thread.user,
            thread.tenant,
            &thread.conversation,
            "strava",
            (Utc::now() - Duration::days(30)).timestamp(),
            1,
        )
    }
}

fn sent_bodies(thread: &Thread) -> Vec<String> {
    thread
        .channel
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|message| match &message.content {
            MessageContent::Text { body } => body.clone(),
            other => panic!("expected a text message, got {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn a_live_read_the_athlete_asked_past_posts_only_the_note() {
    let thread = telegram_thread().await;
    let asked = add(&thread, "user", "How far did I run this month?", None).await;
    add(
        &thread,
        "assistant",
        "Still loading your latest runs.",
        None,
    )
    .await;
    add(&thread, "user", "Should I rest tomorrow?", None).await;

    thread
        .notifier
        .push_backfill_complete(live_read(&thread, Some(&asked)))
        .await;

    let note = strings().render(KEY_LIVE_READ_OVERTAKEN, "en", &[]);
    assert!(!note.is_empty(), "the overtaken note has English copy");
    assert_eq!(
        sent_bodies(&thread),
        vec![note],
        "an overtaken live read posts exactly the one-line note"
    );
    assert!(
        thread.post.asked.lock().unwrap().is_empty(),
        "the question the athlete moved on from is never re-asked"
    );
}

#[tokio::test]
async fn a_live_read_whose_ask_is_still_the_newest_answers_it() {
    let thread = telegram_thread().await;
    add(&thread, "user", "What did I do last week?", None).await;
    add(&thread, "assistant", "A long run and two easy ones.", None).await;
    let asked = add(&thread, "user", "How far did I run this month?", None).await;
    // A slash command is account state, not a newer question.
    add(&thread, "user", "/status", Some(COMMAND_FINISH_REASON)).await;

    thread
        .notifier
        .push_backfill_complete(live_read(&thread, Some(&asked)))
        .await;

    assert_eq!(
        thread.post.asked.lock().unwrap().as_slice(),
        ["How far did I run this month?"],
        "the follow-up re-asks the message the live read was asked from"
    );
    let note = strings().render(KEY_LIVE_READ_OVERTAKEN, "en", &[]);
    let bodies = sent_bodies(&thread);
    assert_eq!(bodies.len(), 1, "one follow-up is sent: {bodies:?}");
    assert_ne!(
        bodies[0], note,
        "a live read still answering the newest ask sends its answer, not the note"
    );
}

/// A live read whose every attempt failed still answers the ask it was bound
/// to, by re-asking it: the agent words the provider being unavailable. When
/// that answer is withheld nothing is sent — the list and the ready nudge
/// both say data landed, and none did.
#[tokio::test]
async fn a_live_read_that_gave_up_re_asks_and_never_posts_the_list() {
    let thread = telegram_thread().await;
    let asked = add(&thread, "user", "How far did I run this month?", None).await;

    thread
        .notifier
        .push_backfill_complete(BackfillCompletion {
            provider_unavailable: true,
            ..live_read(&thread, Some(&asked))
        })
        .await;

    assert_eq!(
        thread.post.asked.lock().unwrap().as_slice(),
        ["How far did I run this month?"],
        "bug: the ask a read that gave up was bound to is not re-asked"
    );
    assert!(
        sent_bodies(&thread).is_empty(),
        "bug: a read that gave up posted a list or a note saying its data landed: {:?}",
        sent_bodies(&thread)
    );
}

/// The same read, asked past: the overtaken note says the data landed, which
/// is untrue here, so nothing is posted and nothing re-asked.
#[tokio::test]
async fn a_live_read_that_gave_up_and_was_asked_past_posts_nothing() {
    let thread = telegram_thread().await;
    let asked = add(&thread, "user", "How far did I run this month?", None).await;
    add(&thread, "user", "Should I rest tomorrow?", None).await;

    thread
        .notifier
        .push_backfill_complete(BackfillCompletion {
            provider_unavailable: true,
            ..live_read(&thread, Some(&asked))
        })
        .await;

    assert!(
        sent_bodies(&thread).is_empty(),
        "bug: a read that gave up told the athlete its data landed"
    );
    assert!(thread.post.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_backfill_is_never_taken_for_overtaken() {
    let thread = telegram_thread().await;
    add(&thread, "user", "Show me my 2022 races", None).await;
    add(&thread, "user", "And 2021?", None).await;

    thread
        .notifier
        .push_backfill_complete(BackfillCompletion::backfill(
            thread.user,
            thread.tenant,
            &thread.conversation,
            "strava",
            (Utc::now() - Duration::days(900)).timestamp(),
            1,
        ))
        .await;

    assert_eq!(
        thread.post.asked.lock().unwrap().as_slice(),
        ["And 2021?"],
        "a backfill re-asks the newest question, whatever asked it"
    );
}

#[tokio::test]
async fn the_newest_user_message_skips_commands_and_other_tenants() {
    let thread = telegram_thread().await;
    let user = thread.user.to_string();
    assert_eq!(
        thread
            .repos
            .chat
            .latest_user_message_id(&thread.conversation, &user, thread.tenant)
            .await
            .unwrap(),
        None,
        "a thread with no question has no newest message"
    );
    let question = add(&thread, "user", "How did I sleep?", None).await;
    add(&thread, "assistant", "Seven hours.", None).await;
    add(&thread, "user", "/help", Some(COMMAND_FINISH_REASON)).await;
    assert_eq!(
        thread
            .repos
            .chat
            .latest_user_message_id(&thread.conversation, &user, thread.tenant)
            .await
            .unwrap()
            .as_deref(),
        Some(question.as_str()),
        "the newest question is the newest non-command user row"
    );
    assert_eq!(
        thread
            .repos
            .chat
            .latest_user_message_id(
                &thread.conversation,
                &user,
                TenantId::from_uuid(Uuid::new_v4())
            )
            .await
            .unwrap(),
        None,
        "another tenant reads nothing of the thread"
    );
}

/// A walk no conversation asked for ends with one line in the athlete's
/// personal thread naming the oldest year and the count (B4) — once, however
/// many replicas finish it. An athlete with no thread yet hears nothing, and
/// gets no thread forged for it.
#[tokio::test]
async fn a_walk_end_posts_one_line_in_the_personal_thread() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user, tenant) = seed_user(&db).await;
    repos.users.update_locale(user, "en").await.unwrap();
    let conversation = seed_conversation(&db, &user.to_string(), tenant).await;
    let notifier = ServerBackfillNotifier::with_resolver_and_platform_post(
        Arc::clone(&repos),
        strings(),
        Arc::new(FakeResolver::new(Arc::new(CapturingChannel::default()))),
        Arc::new(RecordingPost::default()),
        Arc::from(LINK_SECRET),
        BASE_URL.to_owned(),
    );
    let completion = HistoryWalkCompletion {
        user_id: user,
        tenant_id: tenant,
        provider: "sciotte_garmin",
        oldest: Utc.with_ymd_and_hms(2014, 6, 2, 8, 0, 0).unwrap(),
        activity_count: 412,
    };

    notifier.push_history_walk_complete(completion).await;
    notifier.push_history_walk_complete(completion).await;

    let expected = strings().render(KEY_HISTORY_WALK_COMPLETE, "en", &["2014", "412"]);
    assert!(
        expected.contains("2014") && expected.contains("412"),
        "the catalogue's line names the year and the count: {expected}"
    );
    let posted: Vec<String> = repos
        .chat
        .get_messages(&conversation, &user.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.content)
        .collect();
    assert_eq!(
        posted,
        vec![expected],
        "bug: the walk's end is one line in the personal thread, sent once"
    );

    let (stranger, stranger_tenant) = seed_user(&db).await;
    notifier
        .push_history_walk_complete(HistoryWalkCompletion {
            user_id: stranger,
            tenant_id: stranger_tenant,
            ..completion
        })
        .await;
    assert!(
        repos
            .chat
            .personal_conversation(&stranger.to_string(), stranger_tenant)
            .await
            .unwrap()
            .is_none(),
        "bug: an athlete with no thread gets none forged for a walk's end"
    );
}
