// ABOUTME: Derives the OAuth scopes a tool requires from the capability flags it declares
// ABOUTME: One derivation, no per-tool table — a table would drift from what the tools declare
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The capability → scope derivation, and the check that reads it.
//!
//! Every tool already declares what it does through
//! [`ToolCapabilities`](dravr_tronc::mcp::tool::ToolCapabilities). Four of
//! those flags describe what a *caller may do* — `READS_DATA`, `WRITES_DATA`,
//! `PROFILE`, `ADMIN_ONLY` — and those four are what
//! [`OAuthScope`](pierre_core::permissions::scopes::OAuthScope) names. So the
//! scope a tool requires is derived here rather than listed: a list of 110
//! entries would fall out of step with the flags the moment a tool changed
//! what it does, silently, and the tool would keep being reachable under a
//! grant that no longer describes it.
//!
//! The derivation inherits whatever the flags say, which is the honest
//! trade-off of deriving: a tool that under-declares its capabilities
//! under-declares its scope too. That is a reason to fix a tool's flags, not a
//! reason to keep a second list.

use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_core::permissions::scopes::OAuthScope;

/// The scopes a caller must hold to invoke a tool declaring `caps`.
///
/// A tool that neither reads, writes, nor administers requires no scope — it
/// is reachable by any authenticated caller. That is deliberate: a pure
/// calculation over arguments the caller already supplied discloses nothing
/// about the athlete, and demanding a grant for it would train integrations to
/// request more than they need.
#[must_use]
pub fn required_scopes(caps: ToolCapabilities) -> Vec<OAuthScope> {
    let profile = caps.contains(ToolCapabilities::PROFILE);
    let mut required = Vec::new();

    if caps.contains(ToolCapabilities::READS_DATA) {
        required.push(if profile {
            OAuthScope::ProfileRead
        } else {
            OAuthScope::FitnessRead
        });
    }
    if caps.contains(ToolCapabilities::WRITES_DATA) {
        required.push(if profile {
            OAuthScope::ProfileWrite
        } else {
            OAuthScope::FitnessWrite
        });
    }
    if caps.contains(ToolCapabilities::ADMIN_ONLY) {
        required.push(OAuthScope::Admin);
    }

    required
}

/// The first scope `granted` is missing for a tool declaring `caps`, or `None`
/// when the grant covers it.
///
/// Returns the missing scope rather than a bool because RFC 6750 §3.1 requires
/// the `insufficient_scope` challenge to name what was needed. A bool would
/// force the caller to recompute it, and a challenge assembled twice is a
/// challenge that can disagree with the refusal it accompanies.
#[must_use]
pub fn missing_scope(granted: &[OAuthScope], caps: ToolCapabilities) -> Option<OAuthScope> {
    required_scopes(caps)
        .into_iter()
        .find(|required| !granted.contains(required))
}
