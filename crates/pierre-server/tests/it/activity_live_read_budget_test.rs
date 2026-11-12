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

use std::sync::Arc;
#[cfg(feature = "client-messaging")]
use std::sync::Mutex;
use std::time::Duration as StdDuration;

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
    BackfillCompletion, BackfillNotifier, HistoryWalkCompletion, ReauthRetry,
};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

const PROVIDER: &str = "sciotte";
const BUDGET: StdDuration = StdDuration::from_millis(200);

/// A provider whose list read waits until the test notifies it, then
/// answers its rows — or, when `expired`, finds its session signed out.
struct GatedProvider {
    config: ProviderConfig,
    gate: Arc<Notify>,
    rows: Vec<Activity>,
    expired: bool,
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
        self.gate.notified().await;
        if self.expired {
            return Err(AppError::provider_auth_required(PROVIDER));
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

fn activity(id: &str, days_ago: i64) -> Activity {
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

fn gated(gate: &Arc<Notify>, rows: Vec<Activity>) -> Box<GatedProvider> {
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
    })
}

fn window(user_id: Uuid, tenant_id: TenantId) -> OwnedLiveRead {
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
fn asked(user_id: Uuid, tenant_id: TenantId, conversation: &str, message: &str) -> OwnedLiveRead {
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
async fn live_read_state(
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
async fn wait_until_finished(runtime: &Arc<dyn ToolRuntime>, user_id: Uuid, tenant_id: TenantId) {
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

fn ids(rows: &[Activity]) -> Vec<&str> {
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
struct FollowUp {
    conversation: String,
    provider: String,
    count: usize,
    kind: BackfillJobKind,
    asked: Option<String>,
}

/// Every follow-up and reconnect answer the live reads sent.
#[cfg(feature = "client-messaging")]
#[derive(Default)]
struct RecordingNotifier {
    sent: Mutex<Vec<FollowUp>>,
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
async fn notified_runtime() -> (Arc<dyn ToolRuntime>, Arc<RecordingNotifier>, Uuid, TenantId) {
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
async fn settled_follow_ups(notifier: &RecordingNotifier) -> Vec<FollowUp> {
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
#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn the_sweep_resumes_a_live_read_whose_instance_died() {
    use pierre_database::repositories::{ActivityBackfillJobRow, JobProgress};
    use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;

    const TOKENLESS: &str = "strava";
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
        [("conv-dead".to_owned(), TOKENLESS.to_owned())],
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
