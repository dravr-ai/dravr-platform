// ABOUTME: The two guards around a messaging turn — per-conversation ordering and the panic boundary
// ABOUTME: Both are load-bearing and invisible when working, so both are driven directly by tests

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What holds a messaging turn together.
//!
//! Three guards wrap every dispatch, and none of them shows up in a reply
//! when it is doing its job:
//!
//! 1. **Ordering.** A webhook returns HTTP 200 before the turn runs, so two
//!    messages sent a second apart in the same conversation start two
//!    background tasks. Without a lock, the second one's reply can land first
//!    — the athlete reads an answer to the question they have not asked yet.
//!    [`acquire_dispatch_lock`] serializes per conversation and lets unrelated
//!    conversations run in parallel.
//!
//! 2. **Containment.** A panic in any pipeline stage would otherwise escape
//!    the spawned task, and the athlete would get silence — no reply, no
//!    error, nothing to report. [`run_guarded`] catches it, turns it into a
//!    structured failure for *this* turn, and lets the caller answer with a
//!    correlation id an operator can grep.
//!
//! 3. **Termination.** A turn runs detached from the request that started it,
//!    so nothing else ever ends it: not the HTTP layer, which answered 200
//!    before the turn began, and not the process, which is free to exit while
//!    the turn is mid-call. [`run_bounded`] gives it both missing endings — a
//!    wall-clock ceiling, and the shutdown drain signal. Without it a dead
//!    turn leaves the athlete's "thinking…" placeholder open permanently,
//!    which reads exactly like a slow answer that is still coming
//!    (registre#109). The two endings are answered differently: a turn past
//!    its ceiling is closed with a notice, while a turn the drain took is
//!    recorded and re-run on the next instance, editing the same placeholder
//!    into the real reply (registre#126, `super::resume`).

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use dashmap::DashMap;
use futures_util::FutureExt;
use pierre_chat_pipeline::ServedTurn;
use pierre_core::error_helpers::panic_payload_str;
use pierre_core::errors::{AppError, ErrorCode};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Per-conversation dispatch locks ensuring sequential LLM processing.
///
/// Without this, concurrent webhook calls for the same conversation race:
/// message 2's dispatch can finish before message 1's, producing out-of-order
/// replies. The lock serializes dispatches per conversation while allowing
/// different conversations to proceed in parallel.
static CONVERSATION_DISPATCH_LOCKS: LazyLock<DashMap<String, Arc<TokioMutex<()>>>> =
    LazyLock::new(DashMap::new);

/// The lock that serializes dispatches for `conversation_id`.
///
/// Callers hold the returned handle for the whole turn, `lock().await` on it,
/// and hand it back to [`evict_idle_dispatch_lock`] when the turn ends.
#[must_use]
pub fn acquire_dispatch_lock(conversation_id: &str) -> Arc<TokioMutex<()>> {
    CONVERSATION_DISPATCH_LOCKS
        .entry(conversation_id.to_owned())
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone()
}

/// Remove the per-conversation lock from the shared map if no other task still
/// holds it.
///
/// Prevents unbounded growth of the lock map under high conversation
/// cardinality while staying safe: if a concurrent dispatch cloned the `Arc`
/// before we got here, the strong count exceeds 2 and we leave the entry in
/// place. The next waiter simply reinserts on a later call if it was already
/// evicted.
pub fn evict_idle_dispatch_lock(conversation_id: &str, local: &Arc<TokioMutex<()>>) {
    // Strong references: the one in the DashMap entry + `local` held here.
    // Any higher count means another dispatch task is waiting on this lock.
    CONVERSATION_DISPATCH_LOCKS.remove_if(conversation_id, |_, stored| {
        Arc::ptr_eq(stored, local) && Arc::strong_count(stored) <= 2
    });
}

/// How one guarded turn ended.
///
/// Three outcomes, because they get three different replies: the athlete's
/// answer, the localized "your plan says no" denial, and the apology with a
/// correlation id. Collapsing the middle one into the last is what teaches an
/// operator to ignore alerts — a budget refusing a turn is the plan working.
pub enum TurnOutcome {
    /// The turn service served the turn. Its own large payload — the
    /// envelope's persisted rows — is already boxed inside [`ServedTurn`], so
    /// this variant stays the size of the two error variants beside it.
    Delivered(ServedTurn),
    /// A usage cap or rate limit refused the turn.
    QuotaDenied(AppError),
    /// The turn failed, including by panicking inside a pipeline stage.
    Failed(AppError),
    /// The turn was still running when something outside it ran out of
    /// patience. It produced no answer; whether one is still coming depends
    /// on the cause — see [`TurnInterruption`].
    Interrupted(TurnInterruption),
}

/// Why a turn was cut short before it produced anything.
///
/// The two causes get two different endings. A hung turn is closed with a
/// notice — re-running it is not a fix. A drained turn is a healthy turn on
/// an instance that is going away, so it is handed to the next instance and
/// answered there; the athlete only reads a notice once the hand-off has been
/// drained too. An operator reading the log needs to tell them apart for the
/// same reason: only the first is a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnInterruption {
    /// The turn exceeded its wall-clock ceiling. Everything below it is
    /// individually bounded, so reaching this means something under the
    /// pipeline has no timeout of its own.
    Watchdog,
    /// The process is shutting down and spent its grace window without this
    /// turn finishing. Not a fault of the turn, which is why the dispatcher
    /// resumes it elsewhere instead of closing it.
    Drain,
}

impl TurnInterruption {
    /// Stable label for logs and the `messaging.error` notify event.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Watchdog => "turn_watchdog",
            Self::Drain => "shutdown_drain",
        }
    }
}

/// Run one turn behind the panic boundary and classify how it ended.
///
/// `AssertUnwindSafe` is sound because a caught panic aborts the whole turn:
/// none of the borrowed state is touched afterwards, the caller reports and
/// returns.
///
/// The turn is pinned to the heap first. A chat turn's state machine is tens
/// of kilobytes — every stage's locals live in it — and leaving it inline
/// would put all of that on the stack of every webhook task that awaits a
/// dispatch.
pub async fn run_guarded<F>(run: F) -> TurnOutcome
where
    F: Future<Output = Result<ServedTurn, AppError>>,
{
    match AssertUnwindSafe(Box::pin(run)).catch_unwind().await {
        Ok(Ok(served)) => TurnOutcome::Delivered(served),
        Ok(Err(e))
            if matches!(
                e.code,
                ErrorCode::QuotaExceeded | ErrorCode::RateLimitExceeded
            ) =>
        {
            TurnOutcome::QuotaDenied(e)
        }
        Ok(Err(e)) => TurnOutcome::Failed(e),
        Err(panic) => TurnOutcome::Failed(AppError::internal(format!(
            "chat pipeline panicked: {}",
            panic_payload_str(panic.as_ref())
        ))),
    }
}

/// Run one guarded turn under both of its missing endings.
///
/// `run` is the already-guarded turn (so a panic inside it still classifies as
/// [`TurnOutcome::Failed`], not as an interruption). This adds the two ways a
/// turn ends without the pipeline having any say:
///
/// - `budget` elapses. Every stage under the pipeline is separately bounded —
///   a loopback tool call at 90s, an ACP message gap at 120s, a whole ACP
///   prompt at 300s — so this ceiling is not there to cut a slow turn short.
///   It exists because a turn that outlives all of those has found something
///   with no bound at all, and an unbounded turn holds its conversation's
///   dispatch lock, so the athlete's *next* question queues behind it too.
/// - `drain` fires. The process is going away; see
///   `services::turn_lifecycle::InFlightTurns`.
///
/// Whichever arrives first wins, and the turn future is dropped at that point
/// — cancellation, not abortion, so every stage unwinds through its own
/// `Drop`. The caller is expected to answer a [`TurnOutcome::Interrupted`]
/// rather than fall silent: a watchdog with a closing message, a drain with
/// the durable hand-off that lets another instance deliver the answer.
///
/// The turn is pinned to the heap for the same reason [`run_guarded`] pins its
/// own: a `select!` holds every branch inline, so an unboxed turn would put
/// the whole chat state machine on the stack of the task awaiting it.
pub async fn run_bounded<F>(run: F, budget: Duration, drain: &CancellationToken) -> TurnOutcome
where
    F: Future<Output = TurnOutcome>,
{
    let run = Box::pin(run);
    tokio::select! {
        // Biased: when a turn completes in the same poll as the deadline, the
        // completed turn is the truthful answer. Left to chance, a turn that
        // finished would sometimes be reported to the athlete as interrupted.
        biased;
        outcome = run => outcome,
        () = drain.cancelled() => TurnOutcome::Interrupted(TurnInterruption::Drain),
        () = sleep(budget) => TurnOutcome::Interrupted(TurnInterruption::Watchdog),
    }
}

/// A fresh correlation id and the short form that goes in the athlete's reply.
///
/// The id is surfaced in the user-facing reply and the log record so an
/// operator receiving a Slack alert can grep Cloud Logging for the full error
/// chain without access to conversation ids (which are PII-adjacent). Eight
/// hex characters is enough to find one turn and short enough to read back
/// over the phone.
#[must_use]
pub fn new_correlation_id() -> (Uuid, String) {
    let correlation_id = Uuid::new_v4();
    let short = correlation_id.simple().to_string()[..8].to_owned();
    (correlation_id, short)
}
