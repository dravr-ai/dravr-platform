// ABOUTME: Guardian per-turn taint + budget state and the tenant-scoped shared store keyed by turn.
// ABOUTME: Accumulates across ReAct iterations so an untrusted source gates a later consequential sink.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-turn Guardian state.
//!
//! A "turn" is one logical chat interaction. Tool calls within a turn execute
//! sequentially (tool execution within a turn is not concurrent), so one [`TurnState`]
//! accumulates taint and consequential-action counts across the loop's
//! iterations: an `UNTRUSTED_OUTPUT` source read in iteration 1 is visible when
//! a sink is checked in iteration 3.
//!
//! State lives in a process-wide [`GuardianTurns`] store keyed by
//! [`TurnKey`] = `(tenant_id, turn_token)` rather than on the executor, because
//! the production Copilot-headless loopback builds a fresh executor per `/mcp`
//! tool call — only a shared store keyed by a stable turn token accumulates
//! across that path. The key is tenant-scoped so two tenants never share a
//! taint bucket.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::DenyReason;
use crate::security::SecurityLabels;

/// How long a turn's state is retained before it is eligible for eviction.
/// Generous relative to a turn (seconds-to-minutes) so accumulation is never
/// lost mid-turn, tight enough that abandoned turns do not pile up.
const TURN_TTL: Duration = Duration::from_mins(30);

/// Soft cap on retained turns; a stale-sweep runs once the map grows past it.
const SWEEP_THRESHOLD: usize = 50_000;

/// Hard cap the sweep drops the map to (by evicting the oldest) when TTL alone
/// does not bring it under, so a burst of fresh high-cardinality keys cannot
/// grow the store without bound or force an O(n) sweep on every later call.
const HARD_CAP: usize = 40_000;

/// Minimum interval between time-driven TTL sweeps. Below [`SWEEP_THRESHOLD`]
/// the size trigger never fires, so this cadence is what actually enforces
/// [`TURN_TTL`] on a normal-sized store — without paying an O(n) `retain` on
/// every call. Comfortably tighter than `TURN_TTL` so stale turns are reclaimed
/// within one interval of expiring.
const SWEEP_MIN_INTERVAL: Duration = Duration::from_mins(5);

/// Hard cap on retained unconsumed headless denial-flag entries (#10). Only the
/// headless loop consumes them, so a chat/direct block for a user who never runs
/// a headless turn would otherwise linger; blocks are rare enough that this is
/// unreachable in practice, and overflow simply drops the abandoned entries.
const DENIAL_CAP: usize = 10_000;

/// Identifies one logical turn for taint accumulation.
///
/// `turn_token` is the Pierre conversation id (chat loops), the ACP `turn_id`
/// claim (Copilot headless loopback), or a per-request nonce (one-shot
/// MCP-direct / A2A calls — which then accumulate nothing, correctly, since they
/// are independent calls). `tenant_id` scopes the key for tenant isolation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnKey {
    tenant_id: Option<Uuid>,
    turn_token: String,
}

impl TurnKey {
    /// Build a turn key from the resolved tenant and a stable turn token.
    #[must_use]
    pub fn new(tenant_id: Option<Uuid>, turn_token: impl Into<String>) -> Self {
        Self {
            tenant_id,
            turn_token: turn_token.into(),
        }
    }
}

/// Mutable taint + budget accumulator for one turn.
#[derive(Debug, Clone, Default)]
pub struct TurnState {
    /// Union of taint-bearing labels (currently `UNTRUSTED_OUTPUT`) observed
    /// from successfully executed tools this turn.
    taint: SecurityLabels,
    /// Successful `WRITES_DATA` executions this turn (budget input).
    writes: u16,
    /// Successful `IRREVERSIBLE` executions this turn (budget input).
    destructive: u16,
    /// Tool names that introduced taint — for human-readable deny reasons and
    /// telemetry. Never contains arguments or PII, only registered tool names.
    sources: Vec<String>,
}

impl TurnState {
    /// Whether an untrusted-output source has executed this turn.
    #[must_use]
    pub const fn is_tainted(&self) -> bool {
        self.taint.is_untrusted_output()
    }

    /// Count of consequential irreversible actions taken this turn.
    #[must_use]
    pub const fn destructive_count(&self) -> u16 {
        self.destructive
    }

    /// Count of data-writing actions taken this turn.
    #[must_use]
    pub const fn write_count(&self) -> u16 {
        self.writes
    }

    /// Tool names that introduced taint this turn (for deny reasons / logs).
    #[must_use]
    pub fn taint_sources(&self) -> &[String] {
        &self.sources
    }

    /// Fold a successfully executed tool into the turn's taint + budget counts.
    ///
    /// `writes_data` is the tronc `WRITES_DATA` capability (a tool can be a
    /// write sink without any Guardian label); `labels` is the Guardian
    /// security class. Only *successful* executions move counters, so a denied
    /// or failed call never consumes budget or taints the turn. Retained for
    /// test setup; the live chokepoint uses [`Self::reserve`] + [`Self::add_taint`].
    pub fn record_success(&mut self, tool_name: &str, labels: SecurityLabels, writes_data: bool) {
        self.add_taint(tool_name, labels);
        self.reserve(labels, writes_data);
    }

    /// Reserve budget for a consequential action *before* it executes, so a
    /// concurrent same-key dispatch cannot also pass the cap (the E2 TOCTOU fix).
    /// Refund via [`Self::refund`] if the execution then fails/denies.
    pub fn reserve(&mut self, labels: SecurityLabels, writes_data: bool) {
        if labels.is_irreversible() {
            self.destructive = self.destructive.saturating_add(1);
        }
        if writes_data {
            self.writes = self.writes.saturating_add(1);
        }
    }

    /// Release a previously-[`reserve`](Self::reserve)d budget slot when the
    /// execution failed (so only successful consequential actions consume budget).
    pub fn refund(&mut self, labels: SecurityLabels, writes_data: bool) {
        if labels.is_irreversible() {
            self.destructive = self.destructive.saturating_sub(1);
        }
        if writes_data {
            self.writes = self.writes.saturating_sub(1);
        }
    }

    /// Fold an `UNTRUSTED_OUTPUT` execution into the turn's taint. Called on any
    /// execution — success **or** error — because a failed untrusted-source tool
    /// still surfaces its content to the LLM (the S1 error-path taint escape).
    pub fn add_taint(&mut self, tool_name: &str, labels: SecurityLabels) {
        if labels.is_untrusted_output() {
            self.taint |= SecurityLabels::UNTRUSTED_OUTPUT;
            if !self.sources.iter().any(|s| s == tool_name) {
                self.sources.push(tool_name.to_owned());
            }
        }
    }
}

/// Process-wide, tenant-scoped store of per-turn Guardian state.
///
/// Shared via the runtime so every dispatch path (api/cli chat loops and the
/// per-request Copilot-headless loopback executors) reaches the same turn
/// bucket. Entries are evicted by TTL with an opportunistic sweep once the map
/// grows past [`SWEEP_THRESHOLD`], bounding memory without an O(n) sweep on
/// every call.
#[derive(Debug)]
pub struct GuardianTurns {
    inner: Mutex<Store>,
    /// Most-recent enforce-mode block per `(tenant, user)` headless key, set by
    /// the dispatch chokepoint and consumed by the headless loop. A block that
    /// fires inside the Copilot ACP subprocess's `/mcp` loopback (a separate HTTP
    /// task) can't surface back through the subprocess, so the loop reads this to
    /// render a deterministic refusal or confirmation ask instead of letting the
    /// model paraphrase it (#10). Separate lock from `inner` — it is touched only
    /// on the rare block path and the headless turn boundaries, never on the hot
    /// allow path.
    blocks: Mutex<HashMap<TurnKey, HeadlessBlock>>,
}

/// A block the dispatch chokepoint recorded for the headless loop to render
/// deterministically instead of the model's paraphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessBlock {
    /// An enforce-mode denial (rendered as the localized denied reply).
    Denied(DenyReason),
    /// A parked confirm-required call (rendered as the confirmation prompt).
    ConfirmRequired {
        /// Registry identifier of the parked tool.
        tool_name: String,
        /// Opaque claim token the user resolves with `/confirm`·`/deny`.
        pending_id: String,
    },
}

/// Locked interior: the per-turn map plus the last stale-sweep time.
///
/// Tracking `last_sweep` lets TTL eviction fire on a time cadence
/// ([`SWEEP_MIN_INTERVAL`]) even while the map stays below [`SWEEP_THRESHOLD`] —
/// otherwise finished turns would linger for the whole process lifetime, far
/// past [`TURN_TTL`].
#[derive(Debug)]
struct Store {
    map: HashMap<TurnKey, (Instant, TurnState)>,
    last_sweep: Instant,
}

impl GuardianTurns {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Store {
                map: HashMap::new(),
                last_sweep: Instant::now(),
            }),
            blocks: Mutex::new(HashMap::new()),
        }
    }

    /// Record an enforce-mode block under `key` (a `(tenant, user)` headless
    /// key), so the headless loop can detect one that occurred inside the ACP
    /// subprocess's loopback (#10). Overwrites any prior unconsumed block for the
    /// key. A poisoned lock is skipped — the turn then falls back to the model's
    /// paraphrase (today's behaviour), never a panic.
    pub fn record_block(&self, key: &TurnKey, block: HeadlessBlock) {
        if let Ok(mut blocks) = self.blocks.lock() {
            // Bound memory: only the headless loop consumes/clears entries, so a
            // chat/direct block whose user never runs a headless turn would linger.
            // Blocks are rare, so this cap is effectively unreachable; if it is hit
            // the entries are abandoned anyway, and a dropped pending block just
            // falls back to the model's paraphrase (safe).
            if blocks.len() >= DENIAL_CAP {
                blocks.clear();
            }
            blocks.insert(key.clone(), block);
        }
    }

    /// Consume and return the block recorded under `key`, if any.
    #[must_use]
    pub fn take_block(&self, key: &TurnKey) -> Option<HeadlessBlock> {
        self.blocks.lock().ok()?.remove(key)
    }

    /// Drop any stale block for `key` before a headless turn starts, so only a
    /// block raised during *this* turn's subprocess is later taken.
    pub fn clear_block(&self, key: &TurnKey) {
        if let Ok(mut blocks) = self.blocks.lock() {
            blocks.remove(key);
        }
    }

    /// Atomically evaluate `decide` against the live turn state and, under the
    /// same lock, reserve the action's budget if it will execute — so a
    /// concurrent same-key dispatch cannot also pass a cap (the E2 TOCTOU fix).
    ///
    /// `decide` runs against the current turn state; `will_execute(&decision)`
    /// says whether to reserve (true for an allow, and for a deny in a
    /// non-blocking mode that executes anyway). Returns `(decision, reserved)`;
    /// call [`Self::refund`] with the same labels if the execution then fails.
    pub fn decide_and_reserve<D>(
        &self,
        key: &TurnKey,
        tool_name: &str,
        labels: SecurityLabels,
        writes_data: bool,
        decide: impl FnOnce(&TurnState) -> D,
        will_execute: impl FnOnce(&D) -> bool,
    ) -> (D, bool) {
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        Self::prune_locked(&mut guard, now);
        let entry = guard
            .map
            .entry(key.clone())
            .or_insert_with(|| (now, TurnState::default()));
        entry.0 = now;
        let decision = decide(&entry.1);
        let reserved = will_execute(&decision);
        if reserved {
            entry.1.reserve(labels, writes_data);
            // Fold taint atomically with the decision (S1 + E2). The taint label
            // is static, so an `UNTRUSTED_OUTPUT` source taints the turn the
            // instant it is cleared to run — before its body executes — closing
            // the window where a concurrent same-turn sink decides against a
            // pre-taint snapshot. `add_taint` no-ops for non-untrusted tools, and
            // taint is never refunded (a source that then errors still surfaced
            // its content to the LLM). Because reservation happens iff the tool
            // will execute, this taints exactly when a post-execution fold would.
            entry.1.add_taint(tool_name, labels);
        }
        (decision, reserved)
    }

    /// Release a budget slot reserved by [`Self::decide_and_reserve`] when the
    /// tool then failed, so only successful consequential actions consume budget.
    pub fn refund(&self, key: &TurnKey, labels: SecurityLabels, writes_data: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, state)) = guard.map.get_mut(key) {
            state.refund(labels, writes_data);
        }
    }

    /// Fold an `UNTRUSTED_OUTPUT` execution's taint into the turn — on success
    /// **or** error (S1: a failed untrusted-source tool still feeds content to
    /// the LLM). No-op for non-untrusted tools.
    pub fn record_taint(&self, key: &TurnKey, tool_name: &str, labels: SecurityLabels) {
        if !labels.is_untrusted_output() {
            return;
        }
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = guard
            .map
            .entry(key.clone())
            .or_insert_with(|| (now, TurnState::default()));
        entry.0 = now;
        entry.1.add_taint(tool_name, labels);
    }

    /// Fold a successful execution into `key`'s state, creating it if new.
    ///
    /// Retained for callers/tests that record a completed success in one shot;
    /// the live chokepoint uses [`Self::decide_and_reserve`] + [`Self::record_taint`].
    pub fn record_success(
        &self,
        key: &TurnKey,
        tool_name: &str,
        labels: SecurityLabels,
        writes_data: bool,
    ) {
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        Self::prune_locked(&mut guard, now);
        let entry = guard
            .map
            .entry(key.clone())
            .or_insert_with(|| (now, TurnState::default()));
        entry.0 = now;
        entry.1.record_success(tool_name, labels, writes_data);
    }

    /// Bound the store (S7): evict TTL-stale turns, then hard-cap the survivors
    /// by evicting the oldest, so a burst of fresh high-cardinality keys cannot
    /// grow it without bound or force an O(n) sweep on every later call. Runs
    /// only past [`SWEEP_THRESHOLD`] and drops below [`HARD_CAP`], so it will
    /// not re-sweep until the map grows again.
    fn prune_locked(store: &mut Store, now: Instant) {
        // Sweep on a time cadence (so TTL is enforced even below the size
        // threshold) OR when the map grows past the soft cap. Below both, skip —
        // no O(n) `retain` on the hot path.
        let due_by_time = now.duration_since(store.last_sweep) >= SWEEP_MIN_INTERVAL;
        let due_by_size = store.map.len() > SWEEP_THRESHOLD;
        if !due_by_time && !due_by_size {
            return;
        }
        store.last_sweep = now;
        store
            .map
            .retain(|_, (seen, _)| now.duration_since(*seen) < TURN_TTL);
        if store.map.len() > HARD_CAP {
            let mut by_age: Vec<(TurnKey, Instant)> = store
                .map
                .iter()
                .map(|(k, (seen, _))| (k.clone(), *seen))
                .collect();
            by_age.sort_by_key(|&(_, seen)| seen);
            let excess = store.map.len() - HARD_CAP;
            for (k, _) in by_age.into_iter().take(excess) {
                store.map.remove(&k);
            }
        }
    }
}

impl Default for GuardianTurns {
    fn default() -> Self {
        Self::new()
    }
}
