// ABOUTME: Form claims in a reply — a share of the athlete's fitness, a band name, a bare TSB — checked against the athlete's own FormReading
// ABOUTME: Shares and bands are judged as the product renders them; only a bare TSB meets the raw number, with a tolerance that scales with CTL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Form claims
//!
//! The product never shows form as a raw TSB on its own. It says "form at -64%
//! of your fitness, deep fatigue": TSB as a share of the CTL it is read
//! against, banded by [`FormBand`]. A reply that repeats that framing is
//! checked here against the same [`FormReading`] the tools render, in three
//! kinds of claim:
//!
//! - **A share of fitness** ("you're at -30% of your fitness", "-64% of CTL")
//!   is compared to `form_pct` within [`FORM_PCT_TOLERANCE_POINTS`] percentage
//!   points.
//! - **A band** ("deep fatigue", "fresh", «fatigue profonde», «très frais») is
//!   compared to the reading's `band`. The names are the product's own
//!   (`home.status.band.*` in the five shipped locales), and the band edges
//!   are cageux's: this module never re-derives one.
//! - **A bare TSB** ("your TSB is -77") is compared to the raw number, because
//!   the sentence states the raw number. The tolerance scales with the CTL the
//!   TSB is a share of: a fixed ±5 points is a narrow band at CTL 110 and a
//!   wide one at CTL 30.
//!
//! A percentage is never compared to raw TSB, whatever keyword it stands beside:
//! a correct "-64%" next to "current form" would be contradicted by a TSB of
//! -77, and a wrong "-77%" supported by it.
//!
//! ## What is deliberately left unchecked
//!
//! A reply is checked only when it states the athlete's own form. An unsigned
//! share that names no fitness ("your form improved 5%") is a change, not a
//! reading, and is skipped. One that names its fitness is read as the product
//! writes it: "30% below your fitness" names its side, and "Form 12% of your
//! fitness" is a positive form, because the product signs only a negative one
//! and the sentence is about form. A share whose sentence names another load
//! last ("your acute load is 40% above your chronic load") is that load's, not
//! form's, and is skipped.
//!
//! A figure that is a limit rather than a reading is skipped, and so is any
//! band name in its sentence: "deep fatigue is below -30% of CTL" and "heavy
//! block is -30% to -20%" define the bands, which is what the agent is asked to
//! do when an athlete asks how form is read. A band word that is also everyday
//! English ("balanced", "productive", "fresh") needs an anchor in the same
//! sentence, a form label or "you're in", so "a balanced diet" is not read as a
//! claim about form. "You're in" alone anchors only the bands that name a state:
//! "heavy block", "productive" and "balanced" also name a block or a week, so
//! "you're in a heavy block this week" is not a claim about form.

use std::collections::BTreeMap;

use pierre_core::models::{FormBand, FormReading};
use pierre_memory::ClaimStatus;

use super::{contradicted_verdict, supported_verdict, verdict_for, PersonalizedContext};
use crate::deterministic_bounds::WINDOW_BYTES;
use crate::verdict_engine::VerdictOutcome;

/// Half-width, in percentage points of the athlete's own fitness, within which
/// a stated form is called consistent with the reading.
///
/// Half the width of the narrowest band (10 points: heavy block, productive),
/// so a stated share the check accepts is never more than half a band from the
/// truth. The product prints the share rounded to a whole percent, which a
/// tolerance this wide absorbs several times over. At CTL 100 it is 5 TSB
/// points.
const FORM_PCT_TOLERANCE_POINTS: f64 = 5.0;

/// The smallest tolerance on a bare TSB, in TSB points. The product renders
/// TSB rounded to a whole number, so anything tighter would contradict the
/// figure it printed.
const MIN_TSB_TOLERANCE: f64 = 1.0;

/// How many characters after a figure are read for the fitness it is a share
/// of ("% of your fitness", «% de ta condition physique»).
const TAIL_CHARS: usize = 32;

/// Characters that end the clause a figure's tail is read in.
const SENTENCE_END: [char; 5] = ['.', '!', '?', ';', '\n'];

/// Labels a reply puts on form, in the five shipped locales. A figure beside
/// one is a claim about form: a bare number is read as a TSB, a percentage as a
/// share of fitness.
const FORM_LABELS: &[&str] = &[
    "tsb",
    "training stress balance",
    "current form",
    // fr. "indice de fatigue" is here because the agent invented it
    // live on 2026-09-02 and then repeated it for fifteen turns — a
    // label the model actually uses is worth probing whether or not
    // anybody chose it (registre#204).
    "forme actuelle",
    "équilibre de charge",
    "indice de fatigue",
    "indice de forme",
    // es
    "forma actual",
    "balance de carga",
    "índice de fatiga",
    // de
    "aktuelle form",
    "formwert",
    "ermüdungsindex",
    // pt
    "forma atual",
    "equilíbrio de carga",
    "índice de fadiga",
];

/// The possessive forms the product itself uses for form ("Your form is far
/// below your own fitness", «Ta forme est très en dessous…»). They anchor a
/// percentage or a band name, never a bare number: «ta forme sur les 7
/// derniers jours» carries a 7 that is not a TSB.
const FORM_POSSESSIVES: &[&str] = &[
    "your form",
    "votre forme",
    "ta forme",
    "tu forma",
    "su forma",
    "deine form",
    "ihre form",
    "a tua forma",
    "tua forma",
    "a sua forma",
    "sua forma",
];

/// Second-person state phrases that put a band name on the athlete ("you're in
/// deep fatigue"). They anchor a band name, never a number.
const STATE_PHRASES: &[&str] = &[
    "you're in",
    "you are in",
    "you're at",
    "you are at",
    "tu es en",
    "tu es à",
    "tu es dans",
    "estás en",
    "estás al",
    "du bist in",
    "du befindest dich in",
    "estás em",
];

/// Loads other than form that a reply measures against the athlete's fitness,
/// in the five shipped locales. "Your acute load is 40% above your chronic
/// load" states the acute:chronic ratio: read as form, it would put +40% on an
/// athlete whose form is -40% and contradict a true sentence. Fatigue counts
/// only with a possessive, because "deep fatigue" is a band name.
const OTHER_LOADS: &[&str] = &[
    // en
    "atl",
    "acwr",
    "acute",
    "load",
    "volume",
    "mileage",
    "your fatigue",
    // fr
    "charge",
    "aiguë",
    "ta fatigue",
    "votre fatigue",
    "sa fatigue",
    // es. "carga" and "aguda" are spelled the same in pt.
    "carga",
    "aguda",
    "volumen",
    "tu fatiga",
    "su fatiga",
    // de
    "belastung",
    "akute",
    "akuten",
    "umfang",
    "deine ermüdung",
    "ihre ermüdung",
    // pt
    "tua fadiga",
    "sua fadiga",
];

/// What a share of form is a share of, in the five shipped locales.
const FITNESS_NOUNS: &[&str] = &[
    "fitness",
    "ctl",
    "chronic load",
    "chronic training load",
    "condition physique",
    "forme physique",
    "charge chronique",
    "condición física",
    "carga crónica",
    "carga crônica",
    "condição física",
    "chronische belastung",
    "chronischen belastung",
];

/// Words that open "% of your fitness" in the five shipped locales: the
/// preposition, or in German the genitive pronoun that stands in for it
/// («Form bei 18 % deiner Fitness»).
const PARTICLES: &[&str] = &[
    "of", "de", "del", "der", "von", "vom", "da", "do", "deiner", "deines",
];

/// The bare word for form. It does not anchor a number or a band on its own,
/// but beside a share of fitness it says the share is form's: the product
/// writes a positive form without a sign («Form 18% of your fitness»).
const FORM_WORDS: &[&str] = &["form", "forme", "forma"];

/// What a bare number can be counting instead of TSB: «ta forme actuelle sur
/// 7 jours» carries a 7 that is a span of days.
const DURATION_UNITS: &[&str] = &[
    "day", "days", "jour", "jours", "día", "días", "dia", "dias", "tag", "tage", "tagen", "week",
    "weeks", "semaine", "semaines", "semana", "semanas", "woche", "wochen",
];

/// Words that, before a figure, make it a limit rather than a reading ("below
/// -30%", «au moins -20 %»), besides the side words in [`BELOW_WORDS`] and
/// [`ABOVE_WORDS`].
const LIMIT_WORDS: &[&str] = &[
    "than",
    "least",
    "most",
    "up to",
    "au moins",
    "au plus",
    "moins",
    "plus",
    "al menos",
    "hasta",
    "menos",
    "más",
    "mindestens",
    "höchstens",
    "bis",
    "pelo menos",
    "até",
    "mais",
];

/// Particles that may stand between a limit word and its figure ("below of"
/// does not occur; «en dessous de -30 %» and «por debajo de -30 %» do).
const LIMIT_LINKS: &[&str] = &["de", "d'", "del", "da", "do", "of"];

/// Words that put a share below the athlete's fitness.
const BELOW_WORDS: &[&str] = &[
    "below",
    "under",
    "en dessous",
    "en-dessous",
    "au-dessous",
    "au dessous",
    "por debajo",
    "debajo",
    "unter",
    "abaixo",
];

/// Words that put a share above the athlete's fitness.
const ABOVE_WORDS: &[&str] = &[
    "above",
    "over",
    "au-dessus",
    "au dessus",
    "por encima",
    "encima",
    "über",
    "acima",
];

/// Spelled-out percent units that follow a number, besides the `%` sign.
const PERCENT_WORDS: &[&str] = &[
    "percent",
    "per cent",
    "pour cent",
    "pourcent",
    "por ciento",
    "por cento",
    "prozent",
];

/// Words that join the two ends of a range ("-20 to -30%").
const RANGE_CONNECTORS: &[&str] = &[
    "to", "and", "-", "–", "—", "/", "à", "a", "et", "y", "e", "bis", "und",
];

/// The names of the bands in the five shipped locales, as the product spells
/// them (`home.status.band.*`), feminine forms included because «forme» and
/// «forma» agree with them. `None` marks a phrase that contains a band name
/// without naming one: "the productive zone" is how the product describes the
/// heavy block, so the "productive" inside it is not a claim of that band.
const BAND_TERMS: &[(&str, Option<FormBand>)] = &[
    // en
    ("deep fatigue", Some(FormBand::DeepFatigue)),
    ("heavy block", Some(FormBand::HeavyBlock)),
    ("productive zone", None),
    ("productive", Some(FormBand::Productive)),
    ("balanced", Some(FormBand::Balanced)),
    ("very fresh", Some(FormBand::Detraining)),
    ("fresh", Some(FormBand::Fresh)),
    // fr
    ("fatigue profonde", Some(FormBand::DeepFatigue)),
    ("bloc chargé", Some(FormBand::HeavyBlock)),
    ("zone productive", None),
    ("productif", Some(FormBand::Productive)),
    ("équilibré", Some(FormBand::Balanced)),
    ("équilibrée", Some(FormBand::Balanced)),
    ("très frais", Some(FormBand::Detraining)),
    ("très fraîche", Some(FormBand::Detraining)),
    ("frais", Some(FormBand::Fresh)),
    ("fraîche", Some(FormBand::Fresh)),
    // es
    ("fatiga profunda", Some(FormBand::DeepFatigue)),
    ("bloque exigente", Some(FormBand::HeavyBlock)),
    ("zona productiva", None),
    ("productivo", Some(FormBand::Productive)),
    ("productiva", Some(FormBand::Productive)),
    ("equilibrado", Some(FormBand::Balanced)),
    ("equilibrada", Some(FormBand::Balanced)),
    ("muy fresco", Some(FormBand::Detraining)),
    ("muy fresca", Some(FormBand::Detraining)),
    ("fresco", Some(FormBand::Fresh)),
    ("fresca", Some(FormBand::Fresh)),
    // de
    ("tiefe ermüdung", Some(FormBand::DeepFatigue)),
    ("harter block", Some(FormBand::HeavyBlock)),
    ("produktiven zone", None),
    ("produktiv", Some(FormBand::Productive)),
    ("ausgeglichen", Some(FormBand::Balanced)),
    ("sehr frisch", Some(FormBand::Detraining)),
    ("frisch", Some(FormBand::Fresh)),
    // pt. "equilibrado", "equilibrada", "fresco" and "fresca" are spelled as in es.
    ("fadiga profunda", Some(FormBand::DeepFatigue)),
    ("bloco exigente", Some(FormBand::HeavyBlock)),
    ("zona produtiva", None),
    ("produtivo", Some(FormBand::Productive)),
    ("produtiva", Some(FormBand::Productive)),
    ("muito fresco", Some(FormBand::Detraining)),
    ("muito fresca", Some(FormBand::Detraining)),
];

/// A number in a sentence, with what the scan needs to know about how it was
/// written.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Figure {
    /// The value as written; negative only when a minus was written.
    value: f64,
    /// Whether a `+`, `-` or `−` was written in front of the digits.
    signed: bool,
    /// Whether a percent sign or word follows the digits.
    percent: bool,
    /// Whether the figure states a limit instead of a reading: it follows
    /// "below", «au moins»…, or is an end of a percent range.
    limit: bool,
    /// Byte offset of the first character, the sign included.
    start: usize,
    /// Byte offset just past the number and its percent unit.
    end: usize,
}

/// Which side of the athlete's fitness a share is said to sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Below,
    Above,
}

/// The figures of a reply that state form, and whether the sentence is about
/// form at all.
#[derive(Debug)]
struct Stated {
    /// Shares of fitness, signed, in percentage points.
    percents: Vec<f64>,
    /// Bare numbers read as a TSB.
    tsbs: Vec<f64>,
    /// Whether a form label, a possessive form or a share-of-fitness phrase is
    /// in the sentence.
    anchored: bool,
    /// Whether a second-person state phrase ("you're in") is in the sentence.
    in_state: bool,
    /// Whether the sentence states a limit, which makes any band name in it
    /// part of a definition.
    defines: bool,
}

/// Check one claim's form statements against the athlete's reading.
///
/// `None` when the sentence states no form, or states it in a way the
/// reading cannot settle: no chronic base to take a share of, an unsigned
/// share, a band the reading sits within tolerance of.
pub(super) fn check(text: &str, ctx: &PersonalizedContext<'_>) -> Option<VerdictOutcome> {
    let form = ctx.metrics.form.as_ref()?;
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    let stated = stated_form(&lower, &figures(&lower));

    let verdicts: Vec<VerdictOutcome> = stated
        .percents
        .iter()
        .filter_map(|&pct| percent_verdict(ctx, form, pct))
        .chain(band_verdict(&lower, form, &stated))
        .chain(
            stated
                .tsbs
                .iter()
                .filter_map(|&tsb| tsb_verdict(ctx, form, tsb)),
        )
        .collect();
    first_decisive(verdicts)
}

/// A contradiction wins over a support: one wrong figure in a sentence makes
/// the sentence wrong however many of its other figures are right.
fn first_decisive(verdicts: Vec<VerdictOutcome>) -> Option<VerdictOutcome> {
    let mut supported = None;
    for verdict in verdicts {
        match verdict.status {
            ClaimStatus::Contradicted => return Some(verdict),
            ClaimStatus::Supported => supported = supported.or(Some(verdict)),
            _ => {}
        }
    }
    supported
}

/// A stated share of fitness against the reading's `form_pct`.
fn percent_verdict(
    ctx: &PersonalizedContext<'_>,
    form: &FormReading,
    stated: f64,
) -> Option<VerdictOutcome> {
    let actual = form.form_pct?;
    verdict_for(
        ctx,
        (
            actual - FORM_PCT_TOLERANCE_POINTS,
            actual + FORM_PCT_TOLERANCE_POINTS,
        ),
        stated,
        &format!(
            "form {stated:+.0}% of your fitness vs your {:+.0}%",
            rendered_pct(actual)
        ),
    )
}

/// A share as the product prints it. `{:.0}` and `round()` disagree on exact
/// halves (-192.5 formats as -192 and rounds to -193), and an explanation must
/// quote the figure the athlete was shown (see `FormReading::inline`).
fn rendered_pct(pct: f64) -> f64 {
    pct.round()
}

/// A stated bare TSB against the reading's raw `tsb`.
fn tsb_verdict(
    ctx: &PersonalizedContext<'_>,
    form: &FormReading,
    stated: f64,
) -> Option<VerdictOutcome> {
    let tolerance = tsb_tolerance(form);
    verdict_for(
        ctx,
        (form.tsb - tolerance, form.tsb + tolerance),
        stated,
        &format!(
            "TSB {stated:.0} vs your {:.0} at CTL {:.0}",
            form.tsb, form.form_ctl
        ),
    )
}

/// The slack a bare TSB is allowed: [`FORM_PCT_TOLERANCE_POINTS`] of the CTL
/// the TSB is a share of, so the same tolerance in the product's own terms
/// buys fewer TSB points at CTL 30 than at CTL 110.
fn tsb_tolerance(form: &FormReading) -> f64 {
    (form.form_ctl.abs() * FORM_PCT_TOLERANCE_POINTS / 100.0).max(MIN_TSB_TOLERANCE)
}

/// A stated band against the reading's band.
///
/// The same band is supported. A band the reading sits within tolerance of is
/// left alone, as a share within the strategy's buffer is. Any other band is
/// contradicted. The strategy's margin does not apply: a band is a name, not a
/// value it could be a fraction of.
fn band_verdict(lower: &str, form: &FormReading, stated: &Stated) -> Option<VerdictOutcome> {
    let actual_pct = form.form_pct?;
    if stated.defines {
        return None;
    }
    let claimed = claimed_band(lower)?;
    if !(stated.anchored || stated.in_state && names_a_state(claimed)) {
        return None;
    }
    let descr = format!(
        "form band \"{}\" vs your \"{}\" ({:+.0}% of your fitness)",
        band_name(claimed),
        band_name(form.band),
        rendered_pct(actual_pct),
    );
    if claimed == form.band {
        Some(supported_verdict(&descr))
    } else if near_band(claimed, actual_pct) {
        None
    } else {
        Some(contradicted_verdict(&descr))
    }
}

/// Whether a state phrase alone puts `band` on the athlete. "Heavy block",
/// "productive" and "balanced" also describe a block or a week ("you're in a
/// heavy block this week"), so they need a form label, a possessive form or a
/// share of fitness beside them.
const fn names_a_state(band: FormBand) -> bool {
    matches!(
        band,
        FormBand::DeepFatigue | FormBand::Fresh | FormBand::Detraining
    )
}

/// The band's name without its description: `FormBand::label` reads
/// "deep fatigue - form far below this athlete's own fitness".
fn band_name(band: FormBand) -> &'static str {
    let label = band.label();
    label.split_once(" - ").map_or(label, |(name, _)| name)
}

/// Whether a form within [`FORM_PCT_TOLERANCE_POINTS`] of `pct` falls in
/// `band`. Every finite band is at least 10 points wide, so samples 2.5 points
/// apart across the ±5 window cannot step over one.
fn near_band(band: FormBand, pct: f64) -> bool {
    [-1.0_f64, -0.5, 0.0, 0.5, 1.0].into_iter().any(|step| {
        FormBand::from_form_pct(Some(step.mul_add(FORM_PCT_TOLERANCE_POINTS, pct))) == band
    })
}

/// The one band a sentence names, `None` when it names none or several ("from
/// balanced to fresh" is a movement, not a reading).
///
/// A name inside a longer one does not count on its own: "fresh" in "very
/// fresh" is not a claim of the fresh band.
fn claimed_band(lower: &str) -> Option<FormBand> {
    let found: Vec<(usize, usize, Option<FormBand>)> = BAND_TERMS
        .iter()
        .flat_map(|&(term, band)| {
            word_matches(lower, term).map(move |(start, end)| (start, end, band))
        })
        .collect();
    let mut named = found
        .iter()
        .filter(|&&(start, end, _)| {
            !found
                .iter()
                .any(|&(s, e, _)| s <= start && e >= end && e - s > end - start)
        })
        .filter_map(|&(_, _, band)| band);
    let first = named.next()?;
    named.all(|band| band == first).then_some(first)
}

/// Which figures of the sentence state form, and whether it is about form.
fn stated_form(lower: &str, figures: &[Figure]) -> Stated {
    // Keyed by where the figure starts, so a figure two labels both sit
    // beside is judged once.
    let mut percents: BTreeMap<usize, f64> = BTreeMap::new();
    let mut tsbs: BTreeMap<usize, f64> = BTreeMap::new();
    let mut labelled = false;

    for (labels, reads_bare_tsb) in [(FORM_LABELS, true), (FORM_POSSESSIVES, false)] {
        for (start, end) in labels
            .iter()
            .filter_map(|label| word_matches(lower, label).next())
        {
            labelled = true;
            let Some(nearest) = nearest_figure(figures, start, end).filter(|f| !f.limit) else {
                continue;
            };
            if nearest.percent {
                if let Some(pct) = stated_percent(lower, nearest, true)
                    .filter(|_| !shares_another_load(lower, nearest))
                {
                    percents.insert(nearest.start, pct);
                }
            } else if reads_bare_tsb && !counts_duration(lower, nearest) {
                tsbs.insert(nearest.start, nearest.value);
            }
        }
    }

    let about_form = labelled || FORM_WORDS.iter().any(|word| has_word(lower, word));
    let mut shares_fitness = false;
    for figure in figures
        .iter()
        .filter(|f| f.percent && !shares_another_load(lower, f))
    {
        let tail = tail_after(lower, figure.end);
        if names_fitness(tail) && (starts_with_particle(tail) || side_of(tail).is_some()) {
            shares_fitness = true;
            if let Some(pct) = stated_percent(lower, figure, about_form).filter(|_| !figure.limit) {
                percents.insert(figure.start, pct);
            }
        }
    }

    Stated {
        percents: percents.into_values().collect(),
        tsbs: tsbs.into_values().collect(),
        anchored: labelled || shares_fitness,
        in_state: STATE_PHRASES.iter().any(|phrase| has_word(lower, phrase)),
        defines: figures.iter().any(|f| f.limit),
    }
}

/// Whether the number counts days or weeks rather than TSB points.
fn counts_duration(lower: &str, figure: &Figure) -> bool {
    let tail = tail_after(lower, figure.end).trim_start();
    let unit: String = tail.chars().take_while(|c| c.is_alphabetic()).collect();
    DURATION_UNITS.contains(&unit.as_str())
}

/// The figure nearest the label at `start..end`, within the window a keyword
/// reaches elsewhere in this layer.
///
/// The gap is measured from the label's edge on the figure's side, so a long
/// label does not push its own value further away than a figure before it. At
/// an equal gap the figure after the label wins, because a label is written
/// before its value: "ATL 140 TSB -40" puts -40 on the TSB, not 140.
fn nearest_figure(figures: &[Figure], start: usize, end: usize) -> Option<&Figure> {
    figures
        .iter()
        .filter(|f| f.start + WINDOW_BYTES >= start && f.start <= end.saturating_add(WINDOW_BYTES))
        .min_by_key(|f| {
            if f.start >= end {
                (f.start - end, false)
            } else {
                (start.saturating_sub(f.end), true)
            }
        })
}

/// Whether the clause before a share makes it a share of another load: the
/// last quantity named in the sentence before the figure is the acute load, a
/// volume or a fatigue rather than form or the athlete. "Your acute load is
/// high, and your form is 40% below your fitness" names form last, so its
/// share is still form's.
fn shares_another_load(lower: &str, figure: &Figure) -> bool {
    let before = lower.get(..figure.start).unwrap_or_default();
    let head = before
        .rfind(SENTENCE_END)
        .and_then(|i| before.get(i + 1..))
        .unwrap_or(before);
    let last_end = |terms: &[&str]| {
        terms
            .iter()
            .filter_map(|term| word_matches(head, term).last())
            .map(|(_, end)| end)
            .max()
    };
    let other = last_end(OTHER_LOADS);
    let form = [FORM_LABELS, FORM_POSSESSIVES, FORM_WORDS, STATE_PHRASES]
        .into_iter()
        .filter_map(last_end)
        .max();
    other.is_some_and(|other| form.is_none_or(|form| other > form))
}

/// The share of fitness a percentage figure states, signed.
///
/// A written sign is taken as written. Without one, the clause must name
/// the fitness the share is of: a side ("30% below your fitness") fixes the
/// sign, and otherwise the share is positive when the sentence is about form,
/// because the product renders only a negative form with a sign. An unsigned
/// share that names no fitness ("your form improved 5%") is not a reading.
fn stated_percent(lower: &str, figure: &Figure, about_form: bool) -> Option<f64> {
    if figure.signed {
        return Some(figure.value);
    }
    let tail = tail_after(lower, figure.end);
    if !names_fitness(tail) {
        return None;
    }
    match side_of(tail) {
        Some(Side::Below) => Some(-figure.value),
        Some(Side::Above) => Some(figure.value),
        None => (about_form && starts_with_particle(tail)).then_some(figure.value),
    }
}

/// The clause after a figure, at most [`TAIL_CHARS`] characters and never past
/// the end of the sentence.
fn tail_after(lower: &str, from: usize) -> &str {
    let rest = lower.get(from..).unwrap_or_default();
    let sentence_end = rest
        .char_indices()
        .take(TAIL_CHARS)
        .find(|(_, c)| SENTENCE_END.contains(c))
        .map(|(i, _)| i);
    let cap = rest
        .char_indices()
        .nth(TAIL_CHARS)
        .map_or(rest.len(), |(i, _)| i);
    rest.get(..sentence_end.unwrap_or(cap)).unwrap_or_default()
}

fn names_fitness(tail: &str) -> bool {
    FITNESS_NOUNS.iter().any(|noun| has_word(tail, noun))
}

/// Whether the clause opens with "of", «de», «der»… — the shape of "% of your
/// fitness".
fn starts_with_particle(tail: &str) -> bool {
    let tail = tail.trim_start();
    let elided = tail
        .strip_prefix('d')
        .is_some_and(|rest| rest.starts_with(['\'', '’']));
    elided
        || PARTICLES.iter().any(|particle| {
            tail.strip_prefix(particle)
                .is_some_and(|rest| rest.starts_with([' ', '\u{a0}']))
        })
}

fn side_of(tail: &str) -> Option<Side> {
    if BELOW_WORDS.iter().any(|word| has_word(tail, word)) {
        Some(Side::Below)
    } else if ABOVE_WORDS.iter().any(|word| has_word(tail, word)) {
        Some(Side::Above)
    } else {
        None
    }
}

/// Every whole-word occurrence of `term` in `lower`, as byte spans. A letter
/// on either side disqualifies it: "unproductive" is not "productive".
fn word_matches<'a>(lower: &'a str, term: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    lower.match_indices(term).filter_map(move |(start, found)| {
        let end = start + found.len();
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphabetic());
        let before = lower.get(..start).and_then(|s| s.chars().next_back());
        let after = lower.get(end..).and_then(|s| s.chars().next());
        (edge(before) && edge(after)).then_some((start, end))
    })
}

fn has_word(lower: &str, term: &str) -> bool {
    word_matches(lower, term).next().is_some()
}

/// Every number in the text, in order.
fn figures(lower: &str) -> Vec<Figure> {
    let chars: Vec<(usize, char)> = lower.char_indices().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some((figure, next)) = scan_figure(lower, &chars, i) {
            found.push(figure);
            i = next;
        } else {
            i += 1;
        }
    }
    mark_percent_ranges(lower, &mut found);
    found
}

/// The figure starting at character `i`, with the index to resume from.
///
/// A sign belongs to the number only when no digit precedes it, which is what
/// keeps "5-15" a range. A digit that follows a letter is part of a word
/// ("vo2max") and is not read. A comma between digits is a decimal separator,
/// as it is in four of the five shipped locales.
fn scan_figure(lower: &str, chars: &[(usize, char)], i: usize) -> Option<(Figure, usize)> {
    let char_at = |idx: usize| chars.get(idx).map(|&(_, c)| c);
    let byte_at = |idx: usize| chars.get(idx).map_or(lower.len(), |&(b, _)| b);
    let is_digit = |idx: usize| char_at(idx).is_some_and(|c| c.is_ascii_digit());

    let first = char_at(i)?;
    let previous = i.checked_sub(1).and_then(char_at);
    let signed = matches!(first, '-' | '\u{2212}' | '+');
    if signed {
        if previous.is_some_and(|c| c.is_ascii_digit()) || !is_digit(i + 1) {
            return None;
        }
    } else if !first.is_ascii_digit() || previous.is_some_and(char::is_alphabetic) {
        return None;
    }

    let digits_from = if signed { i + 1 } else { i };
    let mut j = digits_from;
    while is_digit(j) {
        j += 1;
    }
    if matches!(char_at(j), Some('.' | ',')) && is_digit(j + 1) {
        j += 1;
        while is_digit(j) {
            j += 1;
        }
    }
    let digits: String = chars
        .get(digits_from..j)?
        .iter()
        .map(|&(_, c)| if c == ',' { '.' } else { c })
        .collect();
    let magnitude: f64 = digits.parse().ok()?;

    let mut unit_from = j;
    while char_at(unit_from).is_some_and(|c| matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '\u{2009}'))
    {
        unit_from += 1;
    }
    let unit_len = percent_unit_len(lower.get(byte_at(unit_from)..)?);
    let figure = Figure {
        value: if matches!(first, '-' | '\u{2212}') {
            -magnitude
        } else {
            magnitude
        },
        signed,
        percent: unit_len.is_some(),
        limit: follows_limit_word(lower, byte_at(i)),
        start: byte_at(i),
        end: unit_len.map_or_else(|| byte_at(j), |len| byte_at(unit_from) + len),
    };
    Some((figure, j))
}

/// The length of the percent unit `rest` opens with: a `%` sign or a spelled
/// word ("pour cent"), the word not running into a longer one ("percentage").
fn percent_unit_len(rest: &str) -> Option<usize> {
    if rest.starts_with('%') {
        return Some('%'.len_utf8());
    }
    PERCENT_WORDS
        .iter()
        .find(|word| {
            rest.strip_prefix(**word)
                .is_some_and(|after| after.chars().next().is_none_or(|c| !c.is_alphabetic()))
        })
        .map(|word| word.len())
}

/// Whether the words before `start` make the figure there a limit ("below
/// -30%", «en dessous de -30 %», «au moins -20 %»).
fn follows_limit_word(lower: &str, start: usize) -> bool {
    let mut before = lower.get(..start).unwrap_or_default().trim_end();
    if let Some(link) = LIMIT_LINKS.iter().find(|link| {
        before
            .strip_suffix(**link)
            .is_some_and(|rest| rest.chars().next_back().is_some_and(|c| !c.is_alphabetic()))
    }) {
        before = before.strip_suffix(*link).unwrap_or(before).trim_end();
    }
    BELOW_WORDS
        .iter()
        .chain(ABOVE_WORDS)
        .chain(LIMIT_WORDS)
        .any(|word| {
            before
                .strip_suffix(word)
                .is_some_and(|rest| rest.chars().next_back().is_none_or(|c| !c.is_alphabetic()))
        })
}

/// Mark the two ends of a percent range as limits, and give the first end the
/// unit of the last: in «-20 à -30 %» both ends are shares of fitness, and the
/// first must not be taken for a bare TSB. A range of bare numbers is left as
/// it is.
fn mark_percent_ranges(lower: &str, found: &mut [Figure]) {
    for idx in (0..found.len().saturating_sub(1)).rev() {
        let joined = found.get(idx..=idx + 1).is_some_and(|pair| {
            (pair[0].percent || pair[1].percent)
                && lower
                    .get(pair[0].end..pair[1].start)
                    .is_some_and(|between| RANGE_CONNECTORS.contains(&between.trim()))
        });
        if joined {
            found[idx].percent = true;
            found[idx].limit = true;
            found[idx + 1].limit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use pierre_core::models::DailyTrainingState;

    use super::*;
    use crate::personalized::{AthleteMetrics, TightStrategy};

    fn lower_figures(text: &str) -> Vec<Figure> {
        figures(&text.to_lowercase())
    }

    fn reading(ctl: f64, atl: f64) -> FormReading {
        DailyTrainingState {
            ctl,
            atl,
            tsb: ctl - atl,
            form_ctl: ctl,
            ..DailyTrainingState::zero(NaiveDate::from_ymd_opt(2026, 10, 9).unwrap())
        }
        .form_reading()
    }

    #[test]
    fn a_sign_is_read_where_a_range_hyphen_is_not() {
        let signed = lower_figures("form at -30% today");
        assert_eq!(signed.len(), 1);
        assert!((signed[0].value + 30.0).abs() < f64::EPSILON);
        assert!(signed[0].signed && signed[0].percent);

        let range = lower_figures("zone 2-3");
        assert_eq!(
            range.iter().map(|f| f.value).collect::<Vec<_>>(),
            [2.0, 3.0]
        );
        assert!(range.iter().all(|f| !f.signed));
    }

    #[test]
    fn the_unicode_minus_and_a_typographic_space_before_the_percent_are_read() {
        let figures = lower_figures("Forme à −30\u{202f}% de ta condition physique");
        assert_eq!(figures.len(), 1);
        assert!((figures[0].value + 30.0).abs() < f64::EPSILON);
        assert!(figures[0].percent, "a narrow no-break space is typography");
    }

    #[test]
    fn a_decimal_comma_is_a_decimal_and_a_sentence_period_is_not() {
        let figures = lower_figures("Forme à -64,5 % de ta condition physique. Puis 12.");
        assert!((figures[0].value + 64.5).abs() < 1e-9);
        assert!((figures[1].value - 12.0).abs() < f64::EPSILON);
        assert_eq!(figures.len(), 2);
    }

    #[test]
    fn a_spelled_percent_counts_and_a_longer_word_does_not() {
        assert!(lower_figures("moins 30 pour cent de ta fitness")[0].percent);
        assert!(lower_figures("-30 prozent deiner fitness")[0].percent);
        assert!(!lower_figures("30 percentage points")[0].percent);
    }

    #[test]
    fn a_digit_inside_a_word_is_not_a_figure() {
        assert!(lower_figures("your vo2max is fine").is_empty());
    }

    #[test]
    fn a_range_ending_in_a_percent_gives_its_first_end_the_unit_and_makes_both_limits() {
        let figures = lower_figures("form between -20 and -30% of your fitness");
        assert_eq!(figures.len(), 2);
        assert!(figures.iter().all(|f| f.percent && f.limit));
    }

    #[test]
    fn a_range_of_bare_numbers_stays_a_pair_of_readings() {
        let figures = lower_figures("tsb between 70-77 this week");
        assert_eq!(figures.len(), 2);
        assert!(figures.iter().all(|f| !f.percent && !f.limit));
    }

    #[test]
    fn a_figure_after_a_limit_word_is_a_limit() {
        for text in [
            "form below -30% of ctl",
            "form above +20%",
            "ta forme en dessous de −30 %",
            "tu forma por debajo de -30%",
            "at least -20% of your fitness",
            "unter -30 %",
            "form falls under -30%",
        ] {
            assert!(lower_figures(text).iter().all(|f| f.limit), "{text}");
        }
        for text in ["form at -30%", "your side -30%", "tu es à -30 %"] {
            assert!(lower_figures(text).iter().all(|f| !f.limit), "{text}");
        }
    }

    #[test]
    fn a_name_inside_a_longer_name_is_not_a_claim_of_the_shorter() {
        assert_eq!(
            claimed_band("your form is very fresh"),
            Some(FormBand::Detraining)
        );
        assert_eq!(
            claimed_band("ta forme est très frais"),
            Some(FormBand::Detraining)
        );
        assert_eq!(claimed_band("your form is fresh"), Some(FormBand::Fresh));
    }

    #[test]
    fn the_productive_zone_is_not_the_productive_band() {
        assert_eq!(claimed_band("the deep end of the productive zone"), None);
        assert_eq!(
            claimed_band("your form is productive"),
            Some(FormBand::Productive)
        );
        assert_eq!(claimed_band("an unproductive week"), None);
    }

    #[test]
    fn two_bands_in_one_sentence_are_a_movement_not_a_reading() {
        assert_eq!(claimed_band("from balanced to fresh"), None);
    }

    #[test]
    fn feminine_forms_agree_with_forme_and_forma() {
        assert_eq!(claimed_band("ta forme est fraîche"), Some(FormBand::Fresh));
        assert_eq!(
            claimed_band("tu forma está equilibrada"),
            Some(FormBand::Balanced)
        );
        assert_eq!(
            claimed_band("a tua forma está produtiva"),
            Some(FormBand::Productive)
        );
    }

    #[test]
    fn the_tsb_tolerance_scales_with_the_ctl_it_is_a_share_of() {
        let low = reading(30.0, 50.0);
        let high = reading(110.0, 150.0);
        assert!((tsb_tolerance(&low) - 1.5).abs() < 1e-9);
        assert!((tsb_tolerance(&high) - 5.5).abs() < 1e-9);
    }

    #[test]
    fn an_explanation_quotes_the_share_the_product_printed() {
        // The tools print -192.5 as -193 (`FormReading::inline` rounds); a
        // format that rounded half to even would quote -192 back.
        let form = FormReading {
            form_pct: Some(-192.5),
            ..reading(40.0, 117.0)
        };
        let metrics = AthleteMetrics::default();
        let ctx = PersonalizedContext {
            metrics: &metrics,
            tolerance: &TightStrategy,
        };
        let verdict = percent_verdict(&ctx, &form, -193.0).unwrap();
        assert_eq!(verdict.status, ClaimStatus::Supported);
        assert!(
            verdict.explanation.contains("vs your -193%"),
            "{}",
            verdict.explanation
        );
    }

    #[test]
    fn the_tsb_tolerance_never_drops_below_the_rounding_the_product_prints() {
        let thin = reading(10.0, 12.0);
        assert!((tsb_tolerance(&thin) - MIN_TSB_TOLERANCE).abs() < f64::EPSILON);
    }

    #[test]
    fn a_band_the_reading_sits_beside_is_near_and_a_distant_one_is_not() {
        // -29% of fitness is heavy block, one point from the deep fatigue edge.
        assert!(near_band(FormBand::DeepFatigue, -29.0));
        assert!(near_band(FormBand::Productive, -24.0));
        assert!(!near_band(FormBand::Fresh, -29.0));
        assert!(!near_band(FormBand::Detraining, -64.0));
    }

    #[test]
    fn a_share_names_its_side_or_is_positive_only_when_the_sentence_is_about_form() {
        let below = "30% below your fitness";
        assert_eq!(
            stated_percent(below, &figures(below)[0], false),
            Some(-30.0)
        );
        let above = "30% over your fitness";
        assert_eq!(stated_percent(above, &figures(above)[0], false), Some(30.0));

        let bare = "18% of your fitness";
        assert_eq!(stated_percent(bare, &figures(bare)[0], true), Some(18.0));
        assert_eq!(
            stated_percent(bare, &figures(bare)[0], false),
            None,
            "\"80% of your fitness\" after a layoff is a fitness level, not a form"
        );
    }

    #[test]
    fn an_unsigned_share_that_names_no_fitness_is_not_a_reading() {
        let lower = "your form improved 5% this week";
        assert_eq!(stated_percent(lower, &figures(lower)[0], true), None);
    }

    #[test]
    fn a_reply_is_cut_at_the_end_of_its_sentence_before_the_fitness_is_looked_for() {
        let lower = "30% over the last week. your fitness is up";
        assert_eq!(stated_percent(lower, &figures(lower)[0], true), None);
    }

    #[test]
    fn a_count_of_days_is_not_a_tsb() {
        let lower = "your current form over 7 days";
        let found = figures(lower);
        assert!(counts_duration(lower, &found[0]));
        let lower = "your current form is -12";
        assert!(!counts_duration(lower, &figures(lower)[0]));
    }
}
