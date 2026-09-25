// ABOUTME: Classifies a provider's raw finish_reason into complete, truncated or filtered
// ABOUTME: One reading of the vendor stop vocabularies, so no caller shows a cut-off reply as whole
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Why a provider stopped generating, reduced to what a reader must be told.
//!
//! embacle passes each vendor's `finish_reason` through verbatim, so the same
//! event arrives as `"length"` from an OpenAI-compatible API and as
//! `"MAX_TOKENS"` from Gemini. A reply that stopped on either is a fragment,
//! and a reply stopped by a safety filter is missing what the filter removed —
//! yet both come back as an ordinary `Ok`, indistinguishable from a finished
//! answer unless someone reads this field.

/// Raw stop values meaning the output hit its token budget mid-reply.
const TRUNCATED_REASONS: &[&str] = &["length", "max_tokens"];

/// Raw stop values meaning a provider-side filter cut or blocked the output.
const FILTERED_REASONS: &[&str] = &[
    "content_filter",
    "safety",
    "recitation",
    "blocklist",
    "prohibited_content",
    "spii",
    "refusal",
];

/// How a provider's generation ended, as far as the reader is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStop {
    /// The model finished on its own terms (or reported nothing unusual).
    Complete,
    /// The output hit its token budget: the reply is cut off mid-thought.
    Truncated,
    /// A provider-side content filter stopped or trimmed the output.
    Filtered,
}

impl ProviderStop {
    /// Classify a raw `finish_reason`, matching vendor spellings
    /// case-insensitively. Absent or unrecognized values are
    /// [`ProviderStop::Complete`]: only a stop the vendor names as a
    /// budget or filter stop is reported as one.
    #[must_use]
    pub fn from_finish_reason(finish_reason: Option<&str>) -> Self {
        let Some(reason) = finish_reason.map(str::trim) else {
            return Self::Complete;
        };
        let matches = |set: &[&str]| set.iter().any(|known| known.eq_ignore_ascii_case(reason));
        if matches(TRUNCATED_REASONS) {
            Self::Truncated
        } else if matches(FILTERED_REASONS) {
            Self::Filtered
        } else {
            Self::Complete
        }
    }
}
