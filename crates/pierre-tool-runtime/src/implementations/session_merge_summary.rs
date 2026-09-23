// ABOUTME: LLM-facing summary of the session merge behind an activity list
// ABOUTME: Which recordings each session combines, from which providers, and what they added
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_providers::deduplication::FragmentReport;
use serde::Serialize;

/// LLM-facing summary of the session merge that produced an activity slice.
///
/// Counterpart of [`pierre_providers::deduplication::FragmentReport`] — the
/// provider-side type carries `chrono::DateTime` values and the full sport
/// enum, this serializes them to strings so the JSON shape stays portable
/// across MCP / A2A / REST consumers and stable across cageux upgrades.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct FragmentDedupSummary {
    /// Recordings the merge saw (one per row the providers returned).
    pub raw_count: usize,
    /// Distinct training sessions after merging each group into one.
    pub session_count: usize,
    /// One entry per multi-row group; absent when no fragments were detected
    /// even though `fragment_dedup` itself is present.
    pub groups: Vec<FragmentGroupSummary>,
    /// Pre-formatted human-readable advice line the LLM should echo to the
    /// user when reporting counts. Example: "20 GPS recordings represent 12
    /// distinct sessions; the rest are likely re-uploads or auto-splits".
    pub advice: String,
}

/// One group of recordings merged into a single session, surfaced to the LLM.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct FragmentGroupSummary {
    /// Activity id selected as the canonical session for this group.
    pub canonical_id: String,
    /// All member ids, canonical included — preserved for callers that want
    /// to render or audit the grouping decision.
    pub fragment_ids: Vec<String>,
    /// Providers that recorded the session, the canonical row's first.
    pub providers: Vec<String>,
    /// Fields the session took from another recording, each named with the
    /// provider it came from.
    pub filled_fields: Vec<FilledFieldSummary>,
    /// Sport type of the canonical member (RFC-safe rendering of the enum).
    pub sport_type: String,
    /// Earliest start time across the group, ISO 8601.
    pub window_start: String,
    /// Latest end time across the group, ISO 8601.
    pub window_end: String,
}

/// One field a merged session took from another recording of it.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct FilledFieldSummary {
    /// Activity field name.
    pub field: String,
    /// Provider of the recording the value came from.
    pub provider: String,
}

impl FragmentDedupSummary {
    /// Build an LLM-facing summary from a provider-side [`FragmentReport`].
    /// Returns `None` when the report contains no fragment groups — the
    /// caller skips serializing `fragment_dedup` in that case so the JSON
    /// shape stays compact for the 95% of queries that have no fragments.
    pub(crate) fn from_report(report: &FragmentReport) -> Option<Self> {
        if !report.has_fragments() {
            return None;
        }
        let groups = report
            .groups
            .iter()
            .map(|g| FragmentGroupSummary {
                canonical_id: g.canonical_id.clone(),
                fragment_ids: g.fragment_ids.clone(),
                providers: g.providers.clone(),
                filled_fields: g
                    .filled_fields
                    .iter()
                    .map(|f| FilledFieldSummary {
                        field: f.field.to_owned(),
                        provider: f.provider.clone(),
                    })
                    .collect(),
                sport_type: format!("{:?}", g.sport_type),
                window_start: g.window_start.to_rfc3339(),
                window_end: g.window_end.to_rfc3339(),
            })
            .collect();
        let advice = format!(
            "Session merge: {raw} recordings were merged into {sessions} distinct training \
             sessions (Garmin auto-splits, dual-device recordings, re-uploads, or the same \
             workout from two providers). Every activity returned is already one session, with \
             the fields its other recordings added. When reporting counts to the user, cite \
             session_count ({sessions}), not raw_count ({raw}). The groups list names which \
             recordings each session combines.",
            raw = report.raw_count,
            sessions = report.session_count,
        );
        Some(Self {
            raw_count: report.raw_count,
            session_count: report.session_count,
            groups,
            advice,
        })
    }
}
