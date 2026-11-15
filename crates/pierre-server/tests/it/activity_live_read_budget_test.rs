// ABOUTME: A live activity read that outlives the refresh budget answers from the cache, never fails the call
// ABOUTME: The read is a durable live_read job: it lands for the next turn, the sweep resumes it, and its follow-up answers the newest ask
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A healthy scrape takes about two minutes and the chat tool loop gives a
//! tool call ninety seconds, so `get_activities` reading a scraped provider
//! inline could fail on a provider that was working. These tests drive
//! [`read_live_window_within_budget`] with a provider that answers only when
//! the test lets it. The read is a `live_read` job row from the moment it
//! starts, so an instance that dies under it leaves the follow-up owed.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
#[cfg(feature = "client-messaging")]
use std::sync::{Mutex, OnceLock};
use std::time::Duration as StdDuration;
#[cfg(feature = "client-messaging")]
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ActivityBuilder, Athlete, SportType, Stats, TenantId};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_database::repositories::activity_cache::{
    ActivityFetchFailure, ActivityFetchFailureRecord,
};
use pierre_database::repositories::{BackfillJobKind, BackfillJobState};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_tool_runtime::activity_fetch::budgeted_read::{
    read_live_window_within_budget, BudgetedRead, OwnedLiveRead,
};
use pierre_tool_runtime::activity_fetch::{refresh_stale_head, StaleReason};
use pierre_tool_runtime::context::TurnAsk;
use pierre_tool_runtime::runtime::ToolRuntime;
#[cfg(feature = "client-messaging")]
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, DetailWalkCompletion, HistoryWalkCompletion, ReauthRetry,
};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

pub(crate) const PROVIDER: &str = "sciotte";
pub(crate) const BUDGET: StdDuration = StdDuration::from_millis(200);

/// A provider whose list read waits until the test notifies it, then
/// answers its rows — or, when `expired`, finds its session signed out, and
/// when `fails`, fails as a scrape that broke does.
pub(crate) struct GatedProvider {
    config: ProviderConfig,
    gate: Arc<Notify>,
    rows: Vec<Activity>,
    pub(crate) expired: bool,
    pub(crate) fails: bool,
    /// Every list read the provider was asked for, counted as it starts.
    pub(crate) reads: Arc<AtomicUsize>,
}

#[async_trait]
impl FitnessProvider for GatedProvider {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.gate.notified().await;
        if self.expired {
            return Err(AppError::provider_auth_required(PROVIDER));
        }
        if self.fails {
            return Err(AppError::external_service(PROVIDER, "the scrape broke"));
        }
        Ok(self.rows.clone())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

pub(crate) fn activity(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        "Tempo run",
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        3_000,
        PROVIDER,
    )
    .distance_meters(10_000.0)
    .build()
}

pub(crate) fn gated(gate: &Arc<Notify>, rows: Vec<Activity>) -> Box<GatedProvider> {
    Box::new(GatedProvider {
        config: ProviderConfig {
            name: PROVIDER.to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: Vec::new(),
        },
        gate: Arc::clone(gate),
        rows,
        expired: false,
        fails: false,
        reads: Arc::new(AtomicUsize::new(0)),
    })
}

pub(crate) fn window(user_id: Uuid, tenant_id: TenantId) -> OwnedLiveRead {
    OwnedLiveRead {
        provider_slug: PROVIDER.to_owned(),
        user_id,
        tenant_id,
        params: ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(30)).timestamp()),
            before: None,
            limit: Some(50),
            offset: None,
        },
        attempt_started_at: Utc::now(),
        conversation_id: None,
        ask: None,
    }
}

/// A read asked from `conversation` by the athlete's message `message`.
pub(crate) fn asked(
    user_id: Uuid,
    tenant_id: TenantId,
    conversation: &str,
    message: &str,
) -> OwnedLiveRead {
    OwnedLiveRead {
        conversation_id: Some(conversation.to_owned()),
        ask: Some(TurnAsk {
            turn_id: format!("turn-{message}"),
            user_message_id: Some(message.to_owned()),
        }),
        ..window(user_id, tenant_id)
    }
}

/// What the ledger holds for the pair's live read.
pub(crate) async fn live_read_state(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<BackfillJobState> {
    runtime
        .repos()
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant_id, PROVIDER, BackfillJobKind::LiveRead)
        .await
        .unwrap()
}

/// Wait until the pair's live read row is gone: the read finished it.
pub(crate) async fn wait_until_finished(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let finished = timeout(StdDuration::from_secs(10), async {
        while live_read_state(runtime, user_id, tenant_id).await.is_some() {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        finished.is_ok(),
        "bug: the landed live read never finished its row"
    );
}

pub(crate) fn ids(rows: &[Activity]) -> Vec<&str> {
    let mut ids: Vec<&str> = rows.iter().map(Activity::id).collect();
    ids.sort_unstable();
    ids
}

async fn runtime_and_user() -> (Arc<dyn ToolRuntime>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    (resources, user_id, tenant_id)
}

#[tokio::test]
async fn a_read_past_the_budget_answers_from_the_cache_and_still_lands() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("cached", 3)])
        .await
        .unwrap();

    // The provider answers only once the test lets it, which is after the
    // turn: a read awaited inline would never return, so bound the wait.
    let gate = Arc::new(Notify::new());
    let outcome = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
            window(user_id, tenant_id),
            BUDGET,
        ),
    )
    .await
    .expect("the turn waited on a read that had not answered");
    let BudgetedRead::Cached(rows) = outcome else {
        panic!("a read past the budget must answer from the cached window");
    };
    assert_eq!(ids(&rows), ["cached"], "the cached window answers the turn");
    assert_eq!(
        live_read_state(&runtime, user_id, tenant_id).await,
        Some(BackfillJobState::Owed),
        "bug: a read the turn stopped waiting for is owed on the ledger, not only in a task"
    );

    // The read was not dropped: once the provider answers, its rows are
    // written through for the next turn.
    gate.notify_one();
    let landed = timeout(StdDuration::from_secs(10), async {
        loop {
            let cached = runtime
                .repos()
                .activity_cache
                .get_cached_activities(
                    user_id,
                    &tenant_id,
                    Some(PROVIDER),
                    Utc::now() - Duration::days(30),
                    Utc::now() + Duration::days(1),
                    50,
                )
                .await
                .unwrap();
            if cached.iter().any(|a| a.id() == "fresh") {
                return;
            }
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        landed.is_ok(),
        "the detached read never wrote its rows through"
    );
}

#[tokio::test]
async fn with_nothing_cached_the_turn_waits_for_the_read() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let gate = Arc::new(Notify::new());
    let release = Arc::clone(&gate);
    tokio::spawn(async move {
        sleep(BUDGET * 3).await;
        release.notify_one();
    });

    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("fresh", 0)]),
        window(user_id, tenant_id),
        BUDGET,
    )
    .await;
    let BudgetedRead::Answered { read, provider } = outcome else {
        panic!("with no cached window the read itself must answer");
    };
    assert_eq!(
        ids(&read.unwrap().activities),
        ["fresh"],
        "the turn waited for the live rows"
    );
    assert!(provider.is_some(), "the provider comes back with the read");
}

#[tokio::test]
async fn a_read_inside_the_budget_answers_live() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let gate = Arc::new(Notify::new());
    gate.notify_one();

    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("fresh", 0)]),
        window(user_id, tenant_id),
        StdDuration::from_secs(10),
    )
    .await;
    let BudgetedRead::Answered { read, .. } = outcome else {
        panic!("a read inside the budget answers live");
    };
    assert_eq!(
        ids(&read.unwrap().activities),
        ["fresh"],
        "the live rows answer the turn"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
}

/// A stale head whose last refreshes failed is not read again this turn, and
/// the window says so instead of passing for current.
#[tokio::test]
async fn a_stale_head_held_back_by_its_failures_is_reported_behind() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let cache = &runtime.repos().activity_cache;
    let last_sync = Utc::now() - Duration::days(2);
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            PROVIDER,
            &[activity("cached", 3)],
            last_sync,
        )
        .await
        .unwrap();
    let failed = Utc::now() - Duration::minutes(1);
    cache
        .record_activity_fetch_failure(
            user_id,
            &tenant_id,
            PROVIDER,
            &ActivityFetchFailureRecord {
                failed_at: failed,
                failure: ActivityFetchFailure::FetchError,
                consecutive: 1,
                streak: 1,
                streak_started_at: failed,
            },
        )
        .await
        .unwrap();

    let mut served = vec![activity("cached", 3)];
    let behind =
        refresh_stale_head(&runtime, PROVIDER, user_id, tenant_id, None, &mut served).await;
    assert_eq!(
        behind,
        Some(StaleReason::Backoff),
        "a stale head held back by its failures reads as behind, and says why"
    );
    assert_eq!(ids(&served), ["cached"], "the cached head is served as is");

    // A closed window has no head to be behind.
    let closed = Some((Utc::now() - Duration::days(10)).timestamp());
    let behind =
        refresh_stale_head(&runtime, PROVIDER, user_id, tenant_id, closed, &mut served).await;
    assert_eq!(behind, None, "a closed window is never behind");
}

/// One follow-up a live read sent.
#[cfg(feature = "client-messaging")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FollowUp {
    conversation: String,
    provider: String,
    count: usize,
    kind: BackfillJobKind,
    pub(crate) asked: Option<String>,
}

/// Every follow-up and reconnect answer the live reads sent.
#[cfg(feature = "client-messaging")]
#[derive(Default)]
pub(crate) struct RecordingNotifier {
    pub(crate) sent: Mutex<Vec<FollowUp>>,
    reauth: Mutex<Vec<(String, String)>>,
}

#[cfg(feature = "client-messaging")]
#[async_trait]
impl BackfillNotifier for RecordingNotifier {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        self.sent.lock().unwrap().push(FollowUp {
            conversation: completion.pierre_conversation_id.to_owned(),
            provider: completion.provider.to_owned(),
            count: completion.activity_count,
            kind: completion.kind,
            asked: completion.asked_message_id.map(ToOwned::to_owned),
        });
    }

    // No live read ends a walk: nothing here reaches it.
    async fn push_history_walk_complete(&self, _: HistoryWalkCompletion<'_>) {}

    async fn push_detail_walk_complete(&self, _: DetailWalkCompletion<'_>) {}

    async fn push_provider_reauth(
        &self,
        _: Uuid,
        _: TenantId,
        pierre_conversation_id: &str,
        provider: &str,
    ) {
        self.reauth
            .lock()
            .unwrap()
            .push((pierre_conversation_id.to_owned(), provider.to_owned()));
    }

    async fn push_reauth_to_linked_channels(
        &self,
        _: Uuid,
        _: TenantId,
        _: &str,
        _: ReauthRetry,
    ) -> usize {
        0
    }
}

/// A runtime whose follow-ups the test reads, with one cached row for the
/// turn to answer from.
#[cfg(feature = "client-messaging")]
pub(crate) async fn notified_runtime(
) -> (Arc<dyn ToolRuntime>, Arc<RecordingNotifier>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let notifier = Arc::new(RecordingNotifier::default());
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier.clone());
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("cached", 3)])
        .await
        .unwrap();
    (runtime, notifier, user_id, tenant_id)
}

/// Wait until at least one follow-up was sent, then a little longer, so a
/// second one would be seen too.
#[cfg(feature = "client-messaging")]
pub(crate) async fn settled_follow_ups(notifier: &RecordingNotifier) -> Vec<FollowUp> {
    let posted = timeout(StdDuration::from_secs(10), async {
        while notifier.sent.lock().unwrap().is_empty() {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        posted.is_ok(),
        "bug: the landed read never answered the conversation"
    );
    sleep(StdDuration::from_millis(200)).await;
    notifier.sent.lock().unwrap().clone()
}

/// The turn told the athlete its window was incomplete; once the read lands,
/// the conversation that asked is answered, exactly once, naming the message
/// that asked (carnet#1481).
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_read_past_the_budget_answers_the_conversation_once_it_lands() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert!(
        matches!(outcome, BudgetedRead::Cached(_)),
        "bug: the turn must answer from the cache past its budget"
    );
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: nothing is posted before the read lands"
    );

    gate.notify_one();
    assert_eq!(
        settled_follow_ups(&notifier).await,
        vec![FollowUp {
            conversation: "conv-1".to_owned(),
            provider: PROVIDER.to_owned(),
            count: 2,
            kind: BackfillJobKind::LiveRead,
            asked: Some("msg-1".to_owned()),
        }],
        "bug: exactly one follow-up, to the conversation and message that asked"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
}

/// A read the turn waited for owes the conversation nothing.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_read_inside_the_budget_posts_no_follow_up() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let gate = Arc::new(Notify::new());
    gate.notify_one();
    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        StdDuration::from_secs(10),
    )
    .await;
    assert!(
        matches!(outcome, BudgetedRead::Answered { .. }),
        "a read inside the budget answers the turn itself"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: a read the turn already answered with posts a second answer"
    );
}

/// The athlete asks again while the first read still runs: the second turn
/// answers from the cache at once, and the one follow-up answers the newer
/// message, not the first.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_re_ask_while_the_read_runs_rebinds_its_follow_up() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let first = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert!(
        matches!(first, BudgetedRead::Cached(_)),
        "the first turn answers from the cache past its budget"
    );

    // The second turn's provider never answers: it must not be read at all.
    let unread = Arc::new(Notify::new());
    let second = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&unread, vec![activity("other", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            StdDuration::from_secs(10),
        ),
    )
    .await
    .expect("bug: a re-ask waited on a second read instead of joining the running one");
    let BudgetedRead::Cached(rows) = second else {
        panic!("bug: a re-ask while a read runs answers from the cache");
    };
    assert_eq!(
        ids(&rows),
        ["cached"],
        "the re-ask is served the cached window"
    );

    gate.notify_one();
    let sent = settled_follow_ups(&notifier).await;
    assert_eq!(
        sent.iter().map(|f| f.asked.as_deref()).collect::<Vec<_>>(),
        vec![Some("msg-2")],
        "bug: one follow-up, answering the newest ask: {sent:?}"
    );
}

/// An instance that died under a live read left its row; the resume sweep
/// takes it on another instance and settles it with the conversation that
/// asked. A provider with no token is the outcome a run can reach without a
/// network here: the conversation is told to reconnect, and the row ends.
/// The row is resolved again through the athlete's read plan, which sends an
/// athlete holding no Strava credential to the hosted sign-in (`sciotte`,
/// carnet#1494), so that is the reconnect the conversation is told.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn the_sweep_resumes_a_live_read_whose_instance_died() {
    use pierre_database::repositories::{ActivityBackfillJobRow, JobProgress};
    use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;

    const TOKENLESS: &str = "strava";
    const SIGN_IN: &str = "sciotte";
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id,
        user_id,
        provider: TOKENLESS.to_owned(),
        kind: BackfillJobKind::LiveRead,
        after_ts: Some((Utc::now() - Duration::days(30)).timestamp()),
        before_ts: None,
        fetch_limit: Some(50),
        conversation_id: Some("conv-dead".to_owned()),
        created_at_ms: long_ago,
        leased_until_ms: long_ago,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
        progress: JobProgress::default(),
        turn_id: Some("turn-dead".to_owned()),
        asked_message_id: Some("msg-dead".to_owned()),
        walk_backend: None,
        walk_sport: None,
    };
    assert!(
        runtime
            .repos()
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "the dead instance's live read is on file"
    );

    let resumed = resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap();
    assert_eq!(resumed, 1, "bug: the sweep never took the lapsed live read");
    let answered = timeout(StdDuration::from_secs(10), async {
        while notifier.reauth.lock().unwrap().is_empty() {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        answered.is_ok(),
        "bug: the resumed live read never answered the conversation that asked"
    );
    assert_eq!(
        notifier.reauth.lock().unwrap().as_slice(),
        [("conv-dead".to_owned(), SIGN_IN.to_owned())],
        "the resumed read answers the conversation its row names"
    );
    let state = runtime
        .repos()
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant_id, TOKENLESS, BackfillJobKind::LiveRead)
        .await
        .unwrap();
    assert_eq!(state, None, "bug: a settled live read leaves no row behind");
}

/// The session a live read was reading expires mid-scrape, after the athlete
/// reconnected: the reconnected connection is newer than the failure, so it
/// stays active and nothing tells them to reconnect again.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_read_that_expires_after_a_reconnect_leaves_the_new_connection_alone() {
    use pierre_core::models::{ConnectionStatus, ConnectionType};

    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let connections = &runtime.repos().provider_connections;
    connections
        .register_connection(user_id, tenant_id, PROVIDER, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let read = asked(user_id, tenant_id, "conv-1", "msg-1");
    sleep(StdDuration::from_millis(20)).await;

    let gate = Arc::new(Notify::new());
    let mut provider = gated(&gate, Vec::new());
    provider.expired = true;
    let outcome = read_live_window_within_budget(&runtime, provider, read, BUDGET).await;
    assert!(
        matches!(outcome, BudgetedRead::Cached(_)),
        "the turn answers from the cache past its budget"
    );
    // The athlete reconnects while the scrape still runs on the old session.
    connections
        .register_connection(user_id, tenant_id, PROVIDER, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    gate.notify_one();
    wait_until_finished(&runtime, user_id, tenant_id).await;

    let status = connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == PROVIDER)
        .expect("the connection exists")
        .status;
    assert_eq!(
        status,
        ConnectionStatus::Active,
        "bug: a reconnect made while the read ran is flagged by the read's stale session"
    );
    assert!(
        notifier.reauth.lock().unwrap().is_empty(),
        "bug: a reconnected athlete is told to reconnect"
    );
}

/// What a turn was answered with, in a form a test can hold on to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The turn's own read answered.
    Answered(Vec<String>),
    /// The cached window answered, labelled stale; a follow-up is owed.
    Cached(Vec<String>),
    /// The cached window answered as a read that did not run; nothing is
    /// owed.
    Unread(Vec<String>),
    /// A read already running landed over the window and answered, current.
    Landed(Vec<String>),
}

pub(crate) fn answer_of(outcome: BudgetedRead) -> Answer {
    let owned = |rows: &[Activity]| ids(rows).into_iter().map(str::to_owned).collect();
    match outcome {
        BudgetedRead::Answered { read, .. } => Answer::Answered(owned(&read.unwrap().activities)),
        BudgetedRead::Cached(rows) => Answer::Cached(owned(&rows)),
        BudgetedRead::Unread(rows) => Answer::Unread(owned(&rows)),
        BudgetedRead::Landed {
            activities,
            served_by_grant,
        } => {
            assert_eq!(served_by_grant, None, "no grant stands behind this session");
            Answer::Landed(owned(&activities))
        }
    }
}

/// A runtime whose follow-ups the test reads, with nothing cached.
#[cfg(feature = "client-messaging")]
pub(crate) async fn uncached_notified_runtime(
) -> (Arc<dyn ToolRuntime>, Arc<RecordingNotifier>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let notifier = Arc::new(RecordingNotifier::default());
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier.clone());
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    (runtime, notifier, user_id, tenant_id)
}

/// A notifier that does what the chat pipeline does with a landed read's
/// follow-up: it re-asks the question, in process, while the read's run
/// still holds the pair, through a provider that must not be read again.
#[cfg(feature = "client-messaging")]
struct ReaskingNotifier {
    runtime: OnceLock<Arc<dyn ToolRuntime>>,
    user_id: Uuid,
    tenant_id: TenantId,
    unread: Arc<Notify>,
    re_asks: Mutex<Vec<Answer>>,
}

#[cfg(feature = "client-messaging")]
#[async_trait]
impl BackfillNotifier for ReaskingNotifier {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        let runtime = Arc::clone(self.runtime.get().expect("the runtime is set"));
        let re_ask = timeout(
            StdDuration::from_secs(5),
            read_live_window_within_budget(
                &runtime,
                gated(&self.unread, vec![activity("other", 0)]),
                asked(
                    self.user_id,
                    self.tenant_id,
                    completion.pierre_conversation_id,
                    completion.asked_message_id.unwrap_or("msg-re-ask"),
                ),
                BUDGET,
            ),
        )
        .await
        .expect("bug: the re-ask waited on a read instead of the rows that just landed");
        self.re_asks.lock().unwrap().push(answer_of(re_ask));
    }

    async fn push_history_walk_complete(&self, _: HistoryWalkCompletion<'_>) {}

    async fn push_detail_walk_complete(&self, _: DetailWalkCompletion<'_>) {}

    async fn push_provider_reauth(&self, _: Uuid, _: TenantId, _: &str, _: &str) {}

    async fn push_reauth_to_linked_channels(
        &self,
        _: Uuid,
        _: TenantId,
        _: &str,
        _: ReauthRetry,
    ) -> usize {
        0
    }
}

/// The read outlived its turn and lands; its follow-up re-asks the question
/// while the run still holds the pair. The re-ask is answered with the rows
/// that just landed, as current — never as a read that could not refresh —
/// and nothing reads the provider a second time. Once the run is over, the
/// next ask reads live again.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_follow_ups_re_ask_reads_what_just_landed_as_current() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let notifier = Arc::new(ReaskingNotifier {
        runtime: OnceLock::new(),
        user_id,
        tenant_id,
        unread: Arc::new(Notify::new()),
        re_asks: Mutex::new(Vec::new()),
    });
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier.clone());
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    assert!(
        notifier.runtime.set(Arc::clone(&runtime)).is_ok(),
        "the notifier is handed the runtime once"
    );
    runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("cached", 3)])
        .await
        .unwrap();

    let gate = Arc::new(Notify::new());
    let first = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert_eq!(
        answer_of(first),
        Answer::Cached(vec!["cached".to_owned()]),
        "the turn answers from the cache past its budget"
    );

    gate.notify_one();
    let re_asked = timeout(StdDuration::from_secs(10), async {
        while notifier.re_asks.lock().unwrap().is_empty() {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(re_asked.is_ok(), "bug: the landed read never followed up");
    assert_eq!(
        notifier.re_asks.lock().unwrap().as_slice(),
        [Answer::Landed(vec![
            "cached".to_owned(),
            "fresh".to_owned()
        ])],
        "bug: the follow-up's re-ask must read what just landed as current, not as stale"
    );

    // The run gives the pair back once its follow-up is done: the next ask
    // reads live instead of joining a run that is over.
    sleep(StdDuration::from_millis(200)).await;
    let open = Arc::new(Notify::new());
    open.notify_one();
    let next = read_live_window_within_budget(
        &runtime,
        gated(
            &open,
            vec![
                activity("cached", 3),
                activity("fresh", 0),
                activity("newest", 0),
            ],
        ),
        asked(user_id, tenant_id, "conv-1", "msg-2"),
        StdDuration::from_secs(10),
    )
    .await;
    assert_eq!(
        answer_of(next),
        Answer::Answered(vec![
            "cached".to_owned(),
            "fresh".to_owned(),
            "newest".to_owned()
        ]),
        "bug: an ask after the run ended must read live"
    );
}

/// A second ask finds the first read running and nothing cached. It waits
/// for that read inside its budget instead of answering an empty window, is
/// answered with the rows the read landed, and is owed no follow-up: it got
/// its answer.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_joined_ask_with_nothing_cached_waits_for_the_running_read() {
    let (runtime, notifier, user_id, tenant_id) = uncached_notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let first_runtime = Arc::clone(&runtime);
    let first_gate = Arc::clone(&gate);
    let first = tokio::spawn(async move {
        read_live_window_within_budget(
            &first_runtime,
            gated(&first_gate, vec![activity("fresh", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-1"),
            BUDGET,
        )
        .await
    });
    sleep(StdDuration::from_millis(100)).await;

    let release = Arc::clone(&gate);
    tokio::spawn(async move {
        sleep(StdDuration::from_millis(400)).await;
        release.notify_one();
    });
    let unread = Arc::new(Notify::new());
    let second = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&unread, vec![activity("other", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            StdDuration::from_secs(10),
        ),
    )
    .await
    .expect("bug: the joined ask never answered");
    assert_eq!(
        answer_of(second),
        Answer::Landed(vec!["fresh".to_owned()]),
        "bug: a joined ask with nothing cached must wait for the running read's rows"
    );
    assert_eq!(
        answer_of(first.await.unwrap()),
        Answer::Answered(vec!["fresh".to_owned()]),
        "the first turn waited for its own read"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
    sleep(StdDuration::from_millis(300)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: an ask answered in its turn is owed no follow-up"
    );
}

/// The read the second ask joined does not land within its budget, and
/// nothing is cached: the turn waits out its budget, not a moment less, then
/// answers the empty window — which the caller labels a read still running
/// — and the read owes it the follow-up.
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_joined_ask_past_its_budget_is_owed_the_follow_up() {
    let (runtime, notifier, user_id, tenant_id) = uncached_notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let first_runtime = Arc::clone(&runtime);
    let first_gate = Arc::clone(&gate);
    let first = tokio::spawn(async move {
        read_live_window_within_budget(
            &first_runtime,
            gated(&first_gate, vec![activity("fresh", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-1"),
            BUDGET,
        )
        .await
    });
    sleep(StdDuration::from_millis(100)).await;

    let joined_budget = StdDuration::from_millis(600);
    let started = Instant::now();
    let unread = Arc::new(Notify::new());
    let second = read_live_window_within_budget(
        &runtime,
        gated(&unread, vec![activity("other", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-2"),
        joined_budget,
    )
    .await;
    assert!(
        started.elapsed() >= joined_budget,
        "bug: a joined ask with nothing cached answered before its budget ({:?})",
        started.elapsed()
    );
    assert_eq!(
        answer_of(second),
        Answer::Cached(Vec::new()),
        "past its budget the joined ask answers the empty window, as still running"
    );

    gate.notify_one();
    assert_eq!(
        answer_of(first.await.unwrap()),
        Answer::Answered(vec!["fresh".to_owned()]),
        "the first turn waited for its own read"
    );
    let sent = settled_follow_ups(&notifier).await;
    assert_eq!(
        sent.iter().map(|f| f.asked.as_deref()).collect::<Vec<_>>(),
        vec![Some("msg-2")],
        "bug: the joined ask that gave up is owed the one follow-up: {sent:?}"
    );
}

/// Another instance owes the pair's live read and nothing is cached here.
/// The ask waits for it inside its budget, and once that instance wrote its
/// rows and finished its row, they answer — still labelled as the read the
/// other instance follows up on, since this ask was rebound to it.
#[tokio::test]
async fn an_ask_beside_a_read_owed_elsewhere_waits_for_its_rows() {
    use pierre_database::repositories::{ActivityBackfillJobRow, JobLease, JobProgress};

    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let now_ms = Utc::now().timestamp_millis();
    let lease = JobLease {
        id: Uuid::new_v4().to_string(),
        token: Uuid::new_v4().to_string(),
    };
    let row = ActivityBackfillJobRow {
        id: lease.id.clone(),
        tenant_id,
        user_id,
        provider: PROVIDER.to_owned(),
        kind: BackfillJobKind::LiveRead,
        after_ts: Some((Utc::now() - Duration::days(30)).timestamp()),
        before_ts: None,
        fetch_limit: Some(50),
        conversation_id: Some("conv-1".to_owned()),
        created_at_ms: now_ms,
        leased_until_ms: now_ms + 600_000,
        attempts: 1,
        lease_token: Some(lease.token.clone()),
        progress: JobProgress::default(),
        turn_id: Some("turn-msg-1".to_owned()),
        asked_message_id: Some("msg-1".to_owned()),
        walk_backend: None,
        walk_sport: None,
    };
    assert!(
        runtime
            .repos()
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "the other instance's live read is on file"
    );

    // The other instance lands: its rows are written through, then its row
    // is finished.
    let elsewhere = Arc::clone(&runtime);
    tokio::spawn(async move {
        sleep(StdDuration::from_millis(400)).await;
        elsewhere
            .repos()
            .activity_cache
            .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("landed", 0)])
            .await
            .unwrap();
        elsewhere
            .repos()
            .activity_backfill_jobs
            .finish_live_read(&lease)
            .await
            .unwrap();
    });

    let unread = Arc::new(Notify::new());
    let outcome = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&unread, vec![activity("other", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            StdDuration::from_secs(10),
        ),
    )
    .await
    .expect("bug: the ask never answered");
    assert_eq!(
        answer_of(outcome),
        Answer::Cached(vec!["landed".to_owned()]),
        "bug: with nothing cached the ask waits for the read owed elsewhere, not an empty window"
    );
}

/// The read a second ask joined ends without rows — its session expired —
/// and nothing is cached. The ask does not answer the empty window: it reads
/// for itself, through the provider its turn authenticated, which the join
/// handed back.
#[tokio::test]
async fn a_joined_ask_whose_read_ends_without_rows_reads_for_itself() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let gate = Arc::new(Notify::new());
    let mut expired = gated(&gate, Vec::new());
    expired.expired = true;
    let first_runtime = Arc::clone(&runtime);
    let first = tokio::spawn(async move {
        read_live_window_within_budget(
            &first_runtime,
            expired,
            asked(user_id, tenant_id, "conv-1", "msg-1"),
            BUDGET,
        )
        .await
    });
    sleep(StdDuration::from_millis(100)).await;

    let release = Arc::clone(&gate);
    tokio::spawn(async move {
        sleep(StdDuration::from_millis(400)).await;
        release.notify_one();
    });
    let own = Arc::new(Notify::new());
    own.notify_one();
    let second = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&own, vec![activity("mine", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            StdDuration::from_secs(10),
        ),
    )
    .await
    .expect("bug: the joined ask never answered");
    assert_eq!(
        answer_of(second),
        Answer::Answered(vec!["mine".to_owned()]),
        "bug: once the joined read ended without rows, the ask reads for itself"
    );
    let BudgetedRead::Answered { read, .. } = first.await.unwrap() else {
        panic!("the first turn waited for its own read");
    };
    assert!(
        read.is_err_and(|e| e.provider_auth_required_provider().is_some()),
        "the first read found its session expired"
    );
}
