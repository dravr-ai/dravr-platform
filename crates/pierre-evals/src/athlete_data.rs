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

use chrono::{Datelike, NaiveDate};
use pierre_core::civil_time::{weekday_forms, ALL_WEEKDAYS};
use pierre_core::models::{resolve_sport_type, sport_family_head, SportType};

use crate::claim_extractor::ExtractedClaim;
use crate::verdict_engine::VerdictOutcome;
use pierre_memory::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

/// One activity as this layer holds it.
///
/// The fields are the ones a coach quotes back and an athlete corrects. Two of
/// them — the date and the sport — carried no representation here at all until
/// registre#249, so every claim about *which day* or *which sport* was
/// structurally unfalsifiable, in every locale.
#[derive(Debug, Clone)]
pub struct RecordedActivity {
    /// Calendar date on the athlete's civil clock.
    pub date: NaiveDate,
    /// Canonical sport for the activity.
    pub sport: SportType,
    /// The provider's name for it — "Road 2 AUS", "Passion rando". This is how
    /// an athlete and a coach both refer to a specific session.
    pub name: String,
    /// Distance in kilometres, when the source carries GPS.
    pub distance_km: Option<f64>,
    /// Duration in minutes. Always present.
    pub duration_min: f64,
    /// Total ascent in metres, when the source carries it.
    pub elevation_m: Option<f64>,
}

/// What we actually hold about an athlete, as far as this layer is concerned.
///
/// Deliberately not the dossier type: this layer needs a handful of facts per
/// activity, and taking the whole dossier would couple claim verification to
/// every future change in athlete modelling.
#[derive(Debug, Clone, Default)]
pub struct AthleteRecord {
    /// Whether the athlete has any connected provider at all.
    ///
    /// `false` is the strong case — it licenses a contradiction, because there
    /// is provably no source for a specific figure.
    pub has_provider: bool,
    /// The activities we hold for the window.
    pub activities: Vec<RecordedActivity>,
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
        self.activities.is_empty()
    }

    /// Every value we hold in `unit`, for matching an asserted figure.
    fn held(&self, unit: Unit) -> Vec<f64> {
        self.activities
            .iter()
            .filter_map(|a| match unit {
                Unit::Kilometres => a.distance_km,
                Unit::Minutes => Some(a.duration_min),
                Unit::Metres => a.elevation_m,
            })
            .collect()
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
    /// Metres of ascent. Every elevation figure in the 2026-09-02 conversation
    /// — 2391 m, 895 m, 414 m — was unfalsifiable because this unit did not
    /// exist (registre#249).
    Metres,
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

    // A claim that names one of the athlete's own activities can be checked on
    // more than its numbers. This runs ahead of the figure logic because "you
    // did that on Sunday" is a sharper, more specific finding than "no figure
    // matched" — and it is the check the athlete was making by hand
    // (registre#249).
    if let Some(verdict) = check_named_activity(&claim.text, record) {
        return Some(verdict);
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

/// Shortest activity name this layer will match on.
///
/// A provider name of three characters or fewer ("Am", "PM", "🚴") appears
/// inside ordinary prose by accident, and a false match here produces a
/// confident contradiction about the wrong session.
const MIN_MATCHABLE_NAME: usize = 4;

/// Check a claim that names one of the athlete's own activities.
///
/// Fires only when the claim names **exactly one** held activity and asserts
/// exactly one weekday or one sport for it. Two named activities, or two
/// weekdays, is an ambiguity this layer cannot resolve — and a wrong
/// contradiction costs more than a missed one.
///
/// This is the check the 2026-09-02 conversation needed and did not have. The
/// coach placed a Tuesday ride on Sunday, called a run a bike session, and both
/// claims passed the verifier untouched while four *benign* coaching
/// prescriptions were flagged (registre#249).
fn check_named_activity(text: &str, record: &AthleteRecord) -> Option<VerdictOutcome> {
    let lower = text.to_lowercase();

    // Word-bounded, like every other token this function matches. A raw
    // `contains` let a longer word "name" an activity — a session called "Cote"
    // matched inside "cotes" — and everything downstream then contradicted a
    // weekday that was never about it (registre#258).
    let mut named = record.activities.iter().filter(|a| {
        a.name.chars().count() >= MIN_MATCHABLE_NAME
            && contains_word(&lower, &a.name.to_lowercase())
    });
    let activity = named.next()?;
    if named.next().is_some() {
        // More than one of their activities is named; which one the weekday
        // belongs to is not decidable from the text.
        return None;
    }

    // Scan the text with the activity's own name removed. "Passion rando"
    // carries the word `rando`, so leaving it in had the record contradict
    // itself — the layer read the athlete's own session title as the coach's
    // claim about its sport.
    let residual = lower.replace(&activity.name.to_lowercase(), " ");

    if let Some(claimed) = sole_weekday(&residual) {
        let actual = activity.date.weekday();
        if claimed != actual {
            return Some(contradicted(format!(
                "Places \"{}\" on {}, but it is on record for {} ({}).",
                activity.name,
                weekday_forms(claimed)[1],
                weekday_forms(actual)[1],
                activity.date
            )));
        }
    }

    if let Some(claimed) = sole_sport(&residual) {
        if !same_family(&claimed, &activity.sport) {
            return Some(contradicted(format!(
                "Calls \"{}\" a {:?}, but it is on record as a {:?}.",
                activity.name, claimed, activity.sport
            )));
        }
    }

    None
}

/// The one weekday the text names, or `None` when it names none or several.
fn sole_weekday(lower: &str) -> Option<chrono::Weekday> {
    let mut found = None;
    for day in ALL_WEEKDAYS {
        if weekday_forms(day)
            .iter()
            .any(|form| contains_word(lower, form))
        {
            if found.is_some() {
                return None;
            }
            found = Some(day);
        }
    }
    found
}

/// The one sport the text names, or `None` when it names none or several
/// unrelated ones.
///
/// Resolution goes through [`resolve_sport_type`], the same alias table the
/// tools accept from the LLM, so this reads no vocabulary of its own.
fn sole_sport(lower: &str) -> Option<SportType> {
    // Surface forms that `resolve_sport_type` actually accepts. It normalises
    // by stripping separators but NOT accents, so "course à pied" resolves to
    // nothing while the bare "course" resolves to Run — which is why the
    // multi-word French form is not in this list.
    // Only words that mean a sport and nothing else in the locale that uses
    // them. Dropped as homographs (registre#258): English "run"/"ride"/"trail"
    // (a run of days, a ride home, a trail of), French "course" (an errand),
    // "marche" (it works) and "marche" the noun, "ski" (resolves to
    // AlpineSkiing, which `sport_family_head` gives no family, so a coach
    // naming a ski discipline exactly right was contradicted).
    //
    // The asymmetry is the same one the weekday table follows: a missed sport
    // check costs one unverified claim, a false one costs a warning banner on a
    // true sentence.
    const CANDIDATES: [&str; 11] = [
        "running",
        "jogging",
        "vélo",
        "velo",
        "cycling",
        "vtt",
        "mtb",
        "gravel",
        "natation",
        "swimming",
        "course à pied",
    ];
    let mut found: Option<SportType> = None;
    for candidate in CANDIDATES {
        if !contains_word(lower, candidate) {
            continue;
        }
        let Some(sport) = resolve_sport_type(candidate) else {
            continue;
        };
        match &found {
            None => found = Some(sport),
            Some(existing) if same_family(existing, &sport) => {}
            Some(_) => return None,
        }
    }
    found
}

/// Whether two sports are the same discipline, collapsing sub-variants onto
/// their head — a mountain bike ride and a gravel ride are both cycling, and a
/// coach calling one the other is not making a false claim about the sport.
fn same_family(a: &SportType, b: &SportType) -> bool {
    let head = |s: &SportType| sport_family_head(s).unwrap_or_else(|| s.clone());
    head(a) == head(b)
}

/// Whether `needle` appears in `haystack` bounded by non-alphanumerics.
///
/// Substring alone matches "mar" inside "marathon" and "run" inside "brunch",
/// which would attribute a weekday or a sport the athlete never named.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// The shared shape of a contradiction this layer can prove from the record.
fn contradicted(explanation: String) -> VerdictOutcome {
    VerdictOutcome {
        status: ClaimStatus::Contradicted,
        evidence_strength: EvidenceStrength::Strong,
        confidence: 0.9,
        layer_fired: VerdictLayer::AthleteData,
        explanation,
        evidence_refs: None,
    }
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
    record
        .held(figure.unit)
        .iter()
        .any(|&actual| (actual - figure.value).abs() <= actual.abs() * MATCH_TOLERANCE)
}

/// Render figures for an explanation a human will read in the admin console.
fn describe(figures: &[AssertedFigure]) -> String {
    figures
        .iter()
        .map(|f| match f.unit {
            Unit::Kilometres => format!("{} km", trim_float(f.value)),
            Unit::Minutes => format!("{} min", trim_float(f.value)),
            Unit::Metres => format!("{} m", trim_float(f.value)),
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
    // Elevation. Checked AFTER minutes so "min" is never read as a bare "m",
    // and the longest forms lead as everywhere else here.
    const METRES: [&str; 4] = ["mètres", "metres", "meters", "m+"];

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
    for form in METRES {
        if tail.starts_with(form) {
            return Some((Unit::Metres, 1.0));
        }
    }
    if bare_form(tail, 'k') {
        return Some((Unit::Kilometres, 1.0));
    }
    if bare_form(tail, 'h') {
        return Some((Unit::Minutes, 60.0));
    }
    // Bare "m" last of all: "min" and "mètres" have already claimed their
    // prefixes, and `bare_form` refuses anything with a letter after it, so a
    // weight in "80 mg" or a pace in "5 min" cannot land here.
    if bare_form(tail, 'm') {
        return Some((Unit::Metres, 1.0));
    }
    None
}

/// Whether `tail` begins with the single-letter unit `letter` standing alone —
/// i.e. not the first letter of a longer word.
///
/// "10k." and "10k" both count; "10kg" does not, which is what stops a body
/// weight from being read as a distance — or, for `m`, an "80 mg" supplement
/// dose from being read as elevation.
fn bare_form(tail: &str, letter: char) -> bool {
    let mut chars = tail.chars();
    if chars.next() != Some(letter) {
        return false;
    }
    chars.next().is_none_or(|c| !c.is_alphanumeric())
}
