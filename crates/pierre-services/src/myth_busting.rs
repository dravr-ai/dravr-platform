// ABOUTME: Phase D Sprint C13 — myth-busting summary service over claim verdicts
// ABOUTME: Lists a tenant's recent flagged claims with their dates, agents and categories for admin review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Myth-busting summary service.
//!
//! The claim verdict pipeline writes one row per agent claim
//! it inspects. This service runs purely on read: it reads the most
//! recent verdicts, keeps the flagged ones ([`is_flagged`]), and rolls them
//! up by claim, by agent and by category.
//!
//! A claim is a whole sentence of an LLM reply, so the same words rarely come
//! back and an occurrence count detects no pattern on its own. The claim list is
//! therefore ordered by date: it answers "what was flagged lately", and the dates
//! say how far back the window reaches. Two verdicts count as one claim when they
//! read the same once case, punctuation and spacing are set aside.
//!
//! No background worker is spawned — Phase D's "myth-busting worker"
//! framing is about analysis, not periodic batch jobs. Computing on
//! read is bounded by `MAX_VERDICTS_SCANNED` and lets the admin tab
//! show fresh state without a separate aggregation table.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_core::admin::scope::AdminScope;
use pierre_database::AgentRepos;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, VerdictDisposition};

use pierre_core::errors::{AppError, AppResult};

use crate::agent_grading::agent_titles;

/// Hard cap on how many recent verdicts the summary considers in a
/// single call. Above this we'd want a dedicated aggregation table.
pub const MAX_VERDICTS_SCANNED: i64 = 500;

/// Default page size when callers omit `limit`.
pub const DEFAULT_VERDICT_LIMIT: i64 = 500;

/// Top-N pattern bucket emitted by the summary.
const TOP_N: usize = 10;

/// One flagged claim in the scanned window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlaggedClaim {
    /// The claim as it was last said, cut to its first 120 characters.
    pub claim_excerpt: String,
    /// Verdict status of the latest occurrence.
    pub status: String,
    /// Pipeline layer that settled the latest occurrence.
    pub layer_fired: String,
    /// Category of the latest occurrence.
    pub category: String,
    /// Occurrences across the scanned window.
    pub occurrences: u64,
    /// Distinct agents recorded on this claim; verdicts with no agent add none.
    pub agent_count: u64,
    /// Oldest occurrence in the window, RFC3339.
    pub first_seen_at: String,
    /// Newest occurrence in the window, RFC3339.
    pub last_seen_at: String,
}

/// Aggregated stat for an agent with recurring unsupported claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPattern {
    /// Agent identifier from `claim_verdicts.agent_id`.
    pub agent_id: String,
    /// The agent's title. `None` when the tenant can no longer resolve the id,
    /// as for an agent since deleted.
    pub agent_name: Option<String>,
    /// Total unsupported/contradicted claims attributed to this agent.
    pub unsupported_total: u64,
    /// Distinct claim categories the agent has flagged on.
    pub categories: Vec<String>,
}

/// Aggregated stat for a claim category (`nutrition`, `physiological`, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryPattern {
    /// Category `snake_case` identifier.
    pub category: String,
    /// Total flagged claims in this category.
    pub flagged_total: u64,
    /// Distinct agents that touched this category.
    pub agent_count: u64,
}

/// Top-level wire response for `GET /admin/myth-busting/summary`.
#[derive(Debug, Clone, Serialize)]
pub struct MythBustingSummary {
    /// The tenants the summary was computed over.
    pub scope: AdminScope,
    /// Total verdicts scanned in this window (clamped to `MAX_VERDICTS_SCANNED`).
    pub verdicts_scanned: u64,
    /// Oldest verdict scanned, RFC3339. `None` when none was.
    pub window_start: Option<String>,
    /// Newest verdict scanned, RFC3339. `None` when none was.
    pub window_end: Option<String>,
    /// Total flagged verdicts in the window; see [`is_flagged`].
    pub flagged_total: u64,
    /// How many different claims those verdicts make.
    pub distinct_claims: u64,
    /// The most recently flagged claims, newest first, at most [`TOP_N`].
    pub recent_claims: Vec<FlaggedClaim>,
    /// Top agents by unsupported-claim count, desc.
    pub top_agents: Vec<AgentPattern>,
    /// Top categories by flagged count, desc.
    pub top_categories: Vec<CategoryPattern>,
}

/// `true` for a verdict this screen counts as flagged: one the athlete was
/// warned about, a contradicted claim or an unsupported one unless it is a
/// training prescription (advice has no corpus to cite), and that support has
/// not since dismissed as a false positive.
///
/// The screen labels this set "warned". It is narrower than the claim verdict
/// health's `flagged`, every unsupported or contradicted verdict, which
/// [`pierre_database::repositories::claim_verdicts::VerdictHealthTotals`] counts
/// because triage reads the excluded rows too.
fn is_flagged(verdict: &ClaimVerdict) -> bool {
    let warned_status = match verdict.status {
        ClaimStatus::Contradicted => true,
        ClaimStatus::Unsupported => verdict.category != ClaimCategory::TrainingPrescription,
        ClaimStatus::Supported | ClaimStatus::Rhetorical | ClaimStatus::Unverifiable => false,
    };
    warned_status && verdict.disposition != Some(VerdictDisposition::FalsePositive)
}

/// Compute the myth-busting summary over `scope`.
///
/// Reads up to `limit.min(MAX_VERDICTS_SCANNED)` recent verdicts, keeps the
/// flagged ones and rolls them up. Pure-read; safe to call on every admin
/// dashboard mount.
///
/// # Errors
///
/// Returns an error if the underlying
/// [`pierre_database::repositories::ClaimVerdictRepository::list_recent_verdicts`]
/// call or an agent lookup fails.
pub async fn compute_summary(
    repos: &AgentRepos,
    scope: AdminScope,
    limit: i64,
) -> AppResult<MythBustingSummary> {
    let scan_limit = limit.clamp(1, MAX_VERDICTS_SCANNED);
    let verdicts = repos
        .claim_verdicts
        .list_recent_verdicts(scope, scan_limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for myth-busting summary");
            AppError::internal(format!("Failed to list verdicts: {e}"))
        })?;

    let mut summary = summarize(scope, &verdicts);
    let ids: BTreeSet<&str> = summary
        .top_agents
        .iter()
        .map(|a| a.agent_id.as_str())
        .collect();
    let names = agent_titles(repos, &verdicts, ids).await?;
    for agent in &mut summary.top_agents {
        agent.agent_name = names.get(&agent.agent_id).cloned();
    }
    Ok(summary)
}

/// The summary `verdicts` add up to, with no agent titles resolved yet.
fn summarize(scope: AdminScope, verdicts: &[ClaimVerdict]) -> MythBustingSummary {
    let flagged: Vec<&ClaimVerdict> = verdicts.iter().filter(|v| is_flagged(v)).collect();
    let (recent_claims, distinct_claims) = recent_flagged_claims(&flagged);
    let timestamps = verdicts.iter().map(|v| v.created_at);

    MythBustingSummary {
        scope,
        verdicts_scanned: usize_to_u64(verdicts.len()),
        window_start: timestamps.clone().min().map(timestamp),
        window_end: timestamps.max().map(timestamp),
        flagged_total: usize_to_u64(flagged.len()),
        distinct_claims,
        recent_claims,
        top_agents: top_agent_patterns(&flagged),
        top_categories: top_category_patterns(&flagged),
    }
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Domain-guaranteed lossless widening of a `usize` row count to `u64`.
/// Verdict counts will never approach `u64::MAX`, so on platforms where
/// `usize == u32` the widening is trivially safe.
fn usize_to_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn truncate_claim(text: &str) -> String {
    const MAX: usize = 120;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(MAX).collect();
    out.push('…');
    out
}

/// What makes two verdicts the same claim: the words, in order, whatever their
/// case, punctuation or spacing. The claim is a sentence the LLM wrote, so this
/// is the difference that varies between two sayings of it.
fn claim_key(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The [`TOP_N`] most recently flagged claims, newest first, and how many
/// different claims there were in all.
fn recent_flagged_claims(flagged: &[&ClaimVerdict]) -> (Vec<FlaggedClaim>, u64) {
    struct Bucket<'verdict> {
        latest: &'verdict ClaimVerdict,
        first_seen: DateTime<Utc>,
        occurrences: u64,
        agents: HashSet<&'verdict str>,
    }
    let mut buckets: HashMap<String, Bucket<'_>> = HashMap::new();
    for v in flagged {
        let entry = buckets.entry(claim_key(&v.claim_text)).or_insert(Bucket {
            latest: v,
            first_seen: v.created_at,
            occurrences: 0,
            agents: HashSet::new(),
        });
        entry.occurrences += 1;
        if let Some(agent) = &v.agent_id {
            entry.agents.insert(agent);
        }
        if v.created_at > entry.latest.created_at {
            entry.latest = v;
        }
        entry.first_seen = entry.first_seen.min(v.created_at);
    }
    let distinct = usize_to_u64(buckets.len());
    let mut ordered: Vec<(String, Bucket<'_>)> = buckets.into_iter().collect();
    // The key breaks a tie on the date, so a refresh shows the same rows.
    ordered.sort_by(|(key_a, a), (key_b, b)| {
        b.latest
            .created_at
            .cmp(&a.latest.created_at)
            .then_with(|| key_a.cmp(key_b))
    });
    let claims = ordered
        .into_iter()
        .take(TOP_N)
        .map(|(_, b)| FlaggedClaim {
            claim_excerpt: truncate_claim(&b.latest.claim_text),
            status: b.latest.status.as_str().to_owned(),
            layer_fired: b.latest.layer_fired.as_str().to_owned(),
            category: b.latest.category.as_str().to_owned(),
            occurrences: b.occurrences,
            agent_count: usize_to_u64(b.agents.len()),
            first_seen_at: timestamp(b.first_seen),
            last_seen_at: timestamp(b.latest.created_at),
        })
        .collect();
    (claims, distinct)
}

fn top_agent_patterns(flagged: &[&ClaimVerdict]) -> Vec<AgentPattern> {
    struct Bucket {
        unsupported_total: u64,
        categories: BTreeSet<String>,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    for v in flagged {
        let Some(agent) = v.agent_id.clone() else {
            continue;
        };
        let entry = buckets.entry(agent).or_insert_with(|| Bucket {
            unsupported_total: 0,
            categories: BTreeSet::new(),
        });
        entry.unsupported_total += 1;
        entry.categories.insert(v.category.as_str().to_owned());
    }
    let mut patterns: Vec<AgentPattern> = buckets
        .into_iter()
        .map(|(agent_id, b)| AgentPattern {
            agent_id,
            agent_name: None,
            unsupported_total: b.unsupported_total,
            categories: b.categories.into_iter().collect(),
        })
        .collect();
    patterns.sort_by(|a, b| {
        Reverse(a.unsupported_total)
            .cmp(&Reverse(b.unsupported_total))
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
    patterns.truncate(TOP_N);
    patterns
}

fn top_category_patterns(flagged: &[&ClaimVerdict]) -> Vec<CategoryPattern> {
    struct Bucket {
        flagged_total: u64,
        agents: HashSet<String>,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    for v in flagged {
        let key = v.category.as_str().to_owned();
        let entry = buckets.entry(key).or_insert_with(|| Bucket {
            flagged_total: 0,
            agents: HashSet::new(),
        });
        entry.flagged_total += 1;
        if let Some(agent) = &v.agent_id {
            entry.agents.insert(agent.clone());
        }
    }
    let mut patterns: Vec<CategoryPattern> = buckets
        .into_iter()
        .map(|(category, b)| CategoryPattern {
            category,
            flagged_total: b.flagged_total,
            agent_count: usize_to_u64(b.agents.len()),
        })
        .collect();
    patterns.sort_by(|a, b| {
        Reverse(a.flagged_total)
            .cmp(&Reverse(b.flagged_total))
            .then_with(|| a.category.cmp(&b.category))
    });
    patterns.truncate(TOP_N);
    patterns
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

    use super::summarize;

    fn verdict(text: &str, status: ClaimStatus, created_at: &str) -> ClaimVerdict {
        ClaimVerdict {
            id: "v".to_owned(),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            agent_id: Some("agent".to_owned()),
            conversation_id: None,
            message_id: None,
            claim_text: text.to_owned(),
            category: ClaimCategory::Nutrition,
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 1.0,
            layer_fired: VerdictLayer::Judge,
            explanation: None,
            evidence_refs: None,
            created_at: DateTime::parse_from_rfc3339(created_at)
                .expect("test timestamp parses")
                .with_timezone(&Utc),
            disposition: None,
            disposition_reason: None,
            disposition_note: None,
            disposed_by: None,
            disposed_at: None,
            transport_policy: TransportPolicy::AnyTransport,
        }
    }

    const JUNE: &str = "2026-06-01T08:00:00Z";
    const JULY: &str = "2026-07-01T08:00:00Z";
    const AUGUST: &str = "2026-08-01T08:00:00Z";

    #[test]
    fn the_same_words_are_one_claim_whatever_their_case_and_punctuation() {
        let verdicts = [
            verdict("Protein cures tendinitis.", ClaimStatus::Contradicted, JUNE),
            verdict(
                "protein  cures tendinitis!",
                ClaimStatus::Unsupported,
                AUGUST,
            ),
            verdict("Protein cures nothing.", ClaimStatus::Contradicted, JULY),
        ];

        let summary = summarize(AdminScope::AllTenants, &verdicts);

        assert_eq!(summary.flagged_total, 3);
        assert_eq!(summary.distinct_claims, 2);
        let claim = &summary.recent_claims[0];
        assert_eq!(claim.occurrences, 2);
        assert_eq!(claim.claim_excerpt, "protein  cures tendinitis!");
        assert_eq!(claim.status, "unsupported");
        assert_eq!(claim.first_seen_at, JUNE);
        assert_eq!(claim.last_seen_at, AUGUST);
    }

    #[test]
    fn claims_run_newest_first_and_the_distinct_count_outlives_the_cap() {
        let texts: Vec<String> = (0..12).map(|i| format!("claim number {i:02}")).collect();
        let mut verdicts = Vec::new();
        for (day, text) in (10..).zip(&texts) {
            let created_at = format!("2026-09-{day}T00:00:00Z");
            verdicts.push(verdict(text, ClaimStatus::Contradicted, &created_at));
        }

        let summary = summarize(AdminScope::AllTenants, &verdicts);

        assert_eq!(summary.distinct_claims, 12);
        assert_eq!(summary.recent_claims.len(), 10, "the list is capped");
        assert_eq!(summary.recent_claims[0].claim_excerpt, "claim number 11");
        assert_eq!(summary.recent_claims[9].claim_excerpt, "claim number 02");
    }

    /// Every row at one instant, with one occurrence each: the order must come
    /// from the claim, not from a seeded hash map, or the list reshuffles on
    /// every refresh.
    #[test]
    fn ties_order_the_same_way_every_time() {
        let verdicts: Vec<ClaimVerdict> = (0..30)
            .map(|i| verdict(&format!("claim {i:02}"), ClaimStatus::Contradicted, JUNE))
            .collect();

        let first = summarize(AdminScope::AllTenants, &verdicts);

        for _ in 0..5 {
            let again = summarize(AdminScope::AllTenants, &verdicts);
            let excerpts = |s: &super::MythBustingSummary| -> Vec<String> {
                s.recent_claims
                    .iter()
                    .map(|c| c.claim_excerpt.clone())
                    .collect()
            };
            assert_eq!(excerpts(&first), excerpts(&again));
        }
    }

    #[test]
    fn the_window_states_how_far_back_the_scan_reached() {
        let verdicts = [
            verdict("One thing.", ClaimStatus::Supported, AUGUST),
            verdict("Other thing.", ClaimStatus::Supported, JUNE),
        ];

        let summary = summarize(AdminScope::AllTenants, &verdicts);

        assert_eq!(summary.verdicts_scanned, 2);
        assert_eq!(summary.window_start.as_deref(), Some(JUNE));
        assert_eq!(summary.window_end.as_deref(), Some(AUGUST));
        assert_eq!(summary.flagged_total, 0);
    }

    #[test]
    fn what_the_athlete_is_never_warned_about_is_not_flagged() {
        let mut prescription = verdict("Run easy today.", ClaimStatus::Unsupported, JUNE);
        prescription.category = ClaimCategory::TrainingPrescription;
        let mut dismissed = verdict("Dismissed by support.", ClaimStatus::Contradicted, JUNE);
        dismissed.disposition = Some(VerdictDisposition::FalsePositive);
        let mut contradicted_prescription =
            verdict("Run 400 km today.", ClaimStatus::Contradicted, JUNE);
        contradicted_prescription.category = ClaimCategory::TrainingPrescription;

        let summary = summarize(
            AdminScope::AllTenants,
            &[prescription, dismissed, contradicted_prescription],
        );

        assert_eq!(summary.flagged_total, 1);
        assert_eq!(summary.recent_claims[0].claim_excerpt, "Run 400 km today.");
    }
}
