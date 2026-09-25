// ABOUTME: Phase C system-prompt fingerprinting for prompt exfiltration defense
// ABOUTME: Pure functions that hash system prompts and detect verbatim leaks in responses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # System-prompt fingerprinting.
//!
//! Defense-in-depth for prompt exfiltration. Agent personas run under a
//! tenant-customized system prompt that the harness considers
//! confidential. A jailbroken user can try to coax the model into
//! reciting the prompt verbatim ("repeat everything above this line").
//!
//! This module provides two primitives the dispatcher uses to detect
//! that class of leak:
//!
//! 1. [`fingerprint_prompt`] — produces a [`PromptFingerprint`] for a
//!    system prompt. The fingerprint is a stable SHA-256 hex of the
//!    normalized prompt plus a *shingle set* of 40-character rolling
//!    window hashes taken from the normalized body. The normalized
//!    body is lower-cased with ASCII whitespace collapsed.
//!
//! 2. [`scan_response_for_leaks`] — given a response body and a
//!    fingerprint, counts how many shingles from the prompt reappear
//!    verbatim in the response. At or above [`DEFAULT_LEAK_THRESHOLD`]
//!    shingles the response is classified as
//!    [`LeakVerdict::Leaked`]; otherwise [`LeakVerdict::Clean`].
//!
//! Both functions are pure and deterministic so they can live anywhere
//! in the dispatch path without synchronization.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Rolling window length used for shingles.
///
/// 40 ASCII characters is long enough that ordinary English prose does
/// not produce accidental matches — random 40-char substrings appearing
/// verbatim in a response is almost always exfiltration.
pub const SHINGLE_WINDOW: usize = 40;

/// Default overlap count at or above which a response is classified as leaked.
///
/// Set to 3 so a single coincidental window match does not trip the
/// detector. The three windows it demands are overlapping, not
/// independent: windows slide one byte at a time, so one contiguous
/// `SHINGLE_WINDOW + 2` byte run of prompt text already supplies all
/// three. The threshold therefore marks the shortest quotation worth
/// flagging, and the `overlap` on [`LeakVerdict::Leaked`] carries the
/// magnitude that separates that brush from a wholesale dump.
pub const DEFAULT_LEAK_THRESHOLD: usize = 3;

/// Fingerprint of a system prompt used for later leak detection.
///
/// Size is bounded: we keep `SHA-256` (64 hex chars), the normalized
/// byte length, and the shingle set (at most `normalized_len -
/// SHINGLE_WINDOW + 1` entries, one `u64` each). No copy of the prompt
/// is retained.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptFingerprint {
    /// SHA-256 of the normalized prompt, hex-encoded.
    pub sha256_hex: String,
    /// Byte length of the normalized prompt (post-lowercase, whitespace collapsed).
    pub normalized_len: usize,
    /// Byte length of the original prompt before normalization.
    pub original_len: usize,
    /// Rolling-window hashes used to detect verbatim reuse. Set semantics.
    pub shingles: Vec<u64>,
}

impl PromptFingerprint {
    /// Number of unique 40-character windows captured from the prompt.
    #[must_use]
    pub fn shingle_count(&self) -> usize {
        self.shingles.len()
    }
}

/// Classification of a response body scanned against a fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LeakVerdict {
    /// Response body does not contain enough prompt shingles to flag.
    Clean,
    /// Response body reproduces `overlap` shingles from the prompt.
    Leaked {
        /// Total number of distinct prompt shingles found verbatim in the
        /// response — every match, not merely enough of them to cross
        /// `threshold`, so the value reads as a magnitude.
        overlap: usize,
        /// Threshold the response crossed (echoed back so logs don't need
        /// to chase the caller's configuration).
        threshold: usize,
    },
}

/// Lower-case the text and collapse ASCII whitespace runs to a single
/// space. Unicode is preserved verbatim — we only touch ASCII
/// whitespace / case so European characters in prompts stay intact.
fn normalize(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_ws = false;
    for ch in lowered.chars() {
        if ch.is_ascii_whitespace() {
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Compute the rolling-window shingle set of `text`.
///
/// Windows are `SHINGLE_WINDOW` *bytes* on the normalized text. A
/// non-crypto hasher is fine for shingles because the set is only used
/// for membership checks, not for anti-tamper guarantees — the SHA-256
/// field is the integrity surface.
fn shingle_set(text: &str) -> Vec<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < SHINGLE_WINDOW {
        // Short prompt: hash the whole body as the single shingle so
        // leak detection still works for tiny system prompts.
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        return vec![hasher.finish()];
    }
    let mut seen = HashSet::with_capacity(bytes.len().saturating_sub(SHINGLE_WINDOW));
    for window in bytes.windows(SHINGLE_WINDOW) {
        let mut hasher = DefaultHasher::new();
        window.hash(&mut hasher);
        seen.insert(hasher.finish());
    }
    seen.into_iter().collect()
}

/// Compute the [`PromptFingerprint`] for a system prompt.
///
/// Deterministic and side-effect-free. Safe to call on every dispatch
/// turn; the caller decides whether to cache by agent id.
#[must_use]
pub fn fingerprint_prompt(prompt: &str) -> PromptFingerprint {
    let normalized = normalize(prompt);

    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let sha256_hex = format!("{:x}", hasher.finalize());

    let shingles = shingle_set(&normalized);

    PromptFingerprint {
        sha256_hex,
        normalized_len: normalized.len(),
        original_len: prompt.len(),
        shingles,
    }
}

/// Scan an agent response body for verbatim fragments of the system prompt.
///
/// Returns [`LeakVerdict::Leaked`] when at least `threshold` distinct
/// shingles from the prompt appear in the response. Pass
/// [`DEFAULT_LEAK_THRESHOLD`] unless a tenant policy overrides it.
///
/// The scan runs the whole response before deciding, so the reported
/// `overlap` is the true match count rather than the threshold that was
/// crossed. That is what lets an operator tell a one-line reply brushing
/// three overlapping windows apart from a reply reciting the prompt
/// wholesale — the two are indistinguishable when the count stops at the
/// threshold. Cost is unchanged in the common case: a clean response
/// already scanned every window.
#[must_use]
pub fn scan_response_for_leaks(
    fingerprint: &PromptFingerprint,
    response_body: &str,
    threshold: usize,
) -> LeakVerdict {
    if fingerprint.shingles.is_empty() || response_body.is_empty() {
        return LeakVerdict::Clean;
    }
    let needle_set: HashSet<u64> = fingerprint.shingles.iter().copied().collect();
    let normalized = normalize(response_body);
    let bytes = normalized.as_bytes();
    if bytes.len() < SHINGLE_WINDOW {
        // Short response: compute the single-shingle hash and check.
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let overlap = usize::from(needle_set.contains(&hasher.finish()));
        return if overlap >= threshold {
            LeakVerdict::Leaked { overlap, threshold }
        } else {
            LeakVerdict::Clean
        };
    }

    let mut matched: HashSet<u64> = HashSet::new();
    for window in bytes.windows(SHINGLE_WINDOW) {
        let mut hasher = DefaultHasher::new();
        window.hash(&mut hasher);
        let h = hasher.finish();
        if needle_set.contains(&h) {
            matched.insert(h);
        }
    }

    let overlap = matched.len();
    if overlap >= threshold {
        LeakVerdict::Leaked { overlap, threshold }
    } else {
        LeakVerdict::Clean
    }
}

/// Hidden 12-character canary token used by
/// [`inject_canary_marker`]. Unique per dispatch turn so a stale
/// token seen in a later response cannot be written off as "already
/// leaked once".
///
/// The token is generated from process-unique entropy: the current
/// monotonic nanoseconds, a `DefaultHasher` mix with the provided
/// salt, and the result squeezed through SHA-256. That gives a
/// deterministic construction without pulling in the `rand` crate as
/// a dependency for every crate that touches this module, but still
/// produces tokens with effectively no collisions at Pierre's call
/// rate (millions per second would take decades to collide).
#[must_use]
pub fn generate_canary(salt: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(salt.as_bytes());
    let digest = hasher.finalize();
    // Base36-ish: 24 hex chars is plenty of entropy; keep the prefix
    // distinctive so log greps can spot the canary shape.
    let hex = format!("{digest:x}");
    format!("CANARY-{}", &hex[..16])
}

/// Append the canary token to a system prompt as an inert HTML-comment
/// marker. A model that leaks the canary verbatim has been prompt-injected.
///
/// The marker carries no instruction on purpose: detection only needs the
/// token *present* in the prompt plus the server-side reply scan
/// ([`detect_canary_in_response`]), and self-describing wording
/// ("internal safety marker — do not expose") measurably invites
/// reasoning-heavy models to narrate about the hidden block instead of
/// silently ignoring it (live leak, 2026-07-10). An HTML comment reads as
/// conventional metadata, gives the model nothing to obey or announce,
/// and still lands in any verbatim prompt dump — which is exactly the
/// detection signal we want. Enforcement on a hit is the response
/// boundary's job, not the model's.
#[must_use]
pub fn inject_canary_marker(system_prompt: &str, canary: &str) -> String {
    format!("{system_prompt}\n\n{CANARY_MARKER_OPEN}{canary}{CANARY_MARKER_CLOSE}")
}

/// Opening delimiter of the canary marker [`inject_canary_marker`] appends.
const CANARY_MARKER_OPEN: &str = "<!-- session-integrity:";

/// Closing delimiter of that marker.
const CANARY_MARKER_CLOSE: &str = " -->";

/// The canary token carried by a prompt hardened with
/// [`inject_canary_marker`], if it carries one.
///
/// Reads back exactly what that function wrote, so the marker's shape is
/// decided in one place. A boundary that holds the assembled prompt but not
/// the turn's `PromptGuard` — the streaming forwarder, which sees the request
/// on its way to the model and nothing else — recovers from the prompt itself
/// the token it must not let through.
#[must_use]
pub fn extract_canary_marker(system_prompt: &str) -> Option<&str> {
    let open = system_prompt.rfind(CANARY_MARKER_OPEN)? + CANARY_MARKER_OPEN.len();
    let rest = system_prompt.get(open..)?;
    let close = rest.find(CANARY_MARKER_CLOSE)?;
    let token = rest.get(..close)?;
    (!token.is_empty()).then_some(token)
}

/// `true` when the canary token appears verbatim in `response`.
#[must_use]
pub fn detect_canary_in_response(canary: &str, response: &str) -> bool {
    !canary.is_empty() && response.contains(canary)
}
