// ABOUTME: Unit tests for playbooks — enum parsing, hash keys and Wilson-bound confidence
// ABOUTME: Pins that small samples are penalized and neutral outcomes leave confidence unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::Utc;
use pierre_memory::playbooks::{
    AdviceStatus, Band, Intervention, InterventionKind, LabelSource, OutcomeLabel, OutcomeMetric,
    Playbook, TriggerKind, TriggerPattern,
};

#[test]
fn enum_string_roundtrips() {
    for k in [
        TriggerKind::MotivationDip,
        TriggerKind::HrvDrop,
        TriggerKind::LoadRamp,
        TriggerKind::Plateau,
        TriggerKind::Travel,
        TriggerKind::PrePlanned,
        TriggerKind::Other,
    ] {
        assert_eq!(TriggerKind::parse_lenient(k.as_str()), k);
    }
    for b in [Band::Low, Band::Moderate, Band::High] {
        assert_eq!(Band::parse_lenient(b.as_str()), b);
    }
    for i in [
        InterventionKind::EasyBlock,
        InterventionKind::AddTempo,
        InterventionKind::AddThreshold,
        InterventionKind::MinimumViable,
        InterventionKind::ReduceVolume,
        InterventionKind::RestDay,
        InterventionKind::CommStyleTerse,
        InterventionKind::CommStyleAnalytical,
        InterventionKind::Other,
    ] {
        assert_eq!(InterventionKind::parse_lenient(i.as_str()), i);
    }
    for l in [
        OutcomeLabel::Success,
        OutcomeLabel::Failure,
        OutcomeLabel::Neutral,
    ] {
        assert_eq!(OutcomeLabel::parse_lenient(l.as_str()), l);
    }
    for s in [LabelSource::DataHeuristic, LabelSource::LlmJudge] {
        assert_eq!(LabelSource::parse_lenient(s.as_str()), s);
    }
    for st in [
        AdviceStatus::Pending,
        AdviceStatus::Labeled,
        AdviceStatus::Expired,
    ] {
        assert_eq!(AdviceStatus::parse_lenient(st.as_str()), st);
    }
}

#[test]
fn unknown_enum_values_fall_back_safely() {
    assert_eq!(TriggerKind::parse_lenient("nope"), TriggerKind::Other);
    assert_eq!(Band::parse_lenient("nope"), Band::Moderate);
    assert_eq!(
        InterventionKind::parse_lenient("nope"),
        InterventionKind::Other
    );
    // The reinforcement-sensitive defaults must be the non-reinforcing ones.
    assert_eq!(OutcomeLabel::parse_lenient("nope"), OutcomeLabel::Neutral);
    assert_eq!(AdviceStatus::parse_lenient("nope"), AdviceStatus::Pending);
}

#[test]
fn trigger_and_intervention_hash_keys_are_stable() {
    let t = TriggerPattern {
        kind: TriggerKind::HrvDrop,
        sport: Some("run".into()),
        magnitude: Band::High,
    };
    assert_eq!(t.hash_key(), "hrv_drop:run:high");
    let t_any = TriggerPattern {
        kind: TriggerKind::HrvDrop,
        sport: None,
        magnitude: Band::High,
    };
    assert_eq!(t_any.hash_key(), "hrv_drop:*:high");
}

#[test]
fn outcome_metric_kind_str() {
    let m = OutcomeMetric::ActivityCompleted {
        window_days: 3,
        sport: Some("run".into()),
    };
    assert_eq!(m.kind_str(), "activity_completed");
    let r = OutcomeMetric::RampRateWithin { ceiling: 1.3 };
    assert_eq!(r.kind_str(), "ramp_rate_within");
}

#[test]
fn outcome_metric_serde_roundtrip() {
    let m = OutcomeMetric::HrvDelta { window_days: 7 };
    let json = serde_json::to_string(&m).unwrap_or_default();
    assert!(!json.is_empty());
    let back = serde_json::from_str::<OutcomeMetric>(&json).ok();
    assert_eq!(back, Some(m));
}

fn sample_playbook() -> Playbook {
    let now = Utc::now();
    Playbook {
        id: "p1".into(),
        tenant_id: "t1".into(),
        user_id: "u1".into(),
        agent_slug: Some("trail".into()),
        trigger: TriggerPattern {
            kind: TriggerKind::MotivationDip,
            sport: Some("run".into()),
            magnitude: Band::Moderate,
        },
        intervention: Intervention {
            kind: InterventionKind::MinimumViable,
            magnitude: None,
        },
        outcome_metric: OutcomeMetric::ActivityCompleted {
            window_days: 2,
            sport: Some("run".into()),
        },
        success_count: 0,
        failure_count: 0,
        neutral_count: 0,
        confidence: 0.0,
        last_outcome_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn wilson_penalizes_small_samples() {
    let now = Utc::now();
    let mut lucky = sample_playbook();
    lucky.record_outcome(OutcomeLabel::Success, now);
    // 1/1 success — high raw rate but low confidence.
    assert!((lucky.success_rate() - 1.0).abs() < f32::EPSILON);
    let lucky_conf = lucky.confidence;

    let mut proven = sample_playbook();
    for _ in 0..18 {
        proven.record_outcome(OutcomeLabel::Success, now);
    }
    for _ in 0..2 {
        proven.record_outcome(OutcomeLabel::Failure, now);
    }
    // 18/20 = 0.90 raw, lower raw rate than 1/1 — but MORE confident.
    assert!(proven.success_rate() < lucky.success_rate());
    assert!(
        proven.confidence > lucky_conf,
        "18/20 ({}) should outrank 1/1 ({lucky_conf})",
        proven.confidence
    );
}

#[test]
fn neutrals_do_not_move_confidence() {
    let now = Utc::now();
    let mut pb = sample_playbook();
    pb.record_outcome(OutcomeLabel::Success, now);
    let after_success = pb.confidence;
    pb.record_outcome(OutcomeLabel::Neutral, now);
    pb.record_outcome(OutcomeLabel::Neutral, now);
    // Neutrals bump the counter but leave the decisive-only confidence intact.
    assert_eq!(pb.neutral_count, 2);
    assert!((pb.confidence - after_success).abs() < f32::EPSILON);
}

#[test]
fn empty_playbook_has_zero_confidence() {
    let pb = sample_playbook();
    assert_eq!(pb.total_outcomes(), 0);
    assert!((pb.wilson_lower_bound() - 0.0).abs() < f32::EPSILON);
}
