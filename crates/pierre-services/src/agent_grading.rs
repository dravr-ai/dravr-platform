// ABOUTME: Phase D Sprint C14 — agent content grading derived from claim verdict history
// ABOUTME: Computes per-agent quality scores from claim_verdicts for store ranking + admin review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent content grading service.
//!
//! Walks the recent `claim_verdicts` for a tenant and produces a
//! per-agent grade summarizing how often the bullshit detector
//! supported, contradicted, or flagged an agent's claims. The grade is
//! a single `0.0..=1.0` score the store can rank by, plus a letter
//! grade for human display.
//!
//! Pure-read; the data lives in `claim_verdicts`. No background
//! worker, no schema changes — recomputed on each admin tab mount or
//! on demand from the store ranking pipeline.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_core::models::TenantId;
use pierre_database::AgentRepos;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict};

use pierre_core::errors::{AppError, AppResult};

/// Hard cap on how many recent verdicts the grading scan considers.
pub const MAX_VERDICTS_SCANNED: i64 = 1000;
/// Default page size when callers omit `limit`.
pub const DEFAULT_VERDICT_LIMIT: i64 = 500;
/// Minimum sample size for an agent grade to be considered confident.
const MIN_SAMPLE_FOR_CONFIDENT_GRADE: u64 = 3;

/// Letter grade derived from a numeric `score` in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LetterGrade {
    /// `score >= 0.90` — agent claims almost always supported.
    A,
    /// `score >= 0.75` — mostly supported with occasional flags.
    B,
    /// `score >= 0.60` — mixed; review recommended.
    C,
    /// `score >= 0.40` — frequent unsupported / contradicted claims.
    D,
    /// `score < 0.40` — agent is producing low-quality claims.
    F,
    /// Sample size below the confidence threshold; grade is provisional.
    Provisional,
}

impl LetterGrade {
    fn from_score(score: f32, sample_size: u64) -> Self {
        if sample_size < MIN_SAMPLE_FOR_CONFIDENT_GRADE {
            return Self::Provisional;
        }
        if score >= 0.90 {
            Self::A
        } else if score >= 0.75 {
            Self::B
        } else if score >= 0.60 {
            Self::C
        } else if score >= 0.40 {
            Self::D
        } else {
            Self::F
        }
    }
}

/// Per-agent grade row returned by [`compute_agent_grades`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGrade {
    /// Agent identifier from `claim_verdicts.agent_id`.
    pub agent_id: String,
    /// Total verdicts attributed to this agent in the scan window.
    pub total_verdicts: u64,
    /// Verdicts classified as `supported`.
    pub supported: u64,
    /// Verdicts classified as `unsupported`.
    pub unsupported: u64,
    /// `Unsupported` training-prescription verdicts, excluded from score
    /// weighting (advice has no evidence corpus to cite).
    pub unsupported_prescription: u64,
    /// Verdicts classified as `contradicted`.
    pub contradicted: u64,
    /// Verdicts classified as `rhetorical` (excluded from score weighting).
    pub rhetorical: u64,
    /// Verdicts classified as `unverifiable` (excluded from score weighting).
    pub unverifiable: u64,
    /// Score in `0.0..=1.0`. Higher is better.
    pub score: f32,
    /// Letter grade derived from `score` and `total_verdicts`.
    pub grade: LetterGrade,
}

/// Top-level wire response for `GET /admin/agent-grading/summary`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGradingSummary {
    /// Tenant the summary was computed for.
    pub tenant_id: String,
    /// Total verdicts scanned in this window.
    pub verdicts_scanned: u64,
    /// Per-agent grades sorted by score ascending (worst first — admins
    /// review the bottom of the leaderboard, not the top).
    pub grades: Vec<AgentGrade>,
}

/// Compute per-agent grades for `tenant_id`.
///
/// # Errors
///
/// Returns an error if the underlying
/// [`pierre_database::repositories::ClaimVerdictRepository::list_recent_verdicts`]
/// call fails.
pub async fn compute_agent_grades(
    repos: &AgentRepos,
    tenant_id: TenantId,
    limit: i64,
) -> AppResult<AgentGradingSummary> {
    let scan_limit = limit.clamp(1, MAX_VERDICTS_SCANNED);
    let verdicts = repos
        .claim_verdicts
        .list_recent_verdicts(tenant_id, scan_limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for coach grading");
            AppError::internal(format!("Failed to list verdicts: {e}"))
        })?;

    let verdicts_scanned = u64::try_from(verdicts.len()).unwrap_or(u64::MAX);

    let mut buckets: HashMap<String, GradeBucket> = HashMap::new();
    for v in &verdicts {
        let Some(agent) = v.agent_id.clone() else {
            continue;
        };
        let entry = buckets.entry(agent).or_default();
        entry.consume(v);
    }

    let mut grades: Vec<AgentGrade> = buckets
        .into_iter()
        .map(|(agent_id, b)| b.finalize(agent_id))
        .collect();
    // Worst grades first so admins see the bottom of the leaderboard.
    grades.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));

    Ok(AgentGradingSummary {
        tenant_id: tenant_id.to_string(),
        verdicts_scanned,
        grades,
    })
}

/// Default score applied to agents the grading service has no
/// verdicts for.
///
/// 0.5 matches the fallback in [`GradeBucket::finalize`] for agents
/// with zero scored verdicts, keeping ungraded agents above known-bad
/// ones but below known-good ones.
pub const DEFAULT_UNGRADED_SCORE: f32 = 0.5;

/// Re-sort `items` in place so higher-graded agents rank first (Sprint C22).
///
/// `agent_id_of` extracts the agent identifier from each item; the
/// function looks the id up in `grading.grades` and falls back to
/// [`DEFAULT_UNGRADED_SCORE`] when no grade is present. Ties preserve
/// the input order (Rust's `sort_by` is stable), which means the
/// caller's original ranking — typically `install_count DESC` from the
/// database — survives within any score bucket.
pub fn rerank_by_grade<T, F>(items: &mut [T], agent_id_of: F, grading: &AgentGradingSummary)
where
    F: Fn(&T) -> String,
{
    use std::collections::HashMap;

    let score_by_agent: HashMap<&str, f32> = grading
        .grades
        .iter()
        .map(|g| (g.agent_id.as_str(), g.score))
        .collect();

    items.sort_by(|a, b| {
        let a_score = score_by_agent
            .get(agent_id_of(a).as_str())
            .copied()
            .unwrap_or(DEFAULT_UNGRADED_SCORE);
        let b_score = score_by_agent
            .get(agent_id_of(b).as_str())
            .copied()
            .unwrap_or(DEFAULT_UNGRADED_SCORE);
        b_score.partial_cmp(&a_score).unwrap_or(Ordering::Equal)
    });
}

#[derive(Default)]
struct GradeBucket {
    total: u64,
    supported: u64,
    unsupported: u64,
    unsupported_prescription: u64,
    contradicted: u64,
    rhetorical: u64,
    unverifiable: u64,
}

impl GradeBucket {
    fn consume(&mut self, v: &ClaimVerdict) {
        self.total += 1;
        match v.status {
            ClaimStatus::Supported => self.supported += 1,
            // Training-prescription advice has no evidence corpus to cite, so an
            // `Unsupported` verdict there is expected — not a quality failure.
            // Mirror the user-facing banner carve-out (chat-pipeline
            // verification.rs) and exclude it from the scored denominator
            // instead of penalizing advice-heavy agents in store rank.
            ClaimStatus::Unsupported if v.category == ClaimCategory::TrainingPrescription => {
                self.unsupported_prescription += 1;
            }
            ClaimStatus::Unsupported => self.unsupported += 1,
            ClaimStatus::Contradicted => self.contradicted += 1,
            ClaimStatus::Rhetorical => self.rhetorical += 1,
            ClaimStatus::Unverifiable => self.unverifiable += 1,
        }
    }

    fn finalize(self, agent_id: String) -> AgentGrade {
        // Score weighting:
        // - supported counts +1
        // - contradicted counts -1 (clamped to 0)
        // - unsupported counts as 0.25 of supported (mild penalty)
        // - rhetorical / unverifiable excluded from denominator
        let scored_total = self.supported + self.unsupported + self.contradicted;
        let score = if scored_total == 0 {
            0.5
        } else {
            #[allow(clippy::cast_precision_loss)]
            let supported = self.supported as f32;
            #[allow(clippy::cast_precision_loss)]
            let unsupported = self.unsupported as f32;
            #[allow(clippy::cast_precision_loss)]
            let contradicted = self.contradicted as f32;
            #[allow(clippy::cast_precision_loss)]
            let denom = scored_total as f32;
            // Weighted score: supported + 0.25 * unsupported - contradicted.
            // The inner sum goes through `mul_add` to satisfy
            // clippy::suboptimal_flops; the outer subtraction is a plain
            // subtract since there's no multiplication to fuse.
            let raw = unsupported.mul_add(0.25, supported) - contradicted;
            (raw / denom).clamp(0.0, 1.0)
        };
        let grade = LetterGrade::from_score(score, self.total);
        AgentGrade {
            agent_id,
            total_verdicts: self.total,
            supported: self.supported,
            unsupported: self.unsupported,
            unsupported_prescription: self.unsupported_prescription,
            contradicted: self.contradicted,
            rhetorical: self.rhetorical,
            unverifiable: self.unverifiable,
            score,
            grade,
        }
    }
}

/// Compute a single agent's grade from its claim verdicts.
///
/// Convenience wrapper over the same `consume`/`finalize` weighting used by
/// [`compute_agent_grades`], for callers that have already grouped verdicts by
/// agent (and for unit-testing the scoring rules in isolation).
#[must_use]
pub fn grade_from_verdicts(agent_id: String, verdicts: &[ClaimVerdict]) -> AgentGrade {
    let mut bucket = GradeBucket::default();
    for v in verdicts {
        bucket.consume(v);
    }
    bucket.finalize(agent_id)
}
