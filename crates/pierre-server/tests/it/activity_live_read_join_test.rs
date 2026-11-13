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
    uncached_notified_runtime, wait_until_finished, window, Answer, BUDGET, PROVIDER,
};
use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLeaseState,
    JobProgress,
};
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_backfill::{BACKFILL_JOB_LEASE, MAX_BACKFILL_ATTEMPTS};
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
    // The failed attempt settles its row, then gives the pair back.
    let idle = timeout(StdDuration::from_secs(5), async {
        while lease_state(runtime, user_id, tenant_id).await != Some(JobLeaseState::Idle) {
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        idle.is_ok(),
        "bug: a failed read holds its lease though nothing runs it"
    );
    sleep(StdDuration::from_millis(100)).await;
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

/// A wider ask with nothing cached joins the read while it runs, and the
/// read then lands over its narrower window. The ask does not wait out its
/// budget on a run that reads nothing more: it reads its own window.
#[tokio::test]
async fn a_wider_ask_joined_before_a_narrower_landing_reads_its_own_window() {
    let (runtime, notifier, user_id, tenant_id) = uncached_notified_runtime().await;
    let gate = Arc::new(Notify::new());
    let first_runtime = Arc::clone(&runtime);
    let first_gate = Arc::clone(&gate);
    let first = tokio::spawn(async move {
        read_live_window_within_budget(
            &first_runtime,
            gated(&first_gate, vec![activity("fresh", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-1"),
            TURN_BUDGET,
        )
        .await
    });
    sleep(StdDuration::from_millis(100)).await;

    let release = Arc::clone(&gate);
    tokio::spawn(async move {
        sleep(StdDuration::from_millis(400)).await;
        release.notify_one();
    });
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
    .expect("bug: the wider ask waited out its budget on a landed run");
    assert_eq!(
        answer_of(wide),
        Answer::Answered(names(&["cached", "fresh", "older"])),
        "bug: a joined ask the landed read does not cover must read its own window"
    );
    assert!(started.elapsed() < StdDuration::from_secs(2));
    assert_eq!(
        answer_of(first.await.unwrap()),
        Answer::Answered(names(&["fresh"])),
        "the first turn waited for its own read"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
    sleep(StdDuration::from_millis(300)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: both asks answered in their turns, yet a follow-up was posted"
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

/// Record, as another instance would, a live read of the pair owed in a
/// tenant of its own and held by a running attempt: the pair's one row,
/// which no ask of `tenant_id` can rebind or take over.
async fn other_tenants_read(runtime: &Arc<dyn ToolRuntime>, user_id: Uuid) {
    let now = Utc::now().timestamp_millis();
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        user_id,
        provider: PROVIDER.to_owned(),
        kind: BackfillJobKind::LiveRead,
        after_ts: Some((Utc::now() - Duration::days(30)).timestamp()),
        before_ts: None,
        fetch_limit: Some(50),
        conversation_id: Some("conv-other".to_owned()),
        created_at_ms: now,
        leased_until_ms: now + 600_000,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
        progress: JobProgress::default(),
        turn_id: Some("turn-other".to_owned()),
        asked_message_id: Some("msg-other".to_owned()),
        walk_backend: None,
    };
    assert!(runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&row)
        .await
        .unwrap());
}

/// The pair's one live-read row is another tenant's, running elsewhere: it
/// answers its own ask, never this one, and this ask can neither rebind it
/// nor record a read of its own. Nothing will follow this ask up, so its
/// cached window must not be answered as a read still running.
#[tokio::test]
async fn an_ask_no_owed_read_answers_is_not_promised_a_follow_up() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    other_tenants_read(&runtime, user_id).await;
    let unread = Arc::new(Notify::new());
    let answer = read_live_window_within_budget(
        &runtime,
        gated(&unread, vec![activity("other", 0)]),
        asked(user_id, tenant_id, "conv-2", "msg-2"),
        BUDGET,
    )
    .await;
    assert_eq!(
        answer_of(answer),
        Answer::Unread(names(&["cached"])),
        "bug: an ask no read owes is answered as a read still running"
    );
    sleep(StdDuration::from_millis(300)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "nothing is posted to an ask no read owes"
    );
}

/// A read of the pair runs here for another tenant's ask, and this tenant's
/// ask joins it: its row is not this tenant's to rebind, so at its deadline
/// the joined read owes this ask nothing. The ask must not be answered as a
/// read still running, and no follow-up reaches it.
#[tokio::test]
async fn an_ask_joined_past_its_deadline_to_a_read_that_owes_it_nothing_is_not_promised_one() {
    let (runtime, notifier, user_id, tenant_id) = notified_runtime().await;
    let other_tenant = TenantId::from_uuid(Uuid::new_v4());
    let gate = Arc::new(Notify::new());
    let other_runtime = Arc::clone(&runtime);
    let other_gate = Arc::clone(&gate);
    let other = tokio::spawn(async move {
        read_live_window_within_budget(
            &other_runtime,
            gated(&other_gate, vec![activity("fresh", 0)]),
            asked(user_id, other_tenant, "conv-1", "msg-1"),
            TURN_BUDGET,
        )
        .await
    });
    sleep(StdDuration::from_millis(100)).await;

    let unread = Arc::new(Notify::new());
    let answer = read_live_window_within_budget(
        &runtime,
        gated(&unread, vec![activity("other", 0)]),
        asked(user_id, tenant_id, "conv-2", "msg-2"),
        BUDGET,
    )
    .await;
    assert_eq!(
        answer_of(answer),
        Answer::Unread(names(&["cached"])),
        "bug: an ask the joined read owes nothing is answered as a read still running"
    );
    gate.notify_one();
    assert_eq!(
        answer_of(other.await.unwrap()),
        Answer::Answered(names(&["fresh"])),
        "the other tenant's turn waited for its own read"
    );
    sleep(StdDuration::from_millis(300)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: a follow-up reached an ask no read owed one"
    );
}

/// A notifier whose first follow-up holds its run until the test lets it
/// go, recording the conversation and message every follow-up answers.
#[derive(Default)]
struct HeldOnceNotifier {
    entered: Notify,
    release: Notify,
    sent: Mutex<Vec<(String, Option<String>)>>,
}

#[async_trait]
impl BackfillNotifier for HeldOnceNotifier {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        let first = {
            let mut sent = self.sent.lock().unwrap();
            sent.push((
                completion.pierre_conversation_id.to_owned(),
                completion.asked_message_id.map(ToOwned::to_owned),
            ));
            sent.len() == 1
        };
        if first {
            self.entered.notify_one();
            self.release.notified().await;
        }
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

/// A read landed over thirty days and its run is busy with its follow-up; a
/// wider ask from another conversation succeeded it and runs past its own
/// budget, owed its follow-up. The landed read's follow-up then re-asks its
/// thirty days: the rows its own run landed answer it as current. It never
/// rebinds the wider read's row, so the wider ask's follow-up still reaches
/// the wider ask's conversation, once.
#[tokio::test]
async fn a_follow_ups_re_ask_never_takes_the_successors_follow_up() {
    let notifier = Arc::new(HeldOnceNotifier::default());
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

    let wide_gate = Arc::new(Notify::new());
    let wide = read_live_window_within_budget(
        &runtime,
        gated(&wide_gate, wider_rows()),
        wider(user_id, tenant_id, "conv-2", "msg-wide"),
        BUDGET,
    )
    .await;
    assert!(
        matches!(answer_of(wide), Answer::Cached(_)),
        "the wider ask reads for itself and outlives its budget"
    );

    let unread = Arc::new(Notify::new());
    let re_ask = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&unread, vec![activity("never", 0)]),
            asked(user_id, tenant_id, "conv-1", "msg-re-ask"),
            TURN_BUDGET,
        ),
    )
    .await
    .expect("bug: the re-ask waited on the wider read");
    assert_eq!(
        answer_of(re_ask),
        Answer::Landed(names(&["cached", "fresh"])),
        "bug: the follow-up's re-ask is not answered from the rows its own run landed"
    );

    wide_gate.notify_one();
    notifier.release.notify_one();
    wait_until_finished(&runtime, user_id, tenant_id).await;
    sleep(StdDuration::from_millis(300)).await;
    assert_eq!(
        notifier.sent.lock().unwrap().as_slice(),
        [
            ("conv-1".to_owned(), Some("msg-1".to_owned())),
            ("conv-2".to_owned(), Some("msg-wide".to_owned())),
        ],
        "bug: the wider ask's follow-up was taken by the older conversation's re-ask"
    );
}

/// A failed read's row is given back idle, yet the sweep waits out the
/// backoff the attempt's lease used to hold it for, so a failing provider
/// is not spent through every attempt in a few minutes. An ask meanwhile
/// takes the idle row over at once and reads in its own turn.
#[tokio::test]
async fn a_failed_read_waits_its_backoff_for_the_sweep_but_not_for_an_ask() {
    let (runtime, _notifier, user_id, tenant_id) = notified_runtime().await;
    fail_past_the_budget(&runtime, user_id, tenant_id).await;

    let swept = runtime
        .repos()
        .activity_backfill_jobs
        .claim_stale_backfill_jobs(BackfillJobClaim {
            now_ms: Utc::now().timestamp_millis(),
            queued_older_than_ms: 0,
            lease_ms: 600_000,
            max_attempts: MAX_BACKFILL_ATTEMPTS,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(
        swept.is_empty(),
        "bug: the sweep retries a failed read before its backoff: {swept:?}"
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
    .expect("bug: the ask waited out the sweep's backoff");
    assert_eq!(
        answer_of(next),
        Answer::Answered(names(&["cached", "fresh"])),
        "bug: an ask takes a failed read's idle row over at once"
    );
    wait_until_finished(&runtime, user_id, tenant_id).await;
}

/// Ask past the budget through a provider that then fails, `ask` taking
/// over the pair's idle row when a failed read left one: the turn answers
/// from the cache, then the attempt fails once it is let through. Returns
/// once the failed attempt settled its row.
async fn fail_an_ask(runtime: &Arc<dyn ToolRuntime>, ask: OwnedLiveRead, last: bool) {
    let (user_id, tenant_id) = (ask.user_id, ask.tenant_id);
    let gate = Arc::new(Notify::new());
    let mut failing = gated(&gate, Vec::new());
    failing.fails = true;
    let answer = read_live_window_within_budget(runtime, failing, ask, BUDGET).await;
    assert!(
        matches!(answer_of(answer), Answer::Cached(_)),
        "the turn answers from the cache past its budget, owed a follow-up"
    );
    gate.notify_one();
    let settled = timeout(StdDuration::from_secs(5), async {
        loop {
            let settled = if last {
                live_read_state(runtime, user_id, tenant_id).await.is_none()
            } else {
                lease_state(runtime, user_id, tenant_id).await == Some(JobLeaseState::Idle)
            };
            if settled {
                return;
            }
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "bug: the failed attempt never settled its row"
    );
}

/// A notifier that records every follow-up and re-asks its question in
/// process, as the chat pipeline does, through a provider that counts its
/// reads and never answers one.
struct UnavailableReAsk {
    runtime: OnceLock<Arc<dyn ToolRuntime>>,
    user_id: Uuid,
    tenant_id: TenantId,
    reads: Arc<AtomicUsize>,
    sent: Mutex<Vec<(String, Option<String>, bool)>>,
    re_asks: Mutex<Vec<(Answer, StdDuration)>>,
}

impl UnavailableReAsk {
    fn new(user_id: Uuid, tenant_id: TenantId) -> Self {
        Self {
            runtime: OnceLock::new(),
            user_id,
            tenant_id,
            reads: Arc::default(),
            sent: Mutex::default(),
            re_asks: Mutex::default(),
        }
    }
}

#[async_trait]
impl BackfillNotifier for UnavailableReAsk {
    async fn push_backfill_complete(&self, completion: BackfillCompletion<'_>) {
        self.sent.lock().unwrap().push((
            completion.pierre_conversation_id.to_owned(),
            completion.asked_message_id.map(ToOwned::to_owned),
            completion.provider_unavailable,
        ));
        let runtime = Arc::clone(self.runtime.get().expect("the runtime is set"));
        let never = Arc::new(Notify::new());
        let provider = gated(&never, vec![activity("never", 0)]);
        let reads = Arc::clone(&provider.reads);
        let started = Instant::now();
        let re_ask = timeout(
            StdDuration::from_secs(5),
            read_live_window_within_budget(
                &runtime,
                provider,
                asked(
                    self.user_id,
                    self.tenant_id,
                    completion.pierre_conversation_id,
                    completion.asked_message_id.unwrap_or("msg-re-ask"),
                ),
                TURN_BUDGET,
            ),
        )
        .await
        .expect("bug: the re-ask of a read that gave up waited on a provider");
        self.reads
            .fetch_add(reads.load(Ordering::SeqCst), Ordering::SeqCst);
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

/// A runtime following up through an [`UnavailableReAsk`], one row cached.
async fn unavailable_runtime() -> (Arc<dyn ToolRuntime>, Arc<UnavailableReAsk>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let notifier = Arc::new(UnavailableReAsk::new(user_id, tenant_id));
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
    (runtime, notifier, user_id, tenant_id)
}

/// Three asks each answered from the cache past their budget, each
/// promised a follow-up, and each attempt fails: the last spends the read's
/// attempts and its row is dropped. The ask it was bound to — the newest —
/// still gets its one follow-up. Its re-ask reads no provider (the read gave
/// up, and another would loop): it answers from what is stored, at once,
/// as a read that did not run, which the model words as the provider being
/// unavailable.
#[tokio::test]
async fn a_read_that_spends_its_attempts_still_follows_up_its_ask_once() {
    let (runtime, notifier, user_id, tenant_id) = unavailable_runtime().await;
    fail_an_ask(
        &runtime,
        asked(user_id, tenant_id, "conv-1", "msg-1"),
        false,
    )
    .await;
    fail_an_ask(
        &runtime,
        asked(user_id, tenant_id, "conv-1", "msg-2"),
        false,
    )
    .await;
    fail_an_ask(&runtime, asked(user_id, tenant_id, "conv-1", "msg-3"), true).await;
    sleep(StdDuration::from_millis(500)).await;

    assert_eq!(
        notifier.sent.lock().unwrap().as_slice(),
        [("conv-1".to_owned(), Some("msg-3".to_owned()), true)],
        "bug: a read that spent its attempts owes the ask it is bound to exactly one follow-up, \
         saying the provider stayed unavailable"
    );
    let re_asks = notifier.re_asks.lock().unwrap().clone();
    assert_eq!(re_asks.len(), 1);
    let (answer, took) = re_asks[0].clone();
    assert_eq!(
        answer,
        Answer::Unread(names(&["cached"])),
        "bug: the re-ask must answer what is stored, as a read that did not run"
    );
    assert!(
        took < StdDuration::from_secs(2),
        "bug: the re-ask waited out its budget ({took:?})"
    );
    assert_eq!(
        notifier.reads.load(Ordering::SeqCst),
        0,
        "bug: the re-ask of a read that gave up read the provider again"
    );
    assert_eq!(
        live_read_state(&runtime, user_id, tenant_id).await,
        None,
        "bug: the re-ask started another read"
    );
}

/// The same three failures for reads no conversation asked (a stale head's
/// top-up): nothing is bound to follow up, and nothing is posted.
#[tokio::test]
async fn a_read_no_ask_is_bound_to_spends_its_attempts_silently() {
    let (runtime, notifier, user_id, tenant_id) = unavailable_runtime().await;
    fail_an_ask(&runtime, window(user_id, tenant_id), false).await;
    fail_an_ask(&runtime, window(user_id, tenant_id), false).await;
    fail_an_ask(&runtime, window(user_id, tenant_id), true).await;
    sleep(StdDuration::from_millis(500)).await;
    assert!(
        notifier.sent.lock().unwrap().is_empty(),
        "bug: a read no ask is bound to posted a follow-up"
    );
}

/// A failed read gives its row back for a retry a whole lease later: the
/// sweep claims it at no earlier instant, not a minute after the failure,
/// and the retry is due once that lease has passed.
#[tokio::test]
async fn a_failed_reads_retry_is_due_a_whole_lease_later() {
    let (runtime, _notifier, user_id, tenant_id) = notified_runtime().await;
    let before = Utc::now().timestamp_millis();
    fail_past_the_budget(&runtime, user_id, tenant_id).await;
    let after = Utc::now().timestamp_millis();
    let lease = i64::try_from(BACKFILL_JOB_LEASE.as_millis()).unwrap();
    let claim_at = |now_ms: i64| BackfillJobClaim {
        now_ms,
        queued_older_than_ms: 0,
        lease_ms: 600_000,
        max_attempts: MAX_BACKFILL_ATTEMPTS,
        limit: 10,
    };
    let ledger = &runtime.repos().activity_backfill_jobs;
    for now_ms in [after + 60_000, before + lease - 2_000] {
        let swept = ledger
            .claim_stale_backfill_jobs(claim_at(now_ms))
            .await
            .unwrap();
        assert!(
            swept.is_empty(),
            "bug: a failed read is retried {}s after it failed, before a whole lease: {swept:?}",
            (now_ms - after) / 1_000
        );
    }
    let swept = ledger
        .claim_stale_backfill_jobs(claim_at(after + lease + 2_000))
        .await
        .unwrap();
    assert_eq!(
        swept.len(),
        1,
        "bug: the failed read is not due once a whole lease has passed"
    );
}
