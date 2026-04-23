// ABOUTME: Per-run broadcast registry keyed by run_id with replay buffer and ownership binding
// ABOUTME: Producers publish serialized events; subscribers consume via tokio broadcast receivers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-run AG-UI broadcast registry.
//!
//! Each active agent run owns a [`tokio::sync::broadcast`] channel keyed
//! by its `run_id`. Producers (the chat pipeline) publish serialized
//! [`super::events::AgUiEvent`] strings; subscribers (SSE route handlers,
//! Telegram adapters) receive copies.
//!
//! Every registration carries an owning [`RunOwner`] — the `(user_id,
//! tenant_id)` pair the turn was issued for. SSE route handlers MUST
//! look up the owner before returning a subscription so a caller can
//! never read another user's run even if they learn the `run_id` (URL
//! capture, log leak, brute-force guess against a short id). The
//! broadcast channel stays private to producer code; subscribers go
//! through the route handler which enforces the owner check.
//!
//! Each registration also retains a bounded ring buffer of the most
//! recent serialized events
//! ([`AGUI_RUN_REPLAY_BUFFER_SIZE`](crate::constants::network_config::AGUI_RUN_REPLAY_BUFFER_SIZE)).
//! A late-subscribing client (the common case: the channel adapter
//! hands the `run_id` back to the browser, which races to open the
//! SSE stream while the pipeline is already emitting) receives the
//! buffered events first and then transitions to the live broadcast
//! — so the HTTP client can always reconstruct at least the most
//! recent window of the run regardless of when it connects.
//!
//! `broadcast` fits AG-UI's fan-out model — a single run can have
//! multiple live consumers (the originating HTTP client plus, say, an
//! ops dashboard) — and drops the oldest messages rather than blocking
//! the producer if a consumer lags, which matches the pipeline's
//! no-back-pressure guarantee.

use crate::constants::network_config::{AGUI_RUN_REPLAY_BUFFER_SIZE, SSE_BROADCAST_CHANNEL_SIZE};
use crate::models::TenantId;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use uuid::Uuid;

/// Identity of the user who owns an AG-UI run. Set at registration
/// time; SSE subscribers must match both `user_id` and `tenant_id`
/// before the route returns a stream.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RunOwner {
    /// Authenticated user that issued the turn this run belongs to.
    pub user_id: Uuid,
    /// Tenant the run was issued under. Must match the caller's
    /// tenant context so cross-tenant reads are impossible even when
    /// the same `user_id` exists in multiple tenants.
    pub tenant_id: TenantId,
}

impl RunOwner {
    /// Construct an owner record.
    #[must_use]
    pub const fn new(user_id: Uuid, tenant_id: TenantId) -> Self {
        Self { user_id, tenant_id }
    }

    /// `true` when `caller` matches this owner on both user and tenant.
    #[must_use]
    pub fn authorizes(&self, caller: Self) -> bool {
        self.user_id == caller.user_id && self.tenant_id == caller.tenant_id
    }
}

/// Outcome of [`RunRegistry::authorize_and_subscribe`].
///
/// A single combined operation so the owner check and the subscribe
/// happen under one [`DashMap`] entry borrow — eliminates the
/// time-of-check / time-of-use window between separate `owner_of`
/// and `subscribe` calls. Without this, a concurrent
/// `unregister` + `register` with a different owner could let a
/// caller attach to a different user's slot in between.
pub enum AuthorizedSubscribe {
    /// Run is registered and the caller owns it.
    Ok(RunSubscription),
    /// Run is registered but owned by someone else.
    Forbidden,
    /// Run is not (or no longer) registered.
    NotFound,
}

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
/// Returned from [`RunRegistry::subscribe`] so the HTTP handler can
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

/// Internal entry per registered run: the broadcast sender, the
/// bounded replay buffer, and the owner identity used for auth
/// checks at subscribe time.
struct RunSlot {
    sender: broadcast::Sender<String>,
    owner: RunOwner,
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

    /// Register a run owned by `owner` and return its broadcast sender.
    ///
    /// Idempotent — if a run with the same `run_id` already exists
    /// with the same owner, the existing sender is returned so
    /// late-arriving subscribers can join an in-flight run. When an
    /// existing registration has a *different* owner, the call logs
    /// and replaces the slot; the old subscribers observe the channel
    /// closing.
    pub fn register(&self, run_id: &str, owner: RunOwner) -> broadcast::Sender<String> {
        if let Some(existing) = self.inner.channels.get(run_id) {
            if existing.value().owner == owner {
                return existing.value().sender.clone();
            }
            drop(existing);
            warn!(run_id = %run_id, "AG-UI run re-registered with new owner");
        }
        let (tx, _rx) = broadcast::channel(SSE_BROADCAST_CHANNEL_SIZE);
        self.inner.channels.insert(
            run_id.to_owned(),
            RunSlot {
                sender: tx.clone(),
                owner,
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
    pub fn register_scoped(&self, run_id: &str, owner: RunOwner) -> RunScope {
        let sender = self.register(run_id, owner);
        RunScope {
            registry: self.clone(),
            run_id: run_id.to_owned(),
            sender,
        }
    }

    /// Return the owner of a registered run, or `None` when the run
    /// has not yet started or has already been unregistered.
    #[must_use]
    pub fn owner_of(&self, run_id: &str) -> Option<RunOwner> {
        self.inner.channels.get(run_id).map(|entry| entry.owner)
    }

    /// Atomically authorize a caller and produce a subscription.
    ///
    /// Holds the [`DashMap`] entry guard across both the owner-match
    /// and the `subscribe` call so a concurrent `unregister` +
    /// `register` with a different owner cannot interleave between
    /// the two and hand the caller a stream against a slot they do
    /// not own. SSE route handlers MUST use this entry point rather
    /// than chaining [`Self::owner_of`] + the lower-level
    /// [`Self::subscribe_self`].
    #[must_use]
    pub fn authorize_and_subscribe(&self, run_id: &str, caller: RunOwner) -> AuthorizedSubscribe {
        let Some(entry) = self.inner.channels.get(run_id) else {
            return AuthorizedSubscribe::NotFound;
        };
        let slot = entry.value();
        if !slot.owner.authorizes(caller) {
            return AuthorizedSubscribe::Forbidden;
        }
        AuthorizedSubscribe::Ok(snapshot(slot))
    }

    /// Subscribe without any authorization check.
    ///
    /// Identity-agnostic so the pipeline can attach to its own runs
    /// (the producer half of the broadcast already proves identity);
    /// network handlers MUST use [`Self::authorize_and_subscribe`]
    /// instead.
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
