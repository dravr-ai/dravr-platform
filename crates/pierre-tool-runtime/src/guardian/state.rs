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
//! [`TurnKey`] = `(tenant_id, turn_id)` rather than on the executor, because
//! one turn runs on more than one: an ACP agent's tools arrive through the
//! tool host's surface and its own executor, while the pipeline's prefetches
//! and the text loop use another. Only a shared store keyed by a stable turn
//! token accumulates across them. The key is tenant-scoped so two tenants never
//! share a taint bucket.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use uuid::Uuid;

use super::DenyReason;
use crate::context::GUARDIAN_TAINT_ORIGIN;
use crate::security::SecurityLabels;

/// How long a turn's state is retained before it is eligible for eviction.
/// Generous relative to a turn (seconds-to-minutes) so accumulation is never
/// lost mid-turn, tight enough that abandoned turns do not pile up.
pub(crate) const TURN_TTL: Duration = Duration::from_mins(30);

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

/// Hard cap on retained unconsumed turn blocks (#10). The tool loop takes a
/// turn's block when it returns, so an entry outlives its turn only when the
/// loop failed or a stage dispatched after it; blocks are rare enough that this
/// is unreachable in practice, and overflow drops the entries past [`TURN_TTL`]
/// first, every entry only if that was not enough.
const DENIAL_CAP: usize = 10_000;

/// Identifies one logical turn for taint accumulation.
///
/// `turn_id` is the utterance's turn id (every path of a chat turn, an ACP
/// agent's tool-host session included), or a per-request nonce (one-shot
/// MCP-direct / A2A calls — which then accumulate nothing, correctly, since they
/// are independent calls). `tenant_id` scopes the key for tenant isolation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnKey {
    tenant_id: Option<Uuid>,
    turn_id: String,
}

impl TurnKey {
    /// Build a turn key from the resolved tenant and a stable turn token.
    #[must_use]
    pub fn new(tenant_id: Option<Uuid>, turn_id: impl Into<String>) -> Self {
        Self {
            tenant_id,
            turn_id: turn_id.into(),
        }
    }
}

/// Who decided a taint-bearing read should run.
///
/// Prefetched activities really are untrusted text in the model's context, so
/// they taint exactly as a read the model asked for does; this only keeps the two
/// apart in the telemetry, so the base rate behind arming `TaintedDestructive`
/// can tell "the model read this and then acted" from "the platform grounded the
/// turn and the model acted".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaintOrigin {
    /// The model asked for the read (the default).
    #[default]
    Model,
    /// The chat pipeline ran it before the model said anything.
    Prefetch,
}

impl TaintOrigin {
    /// The origin of the dispatch running on this task.
    pub(crate) fn current() -> Self {
        GUARDIAN_TAINT_ORIGIN
            .try_with(|origin| *origin)
            .unwrap_or_default()
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
    /// Whether a read the platform prefetched tainted the turn.
    tainted_by_prefetch: bool,
    /// Whether a read the model asked for tainted the turn.
    tainted_by_model: bool,
    /// Reads that succeeded this turn, by [`ReadFingerprint`], and whether
    /// their one repeat has been answered with a pointer back to them.
    reads: HashMap<ReadFingerprint, ReadSeen>,
    /// Whether enforce mode has already denied or parked a call this turn. Once
    /// it has, the reply the athlete gets is about that call, so nothing further
    /// may commit behind it ([`super::DenyReason::TurnBlocked`]).
    blocked: bool,
}

/// A read-only call's tool name and canonical arguments, hashed. Only the
/// hash is kept, so the store never holds an argument value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReadFingerprint(u64);

impl ReadFingerprint {
    /// Fingerprint `tool_name` called with `args`, independent of the order
    /// the arguments' keys were written in.
    #[must_use]
    pub fn of(tool_name: &str, args: &Value) -> Self {
        let mut hasher = DefaultHasher::new();
        tool_name.hash(&mut hasher);
        hash_canonical(args, &mut hasher);
        Self(hasher.finish())
    }
}

/// Hash `value` with every object's keys in sorted order.
fn hash_canonical(value: &Value, hasher: &mut DefaultHasher) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                key.hash(hasher);
                hash_canonical(&map[key.as_str()], hasher);
            }
        }
        Value::Array(items) => {
            items.len().hash(hasher);
            for item in items {
                hash_canonical(item, hasher);
            }
        }
        other => other.to_string().hash(hasher),
    }
}

/// What a turn remembers of a read it served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadSeen {
    /// Served once; an identical call now gets a pointer back to it.
    Served,
    /// Its repeat was answered with that pointer; the next identical call
    /// runs, because a model that asks a third time no longer has it.
    Pointed,
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

    /// Whether enforce mode has already denied or parked a call this turn.
    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// Tool names that introduced taint this turn (for deny reasons / logs).
    #[must_use]
    pub fn taint_sources(&self) -> &[String] {
        &self.sources
    }

    /// Who put the turn's taint there, as logged: `prefetch`, `model` or `both`.
    /// `None` for an untainted turn.
    #[must_use]
    pub const fn taint_origin(&self) -> Option<&'static str> {
        match (self.tainted_by_prefetch, self.tainted_by_model) {
            (true, true) => Some("both"),
            (true, false) => Some("prefetch"),
            (false, true) => Some("model"),
            (false, false) => None,
        }
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
            match TaintOrigin::current() {
                TaintOrigin::Prefetch => self.tainted_by_prefetch = true,
                TaintOrigin::Model => self.tainted_by_model = true,
            }
            if !self.sources.iter().any(|s| s == tool_name) {
                self.sources.push(tool_name.to_owned());
            }
        }
    }
}

/// Process-wide, tenant-scoped store of per-turn Guardian state.
///
/// Shared via the runtime so every dispatch path (api/cli chat loops and an ACP
/// agent's tool-host surface) reaches the same turn bucket. Entries are evicted
/// by TTL with an opportunistic sweep once the map grows past [`SWEEP_THRESHOLD`], bounding memory without an O(n) sweep on
/// every call.
#[derive(Debug)]
pub struct GuardianTurns {
    inner: Mutex<Store>,
    /// The first enforce-mode block of each real turn, set by the dispatch
    /// chokepoint and taken when the turn's tool loop returns. A block that fires
    /// inside an ACP agent's tool-host session (a separate task) can't surface
    /// back through the subprocess, so the loop reads this to render a
    /// deterministic refusal or confirmation ask instead of letting the model
    /// paraphrase it (#10).
    ///
    /// Keyed by the turn, so overlapping turns of one athlete neither erase nor
    /// take each other's block, and a one-shot MCP or A2A denial, which has no
    /// turn to report to, is never recorded (carnet#1404). Separate lock from
    /// `inner` — it is touched only on the rare block path and the turn
    /// boundaries, never on the hot allow path.
    blocks: Mutex<HashMap<TurnKey, (Instant, HeadlessBlock)>>,
}

/// What the gate makes of a call it decided, applied under the same lock as the
/// decision so a concurrent same-turn call cannot slip between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFate {
    /// The call runs: budget is reserved and its taint folded.
    Runs,
    /// Enforce mode denied or parked it: nothing is reserved, and the turn
    /// stops committing ([`TurnState::is_blocked`]).
    Blocked,
    /// The caller only read the state; nothing changes.
    Unchanged,
}

/// A block the dispatch chokepoint recorded for the turn's tool loop to render
/// deterministically instead of the model's paraphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessBlock {
    /// An enforce-mode denial (rendered as the localized denied reply).
    Denied {
        /// Registry identifier of the denied tool, for the server log.
        tool_name: String,
        /// Why it was denied.
        reason: DenyReason,
    },
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
    /// Whether the read `fingerprint` repeats one served earlier in `key`'s
    /// turn, and should be answered with a pointer back to it instead of being
    /// run again. Answers yes once per read: the call after that runs.
    pub fn repeat_of_served_read(&self, key: &TurnKey, fingerprint: ReadFingerprint) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((_, state)) = guard.map.get_mut(key) else {
            return false;
        };
        match state.reads.get_mut(&fingerprint) {
            Some(seen @ ReadSeen::Served) => {
                *seen = ReadSeen::Pointed;
                true
            }
            _ => false,
        }
    }

    /// Remember that the read `fingerprint` was served in `key`'s turn.
    pub fn record_served_read(&self, key: &TurnKey, fingerprint: ReadFingerprint) {
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = guard
            .map
            .entry(key.clone())
            .or_insert_with(|| (now, TurnState::default()));
        entry.0 = now;
        entry.1.reads.insert(fingerprint, ReadSeen::Served);
    }

    /// Forget every read `key`'s turn served: a write may have changed what
    /// they returned, so an identical read after it runs.
    pub fn forget_served_reads(&self, key: &TurnKey) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, state)) = guard.map.get_mut(key) {
            state.reads.clear();
        }
    }

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

    /// Record an enforce-mode block under `key`, the turn it fired in, so the
    /// turn's tool loop can render one that occurred inside an ACP agent's
    /// tool-host session (#10). The turn's FIRST block stands: it is what the
    /// athlete was asked, and everything after it was refused because of it. A
    /// poisoned lock is skipped — the turn then falls back to the model's
    /// paraphrase, never a panic.
    pub fn record_block(&self, key: &TurnKey, block: HeadlessBlock) {
        if let Ok(mut blocks) = self.blocks.lock() {
            let now = Instant::now();
            // Bound memory. A turn whose loop failed never takes its entry, so
            // past the cap the abandoned ones go first; a dropped pending block
            // just falls back to the model's paraphrase (safe), and the turn
            // still commits nothing further.
            if blocks.len() >= DENIAL_CAP {
                blocks.retain(|_, (at, _)| now.duration_since(*at) < TURN_TTL);
                if blocks.len() >= DENIAL_CAP {
                    blocks.clear();
                }
            }
            blocks.entry(key.clone()).or_insert((now, block));
        }
    }

    /// Consume and return the block recorded under `key`, if any.
    #[must_use]
    pub fn take_block(&self, key: &TurnKey) -> Option<HeadlessBlock> {
        self.blocks.lock().ok()?.remove(key).map(|(_, block)| block)
    }

    /// Atomically evaluate `decide` against the live turn state and, under the
    /// same lock, apply what becomes of the call — so a concurrent same-key
    /// dispatch cannot also pass a cap (the E2 TOCTOU fix), nor run behind a
    /// block it decided a moment before.
    ///
    /// `decide` runs against the current turn state; `fate(&decision)` says what
    /// follows from it ([`CallFate`]). Returns `(decision, reserved)`; call
    /// [`Self::refund`] with the same labels if the execution then fails.
    pub fn decide_and_reserve<D>(
        &self,
        key: &TurnKey,
        tool_name: &str,
        labels: SecurityLabels,
        writes_data: bool,
        decide: impl FnOnce(&TurnState) -> D,
        fate: impl FnOnce(&D) -> CallFate,
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
        let call_fate = fate(&decision);
        let reserved = call_fate == CallFate::Runs;
        if call_fate == CallFate::Blocked {
            entry.1.blocked = true;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn denied(reason: DenyReason) -> HeadlessBlock {
        HeadlessBlock::Denied {
            tool_name: "disconnect_provider".to_owned(),
            reason,
        }
    }

    #[test]
    fn a_turn_keeps_its_first_block_and_overlapping_turns_keep_their_own() {
        let store = GuardianTurns::new();
        let first_turn = TurnKey::new(None, "turn-a");
        let second_turn = TurnKey::new(None, "turn-b");

        store.record_block(&first_turn, denied(DenyReason::TaintedSink));
        store.record_block(&first_turn, denied(DenyReason::BudgetExceeded));
        store.record_block(&second_turn, denied(DenyReason::EgressForbidden));

        assert_eq!(
            store.take_block(&second_turn),
            Some(denied(DenyReason::EgressForbidden)),
            "taking one turn's block leaves the other's"
        );
        assert_eq!(
            store.take_block(&first_turn),
            Some(denied(DenyReason::TaintedSink)),
            "a later block never replaces the one the athlete was asked about"
        );
        assert_eq!(store.take_block(&first_turn), None, "taken once");
    }

    #[test]
    fn unconsumed_blocks_stay_bounded_and_a_new_block_still_lands() {
        let store = GuardianTurns::new();
        for turn in 0..DENIAL_CAP {
            store.record_block(
                &TurnKey::new(None, format!("turn-{turn}")),
                denied(DenyReason::BudgetExceeded),
            );
        }
        let past_the_cap = TurnKey::new(None, "turn-past-the-cap");
        store.record_block(&past_the_cap, denied(DenyReason::TaintedSink));
        assert!(store.blocks.lock().expect("lock").len() <= DENIAL_CAP);
        assert_eq!(
            store.take_block(&past_the_cap),
            Some(denied(DenyReason::TaintedSink))
        );
    }

    #[test]
    fn taint_origin_names_who_decided_the_read() {
        let mut turn = TurnState::default();
        assert_eq!(turn.taint_origin(), None);

        GUARDIAN_TAINT_ORIGIN.sync_scope(TaintOrigin::Prefetch, || {
            turn.add_taint("get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
        });
        assert_eq!(turn.taint_origin(), Some("prefetch"));
        assert!(turn.is_tainted(), "a prefetched read taints like any other");
        assert_eq!(turn.taint_sources(), ["get_activities"]);

        turn.add_taint("get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
        assert_eq!(turn.taint_origin(), Some("both"));

        let mut model_only = TurnState::default();
        model_only.add_taint("list_agents", SecurityLabels::UNTRUSTED_OUTPUT);
        assert_eq!(model_only.taint_origin(), Some("model"));

        GUARDIAN_TAINT_ORIGIN.sync_scope(TaintOrigin::Prefetch, || {
            let mut trusted = TurnState::default();
            trusted.add_taint("set_goal", SecurityLabels::empty());
            assert_eq!(trusted.taint_origin(), None, "no taint, no origin");
        });
    }

    #[test]
    fn only_a_blocked_fate_latches_the_turn() {
        let store = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        let is_blocked = |store: &GuardianTurns| {
            store
                .decide_and_reserve(
                    &key,
                    "peek",
                    SecurityLabels::empty(),
                    false,
                    TurnState::is_blocked,
                    |_| CallFate::Unchanged,
                )
                .0
        };

        store.decide_and_reserve(
            &key,
            "write",
            SecurityLabels::empty(),
            true,
            |_| (),
            |()| CallFate::Runs,
        );
        assert!(!is_blocked(&store));

        let ((), reserved) = store.decide_and_reserve(
            &key,
            "delete",
            SecurityLabels::IRREVERSIBLE,
            false,
            |_| (),
            |()| CallFate::Blocked,
        );
        assert!(!reserved, "a blocked call reserves nothing");
        assert!(is_blocked(&store));
    }
}
