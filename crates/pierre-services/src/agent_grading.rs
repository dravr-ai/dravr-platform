// ABOUTME: Phase D Sprint C14 — agent content grading derived from claim verdict history
// ABOUTME: Computes per-agent quality scores from claim_verdicts for admin review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent content grading service.
//!
//! Walks the recent `claim_verdicts` for a tenant and produces a
//! per-agent grade summarizing how often the bullshit detector
//! supported, contradicted, or flagged an agent's claims. The grade is
//! a single `0.0..=1.0` score, plus a letter grade for human display.
//!
//! Only judge-era verdicts count ([`JUDGE_ERA_AFTER`]). Before the LLM judge
//! reached production an inconclusive claim settled as `Unsupported`, which
//! mostly recorded what the corpus did not contain, so the two periods measure
//! different things and do not add up to one grade.
//!
//! A verdict support marked as a false positive counts for nothing: the flag it
//! raised was noise.
//!
//! Pure-read; the data lives in `claim_verdicts`. No background
//! worker, no schema changes — recomputed on each admin tab mount.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_core::admin::scope::AdminScope;
use pierre_database::AgentRepos;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, VerdictDisposition};

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

/// Hard cap on how many recent verdicts the grading scan considers.
pub const MAX_VERDICTS_SCANNED: i64 = 1000;
/// Default page size when callers omit `limit`.
pub const DEFAULT_VERDICT_LIMIT: i64 = 500;
/// Minimum sample size for an agent grade to be considered confident.
const MIN_SAMPLE_FOR_CONFIDENT_GRADE: u64 = 3;

/// The last day before grades count a verdict, as the API reports it: the day
/// `eb43745d7` made the LLM judge reachable in production.
pub const JUDGE_ERA_AFTER: &str = "2026-08-11";

/// The first instant of the day after [`JUDGE_ERA_AFTER`], 2026-08-12T00:00:00Z.
const JUDGE_ERA_STARTS_AT_UNIX: i64 = 1_786_492_800;

/// `true` for a verdict grades count, by [`JUDGE_ERA_AFTER`].
fn is_judge_era(verdict: &ClaimVerdict) -> bool {
    verdict.created_at.timestamp() >= JUDGE_ERA_STARTS_AT_UNIX
}

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
    /// The agent's title. `None` when the tenant can no longer resolve the id,
    /// as for an agent since deleted.
    pub agent_name: Option<String>,
    /// Total judge-era verdicts attributed to this agent in the scan window.
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
    /// Verdicts support marked as a false positive (excluded from score
    /// weighting, whatever their status).
    pub false_positives: u64,
    /// Score in `0.0..=1.0`. Higher is better.
    pub score: f32,
    /// Letter grade derived from `score` and the verdicts behind it.
    pub grade: LetterGrade,
}

/// Top-level wire response for `GET /admin/agent-grading/summary`.
#[derive(Debug, Clone, Serialize)]
pub struct AgentGradingSummary {
    /// The tenants the summary was computed over.
    pub scope: AdminScope,
    /// Total verdicts scanned in this window, of every era.
    pub verdicts_scanned: u64,
    /// Last day whose verdicts do not count; see [`JUDGE_ERA_AFTER`].
    pub judge_era_after: String,
    /// Scanned verdicts left out for predating the judge.
    pub verdicts_before_judge_era: u64,
    /// Judge-era verdicts with no agent recorded, so in no grade below.
    pub unattributed_verdicts: u64,
    /// Per-agent grades sorted by score ascending (worst first — admins
    /// review the bottom of the leaderboard, not the top).
    pub grades: Vec<AgentGrade>,
}

/// Compute per-agent grades over `scope`.
///
/// # Errors
///
/// Returns an error if the underlying
/// [`pierre_database::repositories::ClaimVerdictRepository::list_recent_verdicts`]
/// call fails.
pub async fn compute_agent_grades(
    repos: &AgentRepos,
    scope: AdminScope,
    limit: i64,
) -> AppResult<AgentGradingSummary> {
    let scan_limit = limit.clamp(1, MAX_VERDICTS_SCANNED);
    let verdicts = repos
        .claim_verdicts
        .list_recent_verdicts(scope, scan_limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for coach grading");
            AppError::internal(format!("Failed to list verdicts: {e}"))
        })?;

    let mut summary = grades_of(scope, &verdicts);
    let ids: BTreeSet<&str> = summary.grades.iter().map(|g| g.agent_id.as_str()).collect();
    let names = agent_titles(repos, &verdicts, ids).await?;
    for grade in &mut summary.grades {
        grade.agent_name = names.get(&grade.agent_id).cloned();
    }
    Ok(summary)
}

/// The grades `verdicts` add up to, with no agent titles resolved yet.
fn grades_of(scope: AdminScope, verdicts: &[ClaimVerdict]) -> AgentGradingSummary {
    let mut buckets: HashMap<&str, GradeBucket> = HashMap::new();
    let mut before_judge_era = 0;
    let mut unattributed = 0;
    for v in verdicts {
        if !is_judge_era(v) {
            before_judge_era += 1;
        } else if let Some(agent) = &v.agent_id {
            buckets.entry(agent).or_default().consume(v);
        } else {
            unattributed += 1;
        }
    }

    let mut grades: Vec<AgentGrade> = buckets
        .into_iter()
        .map(|(agent_id, b)| b.finalize(agent_id.to_owned()))
        .collect();
    // Worst grades first so admins see the bottom of the leaderboard, and by id
    // among equals so a refresh does not reorder the table.
    grades.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });

    AgentGradingSummary {
        scope,
        verdicts_scanned: u64::try_from(verdicts.len()).unwrap_or(u64::MAX),
        judge_era_after: JUDGE_ERA_AFTER.to_owned(),
        verdicts_before_judge_era: before_judge_era,
        unattributed_verdicts: unattributed,
        grades,
    }
}

/// The titles of the agents `ids` name, each looked up in the tenant of a verdict
/// that names it, which sees its own agents and the system agents. An id that
/// tenant cannot resolve is simply absent.
///
/// # Errors
///
/// Returns an error if an agent lookup fails.
pub(crate) async fn agent_titles(
    repos: &AgentRepos,
    verdicts: &[ClaimVerdict],
    ids: BTreeSet<&str>,
) -> AppResult<HashMap<String, String>> {
    // The first verdict naming each id, found in one pass over the scan.
    let mut tenant_of: HashMap<&str, &str> = HashMap::with_capacity(ids.len());
    for verdict in verdicts {
        if let Some(id) = verdict.agent_id.as_deref().filter(|id| ids.contains(id)) {
            tenant_of.entry(id).or_insert(verdict.tenant_id.as_str());
        }
    }
    let mut titles = HashMap::with_capacity(ids.len());
    for id in ids {
        let Some(tenant_id) = tenant_of
            .get(id)
            .and_then(|tenant| TenantId::parse_str(tenant).ok())
        else {
            continue;
        };
        if let Some(agent) = repos.agents.get_in_tenant(id, tenant_id).await? {
            titles.insert(id.to_owned(), agent.title);
        }
    }
    Ok(titles)
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
    false_positives: u64,
}

impl GradeBucket {
    fn consume(&mut self, v: &ClaimVerdict) {
        self.total += 1;
        if v.disposition == Some(VerdictDisposition::FalsePositive) {
            self.false_positives += 1;
            return;
        }
        match v.status {
            ClaimStatus::Supported => self.supported += 1,
            // Training-prescription advice has no evidence corpus to cite, so an
            // `Unsupported` verdict there is expected — not a quality failure.
            // Mirror the user-facing banner carve-out (chat-pipeline
            // verification.rs) and exclude it from the scored denominator
            // instead of penalizing advice-heavy agents.
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
        // - unsupported adds 0.25 to the numerator and 1 to the denominator, so
        //   it pulls a higher score down and lifts one below 0.25
        // - rhetorical / unverifiable / false positives excluded from denominator
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
        // The sample is what the score was computed from: verdicts it left out
        // cannot make a neutral 0.5 confident.
        let grade = LetterGrade::from_score(score, scored_total);
        AgentGrade {
            agent_id,
            agent_name: None,
            total_verdicts: self.total,
            supported: self.supported,
            unsupported: self.unsupported,
            unsupported_prescription: self.unsupported_prescription,
            contradicted: self.contradicted,
            rhetorical: self.rhetorical,
            unverifiable: self.unverifiable,
            false_positives: self.false_positives,
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
    for v in verdicts.iter().filter(|v| is_judge_era(v)) {
        bucket.consume(v);
    }
    bucket.finalize(agent_id)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use pierre_core::admin::scope::AdminScope;
    use pierre_core::transport::TransportPolicy;
    use pierre_memory::{
        ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictDisposition,
        VerdictLayer,
    };

    use super::{grade_from_verdicts, grades_of, LetterGrade};

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("test timestamp parses")
            .to_utc()
    }

    fn verdict(agent: Option<&str>, status: ClaimStatus, created_at: &str) -> ClaimVerdict {
        ClaimVerdict {
            id: "v".to_owned(),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            agent_id: agent.map(ToOwned::to_owned),
            conversation_id: None,
            message_id: None,
            claim_text: "claim".to_owned(),
            category: ClaimCategory::Nutrition,
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 1.0,
            layer_fired: VerdictLayer::Judge,
            explanation: None,
            evidence_refs: None,
            created_at: at(created_at),
            disposition: None,
            disposition_reason: None,
            disposition_note: None,
            disposed_by: None,
            disposed_at: None,
            transport_policy: TransportPolicy::AnyTransport,
        }
    }

    const IN_ERA: &str = "2026-10-01T12:00:00Z";

    #[test]
    fn grades_count_verdicts_from_the_day_after_the_judge_landed() {
        let verdicts = [
            verdict(Some("a"), ClaimStatus::Unsupported, "2026-06-01T00:00:00Z"),
            verdict(Some("a"), ClaimStatus::Unsupported, "2026-08-11T23:59:59Z"),
            verdict(Some("a"), ClaimStatus::Supported, "2026-08-12T00:00:00Z"),
            verdict(None, ClaimStatus::Unsupported, "2026-06-01T00:00:00Z"),
        ];

        let summary = grades_of(AdminScope::AllTenants, &verdicts);

        assert_eq!(summary.verdicts_scanned, 4);
        assert_eq!(summary.verdicts_before_judge_era, 3);
        assert_eq!(summary.unattributed_verdicts, 0);
        assert_eq!(summary.grades.len(), 1);
        assert_eq!(summary.grades[0].total_verdicts, 1);
        assert_eq!(summary.grades[0].unsupported, 0);
        assert_eq!(summary.grades[0].supported, 1);
    }

    #[test]
    fn a_judge_era_verdict_with_no_agent_is_reported_not_lost() {
        let verdicts = [
            verdict(None, ClaimStatus::Supported, IN_ERA),
            verdict(None, ClaimStatus::Contradicted, IN_ERA),
            verdict(Some("a"), ClaimStatus::Supported, IN_ERA),
        ];

        let summary = grades_of(AdminScope::AllTenants, &verdicts);

        assert_eq!(summary.unattributed_verdicts, 2);
        assert_eq!(summary.grades.len(), 1);
    }

    /// Three verdicts the score leaves out used to grade a D at the neutral 0.5,
    /// which the screen counted as a failing agent.
    #[test]
    fn nothing_scored_is_provisional_however_many_verdicts_were_left_out() {
        let verdicts: Vec<ClaimVerdict> = [
            ClaimStatus::Rhetorical,
            ClaimStatus::Rhetorical,
            ClaimStatus::Unverifiable,
            ClaimStatus::Unverifiable,
        ]
        .into_iter()
        .map(|status| verdict(Some("a"), status, IN_ERA))
        .collect();

        let grade = grade_from_verdicts("a".to_owned(), &verdicts);

        assert_eq!(grade.total_verdicts, 4);
        assert_eq!(grade.grade, LetterGrade::Provisional);
    }

    #[test]
    fn a_verdict_marked_false_positive_does_not_move_the_score() {
        let mut dismissed = verdict(Some("a"), ClaimStatus::Contradicted, IN_ERA);
        dismissed.disposition = Some(VerdictDisposition::FalsePositive);
        let verdicts = [
            dismissed,
            verdict(Some("a"), ClaimStatus::Supported, IN_ERA),
            verdict(Some("a"), ClaimStatus::Supported, IN_ERA),
            verdict(Some("a"), ClaimStatus::Supported, IN_ERA),
        ];

        let grade = grade_from_verdicts("a".to_owned(), &verdicts);

        assert_eq!(grade.false_positives, 1);
        assert_eq!(grade.contradicted, 0);
        assert!((grade.score - 1.0).abs() < f32::EPSILON, "{}", grade.score);
        assert_eq!(grade.grade, LetterGrade::A);
    }

    #[test]
    fn equal_scores_sort_by_agent_id() {
        let verdicts = [
            verdict(Some("c"), ClaimStatus::Supported, IN_ERA),
            verdict(Some("a"), ClaimStatus::Supported, IN_ERA),
            verdict(Some("b"), ClaimStatus::Supported, IN_ERA),
        ];

        let summary = grades_of(AdminScope::AllTenants, &verdicts);
        let order: Vec<&str> = summary.grades.iter().map(|g| g.agent_id.as_str()).collect();

        assert_eq!(order, ["a", "b", "c"]);
    }
}
