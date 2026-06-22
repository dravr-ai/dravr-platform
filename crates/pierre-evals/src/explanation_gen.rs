// ABOUTME: The explanation stage of the bullshit detector — builds the user-facing explanation for a verdict
// ABOUTME: Pure Rust templater for Phase A; LLM rewriting deferred to Phase D
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Explanation Generator
//!
//! Given a [`VerdictOutcome`] and the originating claim, produces a short
//! human-readable explanation suitable for the "Ask me about this claim"
//! UX drawer. Phase A uses a deterministic template so the explanation is
//! always available even when no LLM is configured; Phase D will add an
//! optional LLM-rewriting pass for better prose.

use crate::claim_extractor::ExtractedClaim;
use crate::verdict_engine::VerdictOutcome;
use pierre_memory::ClaimStatus;

/// Render the user-facing explanation for a verdict.
#[must_use]
pub fn render(claim: &ExtractedClaim, outcome: &VerdictOutcome) -> String {
    let header = match outcome.status {
        ClaimStatus::Supported => "Supported",
        ClaimStatus::Unsupported => "Unsupported",
        ClaimStatus::Contradicted => "Contradicted",
        ClaimStatus::Rhetorical => "Rhetorical",
        ClaimStatus::Unverifiable => "Unverifiable",
    };

    let mut out = format!(
        "**{header}** ({}, {})\n\n> {}\n\n{}",
        claim.category.as_str(),
        outcome.evidence_strength.as_str(),
        claim.text,
        outcome.explanation,
    );

    if let Some(refs) = &outcome.evidence_refs {
        out.push_str("\n\nReferences: ");
        out.push_str(refs);
    }

    out
}
