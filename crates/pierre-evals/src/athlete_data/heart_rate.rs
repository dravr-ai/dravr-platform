// ABOUTME: Reads the average heart rate a reply states about one session, and holds it against that session
// ABOUTME: Only an average or a value that held is read; it is contradicted only on a session the other figures pin

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A heart rate is a figure of one session. « Ta dernière séance … ta FC est
//! restée à 133 en moyenne » is checked against the average the provider
//! recorded for that session (carnet#1188). `133` wears no unit, so the
//! generic figure reader never sees it; this module reads it from the cue
//! that names it a heart rate.
//!
//! Two narrowings keep this from manufacturing a contradiction:
//!
//! - **Only an average is read.** The record holds the session's average, so a
//!   peak (« ta FC est montée à 175 dans la côte »), a cap (« sous 140 »), a
//!   zone bound or a resting value is not compared with it. A figure counts
//!   when its clause calls it an average or says it held.
//! - **A miss is contradicted only on a pinned session.** When the sentence's
//!   other figures match exactly one held session — two of them, or one and
//!   the weekday it was on — the sentence is about that session, and its
//!   recorded average disagreeing is a finding. Anything looser is a miss this
//!   layer cannot settle, as a missed distance is: the record may lack the
//!   session the reply meant.
//!
//! Pinning does not reopen the sync-lag hazard the layer guards against. That
//! hazard is a reply about a session we never synced, matched against one we
//! did; a session we never synced that shares a held session's distance *and*
//! climb, or its distance *and* its weekday, is not a case worth a missed
//! contradiction. A session we hold without a heart rate is unverifiable,
//! never a miss read as an invention.

use chrono::Datelike;

use super::{
    agrees, describe, find_word, sole_weekday, trim_float, AssertedFigure, AthleteRecord,
    RecordedActivity,
};
use crate::claim_extractor::states_held_value;

/// How far a stated average heart rate may sit from the recorded one, in beats
/// per minute. A watch and an app round the same average differently by a
/// beat or two; a 5% band would let 133 pass for 140.
pub(super) const TOLERANCE_BPM: f64 = 3.0;

/// The words that name a heart rate, in the five shipped locales.
const CUES: &[&str] = &[
    "heart rate",
    "hr",
    "pulse",
    "fc",
    "fréquence cardiaque",
    "frequence cardiaque",
    "pouls",
    "frecuencia cardíaca",
    "frecuencia cardiaca",
    "pulso",
    "pulsaciones",
    "puls",
    "herzfrequenz",
    "frequência cardíaca",
    "frequencia cardiaca",
];

/// The words that call a figure an average, in the five shipped locales.
const AVERAGES: &[&str] = &[
    "average",
    "avg",
    "averaged",
    "mean",
    "moyenne",
    "moy",
    "media",
    "promedio",
    "medio",
    "durchschnitt",
    "durchschnittlich",
    "schnitt",
    "média",
];

/// Words that make a heart-rate figure something other than the session's
/// average: a peak, a resting value, a zone or threshold, a cap or a range.
const NOT_THE_AVERAGE: &[&str] = &[
    "max",
    "maximum",
    "maximale",
    "maximal",
    "máxima",
    "maxima",
    "máximo",
    "maximo",
    "peak",
    "pic",
    "pico",
    "spitze",
    "resting",
    "rest",
    "repos",
    "reposo",
    "ruhe",
    "repouso",
    "zone",
    "zona",
    "threshold",
    "seuil",
    "umbral",
    "schwelle",
    "limiar",
    "under",
    "below",
    "above",
    "sous",
    "dessous",
    "dessus",
    "bajo",
    "debajo",
    "unter",
    "über",
    "abaixo",
    "acima",
    "between",
    "entre",
    "zwischen",
];

/// How a sentence's heart rates stand against the record.
pub(super) enum Beats {
    /// It states none, or each one matches a session its other figures fit.
    Held,
    /// One matches no such session, and no single session is pinned.
    Missed,
    /// The session its other figures pin averaged something else.
    Contradicted(String),
    /// The session its other figures pin carries no heart rate on record, so
    /// the heart rate cannot be settled — though every other figure matched.
    Unrecorded(String),
}

/// Whether the unit-less number at `start..end` of `lower` is a session's
/// average heart rate: a heart-rate word names it — before it with no other
/// figure between, or "bpm" right after — and its clause calls it an average
/// or says it held, with nothing making it a peak, a cap or a zone.
pub(super) fn averaged_at(lower: &str, start: usize, end: usize) -> bool {
    let (from, to) = clause_around(lower, start, end);
    let (Some(clause), Some(before), Some(after)) = (
        lower.get(from..to),
        lower.get(from..start),
        lower.get(end..to),
    ) else {
        return false;
    };
    let after = after.trim_start();
    if after.starts_with('%') {
        return false;
    }
    let named = after.starts_with("bpm") || named_before(before);
    named
        && !NOT_THE_AVERAGE
            .iter()
            .any(|w| find_word(clause, w).is_some())
        && (AVERAGES.iter().any(|w| find_word(clause, w).is_some()) || states_held_value(clause))
}

/// Whether a heart-rate word stands in `before` after its last figure, with a
/// word between that figure and it: "2 hr was 140" is a duration, then a
/// number.
fn named_before(before: &str) -> bool {
    let (since, after_a_figure) = before
        .rfind(|c: char| c.is_ascii_digit())
        .and_then(|at| before.get(at + 1..))
        .map_or((before, false), |since| (since, true));
    CUES.iter().any(|cue| {
        find_word(since, cue).is_some_and(|(at, _)| {
            !after_a_figure
                || since
                    .get(..at)
                    .is_some_and(|gap| gap.chars().any(char::is_alphabetic))
        })
    })
}

/// The byte span of the clause holding `start..end`: bounded by `;`, a dash,
/// a parenthesis, a line break, or a comma followed by a space — never the
/// French decimal comma in « 6,7 km ».
fn clause_around(lower: &str, start: usize, end: usize) -> (usize, usize) {
    let hard = |c: char| matches!(c, ';' | '\n' | '(' | ')' | '—' | '–');
    let bytes = lower.as_bytes();
    let soft = |at: usize| bytes.get(at) == Some(&b',') && bytes.get(at + 1) == Some(&b' ');
    let from = lower
        .get(..start)
        .and_then(|head| {
            head.char_indices()
                .rev()
                .find(|&(at, c)| hard(c) || soft(at))
                .map(|(at, c)| at + c.len_utf8())
        })
        .unwrap_or(0);
    let to = lower
        .get(end..)
        .and_then(|tail| {
            tail.char_indices()
                .find(|&(at, c)| hard(c) || soft(end + at))
                .map(|(at, _)| end + at)
        })
        .unwrap_or(lower.len());
    (from, to)
}

/// Hold the heart rates `beats` against the sessions the sentence's `amounts`
/// fit. See the module docs for when a miss is a contradiction.
pub(super) fn assess(
    text: &str,
    beats: &[AssertedFigure],
    amounts: &[AssertedFigure],
    record: &AthleteRecord,
) -> Beats {
    if beats.is_empty() {
        return Beats::Held;
    }
    let sessions: Vec<&RecordedActivity> = record
        .activities
        .iter()
        .filter(|session| {
            amounts.iter().all(|figure| {
                session
                    .value_in(figure.unit)
                    .is_some_and(|actual| agrees(*figure, actual))
            })
        })
        .collect();
    let held = |beat: &AssertedFigure| {
        sessions
            .iter()
            .any(|session| session.average_hr_bpm.is_some_and(|hr| agrees(*beat, hr)))
    };
    if beats.iter().all(held) {
        return Beats::Held;
    }
    let Some(session) = pinned(text, amounts, &sessions) else {
        return Beats::Missed;
    };
    let Some(recorded) = session.average_hr_bpm else {
        return Beats::Unrecorded(format!(
            "Every other figure ({}) matches the session on record for {}, which carries no \
             heart rate, so the heart rate stated cannot be checked.",
            describe(amounts),
            session.date
        ));
    };
    let stated: Vec<AssertedFigure> = beats.iter().copied().filter(|b| !held(b)).collect();
    Beats::Contradicted(format!(
        "States an average heart rate of {} for the session its other figures ({}) place on {}, \
         which is on record at {} bpm.",
        describe(&stated),
        describe(amounts),
        session.date,
        trim_float(recorded)
    ))
}

/// The one held session the sentence is about, when its other figures say so:
/// they fit exactly one session, and there are two of them, or one with the
/// weekday that session was on.
fn pinned<'r>(
    text: &str,
    amounts: &[AssertedFigure],
    sessions: &[&'r RecordedActivity],
) -> Option<&'r RecordedActivity> {
    let [session] = sessions else {
        return None;
    };
    let on_its_day =
        || sole_weekday(&text.to_lowercase()).is_some_and(|(day, _)| day == session.date.weekday());
    (amounts.len() >= 2 || (amounts.len() == 1 && on_its_day())).then_some(*session)
}
