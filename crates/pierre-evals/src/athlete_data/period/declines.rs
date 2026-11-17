// ABOUTME: Which sentences the period check leaves to the per-activity check
// ABOUTME: A goal, part of a session, a pace, a named day, place, gear or race, or a supposition is never a period total

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A figure beside a period phrase is not always the period's total. "45
//! minutes at threshold this week", "8 km at an easy pace this week", "you rode
//! at 28 km/h this week" and "18 km on Monday and Wednesday this week" are each
//! true of a week whose total is something else, and adding the week up would
//! contradict them. Each cue here hands the sentence to the per-activity
//! check, which never contradicts a connected athlete.
//!
//! So is a figure over a subset no cue list can hold: "212 km at Gai-Luron",
//! on a bike the athlete named, or on a quoted name ("on \"Tour du Lac\"").
//! The athlete's own place and gear names come with the record.
//!
//! The asymmetry is the one the vocabulary lists follow: a cue that should not
//! have fired costs one unadjudicated claim, a missing one a warning banner on
//! a true reply.

use pierre_core::civil_time::{relative_day_forms, weekday_forms, RelativeDay, ALL_WEEKDAYS};
use pierre_core::text_fold::fold_for_match;

use super::super::{find_word, spells_rate, AssertedFigure, AthleteRecord, MIN_MATCHABLE_NAME};
use super::vocabulary::{AVERAGE_CUES, DECLINE_CUES, OTHER_METRICS, PART_CUES};
use super::{any_phrase, phrase_matches};

/// Whether the period check leaves this sentence to the per-activity check.
pub(super) fn declines(
    lower: &str,
    tokens: &[&str],
    figures: &[AssertedFigure],
    record: &AthleteRecord,
) -> bool {
    any_phrase(tokens, DECLINE_CUES)
        || any_phrase(tokens, PART_CUES)
        || counts_repetitions(lower, tokens)
        || at_a_rate(lower, tokens, figures)
        || names_a_day(lower, &record.locale)
        || quotes_a_name(lower)
        || names_a_subset(lower, &record.named_subsets)
}

/// Opening and closing marks of a quoted span, in the five shipped locales.
/// An apostrophe is never one: "that's", "l'été".
const QUOTE_PAIRS: [(char, char); 4] = [
    ('"', '"'),
    ('\u{ab}', '\u{bb}'),
    ('\u{201c}', '\u{201d}'),
    ('\u{201e}', '\u{201c}'),
];

/// Whether the sentence quotes a name: "212 km on \"Tour du Lac\"". A quoted
/// name is a filter a period total cannot recount. An empty pair quotes
/// nothing.
fn quotes_a_name(lower: &str) -> bool {
    QUOTE_PAIRS.iter().any(|&(open, close)| {
        let mut rest = lower;
        while let Some((_, after)) = rest.split_once(open) {
            let Some((inside, tail)) = after.split_once(close) else {
                return false;
            };
            if inside.chars().any(char::is_alphanumeric) {
                return true;
            }
            rest = tail;
        }
        false
    })
}

/// Whether the sentence names one of the athlete's places or gear, as a whole
/// word: "212 km at Gai-Luron". A name shorter than [`MIN_MATCHABLE_NAME`]
/// is ordinary prose more often than the place ("Run", "Gai").
fn names_a_subset(lower: &str, subsets: &[String]) -> bool {
    let mut matchable = subsets
        .iter()
        .filter(|subset| subset.chars().count() >= MIN_MATCHABLE_NAME)
        .peekable();
    if matchable.peek().is_none() {
        return false;
    }
    let sentence = fold_for_match(lower);
    matchable.any(|subset| find_word(&sentence, subset).is_some())
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
    use super::super::PeriodHistory;
    use super::*;
    use crate::athlete_data::{extract_figures, word_spans, AthleteRecord, RecordedActivity};
    use chrono::NaiveDate;
    use pierre_core::models::SportType;
    use pierre_core::text_fold::fold_for_match;
    use pierre_memory::ClaimStatus;

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

    /// Wednesday 2026-09-09: 100 + 80 + 80 = 260 km of riding this month and
    /// 20 + 30 = 50 km of running, every day of it vouched for.
    fn month_record(locale: &str, named_subsets: &[&str]) -> AthleteRecord {
        let today = NaiveDate::from_ymd_opt(2026, 9, 9).unwrap();
        let session = |d: u32, sport: SportType, km: f64| RecordedActivity {
            date: NaiveDate::from_ymd_opt(2026, 9, d).unwrap(),
            sport,
            name: String::new(),
            distance_km: Some(km),
            duration_min: km * 3.0,
            elevation_m: None,
            average_hr_bpm: None,
        };
        let sessions = vec![
            session(2, SportType::Ride, 100.0),
            session(3, SportType::Run, 20.0),
            session(5, SportType::Ride, 80.0),
            session(7, SportType::Run, 30.0),
            session(8, SportType::Ride, 80.0),
        ];
        AthleteRecord {
            has_provider: true,
            today,
            locale: locale.to_owned(),
            activities: sessions.clone(),
            history: Some(PeriodHistory {
                sessions,
                by_source: Vec::new(),
                covered_from: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
                synced_through: today,
                block_start: None,
            }),
            named_subsets: named_subsets.iter().map(|n| fold_for_match(n)).collect(),
        }
    }

    /// The period check's verdict on `text`, `None` when it declines.
    fn period_status(text: &str, record: &AthleteRecord) -> Option<ClaimStatus> {
        super::super::check(text, &extract_figures(text), record).map(|v| v.status)
    }

    #[test]
    fn a_figure_at_a_place_the_athlete_named_is_left_to_the_per_activity_check() {
        let text = "You rode 212 km in total at Gai-Luron this month.";
        assert_eq!(
            period_status(text, &month_record("en", &[])),
            Some(ClaimStatus::Contradicted),
            "bug: without the place name the figure must still read as the month's total"
        );
        let named = month_record("en", &["Gai-Luron", "Trek Fuel EX"]);
        assert_eq!(
            period_status(text, &named),
            None,
            "bug: a total at a place the athlete named is a subset, never the month's total"
        );
        assert_eq!(
            period_status(
                "You rode 212 km in total on the trek fuel ex this month.",
                &named
            ),
            None,
            "bug: a total on a bike the athlete named is a subset, never the month's total"
        );
        assert_eq!(
            period_status("You rode 260 km this month.", &named),
            Some(ClaimStatus::Supported),
            "bug: naming places and gear must not silence the month's own total"
        );
    }

    #[test]
    fn a_short_or_empty_name_never_declines_a_sentence() {
        let record = month_record("en", &["", "  ", "Run", "Gai"]);
        assert_eq!(
            period_status("You rode 212 km in total this month.", &record),
            Some(ClaimStatus::Contradicted),
            "bug: an empty or three-letter name matched ordinary prose"
        );
        let luron = month_record("en", &["Luron"]);
        assert_eq!(
            period_status("You rode 212 km in total at Luronne this month.", &luron),
            Some(ClaimStatus::Contradicted),
            "bug: a name matched inside a longer word"
        );
    }

    #[test]
    fn segment_gear_and_race_figures_are_declined_in_every_locale() {
        let en = month_record("en", &[]);
        for text in [
            "You rode 212 km in total on that segment this month.",
            "You ran 35 km in total in those shoes this month.",
            "You rode 212 km in total on that gear this month.",
            "You ran 35 km in total in races this month.",
        ] {
            assert_eq!(period_status(text, &en), None, "bug: not declined: {text}");
        }
        let fr = month_record("fr", &[]);
        for text in [
            "Tu as couru 35 km en tout en compétition ce mois-ci.",
            "Tu as couru 35 km en tout avec ces chaussures ce mois-ci.",
            "Tu as roulé 212 km en tout sur ce segment ce mois-ci.",
        ] {
            assert_eq!(period_status(text, &fr), None, "bug: not declined: {text}");
        }
    }

    #[test]
    fn french_course_is_still_read_as_the_run_total() {
        let fr = month_record("fr", &[]);
        assert_eq!(
            period_status("Tu as couru 50 km en tout ce mois-ci, en 2 courses.", &fr),
            Some(ClaimStatus::Supported),
            "bug: `course` is the French run, never a race cue"
        );
        assert_eq!(
            period_status("Tu as couru 35 km en tout ce mois-ci, en 2 courses.", &fr),
            Some(ClaimStatus::Contradicted),
            "bug: a wrong French run total must still be read"
        );
    }

    #[test]
    fn a_quoted_name_is_declined_and_an_apostrophe_is_not_a_quote() {
        let en = month_record("en", &[]);
        for text in [
            "You rode 212 km in total on \"Tour du Lac\" this month.",
            "You rode 212 km in total on \u{201c}Tour du Lac\u{201d} this month.",
            "You rode 212 km in total on \u{ab} Tour du Lac \u{bb} this month.",
            "You rode 212 km in total on \u{201e}Tour du Lac\u{201c} this month.",
        ] {
            assert_eq!(period_status(text, &en), None, "bug: not declined: {text}");
        }
        assert_eq!(
            period_status("That's 212 km in total you rode this month.", &en),
            Some(ClaimStatus::Contradicted),
            "bug: an apostrophe read as a quoted span"
        );
        assert_eq!(
            period_status("You rode 212 km in total this month \"\".", &en),
            Some(ClaimStatus::Contradicted),
            "bug: an empty pair of quotes names nothing"
        );
    }
}
