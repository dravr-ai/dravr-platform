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
use pierre_contremaitre::messaging_strings::{
    KEY_DETAIL_WALK_COMPLETE, KEY_HISTORY_WALK_COMPLETE, KEY_LIVE_READ_OVERTAKEN,
};
use pierre_core::models::messaging::MessageContent;
use pierre_core::models::{AddMessageParams, TenantId, COMMAND_FINISH_REASON};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::BackfillJobKind;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_mcp_server::services::platform_post::{PlatformPost, PlatformPostRequest, PostedReply};
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, DetailWalkCompletion, HistoryWalkCompletion,
};
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
    telegram_thread_warmed(true).await
}

/// A Telegram thread for an `en` athlete, with a warmed window when `warmed`.
async fn telegram_thread_warmed(warmed: bool) -> Thread {
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
    if warmed {
        seed_activity_cache(&db, user, tenant, &[cached_activity("a-1", "Tempo run", 2)]).await;
    }
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
    assert!(
        thread.post.asked.lock().unwrap().is_empty(),
        "bug: a live read that gave up and was asked past re-asked the athlete"
    );
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

/// A detail walk's end is one line in the athlete's personal thread, the
/// count of activities whose efforts it read, sent once per walk however
/// many replicas end it; a later walk, asked later, posts its own.
#[tokio::test]
async fn a_detail_walk_end_posts_one_line_per_walk_in_the_personal_thread() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user, tenant) = seed_user(&db).await;
    repos.users.update_locale(user, "en").await.unwrap();
    let conversation = seed_conversation(&db, &user.to_string(), tenant).await;
    let catalogue = strings();
    // The line is authored in dravr-contremaitre, which lands first; until
    // the pinned catalogue carries it, a stand-in template is registered so
    // what is pinned here is the send-once, through the catalogue's render.
    if catalogue
        .get_exact(KEY_DETAIL_WALK_COMPLETE, "en")
        .is_none()
    {
        catalogue.update(
            KEY_DETAIL_WALK_COMPLETE,
            "en",
            "{0}".to_owned(),
            "stand-in".to_owned(),
        );
    }
    let notifier = ServerBackfillNotifier::with_resolver_and_platform_post(
        Arc::clone(&repos),
        Arc::clone(&catalogue),
        Arc::new(FakeResolver::new(Arc::new(CapturingChannel::default()))),
        Arc::new(RecordingPost::default()),
        Arc::from(LINK_SECRET),
        BASE_URL.to_owned(),
    );
    let completion = DetailWalkCompletion {
        user_id: user,
        tenant_id: tenant,
        provider: "strava",
        asked_at_ms: 1_763_000_000_000,
        activity_count: 37,
    };

    notifier.push_detail_walk_complete(completion).await;
    notifier.push_detail_walk_complete(completion).await;

    let expected = catalogue.render(KEY_DETAIL_WALK_COMPLETE, "en", &["37"]);
    assert!(
        expected.contains("37"),
        "the catalogue's line names the count: {expected}"
    );
    let posted = |repos: Arc<RepositoryRegistry>, conversation: String| async move {
        repos
            .chat
            .get_messages(&conversation, &user.to_string(), tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| message.role == "assistant")
            .map(|message| message.content)
            .collect::<Vec<String>>()
    };
    assert_eq!(
        posted(Arc::clone(&repos), conversation.clone()).await,
        vec![expected.clone()],
        "bug: a detail walk's end is one line in the personal thread, sent once"
    );

    notifier
        .push_detail_walk_complete(DetailWalkCompletion {
            asked_at_ms: completion.asked_at_ms + 1,
            ..completion
        })
        .await;
    assert_eq!(
        posted(Arc::clone(&repos), conversation).await.len(),
        2,
        "bug: a later walk's end was taken for the first one's"
    );
}

/// A Telegram thread with nothing warmed for the read: the cache read comes
/// back empty, the case the "ask me again" nudge answers for a read that
/// landed. A read that gave up never gets it — the nudge says data landed.
#[tokio::test]
async fn a_live_read_that_gave_up_on_an_empty_window_sends_no_nudge() {
    let empty = telegram_thread_warmed(false).await;
    let asked = add(&empty, "user", "What did I ride in 2019?", None).await;

    empty
        .notifier
        .push_backfill_complete(BackfillCompletion {
            provider_unavailable: true,
            ..live_read(&empty, Some(&asked))
        })
        .await;

    assert_eq!(
        empty.post.asked.lock().unwrap().as_slice(),
        ["What did I ride in 2019?"],
        "the ask a read that gave up was bound to is re-asked"
    );
    assert!(
        sent_bodies(&empty).is_empty(),
        "bug: a read that gave up sent the ready nudge on a channel: {:?}",
        sent_bodies(&empty)
    );
}

/// The in-app arm of a read that gave up. The agent's re-ask is the only
/// thing the athlete may receive: never the ready nudge, never the list,
/// never the "your history is ready" push that a stored answer raises for a
/// read that landed.
#[cfg(feature = "client-notifications")]
mod in_app {
    use std::time::Duration as StdDuration;

    use pierre_chat_pipeline::{AssistantTurn, ReplyBlock};
    use pierre_core::models::MessageRecord;
    use pierre_database::backends::factory::{Database, DatabaseBackend};
    use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
    use std::sync::OnceLock;
    use tokio::time::sleep;
    use tokio_util::task::TaskTracker;

    use super::*;

    /// How long the fire-and-forget push dispatch is given to store its row.
    const DISPATCH_SETTLE: StdDuration = StdDuration::from_millis(400);

    /// A platform post that answers in the agent's words and stores the turn,
    /// or withholds it, as the pipeline does.
    struct AgentPost {
        posted: bool,
        asked: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PlatformPost for AgentPost {
        async fn post(&self, req: PlatformPostRequest<'_>) -> Option<PostedReply> {
            self.asked.lock().unwrap().push(req.prompt.to_owned());
            if !self.posted {
                return None;
            }
            Some(PostedReply {
                assistant: AssistantTurn {
                    message: MessageRecord {
                        id: "msg-agent".to_owned(),
                        conversation_id: req.conversation_id.to_owned(),
                        role: "assistant".to_owned(),
                        content: "Strava is not answering right now.".to_owned(),
                        token_count: None,
                        prompt_tokens: None,
                        model: None,
                        finish_reason: Some("stop".to_owned()),
                        content_blocks: None,
                        created_at: Utc::now().to_rfc3339(),
                        transport_policy: TransportPolicy::AnyTransport,
                        channel_type: None,
                        about_user_id: None,
                        turn_attempted_write: None,
                    },
                    blocks: vec![ReplyBlock::Prose {
                        text: "Strava is not answering right now.".to_owned(),
                    }],
                    finish_reason: Some("stop".to_owned()),
                },
            })
        }
    }

    struct InAppThread {
        /// Held for the thread's life: the pool the notifier writes through.
        _db: Database,
        repos: Arc<RepositoryRegistry>,
        user: Uuid,
        tenant: TenantId,
        conversation: String,
        post: Arc<AgentPost>,
        service: Arc<NotificationService>,
        notifier: Arc<dyn BackfillNotifier>,
    }

    fn notification_service(db: &Database) -> Arc<NotificationService> {
        Arc::new(match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
            }
        })
    }

    /// A web conversation (no channel session) for an `en` athlete, with the
    /// app push wired, whose agent post is stored when `posted`, and with a
    /// warmed window when `warmed`.
    async fn in_app_thread(posted: bool, warmed: bool) -> InAppThread {
        let db = create_test_db().await;
        let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
        let (user, tenant) = seed_user(&db).await;
        repos.users.update_locale(user, "en").await.unwrap();
        let conversation = seed_conversation(&db, &user.to_string(), tenant).await;
        if warmed {
            seed_activity_cache(&db, user, tenant, &[cached_activity("a-1", "Tempo run", 2)]).await;
        }
        let post = Arc::new(AgentPost {
            posted,
            asked: Mutex::new(Vec::new()),
        });
        let service = notification_service(&db);
        let slot: Arc<dyn PlatformPost> = post.clone();
        let notifier = ServerBackfillNotifier::from_handles(
            Arc::clone(&repos),
            strings(),
            Arc::new(OnceLock::from(slot)),
            Arc::from(LINK_SECRET),
            BASE_URL.to_owned(),
            Some(Arc::clone(&service)),
        );
        InAppThread {
            _db: db,
            repos,
            user,
            tenant,
            conversation,
            post,
            service,
            notifier,
        }
    }

    impl InAppThread {
        async fn ask(&self, content: &str) -> String {
            self.repos
                .chat
                .add_message(&AddMessageParams {
                    tenant_id: self.tenant,
                    conversation_id: &self.conversation,
                    user_id: &self.user.to_string(),
                    role: "user",
                    content,
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
                .unwrap()
                .id
        }

        async fn finish(&self, asked: &str, provider_unavailable: bool) {
            self.notifier
                .push_backfill_complete(BackfillCompletion {
                    kind: BackfillJobKind::LiveRead,
                    asked_message_id: Some(asked),
                    provider_unavailable,
                    ..BackfillCompletion::backfill(
                        self.user,
                        self.tenant,
                        &self.conversation,
                        "strava",
                        (Utc::now() - Duration::days(30)).timestamp(),
                        1,
                    )
                })
                .await;
        }

        /// The turns the notifier itself stored in the thread.
        async fn stored_turns(&self) -> Vec<String> {
            self.repos
                .chat
                .get_messages(&self.conversation, &self.user.to_string(), self.tenant)
                .await
                .unwrap()
                .into_iter()
                .filter(|message| message.role == "assistant")
                .map(|message| message.content)
                .collect()
        }

        /// The "your history is ready" pushes the athlete holds.
        async fn ready_pushes(&self) -> usize {
            sleep(DISPATCH_SETTLE).await;
            let (rows, _, _) = self
                .service
                .list_notifications(
                    self.user,
                    CommereTenantId(self.tenant.as_uuid()),
                    50,
                    0,
                    None,
                    false,
                )
                .await
                .unwrap();
            rows.iter()
                .filter(|row| row.notification_type == "backfill_complete")
                .count()
        }
    }

    /// The control: a read that landed and whose answer the pipeline stored
    /// raises the one push that says so. Without it the next test could not
    /// tell a guard from a push that never fires.
    #[tokio::test]
    async fn a_landed_read_whose_answer_was_stored_pings_the_athlete() {
        let thread = in_app_thread(true, true).await;
        let asked = thread.ask("How far did I run this month?").await;
        thread.finish(&asked, false).await;
        assert_eq!(
            thread.ready_pushes().await,
            1,
            "a landed read's stored answer raises the ready push"
        );
    }

    #[tokio::test]
    async fn a_read_that_gave_up_never_pings_its_stored_re_ask() {
        let thread = in_app_thread(true, true).await;
        let asked = thread.ask("How far did I run this month?").await;
        thread.finish(&asked, true).await;
        assert_eq!(
            thread.post.asked.lock().unwrap().as_slice(),
            ["How far did I run this month?"],
            "the ask a read that gave up was bound to is re-asked"
        );
        assert_eq!(
            thread.ready_pushes().await,
            0,
            "bug: a read that gave up pushed \"your history is ready\" over the agent's re-ask"
        );
    }

    #[tokio::test]
    async fn a_read_that_gave_up_stores_no_list_when_its_re_ask_is_withheld() {
        let thread = in_app_thread(false, true).await;
        let asked = thread.ask("How far did I run this month?").await;
        thread.finish(&asked, true).await;
        assert!(
            thread.stored_turns().await.is_empty(),
            "bug: a read that gave up stored the list of what landed: {:?}",
            thread.stored_turns().await
        );
        assert_eq!(thread.ready_pushes().await, 0, "bug: and pinged it");
    }

    #[tokio::test]
    async fn a_read_that_gave_up_on_an_empty_window_stores_no_nudge() {
        let thread = in_app_thread(false, false).await;
        let asked = thread.ask("How far did I run this month?").await;
        thread.finish(&asked, true).await;
        assert_eq!(
            thread.post.asked.lock().unwrap().as_slice(),
            ["How far did I run this month?"],
            "the ask is re-asked, never answered with the nudge"
        );
        assert!(
            thread.stored_turns().await.is_empty(),
            "bug: a read that gave up stored the ready nudge: {:?}",
            thread.stored_turns().await
        );
        assert_eq!(thread.ready_pushes().await, 0, "bug: and pinged it");
    }
}
