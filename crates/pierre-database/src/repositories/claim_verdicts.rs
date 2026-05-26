// ABOUTME: Repository trait definitions for the claim verdict (Tier 5.5 bullshit detector) persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer};

/// Parameters for inserting a claim verdict via [`ClaimVerdictRepository`].
pub struct InsertClaimVerdictParams<'a> {
    /// Tenant that owns the verdict.
    pub tenant_id: TenantId,
    /// User the claim was said to.
    pub user_id: &'a str,
    /// Coach persona that authored the claim, if resolvable.
    pub coach_id: Option<&'a str>,
    /// Conversation the claim came from, if in-dispatch.
    pub conversation_id: Option<&'a str>,
    /// Message the claim was extracted from, if available.
    pub message_id: Option<&'a str>,
    /// Raw claim text.
    pub claim_text: &'a str,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Final pipeline status.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict.
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer produced the verdict.
    pub layer_fired: VerdictLayer,
    /// Optional user-facing rationale.
    pub explanation: Option<&'a str>,
    /// Optional evidence references (DOIs/PMIDs, comma-separated).
    pub evidence_refs: Option<&'a str>,
}

/// Per-status counters rolled up over a verdict scan window.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictStatusBreakdown {
    /// Claims the pipeline marked `supported`.
    pub supported: i64,
    /// Claims the pipeline marked `unsupported`.
    pub unsupported: i64,
    /// Claims the pipeline marked `contradicted`.
    pub contradicted: i64,
    /// Claims the rhetoric filter dropped as rhetorical.
    pub rhetorical: i64,
    /// Claims the pipeline could not confidently verify either way.
    pub unverifiable: i64,
}

/// One day's verdict counts, keyed by the UTC calendar date `YYYY-MM-DD`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictDailyBucket {
    /// UTC calendar date `YYYY-MM-DD`.
    pub date: String,
    /// Status breakdown for that day.
    pub counts: VerdictStatusBreakdown,
}

/// Calibration summary emitted by
/// [`ClaimVerdictRepository::aggregate_verdict_stats`].
///
/// Drives the admin "Verdict calibration" panel in the eval harness
/// tab — shows the live Tier 5.5 verdict mix and how it drifts day to
/// day over the requested window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictCalibrationStats {
    /// Window lower bound (inclusive), RFC3339 UTC.
    pub window_start: String,
    /// Number of days scanned.
    pub window_days: i64,
    /// Totals over the full window.
    pub totals: VerdictStatusBreakdown,
    /// Per-day breakdown, oldest first.
    pub daily: Vec<VerdictDailyBucket>,
}

/// Tier 5.5 claim verdict repository — persists post-LLM detector output.
#[async_trait]
pub trait ClaimVerdictRepository: Send + Sync {
    /// Persist a new claim verdict.
    async fn insert_claim_verdict(
        &self,
        params: &InsertClaimVerdictParams<'_>,
    ) -> AppResult<ClaimVerdict>;

    /// List verdicts for a conversation in chronological order (oldest first).
    async fn list_verdicts_for_conversation(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// List the most recent verdicts for a tenant, newest first. Used by the
    /// admin "flagged claims" dashboard.
    async fn list_recent_verdicts(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// Aggregate verdict counts over the last `window_days` for a tenant.
    ///
    /// Returns both the full-window totals and a per-day breakdown so
    /// the admin calibration panel can show drift. `window_days` is
    /// clamped to `1..=365` by callers.
    async fn aggregate_verdict_stats(
        &self,
        tenant_id: TenantId,
        window_days: i64,
    ) -> AppResult<VerdictCalibrationStats>;
}
