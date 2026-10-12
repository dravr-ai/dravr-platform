// ABOUTME: Which sentences the period check leaves to the per-activity check
// ABOUTME: A goal, part of a session, a pace or a speed, a named day, or a supposition is never a period total

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A figure beside a period phrase is not always the period's total. "45
//! minutes at threshold this week", "8 km at an easy pace this week", "you rode
//! at 28 km/h this week" and "18 km on Monday and Wednesday this week" are each
//! true of a week whose total is something else, and adding the week up would
//! contradict them. Each cue here hands the sentence to the per-activity
//! check, which never contradicts a connected athlete.
//!
//! The asymmetry is the one the vocabulary lists follow: a cue that should not
//! have fired costs one unadjudicated claim, a missing one a warning banner on
//! a true reply.

use pierre_core::civil_time::{relative_day_forms, weekday_forms, RelativeDay, ALL_WEEKDAYS};

use super::super::{find_word, spells_rate, AssertedFigure};
use super::vocabulary::{AVERAGE_CUES, DECLINE_CUES, OTHER_METRICS, PART_CUES};
use super::{any_phrase, phrase_matches};

/// Whether the period check leaves this sentence to the per-activity check.
pub(super) fn declines(
    lower: &str,
    tokens: &[&str],
    figures: &[AssertedFigure],
    locale: &str,
) -> bool {
    any_phrase(tokens, DECLINE_CUES)
        || any_phrase(tokens, PART_CUES)
        || counts_repetitions(lower, tokens)
        || at_a_rate(lower, tokens, figures)
        || names_a_day(lower, locale)
}

/// Whether the text counts repetitions of a piece of a session: "6 x 800 m",
/// "6x800m", "6×800". The figure beside a rep count is one repetition or the
/// set, never the period's total.
fn counts_repetitions(lower: &str, tokens: &[&str]) -> bool {
    let number = |token: &str| !token.is_empty() && token.chars().all(|c| c.is_ascii_digit());
    lower.contains('\u{d7}')
        || tokens
            .windows(2)
            .any(|pair| matches!(pair, [count, "x"] if number(count)))
        || tokens.iter().any(|token| {
            let rest = token.trim_start_matches(|c: char| c.is_ascii_digit());
            rest.len() < token.len() && rest.starts_with('x')
        })
}

/// Whether the sentence puts its figure at a pace, a speed or an intensity
/// rather than stating an amount of training.
///
/// - A figure that is itself a rate ("you rode at 28 km/h", "à 5 min/km").
/// - A pace, speed, heart-rate or power word no average cue qualifies: "at an
///   easy pace", "à allure facile", "a ritmo suave", "in lockerem Tempo". The
///   distance beside it may be a stretch run at that pace.
/// - A rate with no such figure, like the clock-shaped "5:20/km".
///
/// An average pace stated beside a total describes the whole of it — "42 km
/// this week at an average pace of 5:20 min/km" — so a metric an average cue
/// qualifies keeps the sentence, and the rate that goes with it.
fn at_a_rate(lower: &str, tokens: &[&str], figures: &[AssertedFigure]) -> bool {
    let averages: Vec<(usize, usize)> = AVERAGE_CUES
        .iter()
        .flat_map(|cue| phrase_matches(tokens, cue))
        .map(|(a, b, _)| (a, b))
        .collect();
    let metrics: Vec<bool> = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| OTHER_METRICS.contains(token))
        .map(|(at, _)| averages.iter().any(|&(a, b)| b == at || a == at + 1))
        .collect();
    figures.iter().any(|figure| figure.rate)
        || metrics.iter().any(|averaged| !averaged)
        || (metrics.is_empty() && spells_rate(lower))
}

/// Whether the sentence names a day: "18 km on Monday and Wednesday this
/// week", "your 60 km yesterday, your biggest day this week". The figure is
/// that day's, or a sum of some days, never the period's.
///
/// Weekday names are read in every locale, as the per-activity check reads
/// them; today and yesterday only in the reply's locale, because French `hier`
/// is German "here".
fn names_a_day(lower: &str, locale: &str) -> bool {
    let named = |forms: &[&str]| forms.iter().any(|form| find_word(lower, form).is_some());
    ALL_WEEKDAYS
        .into_iter()
        .any(|day| named(weekday_forms(day)))
        || [RelativeDay::Today, RelativeDay::Yesterday]
            .into_iter()
            .any(|day| named(relative_day_forms(day, locale)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::athlete_data::{extract_figures, word_spans};

    fn at_a_rate_in(text: &str) -> bool {
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = word_spans(&lower)
            .into_iter()
            .filter_map(|(a, b)| lower.get(a..b))
            .collect();
        at_a_rate(&lower, &tokens, &extract_figures(text))
    }

    #[test]
    fn a_pace_or_a_speed_is_a_rate_in_every_shipped_locale() {
        for text in [
            "You ran at 5 min/km this week.",
            "You rode at 28 km/h this week.",
            "You rode 120 km at 28 kph this week.",
            "You ran 8 km at 5:20/km this week.",
            "You swam 3 km at 1:45/100m this week.",
            "You ran 8 km at an easy pace this week.",
            "Tu as couru 8 km en allure facile cette semaine.",
            "Esta semana corriste 8 km a ritmo suave.",
            "Diese Woche bist du 8 km in lockerem Tempo gelaufen.",
            "Nesta semana correste 8 km em ritmo leve.",
            "You rode 120 km this week at an average speed of 28 km/h.",
        ] {
            assert!(at_a_rate_in(text), "a pace or a speed: {text}");
        }
    }

    #[test]
    fn an_amount_over_the_period_is_not_a_rate() {
        for text in [
            "You ran 22 km this week.",
            "You ran 22 km this week at an average pace of 5:20 min/km.",
            "You averaged 40 km per week this block.",
            "Tu as couru en moyenne 12,5 km par sortie cette semaine.",
            "You ran 22 km/week this month.",
        ] {
            assert!(!at_a_rate_in(text), "an amount, not a rate: {text}");
        }
    }

    #[test]
    fn a_named_day_is_read_in_every_locale_and_a_relative_day_in_the_replys() {
        assert!(names_a_day("you ran 18 km on monday and wednesday", "en"));
        assert!(names_a_day("esta semana corriste 18 km el sábado", "en"));
        assert!(names_a_day("tu as roulé 60 km hier", "fr"));
        // German `hier` is "here", not yesterday.
        assert!(!names_a_day(
            "hier bist du diese woche 22 km gelaufen",
            "de"
        ));
        assert!(!names_a_day("you ran 22 km this week", "en"));
    }
}
