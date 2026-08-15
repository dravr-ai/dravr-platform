// ABOUTME: Verifies claims about the athlete's OWN records against their own data, not the literature
// ABOUTME: Absent data is a verdict here — a specific figure with no provider behind it is contradicted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete-data layer
//!
//! The other layers answer *"is this true of humans?"*. This one answers *"is
//! this true of **this** human?"*, and it is the only layer that can.
//!
//! ## Why it had to exist
//!
//! The six original claim categories are sports-science propositions checked
//! against a literature corpus, and the extractor dropped anything that did not
//! parse into one of them. So *"nice 12 km ride yesterday!"* — the sentence that
//! started all of this — was never a claim at all. It entered no pipeline, drew
//! no verdict, and could not be contradicted, because nothing in a corpus of
//! papers speaks to whether one person rode 12 km.
//!
//! ## Absence is an answer
//!
//! The important asymmetry: when the athlete has **no connected provider**, a
//! specific figure about their training is not merely unverifiable, it is
//! *contradicted*. We know with certainty there is no record it could have come
//! from, so it was invented. Treating that as a shrug would keep the exact
//! failure this layer exists to catch.
//!
//! When a provider **is** connected and we simply hold nothing for the window,
//! the honest verdict is [`ClaimStatus::Unverifiable`]: the data may exist and
//! be unsynced, and calling the coach a liar over our own sync lag would be
//! worse than saying nothing.
//!
//! When we do hold records, a figure that matches one is supported and a figure
//! that matches none is *unsupported* rather than contradicted — our cache is a
//! window, not the whole truth, and it can legitimately miss an activity.

use crate::claim_extractor::ExtractedClaim;
use crate::verdict_engine::VerdictOutcome;
use pierre_memory::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

/// What we actually hold about an athlete, as far as this layer is concerned.
///
/// Deliberately not the dossier type: this layer needs three facts, and taking
/// the whole dossier would couple claim verification to every future change in
/// athlete modelling.
#[derive(Debug, Clone, Default)]
pub struct AthleteRecord {
    /// Whether the athlete has any connected provider at all.
    ///
    /// `false` is the strong case — it licenses a contradiction, because there
    /// is provably no source for a specific figure.
    pub has_provider: bool,
    /// Distances, in kilometres, of the activities we hold for the window.
    pub distances_km: Vec<f64>,
    /// Durations, in minutes, of the activities we hold for the window.
    pub durations_min: Vec<f64>,
}

impl AthleteRecord {
    /// An athlete with nothing connected — the providerless case.
    #[must_use]
    pub fn providerless() -> Self {
        Self::default()
    }

    /// Whether we hold any activity at all for the window.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.distances_km.is_empty() && self.durations_min.is_empty()
    }
}

/// Relative tolerance when matching an asserted figure to a recorded one.
///
/// A coach rounding 21.4 km to "21 km" is describing the same run, not a
/// different one. 5% keeps that honest rounding while still separating a 21 km
/// run from a 30 km one.
const MATCH_TOLERANCE: f64 = 0.05;

/// A figure the claim asserts about the athlete, normalised to a unit.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AssertedFigure {
    value: f64,
    unit: Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Kilometres,
    Minutes,
}

/// Check a claim about the athlete's own records.
///
/// Returns `None` when the claim is not [`ClaimCategory::AthleteData`] — every
/// other category belongs to a layer that can actually adjudicate it.
#[must_use]
pub fn check(claim: &ExtractedClaim, record: &AthleteRecord) -> Option<VerdictOutcome> {
    if claim.category != ClaimCategory::AthleteData {
        return None;
    }

    let figures = extract_figures(&claim.text);

    if !record.has_provider {
        return Some(if figures.is_empty() {
            unverifiable(
                "The athlete has no connected data source, so nothing about their training \
                 history can be confirmed either way.",
            )
        } else {
            // The strong case. No provider means no record, so a specific
            // figure did not come from anywhere.
            VerdictOutcome {
                status: ClaimStatus::Contradicted,
                evidence_strength: EvidenceStrength::Strong,
                confidence: 0.95,
                layer_fired: VerdictLayer::AthleteData,
                explanation: format!(
                    "States {} about an athlete with no connected data source. There is no \
                     record this figure could have come from, so it was invented.",
                    describe(&figures)
                ),
                evidence_refs: None,
            }
        });
    }

    if record.is_empty() {
        return Some(unverifiable(
            "A provider is connected but no activities are held for this window, so the claim \
             can be neither confirmed nor refuted — the data may exist and be unsynced.",
        ));
    }

    if figures.is_empty() {
        return Some(unverifiable(
            "Makes no specific claim about the athlete's records, so there is nothing to check \
             against them.",
        ));
    }

    // Sleep is an athlete-data claim, but `durations_min` is filled exclusively
    // from activity durations. Matching "you slept eight hours" against a
    // 480-minute ride would report Supported on the strength of an unrelated
    // workout — a verdict that reads as corroboration and is worth less than
    // none. This layer holds no sleep record, so it declines to rule.
    if mentions_sleep(&claim.text) && figures.iter().any(|f| f.unit == Unit::Minutes) {
        return Some(unverifiable(
            "Concerns sleep, and this layer holds only activity durations — a match against a \
             workout would not be evidence about sleep, so the claim cannot be settled here.",
        ));
    }

    let matched = figures
        .iter()
        .filter(|f| matches_record(**f, record))
        .count();

    Some(if matched == figures.len() {
        VerdictOutcome {
            status: ClaimStatus::Supported,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.9,
            layer_fired: VerdictLayer::AthleteData,
            explanation: format!(
                "Every figure stated ({}) matches an activity on record.",
                describe(&figures)
            ),
            evidence_refs: None,
        }
    } else {
        // `Unverifiable`, not `Unsupported`, and the distinction is load-bearing
        // for a *connected* athlete.
        //
        // This layer compares a figure against per-activity distances and
        // durations. It cannot sum, cannot window, and cannot see an activity we
        // failed to sync — so "you covered 40 km this week", a correct statement
        // about a week that was four 10 km runs, matches nothing here. Reporting
        // that as `Unsupported` makes it actionable (see `actionable_flag` in the
        // verification stage: `Unsupported` is `Some(false)`, and anything
        // `Some` drives `reply_action`), which appends a warning banner to a
        // reply that was right.
        //
        // The honest reading of a miss is "this layer cannot settle it", which is
        // exactly `Unverifiable` — non-actionable, still recorded as a verdict.
        // The one case where absence really is evidence is the providerless
        // branch above, and that keeps its `Contradicted`.
        VerdictOutcome {
            status: ClaimStatus::Unverifiable,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.0,
            layer_fired: VerdictLayer::AthleteData,
            explanation: format!(
                "States {} and no single activity on record matches. This layer compares \
                 against per-activity values only — it cannot sum a week or see an unsynced \
                 activity — so it cannot settle the claim either way.",
                describe(&figures)
            ),
            evidence_refs: None,
        }
    })
}

/// Whether the claim is about sleep, in any locale the coach replies in.
///
/// Deliberately a substring test on stems: `dorm` covers "dormi"/"dormir"/
/// "dormido", `schlaf` covers "geschlafen". A false positive here costs one
/// `Unverifiable` verdict; a false negative lets an unrelated workout duration
/// pose as evidence about sleep.
fn mentions_sleep(text: &str) -> bool {
    const STEMS: [&str; 6] = ["sleep", "slept", "sommeil", "dorm", "schlaf", "sono"];
    let lower = text.to_lowercase();
    STEMS.iter().any(|s| lower.contains(s))
}

/// The shared shape of every "cannot say" outcome from this layer.
fn unverifiable(explanation: &str) -> VerdictOutcome {
    VerdictOutcome {
        status: ClaimStatus::Unverifiable,
        evidence_strength: EvidenceStrength::None,
        confidence: 0.0,
        layer_fired: VerdictLayer::AthleteData,
        explanation: explanation.to_owned(),
        evidence_refs: None,
    }
}

/// Whether an asserted figure matches something we hold, within tolerance.
fn matches_record(figure: AssertedFigure, record: &AthleteRecord) -> bool {
    let held: &[f64] = match figure.unit {
        Unit::Kilometres => &record.distances_km,
        Unit::Minutes => &record.durations_min,
    };
    held.iter()
        .any(|&actual| (actual - figure.value).abs() <= actual.abs() * MATCH_TOLERANCE)
}

/// Render figures for an explanation a human will read in the admin console.
fn describe(figures: &[AssertedFigure]) -> String {
    figures
        .iter()
        .map(|f| match f.unit {
            Unit::Kilometres => format!("{} km", trim_float(f.value)),
            Unit::Minutes => format!("{} min", trim_float(f.value)),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `21.0` reads as `21`; `21.4` keeps its decimal.
fn trim_float(v: f64) -> String {
    if (v.fract()).abs() < f64::EPSILON {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Pull distance and duration figures out of a claim.
///
/// Unit-anchored on purpose: a bare number ("you did 3 of those") says nothing
/// checkable, and treating it as a distance would manufacture contradictions
/// out of ordinary sentences. Only a number wearing a unit is a claim about the
/// record.
fn extract_figures(text: &str) -> Vec<AssertedFigure> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    let bytes = lower.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        // `,` counts as a decimal separator only between digits, which is how
        // every French reply writes distance ("21,4 km"). Scanning digits alone
        // would end the number at the comma, then re-enter the loop at `4` and
        // record a 4 km figure — a value the athlete never said, matched against
        // the wrong activity and printed back in the operator explanation.
        while i < bytes.len()
            && (bytes[i].is_ascii_digit()
                || bytes[i] == b'.'
                || (bytes[i] == b','
                    && i + 1 < bytes.len()
                    && bytes[i + 1].is_ascii_digit()
                    && i > start))
        {
            i += 1;
        }
        // A `:` on either side means this is a clock or pace token ("5:00
        // min/km", "1:32:40"), not a plain quantity. The seconds half would
        // otherwise parse as its own figure — "5:00 min/km" yielding a phantom
        // 0 min that can never match anything and breaks the all-figures rule.
        let preceded_by_colon = start > 0 && bytes[start - 1] == b':';
        let followed_by_colon = i < bytes.len() && bytes[i] == b':';
        if preceded_by_colon || followed_by_colon {
            continue;
        }
        let Ok(value) = lower[start..i]
            .trim_end_matches('.')
            .replace(',', ".")
            .parse::<f64>()
        else {
            continue;
        };
        // The unit may be glued to the number ("21km") or follow a space.
        let tail = lower[i..].trim_start();
        if let Some((unit, to_canonical)) = leading_unit(tail) {
            out.push(AssertedFigure {
                value: value * to_canonical,
                unit,
            });
        }
    }
    out
}

/// The unit a claim's tail begins with, and the factor that converts the number
/// in front of it into that unit's canonical scale.
///
/// The factor is why this returns a pair: an hours claim has to become minutes
/// *and* carry its ×60, or "2 hours" compares as 2 against durations held in
/// minutes and matches a 2-minute activity.
///
/// Longest forms first, so `"kilometres"` is not read as `"km"` plus stray
/// text, and `"minutes"` not as `"min"`.
///
/// The bare `k`/`h` forms are matched by [`bare_form`] rather than a literal
/// `"k "`, because requiring the trailing space loses every "10k" that ends a
/// sentence or is followed by punctuation — the most common way the idiom is
/// actually written.
fn leading_unit(tail: &str) -> Option<(Unit, f64)> {
    const KM: [&str; 4] = ["kilometres", "kilometers", "kilomètres", "km"];
    const MIN: [&str; 4] = ["minutes", "minute", "mins", "min"];
    const HOURS: [&str; 4] = ["hours", "hour", "heures", "hrs"];

    for form in KM {
        if tail.starts_with(form) {
            return Some((Unit::Kilometres, 1.0));
        }
    }
    for form in MIN {
        if tail.starts_with(form) {
            return Some((Unit::Minutes, 1.0));
        }
    }
    for form in HOURS {
        if tail.starts_with(form) {
            return Some((Unit::Minutes, 60.0));
        }
    }
    if bare_form(tail, 'k') {
        return Some((Unit::Kilometres, 1.0));
    }
    if bare_form(tail, 'h') {
        return Some((Unit::Minutes, 60.0));
    }
    None
}

/// Whether `tail` begins with the single-letter unit `letter` standing alone —
/// i.e. not the first letter of a longer word.
///
/// "10k." and "10k" both count; "10kg" does not, which is what stops a body
/// weight from being read as a distance.
fn bare_form(tail: &str, letter: char) -> bool {
    let mut chars = tail.chars();
    if chars.next() != Some(letter) {
        return false;
    }
    chars.next().is_none_or(|c| !c.is_alphanumeric())
}
