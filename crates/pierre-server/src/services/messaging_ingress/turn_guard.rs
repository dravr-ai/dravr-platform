// ABOUTME: The two guards around a messaging turn — per-conversation ordering and the panic boundary
// ABOUTME: Both are load-bearing and invisible when working, so both are driven directly by tests

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What holds a messaging turn together.
//!
//! Two guards wrap every dispatch, and neither shows up in a reply when it is
//! doing its job:
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

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock};

use dashmap::DashMap;
use futures_util::FutureExt;
use pierre_chat_pipeline::ServedTurn;
use pierre_core::error_helpers::panic_payload_str;
use pierre_core::errors::{AppError, ErrorCode};
use tokio::sync::Mutex as TokioMutex;
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
