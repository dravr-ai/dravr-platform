// ABOUTME: Per-run broadcast registry keyed by run_id with a bounded replay buffer
// ABOUTME: Producers publish serialized events; the in-process status bridge consumes them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-run AG-UI broadcast registry.
//!
//! Each active messaging run owns a [`tokio::sync::broadcast`] channel keyed
//! by its `run_id`. The producer (the chat pipeline) publishes serialized
//! [`super::events::AgUiEvent`] strings; the consumer
//! (`pierre_services::messaging_status_bridge`) receives copies and turns
//! them into Telegram/Slack/Discord placeholder edits.
//!
//! Both halves live in this process and inside one turn: the dispatcher mints
//! the `run_id`, registers it, and hands it straight to the status consumer it
//! spawns. Nothing outside the process can name a run, so the registry carries
//! no identity and performs no authorization — the in-app surfaces read their
//! progress off the turn's own event stream instead
//! (`pierre_services::chat_stream`).
//!
//! Each registration retains a bounded ring buffer of the most recent
//! serialized events
//! ([`AGUI_RUN_REPLAY_BUFFER_SIZE`](crate::AGUI_RUN_REPLAY_BUFFER_SIZE)). The
//! consumer is spawned a moment after `register_scoped`, so the events emitted
//! in between are already in the buffer and are flushed before it switches to
//! the live receiver.
//!
//! `broadcast` fits the fan-out model — one run can have several live
//! consumers — and drops the oldest messages rather than blocking the producer
//! if a consumer lags, which matches the pipeline's no-back-pressure
//! guarantee.

use crate::AGUI_RUN_REPLAY_BUFFER_SIZE;
use dashmap::DashMap;
use pierre_core::constants::network_config::SSE_BROADCAST_CHANNEL_SIZE;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Outcome of [`RunRegistry::publish`].
///
/// Distinguishes the benign "no subscribers yet" case from the
/// likely-bug "publishing to an unregistered run" case so producer
/// code (e.g. [`super::emitter::BroadcastSink::emit`]) can warn
/// loudly on the latter without spamming the logs on the former.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PublishOutcome {
    /// Event was broadcast to `n` live subscribers (and appended to
    /// the replay buffer regardless).
    Delivered(usize),
    /// The run is registered but currently has no subscribers — the
    /// event was appended to the replay buffer, so any later
    /// subscriber will still see it.
    NoSubscribers,
    /// The `run_id` is not registered.
    ///
    /// The event is silently dropped because there is no buffer to
    /// append it to. Producers should treat this as a programmer
    /// error (typoed id, missing `register`, registration race) and
    /// surface it.
    NoSlot,
}

impl PublishOutcome {
    /// Number of live subscribers that received the event, or zero
    /// when nothing was delivered.
    #[must_use]
    pub const fn delivered(self) -> usize {
        match self {
            Self::Delivered(n) => n,
            Self::NoSubscribers | Self::NoSlot => 0,
        }
    }

    /// `true` when the run is unknown to the registry. Producer code
    /// should log this distinctly from `NoSubscribers`.
    #[must_use]
    pub const fn is_no_slot(self) -> bool {
        matches!(self, Self::NoSlot)
    }
}

/// Snapshot of a run's recent event backlog plus a live receiver.
///
/// Returned from [`RunRegistry::subscribe_self`] so the consumer can
/// flush the backlog first, then switch to the live channel without
/// losing any messages in between.
pub struct RunSubscription {
    /// Already-emitted events retained in the replay buffer, in
    /// publish order. The handler MUST forward these before
    /// consuming `receiver`.
    pub backlog: Vec<String>,
    /// Live receiver for events emitted after the subscribe.
    pub receiver: broadcast::Receiver<String>,
}

/// Internal entry per registered run: the broadcast sender and the
/// bounded replay buffer.
struct RunSlot {
    sender: broadcast::Sender<String>,
    recent: Arc<Mutex<VecDeque<String>>>,
}

/// Registry of active agent runs and their broadcast channels.
///
/// Cloneable via [`Arc`] so handlers, pipeline hooks, and admin
/// endpoints share a single shared registry.
#[derive(Clone, Default)]
pub struct RunRegistry {
    inner: Arc<RunRegistryInner>,
}

#[derive(Default)]
struct RunRegistryInner {
    channels: DashMap<String, RunSlot>,
}

impl RunRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a run and return its broadcast sender.
    ///
    /// Idempotent — if a run with the same `run_id` already exists the
    /// existing sender is returned, so a consumer that attaches after the
    /// producer started joins the same channel rather than replacing it.
    ///
    /// Run ids are minted per turn by the producer (`Uuid::new_v4`) and never
    /// leave the process: the only consumer is the in-process messaging
    /// status bridge, which subscribes through [`Self::subscribe_self`] on
    /// the id it was just handed. There is no network surface to authorize.
    pub fn register(&self, run_id: &str) -> broadcast::Sender<String> {
        if let Some(existing) = self.inner.channels.get(run_id) {
            return existing.value().sender.clone();
        }
        let (tx, _rx) = broadcast::channel(SSE_BROADCAST_CHANNEL_SIZE);
        self.inner.channels.insert(
            run_id.to_owned(),
            RunSlot {
                sender: tx.clone(),
                recent: Arc::new(Mutex::new(VecDeque::with_capacity(
                    AGUI_RUN_REPLAY_BUFFER_SIZE,
                ))),
            },
        );
        debug!(run_id = %run_id, "registered AG-UI run");
        tx
    }

    /// Register a run with an RAII scope that auto-unregisters when
    /// the returned [`RunScope`] drops.
    ///
    /// Prefer this over raw [`Self::register`] + manual
    /// [`Self::unregister`]: channel adapters that return early on
    /// pipeline errors frequently forget the cleanup path and leak
    /// registry entries. The scope guard guarantees the slot is
    /// removed whether the turn succeeded, errored, or panicked.
    #[must_use]
    pub fn register_scoped(&self, run_id: &str) -> RunScope {
        let sender = self.register(run_id);
        RunScope {
            registry: self.clone(),
            run_id: run_id.to_owned(),
            sender,
        }
    }

    /// Subscribe to a run's backlog plus its live channel.
    ///
    /// Identity-agnostic because the only caller is in process and already
    /// holds the `run_id` it registered a moment earlier — the producer half
    /// of the broadcast is the proof of identity.
    #[must_use]
    pub fn subscribe_self(&self, run_id: &str) -> Option<RunSubscription> {
        self.inner
            .channels
            .get(run_id)
            .map(|entry| snapshot(entry.value()))
    }

    /// Publish a pre-serialized event to the run's subscribers and
    /// the replay buffer.
    ///
    /// Returns a [`PublishOutcome`] so producers can distinguish the
    /// benign "no subscribers yet" case from the buggy "publishing
    /// to an unregistered run" case. Non-blocking — the underlying
    /// broadcast channel drops the oldest messages on overflow.
    ///
    /// The buffer push and the broadcast send run under the same
    /// `recent` mutex that [`snapshot`] holds across subscribe + copy,
    /// so a new subscriber cannot observe a state where the event is
    /// both in the backlog *and* still pending delivery on its live
    /// receiver (or vice versa). Either the event was fully published
    /// before the snapshot (in which case it's in the backlog and the
    /// new receiver won't see it) or it lands fully after the snapshot
    /// (in which case the live receiver gets it and the backlog does
    /// not) — never both, never neither.
    #[must_use]
    pub fn publish(&self, run_id: &str, serialized_event: String) -> PublishOutcome {
        let Some(entry) = self.inner.channels.get(run_id) else {
            return PublishOutcome::NoSlot;
        };
        let slot = entry.value();
        let Ok(mut buf) = slot.recent.lock() else {
            // Poisoned mutex is unrecoverable here — drop the event
            // rather than split buffer-vs-broadcast state.
            warn!(run_id = %run_id, "AG-UI replay buffer mutex poisoned; dropping event");
            return PublishOutcome::NoSlot;
        };
        if buf.len() >= AGUI_RUN_REPLAY_BUFFER_SIZE {
            buf.pop_front();
        }
        buf.push_back(serialized_event.clone());
        // `send` only errors when there are zero receivers — the
        // event still landed in the replay buffer above so any
        // late-arriving subscriber will still see it. Held under
        // `recent` lock so the push + broadcast are atomic relative
        // to `snapshot`'s subscribe + copy.
        slot.sender
            .send(serialized_event)
            .map_or(PublishOutcome::NoSubscribers, PublishOutcome::Delivered)
    }

    /// Deregister a run and drop its channel. Any live subscribers
    /// observe a closed channel and terminate their streams.
    pub fn unregister(&self, run_id: &str) {
        if self.inner.channels.remove(run_id).is_some() {
            debug!(run_id = %run_id, "unregistered AG-UI run");
        }
    }

    /// Count of active runs. Primarily used by `/health` to surface
    /// AG-UI liveness.
    #[must_use]
    pub fn active_run_count(&self) -> usize {
        self.inner.channels.len()
    }
}

/// RAII guard that deregisters the run from the [`RunRegistry`] on
/// drop. Obtain one via [`RunRegistry::register_scoped`].
///
/// Implements `Deref<Target = broadcast::Sender<String>>` so producer
/// code can call `.send(...)` directly, but the common path is to
/// hand the scope over to the chat pipeline as part of `AgUiRun` and
/// let it drop when the turn completes.
pub struct RunScope {
    registry: RunRegistry,
    run_id: String,
    sender: broadcast::Sender<String>,
}

impl RunScope {
    /// The `run_id` this scope owns.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The live broadcast sender for producer code.
    ///
    /// Prefer publishing through the sink abstraction
    /// ([`super::emitter::BroadcastSink`]); this accessor is useful
    /// for tests that want to observe the underlying channel.
    #[must_use]
    pub fn sender(&self) -> &broadcast::Sender<String> {
        &self.sender
    }
}

impl Drop for RunScope {
    fn drop(&mut self) {
        self.registry.unregister(&self.run_id);
    }
}

/// Snapshot a registry slot under a held entry borrow.
///
/// Acquires the `recent` mutex first, then subscribes to the broadcast
/// channel, then copies the backlog — all under the same lock. This
/// matches the ordering in [`RunRegistry::publish`] (lock → push →
/// send → unlock) so a concurrent publish either fully precedes the
/// snapshot (event ends up only in the backlog; the new receiver was
/// not yet subscribed when `send` ran) or fully follows it (event
/// reaches only the receiver; it was pushed to the buffer after the
/// copy). Without this pairing a publish that interleaves between
/// `subscribe` and the buffer copy would land in both, causing the
/// messaging channel to emit the same payload twice (Bug C3).
fn snapshot(slot: &RunSlot) -> RunSubscription {
    let Ok(buf) = slot.recent.lock() else {
        // Poisoned mutex — return an empty subscription. The receiver
        // still works for any publishes that recover the lock (they
        // short-circuit to NoSlot on poison, so practically this path
        // yields a receiver that never fires; the caller observes the
        // broadcast close when the run unregisters).
        return RunSubscription {
            backlog: Vec::new(),
            receiver: slot.sender.subscribe(),
        };
    };
    let receiver = slot.sender.subscribe();
    let backlog: Vec<String> = buf.iter().cloned().collect();
    drop(buf);
    RunSubscription { backlog, receiver }
}
