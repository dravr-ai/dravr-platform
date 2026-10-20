// ABOUTME: One lock per conversation that every chat turn holds while it runs — in-app and messaging alike
// ABOUTME: The personal thread is shared by the app and the athlete's DM, so their turns on it take turns (carnet#840)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn ordering per conversation.
//!
//! A messaging webhook returns HTTP 200 before its turn runs, so two messages
//! sent a second apart in the same conversation start two background tasks.
//! Without a lock, the second one's reply can land first — the athlete reads
//! an answer to the question they have not asked yet. The athlete's personal
//! thread is also written from the app (carnet#840): a question typed in the
//! app while a Telegram turn is still answering would otherwise read the
//! thread mid-turn and answer beside it.
//!
//! Every turn — the web route's and the messaging dispatcher's — holds the
//! lock for its conversation while it runs. Unrelated conversations run in
//! parallel. The lock is this process's: a Cloud Tasks delivery orders a
//! conversation's messaging turns across instances by its own claim.

use std::sync::{Arc, LazyLock};

use dashmap::DashMap;
use tokio::sync::Mutex as TokioMutex;

/// Per-conversation turn locks.
static CONVERSATION_TURN_LOCKS: LazyLock<DashMap<String, Arc<TokioMutex<()>>>> =
    LazyLock::new(DashMap::new);

/// The lock that serializes turns for `conversation_id`.
///
/// Callers hold the returned handle for the whole turn, `lock().await` on it,
/// and hand it back to [`evict_idle_turn_lock`] when the turn ends.
#[must_use]
pub fn acquire_turn_lock(conversation_id: &str) -> Arc<TokioMutex<()>> {
    CONVERSATION_TURN_LOCKS
        .entry(conversation_id.to_owned())
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone()
}

/// Remove the per-conversation lock from the shared map if no other task still
/// holds it.
///
/// Prevents unbounded growth of the lock map under high conversation
/// cardinality while staying safe: if a concurrent turn cloned the `Arc`
/// before we got here, the strong count exceeds 2 and we leave the entry in
/// place. The next waiter simply reinserts on a later call if it was already
/// evicted.
pub fn evict_idle_turn_lock(conversation_id: &str, local: &Arc<TokioMutex<()>>) {
    // Strong references: the one in the DashMap entry + `local` held here.
    // Any higher count means another turn is waiting on this lock.
    CONVERSATION_TURN_LOCKS.remove_if(conversation_id, |_, stored| {
        Arc::ptr_eq(stored, local) && Arc::strong_count(stored) <= 2
    });
}
