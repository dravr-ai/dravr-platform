// ABOUTME: The athlete-data layer — claims about the athlete's own records, checked against them
// ABOUTME: Pins the asymmetry: no provider contradicts a figure, a thin cache only fails to support it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # What this layer is for
//!
//! *"Nice 12 km ride yesterday!"* to someone who had never connected anything
//! is the sentence that motivated the whole providerless workstream, and before
//! this layer it was not a claim at all: the six original categories are
//! sports-science propositions, and the extractor dropped anything that did not
//! parse into one. No category, no verdict, nothing to contradict.
//!
//! The tests below pin the distinction that makes the layer worth having — the
//! difference between *we know this is invented* and *we cannot tell* — because
//! collapsing the two in either direction is a bug:
//!
//! - Treating a providerless fabrication as merely unverifiable keeps the
//!   original failure.
//! - Treating a cache miss as a contradiction calls the coach a liar over our
//!   own sync lag.

use pierre_evals::athlete_data::{check, AthleteRecord};
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_memory::{ClaimCategory, ClaimStatus, VerdictLayer};

fn claim(text: &str) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::AthleteData,
    }
}

/// The original incident, now caught.
#[test]
fn a_specific_figure_with_no_provider_is_contradicted() {
    let outcome = check(
        &claim("Nice 12 km ride yesterday, that was a solid effort!"),
        &AthleteRecord::providerless(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "no connected provider means no record the figure could come from — this is \
         invention, not uncertainty"
    );
    assert_eq!(outcome.layer_fired, VerdictLayer::AthleteData);
    assert!(
        outcome.explanation.contains("12 km"),
        "the explanation must name the figure it rejected, got: {}",
        outcome.explanation
    );
}

/// Without a figure there is nothing to contradict, even providerless.
///
/// "You have been consistent lately" is unfalsifiable rather than false, and a
/// layer that contradicted it would flag ordinary encouragement.
#[test]
fn a_vague_statement_with_no_provider_is_unverifiable_not_contradicted() {
    let outcome = check(
        &claim("You have been really consistent with your training lately."),
        &AthleteRecord::providerless(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "no specific figure means nothing to disprove, got: {}",
        outcome.explanation
    );
}

/// A figure that matches a held activity is supported.
#[test]
fn a_figure_matching_a_recorded_activity_is_supported() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: vec![21.4, 8.0, 5.2],
        durations_min: vec![118.0, 42.0],
    };

    let outcome = check(
        &claim("Your longest run last month was 21 km, which is solid."),
        &record,
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "21 km is within tolerance of the recorded 21.4 km, got: {}",
        outcome.explanation
    );
    assert_eq!(outcome.layer_fired, VerdictLayer::AthleteData);
}

/// A figure matching nothing is UNVERIFIABLE — not contradicted, and not even
/// unsupported.
///
/// This layer compares against per-activity values. It cannot sum a week, cannot
/// window, and cannot see an activity we failed to sync, so a miss means "I
/// cannot settle this", not "the coach is wrong".
///
/// The status matters beyond wording: `Unsupported` is actionable in the
/// verification stage and appends a warning banner, so choosing it would put a
/// banner on the correct reply "you covered 40 km this week" whenever that week
/// was four 10 km runs. Connected athletes are not what this feature is for.
#[test]
fn a_figure_matching_nothing_is_unverifiable_not_contradicted() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: vec![5.0, 8.0],
        durations_min: vec![30.0],
    };

    let outcome = check(
        &claim("You covered 42 km in that session last weekend."),
        &record,
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "a per-activity comparison cannot settle an aggregate or an unsynced gap, got: {}",
        outcome.explanation
    );
    assert_ne!(
        outcome.status,
        ClaimStatus::Contradicted,
        "only the providerless case licenses a contradiction"
    );
}

/// A connected athlete with an empty window gets uncertainty, not a verdict.
#[test]
fn a_connected_athlete_with_no_cached_window_is_unverifiable() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: Vec::new(),
        durations_min: Vec::new(),
    };

    let outcome = check(
        &claim("You ran 15 km on Tuesday according to your log."),
        &record,
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "the data may exist and be unsynced; our lag is not the coach's lie, got: {}",
        outcome.explanation
    );
}

/// Hours are normalised to minutes before matching.
///
/// Without the conversion "2 hours" compares as 2 against durations held in
/// minutes and matches a 2-minute activity — a false support, which is the
/// worst possible failure for a verification layer.
#[test]
fn an_hours_claim_is_converted_before_matching() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: Vec::new(),
        durations_min: vec![120.0],
    };

    let outcome = check(
        &claim("That long ride took you about 2 hours to finish."),
        &record,
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "2 hours is the recorded 120 minutes, got: {}",
        outcome.explanation
    );

    // And the inverse: a genuine 2-minute activity must not satisfy "2 hours".
    let two_minutes = AthleteRecord {
        has_provider: true,
        distances_km: Vec::new(),
        durations_min: vec![2.0],
    };
    let wrong = check(
        &claim("That long ride took you about 2 hours to finish."),
        &two_minutes,
    )
    .expect("the layer must adjudicate its own category");
    assert_eq!(
        wrong.status,
        ClaimStatus::Unverifiable,
        "a 2-minute activity is not a 2-hour ride — but this layer says so by \
         declining to settle it, not by contradicting"
    );
}

/// Other categories belong to other layers and must fall through untouched.
#[test]
fn claims_from_other_categories_are_not_adjudicated_here() {
    let record = AthleteRecord::providerless();
    for category in [
        ClaimCategory::Physiological,
        ClaimCategory::TrainingPrescription,
        ClaimCategory::Nutrition,
        ClaimCategory::Recovery,
        ClaimCategory::Supplement,
        ClaimCategory::InjuryRehab,
    ] {
        let other = ExtractedClaim {
            text: "Zone 2 work sits around 70 percent of max heart rate.".to_owned(),
            category,
        };
        assert!(
            check(&other, &record).is_none(),
            "{category} belongs to a layer that can actually adjudicate it"
        );
    }
}

/// The layer is only reachable if the extractor labels such sentences, so the
/// classifier is part of the feature rather than a detail behind it.
///
/// Without this the whole layer is a phantom: correct, tested, and never
/// invoked, because nothing in production ever produces an `AthleteData` claim.
#[test]
fn the_heuristic_classifier_routes_personal_history_here() {
    use pierre_evals::claim_extractor::classify_heuristic;

    for sentence in [
        "You ran 21 km on Sunday which was your longest of the block.",
        "Nice 12 km ride yesterday, that was a solid effort!",
        "Your longest run last month was just over 21 km.",
        "Tu as couru 21 km hier, c'était solide.",
    ] {
        assert_eq!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "should route to the athlete-data layer: {sentence}"
        );
    }
}

/// And general physiology must NOT be captured by it.
///
/// The bucket keys on second-person past tense and time references precisely so
/// it does not swallow the sports-science categories, which have layers that can
/// actually verify them against literature.
#[test]
fn general_physiology_is_not_captured_as_athlete_data() {
    use pierre_evals::claim_extractor::classify_heuristic;

    for (sentence, expected) in [
        (
            "Zone 2 training sits around 70 percent of maximum heart rate.",
            ClaimCategory::Physiological,
        ),
        (
            "Creatine monohydrate is dosed at five grams per day.",
            ClaimCategory::Supplement,
        ),
    ] {
        assert_eq!(
            classify_heuristic(sentence),
            Some(expected),
            "must stay with the layer that can verify it: {sentence}"
        );
    }
}

/// Regression guard: an ordinary coaching reply to a CONNECTED athlete must
/// never produce an actionable verdict from this layer.
///
/// Asserted on the outcome rather than the category, because the category alone
/// is not the hazard. A sentence with no figure lands on `Unverifiable` whatever
/// bucket it falls in, which is harmless; the hazard is a *prescription carrying
/// a number* being checked against past activities it was never about, missing,
/// and appending a warning banner to a correct reply.
///
/// This is the regression the workflow audit caught: the layer runs on every
/// verified reply for every user, so a connected athlete meets it constantly
/// even though the feature exists for providerless ones.
#[test]
fn ordinary_replies_to_a_connected_athlete_are_never_actionable() {
    use pierre_evals::claim_extractor::extract_heuristic;

    let record = AthleteRecord {
        has_provider: true,
        // Four 10 km runs — a real week totalling 40 km, held per-activity.
        distances_km: vec![10.0, 10.0, 10.0, 10.0],
        durations_min: vec![50.0, 50.0, 50.0, 50.0],
    };

    for reply in [
        "This week, aim for 40 km at an easy pace.",
        "Last week we built volume, so this week we recover.",
        "You covered 40 km this week, which is a solid block.",
        "Next week you should target 45 km total.",
    ] {
        for claim in extract_heuristic(reply)
            .iter()
            .filter(|c| c.category == ClaimCategory::AthleteData)
        {
            let outcome = check(claim, &record).expect("layer adjudicates its own category");
            assert_ne!(
                outcome.status,
                ClaimStatus::Contradicted,
                "a connected athlete's ordinary reply must never be contradicted: {reply}"
            );
            assert_ne!(
                outcome.status,
                ClaimStatus::Unsupported,
                "`Unsupported` is actionable and would banner a correct reply. This layer \
                 compares per-activity values and cannot sum a week, so a miss must be \
                 `Unverifiable`. Reply: {reply} -> {}",
                outcome.explanation
            );
        }
    }
}

/// A French decimal ("21,4 km") must parse as one figure, not as the fragment
/// after the comma.
///
/// The scanner used to stop at the comma, drop `21`, re-enter at `4`, and record
/// a **4 km** figure the athlete never said — matched against the wrong
/// activity and printed back in the operator explanation. The platform's default
/// locale is French, so this is the common spelling, not an edge case.
#[test]
fn a_comma_decimal_is_one_figure() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: vec![21.4],
        durations_min: vec![],
    };
    let claim = claim("Tu as couru 21,4 km hier.");
    let outcome = check(&claim, &record).expect("layer adjudicates its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "21,4 km must match the recorded 21.4 km run, got: {}",
        outcome.explanation
    );
    assert!(
        outcome.explanation.contains("21.4"),
        "the operator explanation must name the figure actually asserted, got: {}",
        outcome.explanation
    );
    assert!(
        !outcome.explanation.contains("4 km,") && !outcome.explanation.starts_with("States 4 km"),
        "the post-comma fragment must never become the figure: {}",
        outcome.explanation
    );
}

/// A pace expression must not inject a phantom zero-minute figure.
///
/// "5:00 min/km" used to parse the seconds half as its own `0 min` figure, which
/// can never match anything, so it broke the all-figures-must-match rule and
/// surfaced as "0 min" in operator-facing text.
#[test]
fn a_pace_expression_injects_no_phantom_figure() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: vec![10.0],
        durations_min: vec![50.0],
    };
    let claim = claim("You ran 10 km at 5:00 min/km.");
    let outcome = check(&claim, &record).expect("layer adjudicates its own category");

    assert!(
        !outcome.explanation.contains("0 min"),
        "the seconds half of a pace must not become a figure: {}",
        outcome.explanation
    );
    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "the only real figure (10 km) is on record, so the claim is supported: {}",
        outcome.explanation
    );
}

/// The bare "10k" idiom counts as a distance even at the end of a sentence.
#[test]
fn the_bare_k_form_does_not_need_a_trailing_space() {
    let record = AthleteRecord::providerless();
    for text in ["Nice 10k.", "Nice 10k"] {
        let claim = claim(text);
        let outcome = check(&claim, &record).expect("layer adjudicates its own category");
        assert_eq!(
            outcome.status,
            ClaimStatus::Contradicted,
            "a specific distance from a providerless athlete must be contradicted: {text}"
        );
    }
    // ...but a weight is not a distance.
    let claim = claim("You lifted 10kg.");
    let outcome = check(&claim, &record).expect("layer adjudicates its own category");
    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "10kg is not a distance and must not be read as one: {}",
        outcome.explanation
    );
}

/// A sleep claim must not be "supported" by an unrelated workout duration.
///
/// `durations_min` is filled exclusively from activity durations, so matching
/// "you slept 8 hours" against a 480-minute ride would report corroboration
/// this layer does not have.
#[test]
fn a_sleep_claim_is_not_supported_by_a_workout_duration() {
    let record = AthleteRecord {
        has_provider: true,
        distances_km: vec![],
        durations_min: vec![480.0],
    };
    for text in ["You slept 8 hours last night.", "Tu as dormi 8 heures."] {
        let claim = claim(text);
        let outcome = check(&claim, &record).expect("layer adjudicates its own category");
        assert_eq!(
            outcome.status,
            ClaimStatus::Unverifiable,
            "a 480-minute activity is not evidence about sleep: {text} -> {}",
            outcome.explanation
        );
    }
}
