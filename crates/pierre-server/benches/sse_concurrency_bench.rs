// ABOUTME: Criterion benchmarks comparing DashMap vs Arc<RwLock<HashMap>> for SSE-like concurrent access
// ABOUTME: Measures insert/get/remove latency under varying concurrency levels (1–64 tasks)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Benchmarks proving that `DashMap` outperforms `Arc<RwLock<HashMap>>` for
//! concurrent stream management patterns identical to the SSE manager.
//!
//! Each benchmark simulates N concurrent tasks performing insert → get → remove
//! cycles on a shared map, mirroring the lifecycle of an SSE connection.

#![allow(
    clippy::missing_docs_in_private_items,
    clippy::unwrap_used,
    missing_docs
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;

/// Simulated connection metadata (mirrors `ConnectionMetadata` size)
#[derive(Clone, Debug)]
struct FakeMetadata {
    created_at: u64,
    last_activity: u64,
    kind: u8,
}

// ---------------------------------------------------------------------------
// Arc<RwLock<HashMap>> implementation (the "before" pattern)
// ---------------------------------------------------------------------------

async fn rwlock_insert_get_remove(map: &Arc<RwLock<HashMap<String, FakeMetadata>>>, key: String) {
    let meta = FakeMetadata {
        created_at: 1,
        last_activity: 2,
        kind: 0,
    };

    // insert
    map.write().await.insert(key.clone(), meta);

    // read
    {
        let guard = map.read().await;
        let _ = black_box(guard.get(&key));
    }

    // update last_activity (write lock again)
    {
        let mut guard = map.write().await;
        if let Some(m) = guard.get_mut(&key) {
            m.last_activity = 99;
        }
    }

    // remove
    map.write().await.remove(&key);
}

// ---------------------------------------------------------------------------
// DashMap implementation (the "after" pattern)
// ---------------------------------------------------------------------------

fn dashmap_insert_get_remove(map: &Arc<DashMap<String, FakeMetadata>>, key: &str) {
    let meta = FakeMetadata {
        created_at: 1,
        last_activity: 2,
        kind: 0,
    };

    // insert — no global lock
    map.insert(key.to_owned(), meta);

    // read — shard-level lock only
    {
        let _ = black_box(map.get(key));
    }

    // update — shard-level lock only
    if let Some(mut entry) = map.get_mut(key) {
        entry.last_activity = 99;
    }

    // remove
    map.remove(key);
}

// ---------------------------------------------------------------------------
// Benchmark: concurrent insert/get/remove lifecycle
// ---------------------------------------------------------------------------

fn bench_concurrent_lifecycle(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("sse_concurrent_lifecycle");

    for num_tasks in [1, 4, 8, 16, 32, 64] {
        group.throughput(Throughput::Elements(num_tasks));

        // -- RwLock<HashMap> --
        group.bench_with_input(
            BenchmarkId::new("RwLock_HashMap", num_tasks),
            &num_tasks,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
                        let map: Arc<RwLock<HashMap<String, FakeMetadata>>> =
                            Arc::new(RwLock::new(HashMap::new()));

                        let mut handles = Vec::with_capacity(n as usize);
                        for i in 0..n {
                            let m = Arc::clone(&map);
                            let key = format!("session-{i}");
                            handles.push(tokio::spawn(async move {
                                rwlock_insert_get_remove(&m, key).await;
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                });
            },
        );

        // -- DashMap --
        group.bench_with_input(
            BenchmarkId::new("DashMap", num_tasks),
            &num_tasks,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
                        let map: Arc<DashMap<String, FakeMetadata>> = Arc::new(DashMap::new());

                        let mut handles = Vec::with_capacity(n as usize);
                        for i in 0..n {
                            let m = Arc::clone(&map);
                            let key = format!("session-{i}");
                            handles.push(tokio::spawn(async move {
                                dashmap_insert_get_remove(&m, &key);
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: read-heavy contention (95% reads, 5% writes)
// ---------------------------------------------------------------------------

fn bench_read_heavy(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("sse_read_heavy");
    let num_keys: u64 = 200;
    let ops_per_task: u64 = 100;
    let num_tasks: u64 = 16;

    group.throughput(Throughput::Elements(num_tasks * ops_per_task));

    // Pre-populate a map with num_keys entries
    let seed_data: Vec<(String, FakeMetadata)> = (0..num_keys)
        .map(|i| {
            (
                format!("session-{i}"),
                FakeMetadata {
                    created_at: i,
                    last_activity: i,
                    kind: 0,
                },
            )
        })
        .collect();

    // -- RwLock<HashMap> --
    group.bench_function("RwLock_HashMap", |b| {
        b.iter(|| {
            rt.block_on(async {
                let map: Arc<RwLock<HashMap<String, FakeMetadata>>> =
                    Arc::new(RwLock::new(seed_data.iter().cloned().collect()));

                let mut handles = Vec::with_capacity(num_tasks as usize);
                for t in 0..num_tasks {
                    let m = Arc::clone(&map);
                    handles.push(tokio::spawn(async move {
                        for op in 0..ops_per_task {
                            let key = format!("session-{}", (t * ops_per_task + op) % num_keys);
                            if op % 20 == 0 {
                                // 5% writes
                                let mut guard = m.write().await;
                                if let Some(meta) = guard.get_mut(&key) {
                                    meta.last_activity = black_box(op);
                                }
                            } else {
                                // 95% reads
                                let guard = m.read().await;
                                let _ = black_box(guard.get(&key));
                            }
                        }
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            });
        });
    });

    // -- DashMap --
    group.bench_function("DashMap", |b| {
        b.iter(|| {
            rt.block_on(async {
                let map: Arc<DashMap<String, FakeMetadata>> = Arc::new(DashMap::new());
                for (k, v) in &seed_data {
                    map.insert(k.clone(), v.clone());
                }

                let mut handles = Vec::with_capacity(num_tasks as usize);
                for t in 0..num_tasks {
                    let m = Arc::clone(&map);
                    handles.push(tokio::spawn(async move {
                        for op in 0..ops_per_task {
                            let key = format!("session-{}", (t * ops_per_task + op) % num_keys);
                            if op % 20 == 0 {
                                // 5% writes
                                if let Some(mut entry) = m.get_mut(&key) {
                                    entry.last_activity = black_box(op);
                                }
                            } else {
                                // 95% reads
                                let _ = black_box(m.get(&key));
                            }
                        }
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            });
        });
    });

    group.finish();
}

criterion_group!(benches, bench_concurrent_lifecycle, bench_read_heavy);
criterion_main!(benches);
