// ABOUTME: Sprint C22 — unit coverage for services::coach_grading::rerank_by_grade
// ABOUTME: Ensures the store browse rank boosts high-grade coaches and falls back gracefully
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::services::coach_grading::{
    rerank_by_grade, CoachGrade, CoachGradingSummary, LetterGrade,
};

struct RankedCoach {
    id: String,
}

fn grade(coach_id: &str, score: f32) -> CoachGrade {
    CoachGrade {
        coach_id: coach_id.to_owned(),
        total_verdicts: 10,
        supported: 0,
        unsupported: 0,
        contradicted: 0,
        rhetorical: 0,
        unverifiable: 0,
        score,
        grade: LetterGrade::Provisional,
    }
}

#[test]
fn low_graded_coach_drops_below_high_graded_coach_even_with_more_installs() {
    let mut coaches = vec![
        RankedCoach {
            id: "bad".to_owned(),
        },
        RankedCoach {
            id: "good".to_owned(),
        },
    ];
    let grading = CoachGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 20,
        grades: vec![grade("bad", 0.2), grade("good", 0.95)],
    };
    rerank_by_grade(&mut coaches, |c| c.id.clone(), &grading);
    assert_eq!(coaches[0].id, "good");
    assert_eq!(coaches[1].id, "bad");
}

#[test]
fn ungraded_coaches_keep_install_count_order_within_default_bucket() {
    let mut coaches = vec![
        RankedCoach { id: "a".to_owned() },
        RankedCoach { id: "b".to_owned() },
    ];
    let grading = CoachGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 0,
        grades: vec![],
    };
    rerank_by_grade(&mut coaches, |c| c.id.clone(), &grading);
    // Both tie at the default score → stable sort preserves input order.
    assert_eq!(coaches[0].id, "a");
    assert_eq!(coaches[1].id, "b");
}

#[test]
fn graded_coach_outranks_ungraded_when_score_above_default() {
    let mut coaches = vec![
        RankedCoach {
            id: "ungraded".to_owned(),
        },
        RankedCoach {
            id: "graded".to_owned(),
        },
    ];
    let grading = CoachGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 5,
        grades: vec![grade("graded", 0.8)],
    };
    rerank_by_grade(&mut coaches, |c| c.id.clone(), &grading);
    assert_eq!(coaches[0].id, "graded");
    assert_eq!(coaches[1].id, "ungraded");
}

#[test]
fn graded_coach_below_default_drops_behind_ungraded() {
    let mut coaches = vec![
        RankedCoach {
            id: "graded-low".to_owned(),
        },
        RankedCoach {
            id: "ungraded".to_owned(),
        },
    ];
    let grading = CoachGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 5,
        grades: vec![grade("graded-low", 0.3)],
    };
    rerank_by_grade(&mut coaches, |c| c.id.clone(), &grading);
    // Ungraded defaults to 0.5, which outranks the graded-low 0.3.
    assert_eq!(coaches[0].id, "ungraded");
    assert_eq!(coaches[1].id, "graded-low");
}
