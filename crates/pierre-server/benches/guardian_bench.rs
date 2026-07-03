// ABOUTME: Criterion benchmarks for the Guardian dispatch gate — per-call overhead and shared-store lock contention.
// ABOUTME: Answers "does enforcing the Guardian on every tool call slow us down?" with numbers, not vibes.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Guardian runs on EVERY tool dispatch (`guardian_gate` → `decide_and_reserve`),
//! and its per-turn store is a single process-wide `Mutex<..>` shared by every
//! tenant. Two questions matter for latency:
//!
//! 1. **Uncontended per-call cost** — is a single `guardian_gate` cheap enough to
//!    be lost in the noise of a real tool call (DB / provider / LLM work)?
//! 2. **Contended cost** — since the store is one `Mutex`, do concurrent dispatches
//!    across tenants serialize on it? Throughput that stays flat as concurrency
//!    rises would mean the lock is the bottleneck.
//!
//! Run: `cargo bench -p pierre_mcp_server --bench guardian_bench`

#![allow(
    clippy::missing_docs_in_private_items,
    clippy::unwrap_used,
    missing_docs
)]

use std::sync::Arc;
use std::thread;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pierre_tool_runtime::guardian::{
    guardian_gate, Guardian, GuardianPolicy, GuardianTurns, TurnKey,
};
use pierre_tool_runtime::SecurityLabels;

/// The production default policy (enforce, budgets 1/50) — what actually ships.
fn policy() -> GuardianPolicy {
    GuardianPolicy::default()
}

/// One representative dispatch: an `UNTRUSTED_OUTPUT` read (the common taint
/// source), which exercises the decide + atomic taint-fold + reserve path.
fn one_gate(guardian: &Guardian, turns: &GuardianTurns, key: &TurnKey) {
    black_box(guardian_gate(
        black_box(guardian),
        black_box(turns),
        black_box(key),
        black_box("get_activities"),
        black_box(SecurityLabels::UNTRUSTED_OUTPUT),
        black_box(false),
        black_box(None),
    ));
}

/// Uncontended: a single thread hammering the gate. Cycles over a small ring of
/// turn keys so the store holds live entries (realistic — not an empty map).
fn bench_uncontended(c: &mut Criterion) {
    let guardian = Guardian::new(policy());
    let turns = GuardianTurns::new();
    let keys: Vec<TurnKey> = (0..256)
        .map(|i| TurnKey::new(None, format!("turn-{i}")))
        .collect();

    c.bench_function("guardian_gate/uncontended", |b| {
        let mut i = 0usize;
        b.iter(|| {
            one_gate(&guardian, &turns, &keys[i % keys.len()]);
            i += 1;
        });
    });
}

/// Contended: N threads sharing ONE `GuardianTurns` (the process-wide store),
/// each doing a fixed batch of gate calls on its own keys. If the `Mutex`
/// serialized traffic, elements/sec would stay flat as N grows.
fn bench_contended(c: &mut Criterion) {
    const OPS_PER_THREAD: u64 = 2_000;
    let mut group = c.benchmark_group("guardian_gate/contended");

    for threads in [1usize, 4, 16, 64] {
        group.throughput(Throughput::Elements(OPS_PER_THREAD * threads as u64));
        group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, &n| {
            let guardian = Arc::new(Guardian::new(policy()));
            let turns = Arc::new(GuardianTurns::new());
            b.iter(|| {
                thread::scope(|s| {
                    for t in 0..n {
                        let guardian = Arc::clone(&guardian);
                        let turns = Arc::clone(&turns);
                        s.spawn(move || {
                            for j in 0..OPS_PER_THREAD {
                                let key = TurnKey::new(None, format!("t{t}-{j}"));
                                one_gate(&guardian, &turns, &key);
                            }
                        });
                    }
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_uncontended, bench_contended);
criterion_main!(benches);
