// ABOUTME: An ask joins only a live read that is actually running: a failed read's idle row and a landed narrower read are read past
// ABOUTME: The ask then reads for itself in its own turn, answers current, settles the row, and is posted no follow-up
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(feature = "tools-data", feature = "client-messaging"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A live read that outlived its turn is joined by the asks that come after
//! it, so the provider is not read twice. That is only right while the read
//! is running. A read that failed past its budget gives its lease back: its
//! row stays owed for the sweep's retries, but nothing runs it, and the next
//! ask reads for itself. A read that landed over a narrower window than the
//! ask's holds no provider read any more, and the ask reads its own window —
//! its own follow-up's re-ask included, which runs inside that very run.

use crate::activity_live_read_budget_test::{
    activity, answer_of, asked, gated, live_read_state, notified_runtime, settled_follow_ups,
    wait_until_finished, Answer, BUDGET, PROVIDER,
};
use crate::common;

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{BackfillJobKind, BackfillJobState, JobLeaseState};
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::budgeted_read::{
    read_live_window_within_budget, OwnedLiveRead,
};
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, HistoryWalkCompletion, ReauthRetry, ToolRuntime,
};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout, Instant};
use uuid::Uuid;

/// Long enough that a read waiting out its budget is told apart from one
/// that answered in its turn.
const TURN_BUDGET: StdDuration = StdDuration::from_secs(10);

/// An ask from `conversation` reaching ninety days back: wider than the
/// thirty days [`asked`] reads.
fn wider(user_id: Uuid, tenant_id: TenantId, conversation: &str, message: &str) -> OwnedLiveRead {
    OwnedLiveRead {
        params: ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(90)).timestamp()),
            before: None,
            limit: Some(50),
            offset: None,
        },
        ..asked(user_id, tenant_id, conversation, message)
    }
}

/// The rows the wider window holds today: one older than thirty days.
fn wider_rows() -> Vec<Activity> {
    vec![
        activity("cached", 3),
        activity("fresh", 0),
        activity("older", 60),
    ]
}

fn names(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| (*row).to_owned()).collect()
}

/// Whether an attempt holds the lease of the pair's owed live read.
async fn lease_state(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<JobLeaseState> {
    runtime
        .repos()
        .activity_backfill_jobs
        .backfill_job_lease_state(
            user_id,
            tenant_id,
            PROVIDER,
            BackfillJobKind::LiveRead,
            Utc::now().timestamp_millis(),
        )
        .await
        .unwrap()
}

/// Ask once past the budget through a provider that then fails: the turn
/// answers from the cache, and the read fails once it is let through.
async fn fail_past_the_budget(runtime: &Arc<dyn ToolRuntime>, user_id: Uuid, tenant_id: TenantId) {
    let gate = Arc::new(Notify::new());
    let mut failing = gated(&gate, Vec::new());
    failing.fails = true;
    let first = read_live_window_within_budget(
        runtime,
        failing,
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert!(
        matches!(answer_of(first), Answer::Cached(_)),
        "the turn answers from the cache past its budget"
    );
    gate.notify_one();
    // The failed attempt settles its row and gives the pair back.
    sleep(StdDuration::from_millis(400)).await;
    assert_eq!(
        live_read_state(runtime, user_id, tenant_id).await,
        Some(BackfillJobState::Owed),
        "a failed read stays owed for the sweep's retries"
    );
}

/// A read past its budget fails. Its row stays owed for a retry, but no
/// attempt holds it, so it is not "a read still running". The athlete's next
/// ask reads live in its own turn: it answers current, never the cache
/// labelled as a running read; it settles the row; and nobody is posted a
/// follow-up, since the ask answered in its turn is the answer.
#[tokio::test]
async fn an_ask_after_a_failed_read_reads_live_and_settles_the_row() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    fail_past_the_budget(&runtime, user_id, tenant_id).await;
    assert_eq!(
        lease_state(&runtime, user_id, tenant_id).await,
        Some(JobLeaseState::Idle),
        "bug: a failed read holds its lease though nothing runs it"
    );

    let open = Arc::new(Notify::new());
    open.notify_one();
    let next = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&open, vec![activity("cached", 3), activity("fresh", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            TURN_BUDGET,
        ),
    )
    .await
    .expect("bug: the ask waited on a read nothing runs");
    assert_eq!(
        answer_of(next),
        Answer::Answered(names(&["cached", "fresh"])),
        "bug: an ask beside a failed read must read live, not serve the cache as a read still running"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
    sleep(StdDuration::from_millis(300)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: the ask answered in its turn is posted a follow-up too"
    );
}

/// The same with nothing cached: the ask does not wait out its budget on a
/// read nothing runs and answer an empty window; it reads for itself.
#[tokio::test]
async fn an_uncached_ask_after_a_failed_read_does_not_wait_on_it() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    fail_past_the_budget(&runtime, user_id, tenant_id).await;
    runtime
        .repos()
        .activity_cache
        .delete_provider_activities(user_id, &tenant_id, PROVIDER)
        .await
        .unwrap();

    let open = Arc::new(Notify::new());
    open.notify_one();
    let started = Instant::now();
    let next = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&open, vec![activity("fresh", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-2"),
            TURN_BUDGET,
        ),
    )
    .await
    .expect("bug: the ask waited out its budget on a read nothing runs");
    assert_eq!(answer_of(next), Answer::Answered(names(&["fresh"])));
    assert!(started.elapsed() < StdDuration::from_secs(2));
    wait_until_finished(&runtime, user_id, tenant_id).await;
    sleep(StdDuration::from_millis(300)).await;
    assert!(notifier.sent.lock().unwrap().is_empty());
}

/// A notifier whose follow-up holds the landed read's run until the test
/// lets it go, recording which message each follow-up answered.
#[derive(Default)]
struct HeldNotifier {
    entered: Notify,
    release: Notify,
    sent: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl BackfillNotifier for HeldNotifier {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        self.sent
            .lock()
            .unwrap()
            .push(completion.asked_message_id.map(ToOwned::to_owned));
        self.entered.notify_one();
        self.release.notified().await;
    }

    async fn push_history_walk_complete(&self, _: HistoryWalkCompletion<'_>) {}

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

/// A runtime posting through `notifier`, with one row cached.
async fn runtime_with(
    notifier: Arc<dyn BackfillNotifier>,
) -> (Arc<dyn ToolRuntime>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier);
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("cached", 3)])
        .await
        .unwrap();
    (runtime, user_id, tenant_id)
}

/// The read landed over thirty days and its run is busy with its
/// follow-up. An ask reaching ninety days back is not covered by what
/// landed, and the landed run reads nothing more: the ask reads for itself
/// and answers current in its turn, rather than be promised a follow-up the
/// finished row will never send.
#[tokio::test]
async fn a_wider_ask_after_a_read_landed_reads_its_own_window() {
    let notifier = Arc::new(HeldNotifier::default());
    let (runtime, user_id, tenant_id) = runtime_with(notifier.clone()).await;
    let gate = Arc::new(Notify::new());
    let first = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert!(matches!(answer_of(first), Answer::Cached(_)));
    gate.notify_one();
    timeout(StdDuration::from_secs(10), notifier.entered.notified())
        .await
        .expect("the landed read follows up");

    let open = Arc::new(Notify::new());
    open.notify_one();
    let started = Instant::now();
    let wide = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&open, wider_rows()),
            wider(user_id, tenant_id, "conv-1", "msg-2"),
            TURN_BUDGET,
        ),
    )
    .await
    .expect("bug: the wider ask waited on the landed run");
    assert_eq!(
        answer_of(wide),
        Answer::Answered(names(&["cached", "fresh", "older"])),
        "bug: an ask the landed read does not cover must read its own window"
    );
    assert!(started.elapsed() < StdDuration::from_secs(2));

    notifier.release.notify_one();
    sleep(StdDuration::from_millis(300)).await;
    assert_eq!(
        notifier.sent.lock().unwrap().as_slice(),
        [Some("msg-1".to_owned())],
        "bug: the wider ask answered in its turn, so only the first is followed up"
    );
    assert_eq!(
        live_read_state(&runtime, user_id, tenant_id).await,
        None,
        "every read settled its row"
    );
}

/// A notifier that re-asks the athlete's question, in process, as the chat
/// pipeline does — over a ninety-day window the landed read did not cover.
struct WideningNotifier {
    runtime: OnceLock<Arc<dyn ToolRuntime>>,
    user_id: Uuid,
    tenant_id: TenantId,
    re_asks: Mutex<Vec<(Answer, StdDuration)>>,
}

#[async_trait]
impl BackfillNotifier for WideningNotifier {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        let runtime = Arc::clone(self.runtime.get().expect("the runtime is set"));
        let open = Arc::new(Notify::new());
        open.notify_one();
        let started = Instant::now();
        let re_ask = timeout(
            StdDuration::from_secs(5),
            read_live_window_within_budget(
                &runtime,
                gated(&open, wider_rows()),
                wider(
                    self.user_id,
                    self.tenant_id,
                    completion.pierre_conversation_id,
                    "msg-re-ask",
                ),
                TURN_BUDGET,
            ),
        )
        .await
        .expect("bug: the re-ask waited on the run that is awaiting it");
        self.re_asks
            .lock()
            .unwrap()
            .push((answer_of(re_ask), started.elapsed()));
    }

    async fn push_history_walk_complete(&self, _: HistoryWalkCompletion<'_>) {}

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

/// The landed read's own follow-up re-asks the question over a wider
/// window, inside the run it follows up. It reads that window for itself at
/// once, never waiting on the run that awaits it.
#[tokio::test]
async fn a_follow_ups_wider_re_ask_reads_for_itself_inside_the_run() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let notifier = Arc::new(WideningNotifier {
        runtime: OnceLock::new(),
        user_id,
        tenant_id,
        re_asks: Mutex::new(Vec::new()),
    });
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier.clone());
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    assert!(notifier.runtime.set(Arc::clone(&runtime)).is_ok());
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
    assert!(matches!(answer_of(first), Answer::Cached(_)));
    gate.notify_one();
    let re_asked = timeout(StdDuration::from_secs(10), async {
        while notifier.re_asks.lock().unwrap().is_empty() {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(re_asked.is_ok(), "bug: the landed read never followed up");
    let (answer, took) = notifier.re_asks.lock().unwrap()[0].clone();
    assert_eq!(
        answer,
        Answer::Answered(names(&["cached", "fresh", "older"])),
        "bug: the follow-up's wider re-ask must read its own window"
    );
    assert!(
        took < StdDuration::from_secs(2),
        "bug: the re-ask waited on the run that awaits it ({took:?})"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
}

/// The cached-row follow-up path is unchanged: a re-ask with rows cached
/// while the read still runs is served at once and owed the one follow-up.
#[tokio::test]
async fn a_re_ask_while_the_read_runs_is_still_owed_its_follow_up() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let first = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        BUDGET,
    )
    .await;
    assert!(matches!(answer_of(first), Answer::Cached(_)));
    assert_eq!(
        lease_state(&runtime, user_id, tenant_id).await,
        Some(JobLeaseState::Leased),
        "a read past its budget still running holds its lease"
    );
    let unread = Arc::new(Notify::new());
    let second = read_live_window_within_budget(
        &runtime,
        gated(&unread, vec![activity("other", 0)]),
        wider(user_id, tenant_id, "conv-1", "msg-2"),
        TURN_BUDGET,
    )
    .await;
    assert_eq!(answer_of(second), Answer::Cached(names(&["cached"])));
    gate.notify_one();
    let sent = settled_follow_ups(&notifier).await;
    assert_eq!(
        sent.iter().map(|f| f.asked.as_deref()).collect::<Vec<_>>(),
        vec![Some("msg-2")],
        "bug: one follow-up, answering the newest ask: {sent:?}"
    );
}
