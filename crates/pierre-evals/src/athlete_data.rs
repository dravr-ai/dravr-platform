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
                Unit::ElevationMetres => a.elevation_m,
                Unit::DistanceMetres => a.distance_km.map(|km| km * 1000.0),
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
    ElevationMetres,
    /// Metres of distance — a pool set, a track rep, a 400 m interval.
    ///
    /// Split from [`Unit::ElevationMetres`] because one `Metres` variant held
    /// only `elevation_m`, so *"tu as tenu tes 400 m"* was adjudicated against
    /// the session's total ascent: two quantities that share a symbol and
    /// nothing else. Which one a figure means is decided by
    /// [`mentions_elevation`], not by the unit token (registre#260).
    DistanceMetres,
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

/// Shortest token in an activity name this layer will match on.
///
/// A provider name of three characters or fewer ("Am", "PM", "🚴") appears
/// inside ordinary prose by accident, and a false match here produces a
/// confident contradiction about the wrong session.
const MIN_MATCHABLE_NAME: usize = 4;

/// Nouns that name a training session generically, in every locale we reply in.
///
/// Read by two checks, and both fail *safe* on a word that arguably should not
/// be here: the effect is always to decline an adjudication, never to add a
/// contradiction. That inverts the asymmetry the weekday and sport tables
/// carry — a homograph there manufactured a false contradiction, which is why
/// those tables had to be narrowed (registre#258). A false member here costs
/// one unadjudicated claim, so this list is deliberately generous.
const GENERIC_SESSION_NOUNS: &[&str] = &[
    // French
    "sortie",
    "sorties",
    "séance",
    "seance",
    "séances",
    "seances",
    "entraînement",
    "entrainement",
    "entraînements",
    "entrainements",
    "footing",
    // English
    "ride",
    "rides",
    "run",
    "runs",
    "session",
    "sessions",
    "workout",
    "workouts",
    "training",
    "activity",
    // Spanish
    "salida",
    "salidas",
    "sesión",
    "sesion",
    "entrenamiento",
    // German
    "einheit",
    "ausfahrt",
    "lauf",
    // Portuguese
    "treino",
    "treinos",
    "saída",
    "saida",
    "sessão",
    "sessao",
];

/// Adjectives and times of day that qualify a session without identifying one.
///
/// Separate from [`GENERIC_SESSION_NOUNS`] because only a *noun* can be the
/// second referent a weekday attaches to. Both lists decide whether a provider
/// name is distinctive; only the nouns decide whether the text names a second
/// session.
const GENERIC_SESSION_MODIFIERS: &[&str] = &[
    // French
    "longue",
    "longues",
    "facile",
    "récup",
    "recup",
    "matin",
    "soir",
    "midi",
    "dure",
    // English
    "long",
    "easy",
    "hard",
    "tempo",
    "recovery",
    "morning",
    "afternoon",
    "evening",
    "lunch",
    "night",
    "indoor",
    "outdoor",
    // Spanish
    "larga",
    "suave",
    "mañana",
    "manana",
    "tarde",
    "noche",
    // German
    "lang",
    "locker",
    "abend",
    "morgen",
    // Portuguese
    "longo",
    "longa",
    "leve",
    "manhã",
    "manha",
    "noite",
];

/// Whether `word` is generic session vocabulary rather than a session's identity.
fn is_generic_session_word(word: &str) -> bool {
    GENERIC_SESSION_NOUNS.contains(&word) || GENERIC_SESSION_MODIFIERS.contains(&word)
}

/// Every alphanumeric token in `text`, in order, with its byte span.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(idx);
        } else if let Some(s) = start.take() {
            spans.push((s, idx));
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

/// Whether a provider name identifies *one* session rather than describing any
/// session at all.
///
/// Strava names an unedited activity "Morning Ride", "Afternoon Run", "Lunch
/// Ride"; an athlete names one "Sortie" or "Long Run". Each of those is long
/// enough to clear [`MIN_MATCHABLE_NAME`] and word-bounded inside any sentence
/// mentioning the same ordinary words — so every claim containing "morning
/// ride" named that session, and the weekday and sport checks then adjudicated
/// a sentence that was never about it (registre#260).
///
/// Distinctive means: at least one token long enough to matter that is not
/// generic session vocabulary. "Road 2 AUS" keeps `road`; "Morning Ride" keeps
/// nothing.
fn is_distinctive_name(lower_name: &str) -> bool {
    word_spans(lower_name).into_iter().any(|(a, b)| {
        let token = &lower_name[a..b];
        token.chars().count() >= MIN_MATCHABLE_NAME && !is_generic_session_word(token)
    })
}

/// How many whole words separate two spans of the same text.
///
/// The unit of "is this generic noun part of the named session, or a second
/// session of its own?". See [`generic_referent_between`].
fn words_between(text: &str, a: (usize, usize), b: (usize, usize)) -> usize {
    let (lo, hi) = if b.0 >= a.1 { (a.1, b.0) } else { (b.1, a.0) };
    text.get(lo..hi).map_or(0, |gap| word_spans(gap).len())
}

/// Determiners and copulas may sit between a session's generic head noun and
/// its provider name; a clause may not.
///
/// *"ta sortie Road 2 AUS"* is one session named twice — `sortie` is its head
/// noun, the name is its apposition, and nothing separates them but a
/// determiner. *"Road 2 AUS était plus dure que ta sortie de mardi"* is two
/// sessions, and five words lie between the name and the second one.
const ATTACHED_WORD_GAP: usize = 1;

/// Whether the text names a *second*, unnamed session between the named
/// activity and the day or sport being asserted.
///
/// This is the syntax this layer does not have, approximated by position. A
/// single sentence can name one activity and correctly date a different one —
/// *"Road 2 AUS était plus dure que ta sortie de mardi"* asserts nothing at all
/// about which day Road 2 AUS fell on, and contradicting it put a warning
/// banner on a true sentence (registre#260).
///
/// A generic noun *attached* to the name is the same session under its head
/// noun, so it is not a second referent and is ignored.
fn generic_referent_between(text: &str, name: (usize, usize), claim: (usize, usize)) -> bool {
    let (lo, hi) = if claim.0 >= name.1 {
        (name.1, claim.0)
    } else {
        (claim.1, name.0)
    };
    let Some(gap) = text.get(lo..hi) else {
        return false;
    };
    GENERIC_SESSION_NOUNS.iter().any(|noun| {
        find_word(gap, noun)
            .is_some_and(|(a, b)| words_between(text, name, (lo + a, lo + b)) > ATTACHED_WORD_GAP)
    })
}

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
    let mut named = record.activities.iter().filter_map(|a| {
        let name = a.name.to_lowercase();
        if !is_distinctive_name(&name) {
            return None;
        }
        find_word(&lower, &name).map(|span| (a, span))
    });
    let (activity, name_span) = named.next()?;
    if named.next().is_some() {
        // More than one of their activities is named; which one the weekday
        // belongs to is not decidable from the text.
        return None;
    }

    // Blank the activity's own name, keeping every other byte where it was.
    // Removing it was necessary — "Passion rando" carries the word `rando`, so
    // leaving it in had the record contradict itself, the layer reading the
    // athlete's own session title as the coach's claim about its sport. Doing
    // it by `replace` also moved every offset after it, and the checks below
    // need the name's position to decide what a weekday is attached to.
    let residual = blank_span(&lower, name_span);

    if let Some((claimed, span)) = sole_weekday(&residual) {
        let actual = activity.date.weekday();
        if claimed != actual && !generic_referent_between(&residual, name_span, span) {
            return Some(contradicted(format!(
                "Places \"{}\" on {}, but it is on record for {} ({}).",
                activity.name,
                weekday_forms(claimed)[1],
                weekday_forms(actual)[1],
                activity.date
            )));
        }
    }

    if let Some((claimed, span)) = sole_sport(&residual) {
        if !same_family(&claimed, &activity.sport)
            && !generic_referent_between(&residual, name_span, span)
        {
            return Some(contradicted(format!(
                "Calls \"{}\" a {:?}, but it is on record as a {:?}.",
                activity.name, claimed, activity.sport
            )));
        }
    }

    None
}

/// `text` with `span` overwritten by spaces, so every other byte keeps its
/// offset.
///
/// The replaced bytes become ASCII spaces and `span` comes from a word-bounded
/// match, so the result is always valid UTF-8; the fallback exists to keep this
/// total rather than to describe a reachable case.
fn blank_span(text: &str, span: (usize, usize)) -> String {
    let mut bytes = text.as_bytes().to_vec();
    if let Some(slice) = bytes.get_mut(span.0..span.1) {
        slice.fill(b' ');
    }
    String::from_utf8(bytes).unwrap_or_else(|_| text.to_owned())
}

/// The one weekday the text names and where it says it, or `None` when it
/// names none or several.
fn sole_weekday(lower: &str) -> Option<(chrono::Weekday, (usize, usize))> {
    let mut found = None;
    for day in ALL_WEEKDAYS {
        let Some(span) = weekday_forms(day)
            .iter()
            .find_map(|form| find_word(lower, form))
        else {
            continue;
        };
        if found.is_some() {
            return None;
        }
        found = Some((day, span));
    }
    found
}

/// The one sport the text names and where it says it, or `None` when it names
/// none or several unrelated ones.
///
/// Resolution goes through [`resolve_sport_type`], the same alias table the
/// tools accept from the LLM, so this reads no vocabulary of its own.
fn sole_sport(lower: &str) -> Option<(SportType, (usize, usize))> {
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
    let mut found: Option<(SportType, (usize, usize))> = None;
    for candidate in CANDIDATES {
        let Some(span) = find_word(lower, candidate) else {
            continue;
        };
        let Some(sport) = resolve_sport_type(candidate) else {
            continue;
        };
        match &found {
            None => found = Some((sport, span)),
            Some((existing, _)) if same_family(existing, &sport) => {}
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

/// Where `needle` first appears in `haystack` bounded by non-alphanumerics.
///
/// Substring alone matches "vtt" inside "vttiste" and "velo" inside
/// "vélodrome", which would attribute a sport the athlete never named. The span
/// is returned rather than a bare yes/no because
/// [`generic_referent_between`] decides what a weekday is attached to by where
/// it sits.
fn find_word(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(rel) = haystack.get(from..)?.find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack
            .get(..start)
            .and_then(|s| s.chars().next_back())
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack
            .get(end..)
            .and_then(|s| s.chars().next())
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some((start, end));
        }
        from = end;
    }
    None
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
            // The operator reading this in the admin console needs to know
            // which quantity the layer adjudicated, because the two share a
            // symbol and are checked against different fields.
            Unit::ElevationMetres => format!("{} m D+", trim_float(f.value)),
            Unit::DistanceMetres => format!("{} m", trim_float(f.value)),
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
    // Whether "m" in this claim means climbing or distance is a property of the
    // sentence, not of the token, so it is resolved once for the whole text.
    let elevation_cued = mentions_elevation(&lower);
    let mut out = Vec::new();
    let bytes = lower.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits = String::new();
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
            digits.push(char::from(bytes[i]));
            i += 1;
        }
        // Thousands written with a separating space. The digit scan stopped at
        // the space, so "2 391 m" yielded 391 — harmless while the units were
        // kilometres and minutes, where a value over 999 barely occurs, and
        // exactly wrong for metres of climbing, which is where four-digit
        // values live (registre#260).
        //
        // Only a group of exactly three digits, only after a leading group of
        // at most three, and only when no fourth digit follows — the shape of a
        // grouped number and of very little else. It does read "3 400 m" as
        // 3400 rather than as three 400s; that ambiguity is real in the text
        // itself, and the cost of taking it the wrong way is one `Unverifiable`
        // verdict, never a contradiction.
        if digits.len() <= 3 && digits.bytes().all(|b| b.is_ascii_digit()) {
            while let Some(sep) = thousands_separator_at(bytes, i) {
                let group = i + sep;
                let Some(next) = lower.get(group..group + 3) else {
                    break;
                };
                if !next.bytes().all(|b| b.is_ascii_digit())
                    || bytes.get(group + 3).is_some_and(u8::is_ascii_digit)
                {
                    break;
                }
                digits.push_str(next);
                i = group + 3;
            }
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
        let Ok(value) = digits
            .trim_end_matches('.')
            .replace(',', ".")
            .parse::<f64>()
        else {
            continue;
        };
        // The unit may be glued to the number ("21km") or follow a space.
        let tail = lower.get(i..).unwrap_or_default().trim_start();
        if let Some((unit, to_canonical)) = leading_unit(tail, elevation_cued) {
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
fn leading_unit(tail: &str, elevation_cued: bool) -> Option<(Unit, f64)> {
    const KM: [&str; 4] = ["kilometres", "kilometers", "kilomètres", "km"];
    const MIN: [&str; 4] = ["minutes", "minute", "mins", "min"];
    const HOURS: [&str; 4] = ["hours", "hour", "heures", "hrs"];
    // Elevation. Checked AFTER minutes so "min" is never read as a bare "m",
    // and the longest forms lead as everywhere else here.
    const METRES: [&str; 3] = ["mètres", "metres", "meters"];

    // A metres token means climbing when the sentence says so, and distance
    // otherwise. Nothing in "400 m" itself distinguishes a track rep from a
    // hill (registre#260).
    let metres = if elevation_cued {
        Unit::ElevationMetres
    } else {
        Unit::DistanceMetres
    };

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
    // `m+` is its own cue: the symbol means metres of ascent and nothing else,
    // so it does not wait on the sentence to say "dénivelé".
    if tail.starts_with("m+") {
        return Some((Unit::ElevationMetres, 1.0));
    }
    for form in METRES {
        if tail.starts_with(form) {
            return Some((metres, 1.0));
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
        return Some((metres, 1.0));
    }
    None
}

/// A separating space between thousands, and how many bytes it occupies.
///
/// All three are written in practice: a plain space by anyone typing, and the
/// two non-breaking forms by every number formatter that respects French
/// typography.
fn thousands_separator_at(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b' ' => Some(1),
        // U+00A0 NO-BREAK SPACE
        0xC2 if bytes.get(at + 1) == Some(&0xA0) => Some(2),
        // U+202F NARROW NO-BREAK SPACE
        0xE2 if bytes.get(at + 1) == Some(&0x80) && bytes.get(at + 2) == Some(&0xAF) => Some(3),
        _ => None,
    }
}

/// Whether the claim is about climbing, in any locale the coach replies in.
///
/// Deliberately substring tests on stems, like [`mentions_sleep`]: `dénivel`
/// covers "dénivelé"/"dénivelés"/"dénivellation", `grimp` covers
/// "grimpé"/"grimpette", `altimetr` covers "altimetria"/"altimetría". A false
/// positive routes a distance figure to the elevation field and yields
/// `Unverifiable`; a false negative does the same in the other direction. Both
/// are cheap, and neither can manufacture a contradiction for a connected
/// athlete.
fn mentions_elevation(lower: &str) -> bool {
    const STEMS: [&str; 12] = [
        "dénivel",
        "denivel",
        "d+",
        "m+",
        "montée",
        "montee",
        "grimp",
        "ascent",
        "elevation",
        "climb",
        "höhenmeter",
        "desnivel",
    ];
    STEMS.iter().any(|s| lower.contains(s))
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
