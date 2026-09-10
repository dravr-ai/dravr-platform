// ABOUTME: Whether the next fortnight can be written, and what it must respect — decided in Rust, not left to the agent
// ABOUTME: Pure over the plan and the rails' readings; the refusals are as much the answer as the go-ahead

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fortnight decision.
//!
//! `generate-next-fortnight` is *gather, decide, generate*. The gather is the
//! rails already built — the readiness ladder, the compliance verdict, the
//! coverage gaps. This is the decide: a pure function over what they read,
//! answering whether the next two weeks can be written and, if so, the
//! constraints they must respect.
//!
//! It lives here rather than in the agent's prompt because a decision made in
//! prose is a decision that can be argued with. Every open rule is decided
//! here and only here:
//!
//! - **A plan with no phases is not a plan to extend.** The phases are what a
//!   fortnight is written *against*; without them there is no target to hold
//!   the weeks to and nothing to progress toward.
//! - **A block-level readiness refuses the write outright.** At `P0` the
//!   ladder allows recovery alone, and a fortnight of recovery is not a
//!   fortnight — it is a conversation about why. The agent is told to have
//!   that conversation instead of writing two weeks it would immediately
//!   have to unwrite.
//! - **A plan that already covers the fortnight is not extended.** Writing
//!   over weeks the athlete can already see is how a plan acquires two
//!   answers for one Tuesday.
//! - **A guided walk owns the conversation while it runs.** Slash commands
//!   dispatch during a walk, so `/fortnight` is reachable mid-interview; the
//!   brief it leaves for the next turn would be overwritten by the walk's own,
//!   and the fortnight would never be drafted. Told to finish the walk, the
//!   athlete gets the fortnight after — and, in the season walk's case, a plan
//!   worth extending.
//! - **The refusal carries its reason.** Every arm that declines names what
//!   would change it, because "no" without a next step is what sends an
//!   agent inventing one.
//!
//! This decides whether the rail *opens*, not what happens on every one of
//! its turns. Once open, the rail owns the conversation until the weeks are
//! settled and its wrap-up reads them back — so an athlete who answers "make
//! Thursday easier" is answered by the rail that computed the brief rather
//! than by an ordinary coaching turn.
//!

use serde::Serialize;

/// How far ahead the rail writes, in weeks.
///
/// The same fortnight the plan card shows and the prompt block renders: what
/// the athlete acts on, rather than a season they cannot yet judge.
pub const FORTNIGHT_WEEKS: usize = 2;

/// Whether a guided interview owns this conversation.
///
/// Read from the conversation rather than the plan, which is why it is not in
/// [`FortnightInputs`]: an athlete mid-walk is refused whether or not they
/// have a plan, and the walk is the more useful thing to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WalkReading {
    /// A guided walk is active on this conversation.
    Running,
    /// Nothing owns the conversation.
    Idle,
}

/// What the platform knows before deciding.
///
/// Assembled by the caller from readings that already exist; this module
/// reads no repository of its own, which is what keeps it testable against
/// the awkward cases rather than only the happy one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FortnightInputs {
    /// Whether the plan states any phases at all.
    pub has_phases: bool,
    /// The readiness level the athlete's signals cleared, when the ladder
    /// could be read. `None` means no flavour, so no ladder.
    pub readiness: Option<ReadinessReading>,
    /// Where the stored weeks stop covering the outline.
    pub coverage: CoverageReading,
}

/// Whether the stored weeks still cover what the outline promised.
///
/// The narrow reading this decision needs. The rail that computes coverage
/// distinguishes *no week covers today* from *the weeks stop before the last
/// phase ends*; both mean a fortnight is owed, and this collapses them,
/// because the difference changes the agent's words and not the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CoverageReading {
    /// The weeks run past the fortnight ahead: nothing to add.
    Covered,
    /// The plan has stopped covering the athlete, or is about to.
    RunningOut,
}

/// The ladder's answer, as this decision needs it.
///
/// A narrow mirror of the kernel's level on purpose: the decision cares only
/// whether the athlete is blocked, and taking the whole enum would tie this
/// module to a vocabulary it does not otherwise use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReadinessReading {
    /// The ladder allows recovery alone.
    Blocked,
    /// Some quality work is allowed.
    Open,
}

/// What the platform decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum FortnightVerdict {
    /// Write it, holding to these.
    Write {
        /// How many weeks to write.
        weeks: usize,
    },
    /// Do not write it, and this is why.
    Decline {
        /// Which refusal this is, for a caller that branches on it.
        reason: DeclineReason,
    },
}

/// Why a fortnight is not being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// A guided walk owns the conversation.
    WalkRunning,
    /// No active plan to extend.
    NoPlan,
    /// The plan states no phases, so there is nothing to write against.
    NoPhases,
    /// The stored weeks already cover the fortnight ahead.
    AlreadyCovered,
    /// The readiness ladder blocks everything but recovery.
    ReadinessBlocked,
}

impl DeclineReason {
    /// Stable identifier, for a log line or a test.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WalkRunning => "walk_running",
            Self::NoPlan => "no_plan",
            Self::NoPhases => "no_phases",
            Self::AlreadyCovered => "already_covered",
            Self::ReadinessBlocked => "readiness_blocked",
        }
    }
}

/// Decide whether the next fortnight can be written.
///
/// `None` inputs mean no active plan — the one case the caller cannot
/// assemble inputs for, kept in the signature so the refusal is this
/// module's to give rather than the caller's to invent.
#[must_use]
pub fn decide_fortnight(walk: WalkReading, inputs: Option<&FortnightInputs>) -> FortnightVerdict {
    // Before anything about the plan: a walk in progress is the answer to
    // every one of the cases below, and for the season walk it is on its way
    // to producing the plan the other arms are asking about.
    if walk == WalkReading::Running {
        return FortnightVerdict::Decline {
            reason: DeclineReason::WalkRunning,
        };
    }
    let Some(inputs) = inputs else {
        return FortnightVerdict::Decline {
            reason: DeclineReason::NoPlan,
        };
    };
    if !inputs.has_phases {
        return FortnightVerdict::Decline {
            reason: DeclineReason::NoPhases,
        };
    }
    if inputs.readiness == Some(ReadinessReading::Blocked) {
        return FortnightVerdict::Decline {
            reason: DeclineReason::ReadinessBlocked,
        };
    }
    // Covered means the weeks the athlete can already see run past the
    // fortnight: there is nothing to add, and adding anyway is how one
    // Tuesday acquires two answers.
    if inputs.coverage == CoverageReading::Covered {
        return FortnightVerdict::Decline {
            reason: DeclineReason::AlreadyCovered,
        };
    }
    FortnightVerdict::Write {
        weeks: FORTNIGHT_WEEKS,
    }
}
