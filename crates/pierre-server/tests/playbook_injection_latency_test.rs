// ABOUTME: Latency harness for the always-on playbook prompt injection (inject_playbooks hot path)
// ABOUTME: Times the exact repo-call sequence per turn across empty/cold-start, populated, and heavy scenarios
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Playbook injection latency harness.
//!
//! The playbook subsystem injects learned playbooks + cold-start archetype priors
//! into the coach system prompt on **every** chat turn, inline before the LLM
//! call, with no per-tenant kill switch. This harness times the exact repository
//! sequence `inject_playbooks` performs so we can quantify that per-turn cost and
//! decide whether a guard is warranted.
//!
//! It mirrors `pierre_chat_pipeline::stages::memory::inject_playbooks` (which is
//! behind an optional feature) by exercising the same repo calls + render in the
//! same order:
//! 1. `list_playbooks` (+ Rust confidence sort)
//! 2. `render_playbooks_block`
//! 3. cold-start only (no playbooks): `get_cached_activities` → derive sport slugs
//! 4. `list_archetype_priors_for_keys` (one batched query)
//! 5. `render_archetype_block`
//!
//! Numbers are on in-memory `SQLite` (the CPU + query-planning floor); a real
//! Postgres backend adds a network round-trip per query (2–3 queries per turn).
//! The assertion is catastrophic-only (guards a 100×+ regression without flaking
//! on shared CI runners); the real signal is the printed per-phase breakdown.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::repositories::{ArchetypePriorUpsert, RecordedOutcome};
use pierre_database::RepositoryRegistry;
use pierre_memory::playbooks::{
    ArchetypePrior, Band, Intervention, InterventionKind, OutcomeLabel, OutcomeMetric, Playbook,
    TriggerKind, TriggerPattern,
};
use pierre_services::playbook_render::{render_archetype_block, render_playbooks_block};
use serde_json::Value;
use uuid::Uuid;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

/// Iterations per scenario — enough for a stable average on sub-millisecond ops.
const ITERS: u32 = 300;
/// Retrieval limits mirror the pipeline constants.
const PLAYBOOK_INJECT_LIMIT: i64 = 20;
const ARCHETYPE_FETCH_LIMIT: i64 = 20;

/// Per-phase accumulated timings across all iterations of one scenario.
#[derive(Default)]
struct Phase {
    list_playbooks: Duration,
    activity_read: Duration,
    priors: Duration,
    render: Duration,
}

impl Phase {
    fn total(&self) -> Duration {
        self.list_playbooks + self.activity_read + self.priors + self.render
    }
    fn report(&self, name: &str, backend_note: &str) {
        let per = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(ITERS);
        eprintln!("\n── {name} ({backend_note}, avg over {ITERS} iters) ──");
        eprintln!(
            "  list_playbooks           : {:.4} ms",
            per(self.list_playbooks)
        );
        eprintln!(
            "  get_cached_activities    : {:.4} ms",
            per(self.activity_read)
        );
        eprintln!("  list_archetype_priors    : {:.4} ms", per(self.priors));
        eprintln!("  render (both blocks)     : {:.4} ms", per(self.render));
        eprintln!("  TOTAL injection / turn   : {:.4} ms", per(self.total()));
    }
}

/// One `SportType` → its canonical serde slug (matches `memory::sport_slug`).
fn sport_slug(sport: &SportType) -> Option<String> {
    match serde_json::to_value(sport) {
        Ok(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// Time one full injection sequence, accumulating into `phase`. Mirrors
/// `inject_playbooks` + `fetch_archetype_priors` exactly.
async fn time_one_injection(
    repos: &RepositoryRegistry,
    tenant_id: &str,
    user_id: &str,
    coach: Option<&str>,
    phase: &mut Phase,
) {
    let t = Instant::now();
    let playbooks: Vec<Playbook> = repos
        .playbooks
        .list_playbooks(tenant_id, user_id, coach, PLAYBOOK_INJECT_LIMIT)
        .await
        .unwrap();
    phase.list_playbooks += t.elapsed();

    let t = Instant::now();
    let block = render_playbooks_block(&playbooks);
    phase.render += t.elapsed();

    // fetch_archetype_priors: sport keys from playbooks, or (cold-start) activity.
    let mut keys: HashSet<String> = playbooks
        .iter()
        .filter_map(|p| p.trigger.sport.clone())
        .collect();
    if playbooks.is_empty() {
        let t = Instant::now();
        if let (Ok(uuid), Ok(tenant)) = (Uuid::parse_str(user_id), TenantId::parse_str(tenant_id)) {
            let end = Utc::now();
            let start = end - chrono::Duration::days(120);
            if let Ok(acts) = repos
                .activity_cache
                .get_cached_activities(uuid, &tenant, None, start, end, 100)
                .await
            {
                keys.extend(acts.iter().filter_map(|a| sport_slug(a.sport_type())));
            }
        }
        phase.activity_read += t.elapsed();
    }
    keys.insert("any".to_owned());
    let keys: Vec<String> = keys.into_iter().collect();

    let t = Instant::now();
    let priors: Vec<ArchetypePrior> = repos
        .playbooks
        .list_archetype_priors_for_keys(&keys, ARCHETYPE_FETCH_LIMIT)
        .await
        .unwrap();
    phase.priors += t.elapsed();

    let t = Instant::now();
    let _ = render_archetype_block(&priors, &playbooks);
    let _ = block;
    phase.render += t.elapsed();
}

/// Seed `n` distinct, well-evidenced playbooks for one user.
async fn seed_playbooks(repos: &RepositoryRegistry, tenant: &str, user: &str, n: usize) {
    let kinds = [
        TriggerKind::MotivationDip,
        TriggerKind::HrvDrop,
        TriggerKind::LoadRamp,
        TriggerKind::Plateau,
    ];
    let bands = [Band::Low, Band::Moderate, Band::High];
    for i in 0..n {
        let trigger = TriggerPattern {
            kind: kinds[i % kinds.len()],
            sport: Some("run".to_owned()),
            magnitude: bands[i % bands.len()],
        };
        let intervention = Intervention {
            kind: InterventionKind::MinimumViable,
            magnitude: i32::try_from(i).ok(),
        };
        let metric = OutcomeMetric::ActivityCompleted {
            window_days: 2,
            sport: Some("run".to_owned()),
        };
        // 4 successes + 1 failure => clears the evidence + confidence gate.
        for label in [
            OutcomeLabel::Success,
            OutcomeLabel::Success,
            OutcomeLabel::Success,
            OutcomeLabel::Success,
            OutcomeLabel::Failure,
        ] {
            let outcome = RecordedOutcome {
                tenant_id: tenant,
                user_id: user,
                coach_slug: None,
                trigger: &trigger,
                intervention: &intervention,
                outcome_metric: &metric,
                label,
                at: Utc::now(),
            };
            repos
                .playbooks
                .record_playbook_outcome(&outcome)
                .await
                .unwrap();
        }
    }
}

/// Seed `n` archetype priors in the `run` + `any` buckets.
async fn seed_priors(repos: &RepositoryRegistry, n: usize) {
    for i in 0..n {
        let trigger = TriggerPattern {
            kind: TriggerKind::HrvDrop,
            sport: Some(if i % 2 == 0 {
                "run".to_owned()
            } else {
                "any".to_owned()
            }),
            magnitude: Band::High,
        };
        let intervention = Intervention {
            kind: InterventionKind::EasyBlock,
            magnitude: Some(i32::try_from(i).unwrap_or(0)),
        };
        let key = if i % 2 == 0 { "run" } else { "any" };
        let upsert = ArchetypePriorUpsert {
            archetype_key: key,
            trigger_hash: &trigger.hash_key(),
            intervention_hash: &intervention.hash_key(),
            trigger_json: &serde_json::to_string(&trigger).unwrap(),
            intervention_json: &serde_json::to_string(&intervention).unwrap(),
            success_count: 40,
            failure_count: 8,
            distinct_user_count: 30,
        };
        repos
            .playbooks
            .upsert_archetype_prior(&upsert)
            .await
            .unwrap();
    }
}

fn activity(i: usize) -> Activity {
    ActivityBuilder::new(
        format!("act-{i}"),
        format!("Run {i}"),
        SportType::Run,
        Utc::now() - chrono::Duration::days(i64::try_from(i % 100).unwrap_or(0) + 1),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

async fn run_scenario(repos: &RepositoryRegistry, tenant: &str, user: &str) -> Phase {
    let mut phase = Phase::default();
    for _ in 0..ITERS {
        time_one_injection(repos, tenant, user, Some("trail"), &mut phase).await;
    }
    phase
}

#[tokio::test]
async fn playbook_injection_latency_breakdown() {
    // Scenario A — empty / cold-start (the common case: every new user).
    let db = create_test_db().await;
    let (user_a, tenant_a) = seed_user(&db).await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    // A light activity history so the cold-start read scans real rows.
    let acts: Vec<Activity> = (0..50).map(activity).collect();
    repos
        .activity_cache
        .upsert_activities(user_a, &tenant_a, "strava", &acts)
        .await
        .unwrap();
    let phase_a = run_scenario(&repos, &tenant_a.to_string(), &user_a.to_string()).await;

    // Scenario B — established athlete (12 playbooks + 6 priors, no cold-start read).
    let db_b = create_test_db().await;
    let (user_b, tenant_b) = seed_user(&db_b).await;
    let repos_b: Arc<RepositoryRegistry> = Arc::new(db_b.repositories());
    seed_playbooks(&repos_b, &tenant_b.to_string(), &user_b.to_string(), 12).await;
    seed_priors(&repos_b, 6).await;
    let phase_b = run_scenario(&repos_b, &tenant_b.to_string(), &user_b.to_string()).await;

    // Scenario C — cold-start with a heavy activity history (300 activities).
    let db_c = create_test_db().await;
    let (user_c, tenant_c) = seed_user(&db_c).await;
    let repos_c: Arc<RepositoryRegistry> = Arc::new(db_c.repositories());
    let acts_c: Vec<Activity> = (0..300).map(activity).collect();
    repos_c
        .activity_cache
        .upsert_activities(user_c, &tenant_c, "strava", &acts_c)
        .await
        .unwrap();
    let phase_c = run_scenario(&repos_c, &tenant_c.to_string(), &user_c.to_string()).await;

    phase_a.report("A: empty / cold-start (50 activities)", "SQLite in-memory");
    phase_b.report(
        "B: established (12 playbooks + 6 priors)",
        "SQLite in-memory",
    );
    phase_c.report("C: cold-start + 300 activities", "SQLite in-memory");

    // Catastrophic-regression guard only (expected ~sub-ms; allow 500× headroom).
    let per_ms = |p: &Phase| p.total().as_secs_f64() * 1000.0 / f64::from(ITERS);
    for (name, p) in [("A", &phase_a), ("B", &phase_b), ("C", &phase_c)] {
        assert!(
            per_ms(p) < 500.0,
            "scenario {name} injection {:.2} ms/turn — catastrophic regression",
            per_ms(p)
        );
    }
}

/// In-situ confirmation: times the REAL `inject_playbooks` production function
/// (not the replicated sequence above) end to end, so we validate the actual
/// code path the pipeline runs each turn. Gated on `client-chat` (the feature
/// that pulls in the chat pipeline). Run with
/// `cargo test --test playbook_injection_latency_test --features client-chat -- --nocapture`.
#[cfg(feature = "client-chat")]
#[tokio::test]
async fn real_inject_playbooks_latency() {
    use pierre_chat_pipeline::stages::memory::inject_playbooks;

    let iters = 200_u32;
    let per = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(iters);

    // Cold-start (the common case): no playbooks, a real activity history.
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let repos = db.repositories();
    let acts: Vec<Activity> = (0..80).map(activity).collect();
    repos
        .activity_cache
        .upsert_activities(user, &tenant, "strava", &acts)
        .await
        .unwrap();
    let (t_str, u_str) = (tenant.to_string(), user.to_string());
    let mut cold = Duration::ZERO;
    for _ in 0..iters {
        let t = Instant::now();
        let _ = inject_playbooks(
            repos.playbooks.as_ref(),
            repos.activity_cache.as_ref(),
            &t_str,
            &u_str,
            Some("trail"),
            "base-prompt".to_owned(),
        )
        .await;
        cold += t.elapsed();
    }

    // Established: has playbooks (skips the cold-start activity read).
    let db_b = create_test_db().await;
    let (user_b, tenant_b) = seed_user(&db_b).await;
    let repos_b = db_b.repositories();
    seed_playbooks(&repos_b, &tenant_b.to_string(), &user_b.to_string(), 12).await;
    seed_priors(&repos_b, 6).await;
    let (tb, ub) = (tenant_b.to_string(), user_b.to_string());
    let mut established = Duration::ZERO;
    for _ in 0..iters {
        let t = Instant::now();
        let _ = inject_playbooks(
            repos_b.playbooks.as_ref(),
            repos_b.activity_cache.as_ref(),
            &tb,
            &ub,
            Some("trail"),
            "base-prompt".to_owned(),
        )
        .await;
        established += t.elapsed();
    }

    eprintln!("\n── REAL inject_playbooks (SQLite in-memory, avg over {iters} iters) ──");
    eprintln!("  cold-start (80 activities) : {:.4} ms/turn", per(cold));
    eprintln!(
        "  established (12 playbooks)  : {:.4} ms/turn",
        per(established)
    );
    assert!(
        per(cold) < 500.0 && per(established) < 500.0,
        "catastrophic regression"
    );
}
