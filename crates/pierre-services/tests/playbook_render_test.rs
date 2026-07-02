// ABOUTME: Unit tests for playbook prompt rendering — evidence/confidence filtering + human-readable format
// ABOUTME: Proves only well-evidenced, confident playbooks reach the coach's prompt (P5)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for playbook prompt rendering and evidence/confidence filtering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::slice::from_ref;

use chrono::Utc;
use pierre_memory::playbooks::{
    Band, Intervention, InterventionKind, OutcomeMetric, Playbook, TriggerKind, TriggerPattern,
};
use pierre_services::playbook_render::render_playbooks_block;

fn playbook(success: u32, failure: u32, confidence: f32) -> Playbook {
    let now = Utc::now();
    Playbook {
        id: "p".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        coach_slug: None,
        trigger: TriggerPattern {
            kind: TriggerKind::MotivationDip,
            sport: Some("run".to_owned()),
            magnitude: Band::Moderate,
        },
        intervention: Intervention {
            kind: InterventionKind::MinimumViable,
            magnitude: None,
        },
        outcome_metric: OutcomeMetric::ActivityCompleted {
            window_days: 2,
            sport: Some("run".to_owned()),
        },
        success_count: success,
        failure_count: failure,
        neutral_count: 0,
        confidence,
        last_outcome_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn renders_well_evidenced_confident_playbook() {
    let block = render_playbooks_block(&[playbook(5, 1, 0.6)]).expect("renders a block");
    assert!(block.contains("What works for this athlete"));
    // Slugs are humanized for the prompt.
    assert!(block.contains("motivation dip"), "block: {block}");
    assert!(block.contains("minimum viable"));
    // Success / decisive count is shown.
    assert!(block.contains("5/6"), "block: {block}");
    // The guardrail framing is present.
    assert!(block.contains("do not override"));
}

#[test]
fn filters_low_evidence_low_confidence_and_empty() {
    // 2 decisive outcomes < MIN_EVIDENCE(3) -> not surfaced.
    assert!(render_playbooks_block(&[playbook(2, 0, 0.9)]).is_none());
    // Confidence 0.3 < MIN_CONFIDENCE(0.5) -> not surfaced.
    assert!(render_playbooks_block(&[playbook(5, 1, 0.3)]).is_none());
    // No playbooks at all -> nothing to inject.
    assert!(render_playbooks_block(&[]).is_none());
}

#[test]
fn archetype_block_renders_and_excludes_covered_and_weak() {
    use pierre_memory::playbooks::ArchetypePrior;
    use pierre_services::playbook_render::render_archetype_block;

    let prior = ArchetypePrior {
        archetype_key: "run".to_owned(),
        trigger: TriggerPattern {
            kind: TriggerKind::HrvDrop,
            sport: Some("run".to_owned()),
            magnitude: Band::High,
        },
        intervention: Intervention {
            kind: InterventionKind::EasyBlock,
            magnitude: Some(2),
        },
        success_count: 40,
        failure_count: 10,
        distinct_user_count: 25,
        confidence: 0.7,
    };

    // No personal coverage -> the cold-start prior is rendered.
    let block = render_archetype_block(from_ref(&prior), &[]).expect("renders");
    assert!(
        block.contains("Patterns from similar athletes"),
        "block: {block}"
    );
    assert!(block.contains("hrv drop"));
    assert!(block.contains("25 similar athletes"));

    // A personal playbook for the SAME trigger+intervention -> excluded (own data wins).
    let mut personal = playbook(5, 1, 0.6);
    personal.trigger = prior.trigger.clone();
    personal.intervention = prior.intervention.clone();
    assert!(
        render_archetype_block(from_ref(&prior), from_ref(&personal)).is_none(),
        "a personal playbook should suppress the matching archetype prior"
    );

    // A low-confidence prior is not surfaced.
    let mut weak = prior;
    weak.confidence = 0.3;
    assert!(render_archetype_block(&[weak], &[]).is_none());
}
