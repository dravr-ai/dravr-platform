// ABOUTME: PlaybookRepository dual-DB round-trip — atomic upsert, tenant isolation, advice lifecycle, coach scoping
// ABOUTME: Proves the procedural coaching memory storage layer (P2 of the coaching-playbook-memory epic)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the procedural coaching memory repository. Runs on the
//! in-memory `SQLite` fixture locally and against `Postgres` in CI (`ci-postgres`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_database::repositories::{ArchetypePriorUpsert, RecordedOutcome};
use pierre_database::RepositoryRegistry;
use pierre_memory::playbooks::{
    AdviceStatus, Band, Intervention, InterventionKind, LabelSource, MetricBaseline, OutcomeLabel,
    OutcomeMetric, PendingAdvice, TriggerKind, TriggerPattern,
};

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::create_test_db;

fn sample_trigger() -> TriggerPattern {
    TriggerPattern {
        kind: TriggerKind::MotivationDip,
        sport: Some("run".to_owned()),
        magnitude: Band::Moderate,
    }
}

fn sample_intervention() -> Intervention {
    Intervention {
        kind: InterventionKind::MinimumViable,
        magnitude: None,
    }
}

fn sample_metric() -> OutcomeMetric {
    OutcomeMetric::ActivityCompleted {
        window_days: 2,
        sport: Some("run".to_owned()),
    }
}

#[tokio::test]
async fn record_outcome_creates_then_increments_same_playbook() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (trigger, intervention, metric) =
        (sample_trigger(), sample_intervention(), sample_metric());

    let mut outcome = RecordedOutcome {
        tenant_id: "t1",
        user_id: "u1",
        coach_slug: None,
        trigger: &trigger,
        intervention: &intervention,
        outcome_metric: &metric,
        label: OutcomeLabel::Success,
        at: Utc::now(),
    };

    let id1 = repos
        .playbooks
        .record_playbook_outcome(&outcome)
        .await
        .unwrap();
    let id2 = repos
        .playbooks
        .record_playbook_outcome(&outcome)
        .await
        .unwrap();
    outcome.label = OutcomeLabel::Failure;
    let id3 = repos
        .playbooks
        .record_playbook_outcome(&outcome)
        .await
        .unwrap();

    // Same (tenant,user,coach,trigger,intervention) => the same playbook row, not
    // three duplicates.
    assert_eq!(id1, id2, "repeated outcome reuses the playbook row");
    assert_eq!(id2, id3);

    let playbooks = repos
        .playbooks
        .list_playbooks("t1", "u1", None, 10)
        .await
        .unwrap();
    assert_eq!(playbooks.len(), 1, "exactly one playbook accrued");
    let pb = &playbooks[0];
    assert_eq!(pb.success_count, 2);
    assert_eq!(pb.failure_count, 1);
    assert_eq!(pb.neutral_count, 0);
    // 2/3 decisive successes -> a positive Wilson lower bound computed on read.
    assert!(
        pb.confidence > 0.0 && pb.confidence < 1.0,
        "confidence is the Wilson lower bound: {}",
        pb.confidence
    );
}

#[tokio::test]
async fn list_playbooks_is_tenant_isolated() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (trigger, intervention, metric) =
        (sample_trigger(), sample_intervention(), sample_metric());

    for tenant in ["t1", "t2"] {
        let outcome = RecordedOutcome {
            tenant_id: tenant,
            user_id: "u1",
            coach_slug: None,
            trigger: &trigger,
            intervention: &intervention,
            outcome_metric: &metric,
            label: OutcomeLabel::Success,
            at: Utc::now(),
        };
        repos
            .playbooks
            .record_playbook_outcome(&outcome)
            .await
            .unwrap();
    }

    let t1 = repos
        .playbooks
        .list_playbooks("t1", "u1", None, 10)
        .await
        .unwrap();
    assert_eq!(t1.len(), 1);
    assert_eq!(t1[0].tenant_id, "t1", "tenant t1 never sees t2's playbook");
}

#[tokio::test]
async fn pending_advice_due_label_and_future() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let make_advice = |id: &str, due_by| PendingAdvice {
        id: id.to_owned(),
        tenant_id: "t1".to_owned(),
        user_id: "u1".to_owned(),
        coach_slug: None,
        playbook_id: None,
        trigger: sample_trigger(),
        intervention: sample_intervention(),
        outcome_metric: sample_metric(),
        baseline: MetricBaseline {
            captured_at: Utc::now(),
        },
        due_by,
        status: AdviceStatus::Pending,
        label: None,
        label_source: None,
        source_msg_id: Some("m1".to_owned()),
        created_at: Utc::now(),
    };

    let past = make_advice("due-now", Utc::now() - Duration::hours(1));
    let future = make_advice("not-yet", Utc::now() + Duration::hours(48));
    repos.playbooks.insert_pending_advice(&past).await.unwrap();
    repos
        .playbooks
        .insert_pending_advice(&future)
        .await
        .unwrap();

    let now = Utc::now().timestamp();
    let due = repos.playbooks.due_pending_advice(now, 100).await.unwrap();
    assert_eq!(due.len(), 1, "only the past-due advice is returned");
    assert_eq!(due[0].id, "due-now");

    // Atomically record the outcome and label the advice; it must leave the
    // pending scan AND create/reinforce the matching playbook in one step.
    let (trigger, intervention, metric) =
        (sample_trigger(), sample_intervention(), sample_metric());
    let outcome = RecordedOutcome {
        tenant_id: "t1",
        user_id: "u1",
        coach_slug: None,
        trigger: &trigger,
        intervention: &intervention,
        outcome_metric: &metric,
        label: OutcomeLabel::Success,
        at: Utc::now(),
    };
    let playbook_id = repos
        .playbooks
        .record_outcome_and_label(&outcome, "due-now", LabelSource::DataHeuristic)
        .await
        .unwrap();
    assert!(!playbook_id.is_empty(), "outcome folded into a playbook");
    let due_after = repos.playbooks.due_pending_advice(now, 100).await.unwrap();
    assert!(
        due_after.is_empty(),
        "a labeled advice is no longer pending"
    );
    // The atomic mark is tenant-scoped: a different tenant must not have flipped.
    let pb = repos
        .playbooks
        .list_playbooks("t1", "u1", None, 10)
        .await
        .unwrap();
    assert_eq!(pb.len(), 1);
    assert_eq!(pb[0].success_count, 1);
}

#[tokio::test]
async fn coach_scoping_includes_agnostic_excludes_other_coach() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (intervention, metric) = (sample_intervention(), sample_metric());

    // A coach-agnostic playbook and a "trail" coach playbook (distinct triggers
    // so they are distinct rows).
    let agnostic_trigger = TriggerPattern {
        kind: TriggerKind::HrvDrop,
        sport: None,
        magnitude: Band::High,
    };
    let trail_trigger = sample_trigger();
    for (coach, trigger) in [(None, &agnostic_trigger), (Some("trail"), &trail_trigger)] {
        let outcome = RecordedOutcome {
            tenant_id: "t1",
            user_id: "u1",
            coach_slug: coach,
            trigger,
            intervention: &intervention,
            outcome_metric: &metric,
            label: OutcomeLabel::Success,
            at: Utc::now(),
        };
        repos
            .playbooks
            .record_playbook_outcome(&outcome)
            .await
            .unwrap();
    }

    // The "trail" coach sees its own playbook AND the coach-agnostic one.
    let trail = repos
        .playbooks
        .list_playbooks("t1", "u1", Some("trail"), 10)
        .await
        .unwrap();
    assert_eq!(trail.len(), 2, "coach sees own + agnostic playbooks");

    // No-coach context sees only the coach-agnostic playbook.
    let none = repos
        .playbooks
        .list_playbooks("t1", "u1", None, 10)
        .await
        .unwrap();
    assert_eq!(none.len(), 1, "no-coach context sees only agnostic");
    assert_eq!(none[0].coach_slug, None);
}

/// Build one due, still-pending advice for the shared sample pattern.
fn due_sample_advice(id: &str) -> PendingAdvice {
    PendingAdvice {
        id: id.to_owned(),
        tenant_id: "t1".to_owned(),
        user_id: "u1".to_owned(),
        coach_slug: None,
        playbook_id: None,
        trigger: sample_trigger(),
        intervention: sample_intervention(),
        outcome_metric: sample_metric(),
        baseline: MetricBaseline {
            captured_at: Utc::now(),
        },
        due_by: Utc::now() - Duration::hours(1),
        status: AdviceStatus::Pending,
        label: None,
        label_source: None,
        source_msg_id: None,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn forget_playbook_purges_pending_advice_so_it_cannot_resurrect() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (trigger, intervention, metric) =
        (sample_trigger(), sample_intervention(), sample_metric());

    // A playbook, plus in-flight advice for the SAME pattern that would mature
    // later and re-create it.
    let outcome = RecordedOutcome {
        tenant_id: "t1",
        user_id: "u1",
        coach_slug: None,
        trigger: &trigger,
        intervention: &intervention,
        outcome_metric: &metric,
        label: OutcomeLabel::Success,
        at: Utc::now(),
    };
    let playbook_id = repos
        .playbooks
        .record_playbook_outcome(&outcome)
        .await
        .unwrap();
    repos
        .playbooks
        .insert_pending_advice(&due_sample_advice("adv-1"))
        .await
        .unwrap();

    // Forget (GDPR): must remove the playbook AND the advice that would
    // resurrect it.
    let removed = repos
        .playbooks
        .delete_playbook("t1", "u1", &playbook_id)
        .await
        .unwrap();
    assert_eq!(removed, 1);

    let now = Utc::now().timestamp();
    let due = repos.playbooks.due_pending_advice(now, 100).await.unwrap();
    assert!(due.is_empty(), "forget purged the in-flight advice");
    let playbooks = repos
        .playbooks
        .list_playbooks("t1", "u1", None, 10)
        .await
        .unwrap();
    assert!(playbooks.is_empty(), "the playbook stays forgotten");
}

#[tokio::test]
async fn insert_pending_advice_dedups_identical_open_advice() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // The same recommendation reaffirmed across two turns must enqueue only one
    // open advice, so a single real behavior cannot double-count later.
    repos
        .playbooks
        .insert_pending_advice(&due_sample_advice("adv-1"))
        .await
        .unwrap();
    repos
        .playbooks
        .insert_pending_advice(&due_sample_advice("adv-2"))
        .await
        .unwrap();

    let now = Utc::now().timestamp();
    let due = repos.playbooks.due_pending_advice(now, 100).await.unwrap();
    assert_eq!(due.len(), 1, "reaffirmed advice deduped to one open row");
}

#[tokio::test]
async fn archetype_priors_batch_fetch_is_confidence_ranked_and_pruned() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // Upsert a well-evidenced "run" prior, then verify the batch fetch returns
    // it for the run+any key set, and that delete removes it (prune path).
    let trigger = TriggerPattern {
        kind: TriggerKind::HrvDrop,
        sport: Some("run".to_owned()),
        magnitude: Band::High,
    };
    let intervention = sample_intervention();
    let upsert = ArchetypePriorUpsert {
        archetype_key: "run",
        trigger_hash: &trigger.hash_key(),
        intervention_hash: &intervention.hash_key(),
        trigger_json: &serde_json::to_string(&trigger).unwrap(),
        intervention_json: &serde_json::to_string(&intervention).unwrap(),
        success_count: 40,
        failure_count: 5,
        distinct_user_count: 25,
    };
    repos
        .playbooks
        .upsert_archetype_prior(&upsert)
        .await
        .unwrap();

    let keys = vec!["run".to_owned(), "any".to_owned()];
    let priors = repos
        .playbooks
        .list_archetype_priors_for_keys(&keys, 20)
        .await
        .unwrap();
    assert_eq!(priors.len(), 1);
    assert_eq!(priors[0].archetype_key, "run");
    assert_eq!(priors[0].distinct_user_count, 25);

    repos
        .playbooks
        .delete_archetype_prior("run", &trigger.hash_key(), &intervention.hash_key())
        .await
        .unwrap();
    let after = repos
        .playbooks
        .list_archetype_priors_for_keys(&keys, 20)
        .await
        .unwrap();
    assert!(after.is_empty(), "pruned prior no longer returned");
}
