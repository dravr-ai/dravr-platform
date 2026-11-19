// ABOUTME: What a platform post is for — the purpose its once-only claim row is stored under
// ABOUTME: Shared by the post workers that take the claim and the readers that ask when one landed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The purpose of a server-initiated agent post.
//!
//! Each post the platform makes on its own initiative is claimed under its
//! purpose in `platform_post_claims`, so one definition serves the workers
//! that take the claim and every reader that asks when that post landed.

/// What a platform post is for: the key its claim is taken under.
///
/// An athlete gets each such post once however many triggers fire for it —
/// the first-sync post once ever, a check-in once per day its cadence posts
/// on, a landing reaction once per local day or once per ride, as the
/// `activity_landed.frequency` admin parameter says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostPurpose {
    /// The agent's first post once the athlete's data lands.
    FirstSync,
    /// The agent's check-in on the cadence the athlete chose.
    CheckIn,
    /// The agent's reaction to a workout a live sync just landed.
    ActivityLanded,
}

impl PostPurpose {
    /// The subject a purpose owed once per athlete (or once per period) is
    /// claimed under. A purpose claimed per item — a landing reaction per
    /// ride — names the item instead.
    pub const NO_SUBJECT: &'static str = "";

    /// The purpose as its claim row stores it. Persisted values: rows already
    /// carry them, so they never change.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstSync => "first_sync",
            Self::CheckIn => "check_in",
            Self::ActivityLanded => "activity_landed",
        }
    }
}
