// ABOUTME: The fortnight decision — every refusal arm, and the order they are tested in
// ABOUTME: A blocked athlete is refused before a covered plan is, because the reason the agent says out loud differs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_services::fortnight::{
    decide_fortnight, CoverageReading, DeclineReason, FortnightInputs, FortnightVerdict,
    ReadinessReading, WalkReading, FORTNIGHT_WEEKS,
};

/// Every case below is an athlete with nothing else running; the walk arm has
/// its own tests, because it is the one reading that comes from the
/// conversation rather than from the plan.
fn idle() -> WalkReading {
    WalkReading::Idle
}

/// An athlete whose plan is running out and who is fit to train: the one
/// shape that writes. Each test moves one thing away from it.
fn ready_to_write() -> FortnightInputs {
    FortnightInputs {
        has_phases: true,
        readiness: Some(ReadinessReading::Open),
        coverage: CoverageReading::RunningOut,
    }
}

#[test]
fn a_plan_running_out_with_an_open_ladder_is_written() {
    assert_eq!(
        decide_fortnight(idle(), Some(&ready_to_write())),
        FortnightVerdict::Write {
            weeks: FORTNIGHT_WEEKS
        }
    );
}

#[test]
fn no_active_plan_is_refused_by_the_decision_not_the_caller() {
    assert_eq!(
        decide_fortnight(idle(), None),
        FortnightVerdict::Decline {
            reason: DeclineReason::NoPlan
        },
        "the caller cannot assemble inputs for a plan that does not exist, so \
         the refusal is this module's to give"
    );
}

#[test]
fn a_plan_with_no_phases_has_nothing_to_write_against() {
    let inputs = FortnightInputs {
        has_phases: false,
        ..ready_to_write()
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Decline {
            reason: DeclineReason::NoPhases
        }
    );
}

#[test]
fn a_blocked_ladder_refuses_the_write() {
    let inputs = FortnightInputs {
        readiness: Some(ReadinessReading::Blocked),
        ..ready_to_write()
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Decline {
            reason: DeclineReason::ReadinessBlocked
        },
        "at block the ladder allows recovery alone, and a fortnight of \
         recovery is a conversation, not a fortnight"
    );
}

#[test]
fn a_plan_that_already_covers_the_fortnight_is_not_extended() {
    let inputs = FortnightInputs {
        coverage: CoverageReading::Covered,
        ..ready_to_write()
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Decline {
            reason: DeclineReason::AlreadyCovered
        },
        "writing over weeks the athlete can already see is how one Tuesday \
         acquires two answers"
    );
}

#[test]
fn an_athlete_with_no_ladder_is_not_treated_as_blocked() {
    // No flavour means no ladder, which is silence, not a refusal. A plan
    // saved before the flavour rule existed must still be extendable.
    let inputs = FortnightInputs {
        readiness: None,
        ..ready_to_write()
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Write {
            weeks: FORTNIGHT_WEEKS
        }
    );
}

#[test]
fn a_blocked_athlete_hears_about_the_block_not_about_coverage() {
    // Both refusals apply. The order is deliberate: "you are not fit to
    // train" is the thing the athlete needs to hear, and "your plan already
    // covers it" would bury it behind an administrative answer.
    let inputs = FortnightInputs {
        has_phases: true,
        readiness: Some(ReadinessReading::Blocked),
        coverage: CoverageReading::Covered,
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Decline {
            reason: DeclineReason::ReadinessBlocked
        }
    );
}

#[test]
fn a_plan_with_no_phases_is_refused_before_anything_else_is_read() {
    // Nothing else is meaningful without phases: readiness and coverage both
    // describe a plan that has no targets to hold weeks to.
    let inputs = FortnightInputs {
        has_phases: false,
        readiness: Some(ReadinessReading::Blocked),
        coverage: CoverageReading::Covered,
    };
    assert_eq!(
        decide_fortnight(idle(), Some(&inputs)),
        FortnightVerdict::Decline {
            reason: DeclineReason::NoPhases
        }
    );
}

#[test]
fn a_walk_in_progress_is_refused_before_the_plan_is_read() {
    // Slash commands dispatch during a guided walk, so this is reachable. The
    // brief the go-ahead leaves for the next turn would be overwritten by the
    // walk's own directive, and the fortnight would never be written — so the
    // athlete is told to finish the walk rather than promised a draft.
    assert_eq!(
        decide_fortnight(WalkReading::Running, Some(&ready_to_write())),
        FortnightVerdict::Decline {
            reason: DeclineReason::WalkRunning
        }
    );
}

#[test]
fn a_walk_in_progress_outranks_having_no_plan_at_all() {
    // The season walk is on its way to producing the plan the other refusals
    // are asking about; "lay out a season first" would be telling an athlete
    // to start what they are already doing.
    assert_eq!(
        decide_fortnight(WalkReading::Running, None),
        FortnightVerdict::Decline {
            reason: DeclineReason::WalkRunning
        }
    );
}

#[test]
fn every_refusal_carries_a_stable_identifier() {
    for (reason, expected) in [
        (DeclineReason::WalkRunning, "walk_running"),
        (DeclineReason::NoPlan, "no_plan"),
        (DeclineReason::NoPhases, "no_phases"),
        (DeclineReason::AlreadyCovered, "already_covered"),
        (DeclineReason::ReadinessBlocked, "readiness_blocked"),
    ] {
        assert_eq!(reason.as_str(), expected);
    }
}
