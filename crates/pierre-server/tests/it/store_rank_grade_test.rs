// ABOUTME: Sprint C22 — unit coverage for services::agent_grading::rerank_by_grade
// ABOUTME: Ensures the store browse rank boosts high-grade agents and falls back gracefully
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_services::agent_grading::{
    rerank_by_grade, AgentGrade, AgentGradingSummary, LetterGrade,
};

struct RankedAgent {
    id: String,
}

fn grade(agent_id: &str, score: f32) -> AgentGrade {
    AgentGrade {
        agent_id: agent_id.to_owned(),
        total_verdicts: 10,
        supported: 0,
        unsupported: 0,
        unsupported_prescription: 0,
        contradicted: 0,
        rhetorical: 0,
        unverifiable: 0,
        score,
        grade: LetterGrade::Provisional,
    }
}

#[test]
fn low_graded_agent_drops_below_high_graded_agent_even_with_more_installs() {
    let mut agents = vec![
        RankedAgent {
            id: "bad".to_owned(),
        },
        RankedAgent {
            id: "good".to_owned(),
        },
    ];
    let grading = AgentGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 20,
        grades: vec![grade("bad", 0.2), grade("good", 0.95)],
    };
    rerank_by_grade(&mut agents, |c| c.id.clone(), &grading);
    assert_eq!(agents[0].id, "good");
    assert_eq!(agents[1].id, "bad");
}

#[test]
fn ungraded_agents_keep_install_count_order_within_default_bucket() {
    let mut agents = vec![
        RankedAgent { id: "a".to_owned() },
        RankedAgent { id: "b".to_owned() },
    ];
    let grading = AgentGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 0,
        grades: vec![],
    };
    rerank_by_grade(&mut agents, |c| c.id.clone(), &grading);
    // Both tie at the default score → stable sort preserves input order.
    assert_eq!(agents[0].id, "a");
    assert_eq!(agents[1].id, "b");
}

#[test]
fn graded_agent_outranks_ungraded_when_score_above_default() {
    let mut agents = vec![
        RankedAgent {
            id: "ungraded".to_owned(),
        },
        RankedAgent {
            id: "graded".to_owned(),
        },
    ];
    let grading = AgentGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 5,
        grades: vec![grade("graded", 0.8)],
    };
    rerank_by_grade(&mut agents, |c| c.id.clone(), &grading);
    assert_eq!(agents[0].id, "graded");
    assert_eq!(agents[1].id, "ungraded");
}

#[test]
fn graded_agent_below_default_drops_behind_ungraded() {
    let mut agents = vec![
        RankedAgent {
            id: "graded-low".to_owned(),
        },
        RankedAgent {
            id: "ungraded".to_owned(),
        },
    ];
    let grading = AgentGradingSummary {
        tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        verdicts_scanned: 5,
        grades: vec![grade("graded-low", 0.3)],
    };
    rerank_by_grade(&mut agents, |c| c.id.clone(), &grading);
    // Ungraded defaults to 0.5, which outranks the graded-low 0.3.
    assert_eq!(agents[0].id, "ungraded");
    assert_eq!(agents[1].id, "graded-low");
}
