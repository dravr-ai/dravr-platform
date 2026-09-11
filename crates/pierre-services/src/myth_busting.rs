// ABOUTME: Phase D Sprint C13 — myth-busting summary service over claim verdicts
// ABOUTME: Aggregates unsupported/contradicted claims tenant-wide for admin pattern review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Myth-busting summary service.
//!
//! The claim verdict pipeline writes one row per agent claim
//! it inspects. This service runs purely on read: it reads the most
//! recent verdicts, filters to unsupported / contradicted entries,
//! and rolls them up into the patterns admins care about (top claim
//! texts, top categories, top offending agents).
//!
//! No background worker is spawned — Phase D's "myth-busting worker"
//! framing is about analysis, not periodic batch jobs. Computing on
//! read is bounded by `MAX_VERDICTS_SCANNED` and lets the admin tab
//! show fresh state without a separate aggregation table.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_core::models::TenantId;
use pierre_database::AgentRepos;
use pierre_memory::{ClaimStatus, ClaimVerdict};

use pierre_core::errors::{AppError, AppResult};

/// Hard cap on how many recent verdicts the summary considers in a
/// single call. Above this we'd want a dedicated aggregation table.
pub const MAX_VERDICTS_SCANNED: i64 = 500;

/// Default page size when callers omit `limit`.
pub const DEFAULT_VERDICT_LIMIT: i64 = 200;

/// Top-N pattern bucket emitted by the summary.
const TOP_N: usize = 10;

/// Aggregated stat for a recurring offending claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimPattern {
    /// Truncated claim text (first 120 chars) used as the bucket key.
    pub claim_excerpt: String,
    /// Total occurrences across the scanned window.
    pub occurrences: u64,
    /// Distinct agents that emitted this claim.
    pub agent_count: u64,
    /// Most-recent occurrence as RFC3339 timestamp, or `None`.
    pub last_seen_at: Option<String>,
}

/// Aggregated stat for an agent with recurring unsupported claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPattern {
    /// Agent identifier from `claim_verdicts.agent_id`.
    pub agent_id: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MythBustingSummary {
    /// Tenant the summary was computed for.
    pub tenant_id: String,
    /// Total verdicts scanned in this window (clamped to `MAX_VERDICTS_SCANNED`).
    pub verdicts_scanned: u64,
    /// Total verdicts classified as unsupported or contradicted.
    pub flagged_total: u64,
    /// Top recurring claim texts ordered by occurrence count desc.
    pub top_claims: Vec<ClaimPattern>,
    /// Top agents by unsupported-claim count, desc.
    pub top_agents: Vec<AgentPattern>,
    /// Top categories by flagged count, desc.
    pub top_categories: Vec<CategoryPattern>,
}

/// Compute the myth-busting summary for `tenant_id`.
///
/// Reads up to `limit.min(MAX_VERDICTS_SCANNED)` recent verdicts, keeps
/// only the unsupported / contradicted entries, and rolls them up into
/// the three pattern buckets. Pure-read; safe to call on every admin
/// dashboard mount.
///
/// # Errors
///
/// Returns an error if the underlying
/// [`pierre_database::repositories::ClaimVerdictRepository::list_recent_verdicts`]
/// call fails.
pub async fn compute_summary(
    repos: &AgentRepos,
    tenant_id: TenantId,
    limit: i64,
) -> AppResult<MythBustingSummary> {
    let scan_limit = limit.clamp(1, MAX_VERDICTS_SCANNED);
    let verdicts = repos
        .claim_verdicts
        .list_recent_verdicts(tenant_id, scan_limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for myth-busting summary");
            AppError::internal(format!("Failed to list verdicts: {e}"))
        })?;

    let verdicts_scanned = usize_to_u64(verdicts.len());
    let flagged: Vec<&ClaimVerdict> = verdicts
        .iter()
        .filter(|v| {
            matches!(
                v.status,
                ClaimStatus::Unsupported | ClaimStatus::Contradicted
            )
        })
        .collect();

    Ok(MythBustingSummary {
        tenant_id: tenant_id.to_string(),
        verdicts_scanned,
        flagged_total: usize_to_u64(flagged.len()),
        top_claims: top_claim_patterns(&flagged),
        top_agents: top_agent_patterns(&flagged),
        top_categories: top_category_patterns(&flagged),
    })
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

fn top_claim_patterns(flagged: &[&ClaimVerdict]) -> Vec<ClaimPattern> {
    struct Bucket {
        occurrences: u64,
        agents: HashSet<String>,
        last_seen: Option<DateTime<Utc>>,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    for v in flagged {
        let key = truncate_claim(&v.claim_text);
        let entry = buckets.entry(key).or_insert_with(|| Bucket {
            occurrences: 0,
            agents: HashSet::new(),
            last_seen: None,
        });
        entry.occurrences += 1;
        if let Some(agent) = &v.agent_id {
            entry.agents.insert(agent.clone());
        }
        entry.last_seen = match entry.last_seen {
            Some(prev) if prev > v.created_at => Some(prev),
            _ => Some(v.created_at),
        };
    }
    let mut patterns: Vec<ClaimPattern> = buckets
        .into_iter()
        .map(|(claim_excerpt, b)| ClaimPattern {
            claim_excerpt,
            occurrences: b.occurrences,
            agent_count: usize_to_u64(b.agents.len()),
            last_seen_at: b.last_seen.map(|d| d.to_rfc3339()),
        })
        .collect();
    patterns.sort_by_key(|b| Reverse(b.occurrences));
    patterns.truncate(TOP_N);
    patterns
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
            unsupported_total: b.unsupported_total,
            categories: b.categories.into_iter().collect(),
        })
        .collect();
    patterns.sort_by_key(|b| Reverse(b.unsupported_total));
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
    patterns.sort_by_key(|b| Reverse(b.flagged_total));
    patterns.truncate(TOP_N);
    patterns
}
