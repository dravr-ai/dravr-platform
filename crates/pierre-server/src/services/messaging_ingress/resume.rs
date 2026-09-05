// ABOUTME: The durable record every messaging turn runs from — recorded at ingress, leased per run, resumed by whichever instance can
// ABOUTME: start_turn hands a recorded turn to the runner; the sweep re-runs or re-enqueues whatever an instance left behind

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A turn is a row before it is a task.
//!
//! A messaging turn runs after its webhook answered 200, so Cloud Run reads
//! the instance as idle from the athlete's first second and an idle scaledown
//! can take the turn at any moment (registre#126). Two things close that:
//!
//! 1. **Every turn is recorded before it starts.** [`start_turn`] writes one
//!    row to `messaging_resumable_turns` carrying everything a fresh dispatch
//!    needs — the three resolved tenants, the session, the sanitized text, the
//!    locale, the thread — and only then hands the turn to the
//!    [`TurnRunner`]. Whatever kills the instance afterwards, the turn is on
//!    file, and the row is deleted the moment a run reaches any end.
//! 2. **On GCP the turn runs inside a request.** The Cloud Tasks runner
//!    enqueues the row's id; Cloud Tasks delivers `POST
//!    /internal/turns/{id}/run` to this service, the route claims the row and
//!    runs the turn inside that request, and an instance processing a request
//!    is one Cloud Run waits for. Locally the in-process runner claims the
//!    row itself and spawns the turn through the in-flight tracker.
//!
//! A run holds a short lease it renews while it works, so a run that dies
//! without a word — SIGKILL, a crash — frees its row within a couple of
//! minutes. A run the shutdown drain interrupts releases its row at once
//! ([`hand_off_drained_turn`]) and leaves its "thinking…" placeholder
//! standing; the next run attaches to that placeholder, so the athlete's one
//! bubble is the one that becomes the answer. A turn is handed back only
//! while it has attempts left ([`MAX_TURN_ATTEMPTS`]); past that, the apology
//! goes out and the row is finished. The watchdog never hands off: re-running
//! a hung turn is not a fix.
//!
//! The claim keeps conversations in order across instances: a turn is
//! refused while an older turn of the same conversation is still on file, and
//! the runner waits for it rather than answering out of order.
//!
//! [`sweep_resumable_turns`] runs at startup and every [`SWEEP_INTERVAL`]
//! while the process lives — with min instances at zero the next instance
//! boots only when a request arrives, and a surviving sibling may be the one
//! that can answer. In-process it claims and runs what is stale; on Cloud
//! Tasks it enqueues what is stale again under a fresh task name, and the
//! delivery does the claiming.

use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::{
    ResumableTurnClaim, ResumableTurnRepository, ResumableTurnRow, TurnClaim, TurnLease,
};
use pierre_messaging::channel::MessagingChannel;
use pierre_services::periodic::spawn_periodic;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{error, info, warn, Instrument};
use uuid::Uuid;

use super::{dispatch_and_respond, PendingDispatch, ResolvedSession, TurnClose, TurnRecord};
use crate::mcp::resources::ServerContext;
use crate::routes::messaging::adapter_factory::ChannelAdapterFactory;
use crate::services::turn_runner::{CloudTasksRunner, TurnRunner};

/// How many runs a turn is given before the athlete is told it is not coming.
///
/// The run that started it plus one resume. A turn drained twice has met two
/// scaledowns in a row, and a third attempt would keep the athlete waiting on
/// a placeholder that has already outlived two instances.
pub const MAX_TURN_ATTEMPTS: i64 = 2;

/// How often a live instance looks for turns to resume.
///
/// A drained turn is waiting on the next instance to boot, which on a
/// zero-minimum service is whenever the next request arrives; a sibling that
/// is already up answers it within one of these instead.
pub const SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// Upper bound on turns one sweep claims or enqueues.
///
/// Each one is a full LLM dispatch. A scaledown drains a handful of turns at
/// most, so this is headroom, not a throttle.
const SWEEP_BATCH: i64 = 20;

/// How long a run's lease lasts between renewals.
///
/// Short, so a run that died without a word frees its row quickly; renewed
/// every [`LEASE_HEARTBEAT`] while the run is alive, so a long turn is never
/// stolen mid-answer.
pub const TURN_LEASE: Duration = Duration::from_secs(90);

/// How often a running turn renews its lease.
pub const LEASE_HEARTBEAT: Duration = Duration::from_secs(30);

/// How old a never-leased row must be before the sweep takes it.
///
/// A row is recorded a moment before whatever recorded it starts the turn —
/// the in-process claim, or a Cloud Tasks delivery a few seconds out. The
/// sweep must not race that start.
pub const QUEUED_GRACE: Duration = Duration::from_secs(90);

/// How often a blocked claim is retried while a predecessor turn in the same
/// conversation is still answering.
pub(crate) const CLAIM_RETRY: Duration = Duration::from_secs(2);

/// How long the in-process runner waits for a predecessor before leaving the
/// turn to the sweep. The Cloud Tasks runner carries its own, configured wait.
const IN_PROCESS_CLAIM_WAIT: Duration = Duration::from_mins(4);

/// This process's identity on the leases it takes.
///
/// The Cloud Run revision (or hostname) says which deploy took the lease;
/// the suffix separates the instances of one revision, which share it.
static LEASE_HOLDER: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}/{}",
        pierre_logging::host_identity().unwrap_or("unknown"),
        Uuid::new_v4().simple()
    )
});

/// What became of a turn the drain signal interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrainHandOff {
    /// The turn is on file for another run. Nothing was said to the athlete;
    /// the placeholder is left standing.
    Recorded,
    /// The turn has no attempt left, or the record could not be written:
    /// the caller closes the placeholder with the apology.
    Exhausted,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// The lease this process takes on a turn it is about to run.
pub(crate) fn run_lease() -> TurnLease<'static> {
    let now = now_ms();
    // Safe cast: a lease is seconds, not centuries.
    #[allow(clippy::cast_possible_truncation)]
    let lease_ms = TURN_LEASE.as_millis() as i64;
    TurnLease {
        leased_by: &LEASE_HOLDER,
        now_ms: now,
        lease_until_ms: now + lease_ms,
        max_attempts: MAX_TURN_ATTEMPTS,
    }
}

/// The row a dispatch is recorded as.
fn row_from_dispatch(dispatch: &PendingDispatch, attempts: i64) -> ResumableTurnRow {
    ResumableTurnRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: dispatch.session_tenant_id.to_string(),
        channel_tenant_id: dispatch.channel_tenant_id.to_string(),
        user_tenant_id: dispatch.user_tenant_id.to_string(),
        session_id: dispatch.session.session_id.clone(),
        conversation: dispatch.session.conversation.clone(),
        user_id: dispatch.session.user_id.clone(),
        channel: dispatch.channel.clone(),
        sender_id: dispatch.sender_id.clone(),
        conversation_id: dispatch.conversation_id.clone(),
        channel_message_id: dispatch.channel_message_id.clone(),
        thread_id: dispatch.thread_id.clone(),
        text_content: dispatch.text_content.clone(),
        is_group_chat: dispatch.is_group_chat,
        locale: dispatch.locale.clone(),
        turn_id: dispatch.turn_id.to_string(),
        placeholder_message_id: None,
        attempts,
        enqueue_seq: 0,
        created_at_ms: now_ms(),
    }
}

/// Record the turn and hand it to the runner.
///
/// The one entry point every ingress path — webhook, Slack socket, Discord
/// gateway — starts a turn through. The row lands first; then the in-process
/// runner claims it and spawns the turn, or the Cloud Tasks runner enqueues
/// its id and the delivery claims it. A row already on file for the same
/// inbound message is a webhook redelivery of a turn that is already running,
/// and is not started again.
pub(crate) async fn start_turn(resources: &Arc<ServerContext>, dispatch: PendingDispatch) {
    let row = row_from_dispatch(&dispatch, 0);
    let recorded = resources
        .common
        .repos
        .resumable_turns
        .record_resumable_turn(&row)
        .await;
    match recorded {
        Ok(true) => {}
        Ok(false) => {
            info!(
                channel = %dispatch.channel,
                channel_message_id = %dispatch.channel_message_id,
                "inbound message already recorded as a turn; not started again"
            );
            return;
        }
        Err(e) => {
            // The database refused the record. The inbound message itself was
            // stored a moment ago, so this is a transient fault; the turn runs
            // as an unrecorded detached task rather than dropping the athlete's
            // question, and says so.
            warn!(
                error = %e,
                channel = %dispatch.channel,
                "turn could not be recorded; running it detached and unrecorded"
            );
            spawn_turn(resources, dispatch);
            return;
        }
    }

    match resources.common.turn_runner.as_ref() {
        TurnRunner::InProcess => spawn_when_claimable(resources, dispatch, row.id),
        TurnRunner::CloudTasks(runner) => {
            enqueue_turn(
                runner,
                dispatch.session_tenant_id,
                &row.id,
                &dispatch.channel,
                0,
            )
            .await;
        }
    }
}

/// Spawn a dispatch through the in-flight tracker, keeping the caller's span
/// so every log line of the turn carries the ingress ids.
fn spawn_turn(resources: &Arc<ServerContext>, dispatch: PendingDispatch) {
    resources.common.turns.spawn(
        async move {
            // Boxed so the turn's state machine — tens of kilobytes of
            // pipeline locals — lives on the heap rather than on the spawned
            // task's stack for the turn's whole duration.
            Box::pin(dispatch_and_respond(dispatch)).await;
        }
        .in_current_span(),
    );
}

/// In-process runner: claim the row, then run the turn; while an older turn
/// of the same conversation is still answering, wait for it.
///
/// The wait lives on the tracker too, so a drain sees it. A wait that runs
/// out leaves the row for the sweep, which claims it once the predecessor is
/// gone.
fn spawn_when_claimable(resources: &Arc<ServerContext>, dispatch: PendingDispatch, row_id: String) {
    let resources = Arc::clone(resources);
    resources.common.turns.clone().spawn(
        async move {
            let tenant_id = dispatch.session_tenant_id;
            let deadline = Instant::now() + IN_PROCESS_CLAIM_WAIT;
            loop {
                let claim = resources
                    .common
                    .repos
                    .resumable_turns
                    .claim_resumable_turn(tenant_id, &row_id, &run_lease())
                    .await;
                match claim {
                    Ok(TurnClaim::Claimed(row)) => {
                        let mut dispatch = dispatch;
                        dispatch.record = Some(TurnRecord {
                            row_id: row.id,
                            placeholder_message_id: row.placeholder_message_id,
                            attempts: row.attempts,
                        });
                        Box::pin(dispatch_and_respond(dispatch)).await;
                        return;
                    }
                    Ok(TurnClaim::Blocked) if Instant::now() < deadline => {
                        sleep(CLAIM_RETRY).await;
                    }
                    Ok(TurnClaim::Blocked) => {
                        info!(row_id = %row_id, "turn still behind its predecessor; the sweep will run it");
                        return;
                    }
                    Ok(TurnClaim::Missing | TurnClaim::Exhausted) => return,
                    Err(e) => {
                        warn!(error = %e, row_id = %row_id, "turn could not be claimed; the sweep will run it");
                        return;
                    }
                }
            }
        }
        .in_current_span(),
    );
}

/// Enqueue one delivery of a recorded turn. A refusal leaves the row on file
/// for the sweep, which enqueues it again under the next sequence.
async fn enqueue_turn(
    runner: &CloudTasksRunner,
    tenant_id: TenantId,
    row_id: &str,
    channel: &str,
    seq: i64,
) {
    if let Err(e) = runner.enqueue(tenant_id, row_id, seq).await {
        warn!(error = %e, row_id, seq, "turn could not be enqueued; the sweep will enqueue it again");
        info!(
            target: "notify",
            event = "messaging.error",
            tenant_id = %tenant_id,
            channel = %channel,
            error_type = "turn_enqueue_failed",
            "messaging error"
        );
    }
}

/// Hand a drained turn to the next run.
///
/// Runs the instant `run_bounded` reports the drain, before any channel
/// I/O, because it has to land inside what is left of the termination grace
/// once the drain's own 5 s + 2 s windows are spent: one small write, no
/// reads. A recorded run with attempts left releases its row; a recorded run
/// at the cap finishes the row so the caller can apologise; an unrecorded run
/// writes the row it never had.
///
/// A failed write is reported as [`DrainHandOff::Exhausted`] — the athlete is
/// told the answer is not coming rather than left waiting on a resume that
/// was never recorded.
pub(super) async fn hand_off_drained_turn(
    dispatch: &PendingDispatch,
    placeholder_message_id: Option<&str>,
) -> DrainHandOff {
    match dispatch.record.as_ref() {
        Some(record) => release_recorded_turn(dispatch, record).await,
        None => record_unrecorded_run(dispatch, placeholder_message_id).await,
    }
}

/// A recorded run drained: hand the row back while attempts remain, finish it
/// once they are spent.
async fn release_recorded_turn(dispatch: &PendingDispatch, record: &TurnRecord) -> DrainHandOff {
    if record.attempts >= MAX_TURN_ATTEMPTS {
        finish_turn_record(dispatch).await;
        return DrainHandOff::Exhausted;
    }
    match dispatch
        .resources
        .common
        .repos
        .resumable_turns
        .release_resumable_turn(dispatch.session_tenant_id, &record.row_id, now_ms())
        .await
    {
        Ok(()) => DrainHandOff::Recorded,
        Err(e) => {
            warn!(
                error = %e,
                row_id = %record.row_id,
                "drained turn could not release its lease; apologising"
            );
            DrainHandOff::Exhausted
        }
    }
}

/// A run that had no row — the record failed at ingress — drained: write the
/// row a resume rebuilds the dispatch from, counting this run.
async fn record_unrecorded_run(
    dispatch: &PendingDispatch,
    placeholder_message_id: Option<&str>,
) -> DrainHandOff {
    let mut row = row_from_dispatch(dispatch, 1);
    row.placeholder_message_id = placeholder_message_id.map(str::to_owned);
    match dispatch
        .resources
        .common
        .repos
        .resumable_turns
        .record_resumable_turn(&row)
        .await
    {
        // `false` means the row already exists — the same inbound message was
        // recorded before this run, which is the idempotency the unique index
        // exists for; either way the turn is on file.
        Ok(_) => {
            info!(
                row_id = %row.id,
                channel = %dispatch.channel,
                conversation_id = %dispatch.session.conversation,
                turn_id = %dispatch.turn_id,
                placeholder = placeholder_message_id.is_some(),
                "drained turn recorded for resume on the next instance"
            );
            DrainHandOff::Recorded
        }
        Err(e) => {
            warn!(
                error = %e,
                channel = %dispatch.channel,
                conversation_id = %dispatch.session.conversation,
                "drained turn could not be recorded for resume; apologising"
            );
            DrainHandOff::Exhausted
        }
    }
}

/// Delete the row a recorded dispatch was running from.
///
/// Called on every end a turn can reach, so the lease can never expire on a
/// turn that was already answered and hand it to a sibling for a second
/// reply. A no-op for an unrecorded run.
pub(super) async fn finish_turn_record(dispatch: &PendingDispatch) {
    let Some(record) = dispatch.record.as_ref() else {
        return;
    };
    if let Err(e) = dispatch
        .resources
        .common
        .repos
        .resumable_turns
        .finish_resumable_turn(dispatch.session_tenant_id, &record.row_id)
        .await
    {
        warn!(
            error = %e,
            row_id = %record.row_id,
            "turn ended but its row could not be finished; the lease expires it"
        );
    }
}

/// Record the placeholder a run just opened, so a later run of the same turn
/// edits that message instead of sending a second one.
pub(super) async fn note_placeholder(dispatch: &PendingDispatch, placeholder_message_id: &str) {
    let Some(record) = dispatch.record.as_ref() else {
        return;
    };
    if let Err(e) = dispatch
        .resources
        .common
        .repos
        .resumable_turns
        .set_resumable_turn_placeholder(
            dispatch.session_tenant_id,
            &record.row_id,
            placeholder_message_id,
        )
        .await
    {
        warn!(
            error = %e,
            row_id = %record.row_id,
            "turn's placeholder could not be recorded; a resumed run would open a second one"
        );
    }
}

/// The lease renewal a running turn keeps alive; aborted when dropped.
pub(super) struct LeaseHeartbeat(JoinHandle<()>);

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Renew the run's lease every [`LEASE_HEARTBEAT`] until the guard drops.
///
/// A renewal that finds the lease gone or held by someone else logs it: the
/// row was finished or reclaimed under this run, which happens only if this
/// run stalled past [`TURN_LEASE`] without renewing — a stalled runtime, not
/// a stalled turn.
pub(super) fn keep_lease(dispatch: &PendingDispatch) -> Option<LeaseHeartbeat> {
    let record = dispatch.record.as_ref()?;
    let resources = Arc::clone(&dispatch.resources);
    let tenant_id = dispatch.session_tenant_id;
    let row_id = record.row_id.clone();
    let handle = tokio::spawn(async move {
        loop {
            sleep(LEASE_HEARTBEAT).await;
            let lease = run_lease();
            match resources
                .common
                .repos
                .resumable_turns
                .renew_resumable_turn_lease(
                    tenant_id,
                    &row_id,
                    lease.leased_by,
                    lease.lease_until_ms,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    warn!(row_id = %row_id, "turn lease is no longer this run's; a sibling may be re-running it");
                    return;
                }
                Err(e) => warn!(error = %e, row_id = %row_id, "turn lease could not be renewed"),
            }
        }
    });
    Some(LeaseHeartbeat(handle))
}

/// Re-run or re-enqueue every turn an instance left behind.
///
/// Returns how many turns this pass took. Rows past the attempt cap whose
/// lease ended are reaped first: the run that was to close their placeholder
/// died too, and nothing will claim them again.
pub async fn sweep_resumable_turns(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
) -> usize {
    let lease = run_lease();
    reap_exhausted(resources, lease.now_ms).await;
    // Safe cast: the grace is seconds, not centuries.
    #[allow(clippy::cast_possible_truncation)]
    let grace_ms = QUEUED_GRACE.as_millis() as i64;
    let claim = ResumableTurnClaim {
        lease,
        queued_older_than_ms: lease.now_ms - grace_ms,
        limit: SWEEP_BATCH,
    };
    match resources.common.turn_runner.as_ref() {
        TurnRunner::InProcess => sweep_in_process(resources, adapters, &claim).await,
        TurnRunner::CloudTasks(runner) => sweep_cloud_tasks(resources, runner, &claim).await,
    }
}

/// Drop the rows nothing will ever claim again, saying so at ERROR: each is
/// an athlete who was never told.
async fn reap_exhausted(resources: &Arc<ServerContext>, now_ms: i64) {
    match resources
        .common
        .repos
        .resumable_turns
        .reap_exhausted_turns(now_ms, MAX_TURN_ATTEMPTS)
        .await
    {
        Ok(0) => {}
        Ok(reaped) => error!(
            reaped,
            "turns past the attempt cap whose closing run died too were dropped; each is an athlete never told"
        ),
        Err(e) => warn!(error = %e, "exhausted turns could not be reaped"),
    }
}

/// In-process: claim what is stale and run it here.
async fn sweep_in_process(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    claim: &ResumableTurnClaim<'_>,
) -> usize {
    let claimed = match resources
        .common
        .repos
        .resumable_turns
        .claim_resumable_turns(claim)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %e, "resumable turn sweep could not claim rows");
            return 0;
        }
    };
    let count = claimed.len();
    for row in claimed {
        let Some(dispatch) = rebuild_dispatch(resources, adapters, row).await else {
            continue;
        };
        info!(
            channel = %dispatch.channel,
            conversation_id = %dispatch.session.conversation,
            turn_id = %dispatch.turn_id,
            attempts = dispatch.record.as_ref().map_or(0, |r| r.attempts),
            "resuming a messaging turn"
        );
        spawn_turn(resources, dispatch);
    }
    count
}

/// Cloud Tasks: enqueue what is stale again under the next sequence, and let
/// the delivery claim it.
async fn sweep_cloud_tasks(
    resources: &Arc<ServerContext>,
    runner: &CloudTasksRunner,
    claim: &ResumableTurnClaim<'_>,
) -> usize {
    let repo = resources.common.repos.resumable_turns.as_ref();
    let stale = match repo.list_stale_resumable_turns(claim).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %e, "resumable turn sweep could not list stale rows");
            return 0;
        }
    };
    let count = stale.len();
    for row in stale {
        re_enqueue_stale(repo, runner, &row).await;
    }
    count
}

/// Count one more enqueue of a stale row and put it back on the queue under
/// that sequence.
async fn re_enqueue_stale(
    repo: &dyn ResumableTurnRepository,
    runner: &CloudTasksRunner,
    row: &ResumableTurnRow,
) {
    let Ok(tenant_id) = TenantId::parse_str(&row.tenant_id) else {
        warn!(row_id = %row.id, "stale turn carries a malformed tenant id; leaving it for an operator");
        return;
    };
    let seq = match repo.bump_resumable_turn_enqueue(tenant_id, &row.id).await {
        Ok(Some(seq)) => seq,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e, row_id = %row.id, "stale turn's enqueue could not be counted");
            return;
        }
    };
    info!(row_id = %row.id, seq, "re-enqueuing a stale messaging turn");
    enqueue_turn(runner, tenant_id, &row.id, &row.channel, seq).await;
}

/// Run a claimed turn on this instance and report how it closed.
///
/// The turn is spawned through the in-flight tracker so a drain sees it and
/// so the caller's own future — a request handler whose connection may drop —
/// cannot cancel it by being dropped; the caller awaits its close through a
/// channel. `None` when the row could not be turned back into a dispatch;
/// [`rebuild_dispatch`] has already finished or released it.
pub(crate) async fn run_recorded_turn(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    row: ResumableTurnRow,
) -> Option<TurnClose> {
    let dispatch = rebuild_dispatch(resources, adapters, row).await?;
    info!(
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        turn_id = %dispatch.turn_id,
        attempts = dispatch.record.as_ref().map_or(0, |r| r.attempts),
        "running a delivered messaging turn"
    );
    let (tx, rx) = oneshot::channel();
    resources.common.turns.spawn(
        async move {
            let close = Box::pin(dispatch_and_respond(dispatch)).await;
            // A receiver that went away is a caller that stopped waiting; the
            // turn's own bookkeeping already closed its row.
            let _ = tx.send(close);
        }
        .in_current_span(),
    );
    rx.await.ok()
}

/// Rebuild the dispatch a row describes.
///
/// A row that can never be dispatched — malformed ids, no channel config, a
/// sender who no longer authenticates — is finished here; a row whose config
/// lookup merely failed is handed back for the next run.
pub(crate) async fn rebuild_dispatch(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    row: ResumableTurnRow,
) -> Option<PendingDispatch> {
    let session_tenant_id = parse_tenant(&row.tenant_id)?;
    let channel_tenant_id = parse_tenant(&row.channel_tenant_id)?;
    let user_tenant_id = parse_tenant(&row.user_tenant_id)?;
    let repo = resources.common.repos.resumable_turns.as_ref();

    let Some((channel_type, turn_id)) = parse_row_ids(&row) else {
        let _ = repo.finish_resumable_turn(session_tenant_id, &row.id).await;
        return None;
    };

    let adapter =
        match load_adapter(resources, adapters, channel_type, channel_tenant_id, &row).await {
            AdapterLookup::Built(adapter) => adapter,
            AdapterLookup::Unusable => {
                let _ = repo.finish_resumable_turn(session_tenant_id, &row.id).await;
                return None;
            }
            AdapterLookup::Unavailable => {
                let _ = repo
                    .release_resumable_turn(session_tenant_id, &row.id, now_ms())
                    .await;
                return None;
            }
        };

    let Some(auth_result) = authenticate_sender(resources, channel_tenant_id, &row).await else {
        let _ = repo.finish_resumable_turn(session_tenant_id, &row.id).await;
        return None;
    };

    Some(PendingDispatch {
        resources: Arc::clone(resources),
        adapter,
        auth_result,
        session: ResolvedSession {
            session_id: row.session_id,
            conversation: row.conversation,
            user_id: row.user_id,
        },
        channel_tenant_id,
        user_tenant_id,
        session_tenant_id,
        channel_type,
        channel: row.channel,
        sender_id: row.sender_id,
        conversation_id: row.conversation_id,
        text_content: row.text_content,
        channel_message_id: row.channel_message_id,
        thread_id: row.thread_id,
        is_group_chat: row.is_group_chat,
        locale: row.locale,
        turn_id,
        status_api_base: adapters.status_api_base(),
        record: Some(TurnRecord {
            row_id: row.id,
            placeholder_message_id: row.placeholder_message_id,
            attempts: row.attempts,
        }),
    })
}

/// The channel and turn id a row names, or `None` when either is malformed
/// — a row nothing can dispatch.
fn parse_row_ids(row: &ResumableTurnRow) -> Option<(ChannelType, ConversationTurnId)> {
    let Ok(channel_type) = ChannelType::from_str(&row.channel) else {
        warn!(row_id = %row.id, channel = %row.channel, "recorded turn names an unknown channel; finishing it");
        return None;
    };
    let Ok(turn_id) = Uuid::parse_str(&row.turn_id) else {
        warn!(row_id = %row.id, "recorded turn carries a malformed turn id; finishing it");
        return None;
    };
    Some((channel_type, ConversationTurnId::from_uuid(turn_id)))
}

/// What loading a row's channel adapter came to.
enum AdapterLookup {
    /// The tenant's stored config built an adapter.
    Built(Arc<dyn MessagingChannel>),
    /// No config, or one that cannot build an adapter: nothing can ever
    /// reply on this row.
    Unusable,
    /// The lookup itself failed; the next run tries again.
    Unavailable,
}

async fn load_adapter(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    channel_type: ChannelType,
    channel_tenant_id: TenantId,
    row: &ResumableTurnRow,
) -> AdapterLookup {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let config = match db.get_channel_config(channel_tenant_id, &row.channel).await {
        Ok(Some(config)) => config,
        Ok(None) => {
            warn!(row_id = %row.id, channel = %row.channel, "recorded turn has no channel config; finishing it");
            return AdapterLookup::Unusable;
        }
        Err(e) => {
            warn!(error = %e, row_id = %row.id, "recorded turn: channel config lookup failed; handing it back");
            return AdapterLookup::Unavailable;
        }
    };
    adapters.build(channel_type, &config).map_or_else(
        || {
            warn!(row_id = %row.id, channel = %row.channel, "recorded turn's channel config builds no adapter; finishing it");
            AdapterLookup::Unusable
        },
        AdapterLookup::Built,
    )
}

/// Re-run the channel authentication the webhook ran, so a sender
/// deactivated or unlinked since the record is refused here exactly as a
/// fresh message would be.
async fn authenticate_sender(
    resources: &Arc<ServerContext>,
    channel_tenant_id: TenantId,
    row: &ResumableTurnRow,
) -> Option<AuthResult> {
    match resources
        .auth
        .auth_middleware
        .authenticate_channel(channel_tenant_id, &row.channel, &row.sender_id)
        .await
    {
        Ok(auth) => Some(auth),
        Err(e) => {
            warn!(error = %e, row_id = %row.id, channel = %row.channel, "recorded turn's sender no longer authenticates; finishing it");
            info!(
                target: "notify",
                event = "messaging.error",
                tenant_id = %channel_tenant_id,
                channel = %row.channel,
                error_type = "resume_unauthenticated",
                "messaging error"
            );
            None
        }
    }
}

fn parse_tenant(raw: &str) -> Option<TenantId> {
    match TenantId::parse_str(raw) {
        Ok(id) => Some(id),
        Err(e) => {
            warn!(error = %e, "recorded turn carries a malformed tenant id; leaving the row for an operator");
            None
        }
    }
}

/// Start the resume sweeper: one pass now, then one every
/// [`SWEEP_INTERVAL`] for the life of the process.
///
/// The immediate pass is the one that matters on a zero-minimum service —
/// this instance may exist only because the athlete's next message arrived,
/// and the turn their previous message was drained from is waiting for it.
/// The periodic passes are for a sibling that outlived the drained instance.
pub fn start_turn_resume_sweeper(
    resources: Arc<ServerContext>,
    adapters: Arc<dyn ChannelAdapterFactory>,
) {
    let startup_resources = Arc::clone(&resources);
    let startup_adapters = Arc::clone(&adapters);
    tokio::spawn(async move {
        let taken = sweep_resumable_turns(&startup_resources, startup_adapters.as_ref()).await;
        if taken > 0 {
            info!(taken, "startup sweep took over messaging turns");
        }
    });
    spawn_periodic("turn resume sweeper", SWEEP_INTERVAL, move || {
        let resources = Arc::clone(&resources);
        let adapters = Arc::clone(&adapters);
        async move {
            let taken = sweep_resumable_turns(&resources, adapters.as_ref()).await;
            if taken > 0 {
                info!(taken, "sweep took over messaging turns");
            }
            Ok(())
        }
    });
}
