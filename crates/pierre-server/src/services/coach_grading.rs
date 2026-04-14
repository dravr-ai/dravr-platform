// ABOUTME: Phase D Sprint C14 — coach content grading derived from Tier 5.5 verdict history
// ABOUTME: Computes per-coach quality scores from claim_verdicts for store ranking + admin review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach content grading service.
//!
//! Walks the recent `claim_verdicts` for a tenant and produces a
//! per-coach grade summarizing how often the Tier 5.5 detector
//! supported, contradicted, or flagged a coach's claims. The grade is
//! a single `0.0..=1.0` score the store can rank by, plus a letter
//! grade for human display.
//!
//! Pure-read; the data lives in `claim_verdicts`. No background
//! worker, no schema changes — recomputed on each admin tab mount or
//! on demand from the store ranking pipeline.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::{ClaimStatus, ClaimVerdict};

use crate::errors::{AppError, AppResult};

/// Hard cap on how many recent verdicts the grading scan considers.
pub const MAX_VERDICTS_SCANNED: i64 = 1000;
/// Default page size when callers omit `limit`.
pub const DEFAULT_VERDICT_LIMIT: i64 = 500;
/// Minimum sample size for a coach grade to be considered confident.
const MIN_SAMPLE_FOR_CONFIDENT_GRADE: u64 = 3;

/// Letter grade derived from a numeric `score` in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LetterGrade {
    /// `score >= 0.90` — coach claims almost always supported.
    A,
    /// `score >= 0.75` — mostly supported with occasional flags.
    B,
    /// `score >= 0.60` — mixed; review recommended.
    C,
    /// `score >= 0.40` — frequent unsupported / contradicted claims.
    D,
    /// `score < 0.40` — coach is producing low-quality claims.
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

/// Per-coach grade row returned by [`compute_coach_grades`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachGrade {
    /// Coach identifier from `claim_verdicts.coach_id`.
    pub coach_id: String,
    /// Total verdicts attributed to this coach in the scan window.
    pub total_verdicts: u64,
    /// Verdicts classified as `supported`.
    pub supported: u64,
    /// Verdicts classified as `unsupported`.
    pub unsupported: u64,
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

/// Top-level wire response for `GET /admin/coach-grading/summary`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachGradingSummary {
    /// Tenant the summary was computed for.
    pub tenant_id: String,
    /// Total verdicts scanned in this window.
    pub verdicts_scanned: u64,
    /// Per-coach grades sorted by score ascending (worst first — admins
    /// review the bottom of the leaderboard, not the top).
    pub grades: Vec<CoachGrade>,
}

/// Compute per-coach grades for `tenant_id`.
///
/// # Errors
///
/// Returns an error if the underlying
/// [`pierre_database::repositories::ClaimVerdictRepository::list_recent_verdicts`]
/// call fails.
pub async fn compute_coach_grades(
    repos: &Arc<RepositoryRegistry>,
    tenant_id: TenantId,
    limit: i64,
) -> AppResult<CoachGradingSummary> {
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
        let Some(coach) = v.coach_id.clone() else {
            continue;
        };
        let entry = buckets.entry(coach).or_default();
        entry.consume(v);
    }

    let mut grades: Vec<CoachGrade> = buckets
        .into_iter()
        .map(|(coach_id, b)| b.finalize(coach_id))
        .collect();
    // Worst grades first so admins see the bottom of the leaderboard.
    grades.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));

    Ok(CoachGradingSummary {
        tenant_id: tenant_id.to_string(),
        verdicts_scanned,
        grades,
    })
}

#[derive(Default)]
struct GradeBucket {
    total: u64,
    supported: u64,
    unsupported: u64,
    contradicted: u64,
    rhetorical: u64,
    unverifiable: u64,
}

impl GradeBucket {
    fn consume(&mut self, v: &ClaimVerdict) {
        self.total += 1;
        match v.status {
            ClaimStatus::Supported => self.supported += 1,
            ClaimStatus::Unsupported => self.unsupported += 1,
            ClaimStatus::Contradicted => self.contradicted += 1,
            ClaimStatus::Rhetorical => self.rhetorical += 1,
            ClaimStatus::Unverifiable => self.unverifiable += 1,
        }
    }

    fn finalize(self, coach_id: String) -> CoachGrade {
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
        CoachGrade {
            coach_id,
            total_verdicts: self.total,
            supported: self.supported,
            unsupported: self.unsupported,
            contradicted: self.contradicted,
            rhetorical: self.rhetorical,
            unverifiable: self.unverifiable,
            score,
            grade,
        }
    }
}
