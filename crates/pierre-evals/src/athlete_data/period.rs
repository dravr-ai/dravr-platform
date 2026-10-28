// ABOUTME: Checks a claim that sums or averages the athlete's training over a stated period
// ABOUTME: Resolves the window on the athlete's calendar, computes the same aggregate, and declines when it cannot

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Totals and averages over a period
//!
//! *"You ran 42 km this week"* is the figure a coach writes most often, and it
//! matches no single activity: it is four runs added up. This module reads the
//! period a claim names, adds up the same sessions over the same days, and
//! compares.
//!
//! ## Window semantics
//!
//! Every window is a closed range of the athlete's **civil** days: the record's
//! `today` and every session date are already on the athlete's own clock (the
//! verification stage converts them with `pierre_core::civil_time`), so a
//! Sunday 22:30 run in Toronto belongs to Sunday, not to the UTC Monday it lands
//! in. Weeks start on Monday, as Home's weekly volume does.
//!
//! A period people say two ways is checked under both readings, and a figure
//! that agrees with **either** is supported:
//!
//! | Period | Readings |
//! |---|---|
//! | this week | the calendar week (Monday to today) · the last 7 days (today and the six before) |
//! | last week | the previous calendar week (Monday to Sunday) · the 7 days before the last 7 |
//! | the last *n* days / weeks, the past week | the *n* days ending today |
//! | this month | the calendar month to today |
//! | last month | the previous calendar month · the month ending today |
//! | this block | the current phase of the athlete's active plan, from its start to today |
//!
//! Accepting either reading costs a little precision against a wrong figure
//! that happens to equal the other reading. Picking one would put a warning on
//! a correct reply every time the coach meant the other, which is the failure
//! this verifier exists to avoid.
//!
//! ## When it declines
//!
//! A period total can only be contradicted when the record vouches for every
//! day of every reading. The `coverage` module states which days those are and
//! which signal vouches for them: each live source's latest head read bounds
//! the end, today included only when that read is this turn's, and the depth
//! its current capture's backfill reached bounds the start, by the same rule
//! the historical `get_activities` gate serves the cache by. Otherwise the
//! window includes days we may simply not hold, and the claim is
//! [`ClaimStatus::Unverifiable`]. The same goes for a block when the athlete
//! has no plan phase covering today, and for an average that never says per
//! what.
//!
//! A sentence carrying a target, a remaining distance or a comparison
//! ("8 km short of your goal", "12 km more than last week") is left to the
//! per-activity check: its figures are differences and plans, not record
//! values, and adding up the week would contradict a true sentence. So is one
//! whose figure measures part of a session, one intensity or a subset of the
//! week ("45 minutes at threshold", "40 minutes in zone 2", "6 x 800 m", "18 km
//! of easy running", "your long run", "on the treadmill"), one that puts it at
//! a pace or a speed ("at an easy pace", "at 28 km/h", "à 5 min/km"), one that
//! names a day ("on Monday and Wednesday", "yesterday"), and one that only
//! supposes the figure ("if you ran 30 km this week"). The `declines` module
//! reads them, in the five shipped locales.
//!
//! ## An unworded figure is contradicted only above the total
//!
//! With no word saying sum or average, "you ran 18 km this week" may state the
//! week's total, one session of it, or only some of its sessions: "18 km with
//! the club this week", "18 km over your last two runs this week". No
//! vocabulary can list every way a sentence picks a subset out, but a subset
//! never exceeds the whole. So such a figure is contradicted only when it is
//! above the total of every vouched reading, and so above every session in
//! them; one below a total that matches no session is
//! [`ClaimStatus::Unverifiable`]. A figure said to be the total or an average
//! is wrong whichever side of the record it falls. The `assess` module holds
//! that arithmetic and folds every figure's call into the one verdict.
//!
//! ## Which sessions a reading adds up
//!
//! The merged sessions of every source, for all sports and for each sport
//! trained in the window, or for the one sport the claim names — by a noun
//! ("ciclismo", "Radfahren") or by the verb ("you rode", "pedaleaste",
//! "geradelt", "você nadou"), in the five shipped locales. When the athlete's
//! sources hold different sessions, each source's own total is read too: a
//! tool that reads one provider adds up that provider's week, and a figure
//! read off it is that source's record, not an invention.
//!
//! [`ClaimStatus::Unverifiable`]: pierre_memory::ClaimStatus::Unverifiable

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use super::{describe, unverifiable, word_spans, AssertedFigure, AthleteRecord};
use crate::verdict_engine::VerdictOutcome;

mod assess;
use assess::{assess, verdict, Call};

mod coverage;
pub use coverage::{
    BackfillDepth, CacheRead, HistoryInputs, PeriodHistory, SourceCoverage, SourceSessions,
    ATHLETE_RECORD_WINDOW,
};

mod declines;
use declines::declines;

mod readings;
use readings::named_family;

mod vocabulary;
use vocabulary::{
    AVERAGE_CUES, NUMBER_WORDS, OTHER_METRICS, PERIOD_PHRASES, PER_FORMS, TOTAL_CUES,
};

/// A period a claim can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Period {
    ThisWeek,
    LastWeek,
    /// The given number of days ending today.
    LastDays(u32),
    ThisMonth,
    LastMonth,
    ThisBlock,
}

impl Period {
    /// How an operator reads the period in an explanation.
    fn label(self) -> String {
        match self {
            Self::ThisWeek => "this week".to_owned(),
            Self::LastWeek => "last week".to_owned(),
            Self::LastDays(days) => format!("the last {days} days"),
            Self::ThisMonth => "this month".to_owned(),
            Self::LastMonth => "last month".to_owned(),
            Self::ThisBlock => "this block".to_owned(),
        }
    }
}

/// What a period phrase resolves to: a fixed period, or a count of days per
/// unit when the phrase carries a number ("the last 14 days", "les 2
/// dernières semaines").
#[derive(Debug, Clone, Copy)]
enum Phrase {
    Is(Period),
    Count(u32),
}

/// Longest span a counted period may name. The record holds a quarter, so a
/// longer window is never covered; the bound only keeps the date arithmetic
/// away from overflow.
const MAX_PERIOD_DAYS: u32 = 366;

impl Phrase {
    fn resolve(self, count: Option<u32>) -> Option<Period> {
        match self {
            Self::Is(period) => Some(period),
            Self::Count(per_unit) => count
                .and_then(|n| n.checked_mul(per_unit))
                .filter(|days| (1..=MAX_PERIOD_DAYS).contains(days))
                .map(Period::LastDays),
        }
    }
}

/// What an average is taken per.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Per {
    Session,
    Day,
    Week,
}

/// What the claim says the figure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// No word says sum or average: "you ran 42 km this week". Read as the
    /// period's total, or as one session in it — "a 12 km run this week" is
    /// the same sentence shape — and, below the total, as possibly some of its
    /// sessions ("with the club", "over your last two runs").
    Implicit,
    /// Said to be a sum: "42 km in total this week".
    Total,
    /// Said to be an average per session, day or week.
    Average(Per),
    /// Said to be an average without saying per what, so it cannot be
    /// computed.
    UnstatedAverage,
}

/// One reading of a period: a closed range of civil days.
#[derive(Debug, Clone, Copy)]
struct Window {
    reading: &'static str,
    start: NaiveDate,
    end: NaiveDate,
}

impl Window {
    fn contains(&self, date: NaiveDate) -> bool {
        (self.start..=self.end).contains(&date)
    }

    fn days(&self) -> i64 {
        self.end.signed_duration_since(self.start).num_days() + 1
    }

    /// How many Monday-to-Sunday weeks the window touches, the denominator of
    /// a per-week average as a coach counts it ("over three weeks").
    fn weeks_touched(&self) -> i64 {
        match (monday_of(self.start), monday_of(self.end)) {
            (Some(first), Some(last)) => last.signed_duration_since(first).num_days() / 7 + 1,
            _ => 1,
        }
    }

    fn describe(&self) -> String {
        format!("{} {} to {}", self.reading, self.start, self.end)
    }
}

/// The Monday of the week `date` falls in.
fn monday_of(date: NaiveDate) -> Option<NaiveDate> {
    date.week(Weekday::Mon).checked_first_day()
}

/// Check a claim that states a total or an average over a period.
///
/// Returns `None` when the claim names no period, names several, carries a
/// goal or a comparison, measures part of a session, one intensity or a subset
/// of the period, states a pace or a speed, names a day, or only supposes its
/// figure — the per-activity check then runs as before. Returns
/// `None` too for a figure with no aggregate wording when the record cannot
/// vouch for any window, so a record built without a history keeps the
/// per-activity behaviour.
pub(super) fn check(
    text: &str,
    figures: &[AssertedFigure],
    record: &AthleteRecord,
) -> Option<VerdictOutcome> {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = word_spans(&lower)
        .into_iter()
        .filter_map(|(a, b)| lower.get(a..b))
        .collect();
    let period = sole_period(&tokens)?;
    if declines(&lower, &tokens, figures, &record.locale) {
        return None;
    }
    let shape = shape_of(&tokens);
    let stated = describe(figures);
    let label = period.label();

    let Some(history) = record.history.as_ref() else {
        return (shape != Shape::Implicit).then(|| {
            unverifiable(&format!(
                "States {stated} as an aggregate over {label}, but the record carries no sync \
                 coverage, so no window can be vouched for."
            ))
        });
    };
    if shape == Shape::UnstatedAverage {
        return Some(unverifiable(&format!(
            "States {stated} as an average over {label} without saying per session, per day or \
             per week, so the figure cannot be computed."
        )));
    }
    let Some(windows) = windows(period, record.today, history.block_start) else {
        return Some(unverifiable(&format!(
            "States {stated} for {label}, but the window cannot be placed on the calendar: for a \
             block, the athlete's active plan has no phase covering today."
        )));
    };

    let named = named_family(&tokens);
    let calls: Vec<Call> = figures
        .iter()
        .map(|figure| assess(*figure, shape, &windows, history, named.as_ref()))
        .collect();
    Some(verdict(&stated, &label, &calls, &windows, history))
}

/// The one period the text names, or `None` when it names none or several.
fn sole_period(tokens: &[&str]) -> Option<Period> {
    let found: Vec<(usize, usize, Period)> = PERIOD_PHRASES
        .iter()
        .flat_map(|&(phrase, form)| {
            phrase_matches(tokens, phrase)
                .into_iter()
                .filter_map(move |(a, b, count)| Some((a, b, form.resolve(count)?)))
        })
        .collect();
    // A phrase inside a longer one is part of it: "the last week" is not also
    // "last week".
    let mut outer = found.iter().filter(|(a, b, _)| {
        !found
            .iter()
            .any(|(c, d, _)| c <= a && b <= d && d - c > b - a)
    });
    let (_, _, first) = *outer.next()?;
    outer.all(|(_, _, p)| *p == first).then_some(first)
}

/// Every place `phrase` occurs in `tokens`, with the count its `#` read.
fn phrase_matches(tokens: &[&str], phrase: &str) -> Vec<(usize, usize, Option<u32>)> {
    let words: Vec<&str> = phrase.split(' ').collect();
    (0..tokens.len())
        .filter_map(|start| {
            let span = tokens.get(start..start + words.len())?;
            let mut count = None;
            for (token, word) in span.iter().zip(&words) {
                if *word == "#" {
                    count = Some(count_of(token)?);
                } else if token != word {
                    return None;
                }
            }
            Some((start, start + words.len(), count))
        })
        .collect()
}

/// The number a count token spells, in digits or as a word.
fn count_of(token: &str) -> Option<u32> {
    token.parse::<u32>().ok().or_else(|| {
        NUMBER_WORDS
            .iter()
            .find(|(word, _)| *word == token)
            .map(|&(_, n)| n)
    })
}

fn any_phrase(tokens: &[&str], phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| !phrase_matches(tokens, phrase).is_empty())
}

/// What the claim says its figures are.
fn shape_of(tokens: &[&str]) -> Shape {
    let averaged = AVERAGE_CUES.iter().any(|cue| {
        phrase_matches(tokens, cue)
            .iter()
            .any(|&(a, b, _)| !beside_other_metric(tokens, a, b))
    });
    let per = PER_FORMS
        .iter()
        .find(|&&(form, _, weak)| (averaged || !weak) && any_phrase(tokens, &[form]))
        .map(|&(_, per, _)| per);
    match (per, averaged) {
        (Some(per), _) => Shape::Average(per),
        (None, true) => Shape::UnstatedAverage,
        (None, false) if any_phrase(tokens, TOTAL_CUES) => Shape::Total,
        (None, false) => Shape::Implicit,
    }
}

/// Whether the words on either side of `[a, b)` name a metric this module does
/// not sum.
fn beside_other_metric(tokens: &[&str], a: usize, b: usize) -> bool {
    a.checked_sub(1)
        .and_then(|i| tokens.get(i))
        .into_iter()
        .chain(tokens.get(b))
        .any(|token| OTHER_METRICS.contains(token))
}

/// The readings of `period`, or `None` when it cannot be placed on the
/// calendar — a block with no plan phase covering today.
fn windows(
    period: Period,
    today: NaiveDate,
    block_start: Option<NaiveDate>,
) -> Option<Vec<Window>> {
    let back = |days: u64| today.checked_sub_days(Days::new(days));
    let window = |reading, start, end| Window {
        reading,
        start,
        end,
    };
    Some(match period {
        Period::ThisWeek => vec![
            window("the calendar week", monday_of(today)?, today),
            window("the last 7 days", back(6)?, today),
        ],
        Period::LastWeek => {
            let monday = monday_of(today)?;
            vec![
                window(
                    "the previous calendar week",
                    monday.checked_sub_days(Days::new(7))?,
                    monday.pred_opt()?,
                ),
                window("the 7 days before the last 7", back(13)?, back(7)?),
            ]
        }
        Period::LastDays(days) => vec![window(
            "the rolling window",
            back(u64::from(days.saturating_sub(1)))?,
            today,
        )],
        Period::ThisMonth => vec![window("the calendar month", today.with_day(1)?, today)],
        Period::LastMonth => {
            let first = today.with_day(1)?;
            vec![
                window(
                    "the previous calendar month",
                    first.checked_sub_months(Months::new(1))?,
                    first.pred_opt()?,
                ),
                window(
                    "the month ending today",
                    today.checked_sub_months(Months::new(1))?,
                    today,
                ),
            ]
        }
        Period::ThisBlock => vec![window(
            "the current block",
            block_start.filter(|start| *start <= today)?,
            today,
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::super::RecordedActivity;
    use super::*;
    use crate::athlete_data::check as layer_check;
    use crate::claim_extractor::{ClaimSource, ExtractedClaim};
    use chrono::{TimeZone, Utc};
    use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
    use pierre_core::models::SportType;
    use pierre_memory::{ClaimCategory, ClaimStatus};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn session(date: NaiveDate, sport: SportType, km: f64, minutes: f64) -> RecordedActivity {
        RecordedActivity {
            date,
            sport,
            name: String::new(),
            distance_km: Some(km),
            duration_min: minutes,
            elevation_m: None,
            average_hr_bpm: None,
        }
    }

    /// Wednesday 2026-09-09. The calendar week began Monday 09-07; the last 7
    /// days began Thursday 09-03.
    fn wednesday() -> NaiveDate {
        day(2026, 9, 9)
    }

    /// A record whose history is complete from August and synced today.
    fn record(today: NaiveDate, sessions: Vec<RecordedActivity>) -> AthleteRecord {
        AthleteRecord {
            has_provider: true,
            today,
            locale: "en".to_owned(),
            activities: sessions.clone(),
            history: Some(PeriodHistory {
                sessions,
                by_source: Vec::new(),
                covered_from: day(2026, 7, 1),
                synced_through: today,
                block_start: None,
            }),
        }
    }

    /// Calendar week: Mon 10 km + Tue 12 km = 22 km. Last 7 days add Sunday's
    /// 20 km and Thursday's 8 km: 50 km.
    fn this_week_runs() -> Vec<RecordedActivity> {
        vec![
            session(day(2026, 9, 3), SportType::Run, 8.0, 45.0),
            session(day(2026, 9, 6), SportType::Run, 20.0, 110.0),
            session(day(2026, 9, 7), SportType::Run, 10.0, 55.0),
            session(day(2026, 9, 8), SportType::Run, 12.0, 65.0),
        ]
    }

    fn status(text: &str, record: &AthleteRecord) -> ClaimStatus {
        verdict_for(text, record).status
    }

    fn verdict_for(text: &str, record: &AthleteRecord) -> VerdictOutcome {
        let claim = ExtractedClaim {
            text: text.to_owned(),
            category: ClaimCategory::AthleteData,
            source: ClaimSource::Reply,
        };
        layer_check(&claim, record).unwrap()
    }

    #[test]
    fn this_week_reads_as_the_calendar_week() {
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("You ran 22 km this week.", &record),
            ClaimStatus::Supported
        );
    }

    #[test]
    fn this_week_reads_as_the_last_seven_days() {
        let record = record(wednesday(), this_week_runs());
        let outcome = verdict_for("You ran 50 km this week.", &record);
        assert_eq!(outcome.status, ClaimStatus::Supported);
        assert!(
            outcome.explanation.contains("the last 7 days"),
            "the explanation names the reading that agreed: {}",
            outcome.explanation
        );
    }

    #[test]
    fn a_total_that_agrees_with_neither_reading_is_contradicted() {
        let record = record(wednesday(), this_week_runs());
        let outcome = verdict_for("You ran 35 km in total this week.", &record);
        assert_eq!(outcome.status, ClaimStatus::Contradicted);
        assert!(
            outcome.explanation.contains("22 km") && outcome.explanation.contains("50 km"),
            "the explanation names both totals on record: {}",
            outcome.explanation
        );
    }

    #[test]
    fn the_last_seven_days_has_only_the_rolling_reading() {
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("You covered 50 km over the last 7 days.", &record),
            ClaimStatus::Supported
        );
        // 22 km is this calendar week, not the last seven days. Unworded, it
        // may be some of the last seven days' runs; said to be the total, it
        // is wrong.
        assert_eq!(
            status("You covered 22 km over the last 7 days.", &record),
            ClaimStatus::Unverifiable
        );
        assert_eq!(
            status("You covered 22 km in total over the last 7 days.", &record),
            ClaimStatus::Contradicted
        );
        assert_eq!(
            status("You covered 50 km over the last seven days.", &record),
            ClaimStatus::Supported
        );
    }

    /// The calendar week is the athlete's, not UTC's: a run at 22:30 on Sunday
    /// in Toronto is 02:30 Monday in UTC, and belongs to the week before.
    #[test]
    fn the_calendar_week_is_the_athletes_own() {
        let toronto = resolve_zone(Some("America/Toronto"));
        let run_start = Utc.with_ymd_and_hms(2026, 9, 7, 2, 30, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 15, 0, 0).unwrap();
        let claim = "You ran 15 km last week.";

        let in_zone = |zone| {
            let today = clock_date(now, zone);
            let run = session(local_date(run_start, zone), SportType::Run, 15.0, 80.0);
            status(claim, &record(today, vec![run]))
        };

        // Toronto: Sunday 09-06, inside the previous calendar week.
        assert_eq!(in_zone(toronto), ClaimStatus::Supported);
        // UTC: Monday 09-07, inside this week — last week held nothing.
        assert_eq!(in_zone(resolve_zone(None)), ClaimStatus::Contradicted);
    }

    #[test]
    fn a_named_sport_is_summed_alone() {
        let mut sessions = this_week_runs();
        sessions.push(session(day(2026, 9, 8), SportType::GravelRide, 60.0, 150.0));
        let record = record(wednesday(), sessions);
        // Cycling this calendar week: the 60 km gravel ride.
        assert_eq!(
            status("Tu as roulé 60 km en vélo cette semaine.", &record),
            ClaimStatus::Supported
        );
        // 82 km is runs and ride together — not the cycling total.
        assert_eq!(
            status("Tu as roulé 82 km en vélo cette semaine.", &record),
            ClaimStatus::Contradicted
        );
        // Unnamed, 82 km is every sport this calendar week, and 22 km the runs.
        assert_eq!(
            status("You covered 82 km this week.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You covered 22 km this week.", &record),
            ClaimStatus::Supported
        );
    }

    #[test]
    fn a_window_past_the_last_sync_is_unverifiable() {
        let mut record = record(wednesday(), this_week_runs());
        if let Some(history) = record.history.as_mut() {
            history.synced_through = day(2026, 9, 8);
        }
        let outcome = verdict_for("You ran 35 km this week.", &record);
        assert_eq!(outcome.status, ClaimStatus::Unverifiable);
        assert!(outcome.explanation.contains("synced through 2026-09-08"));
    }

    #[test]
    fn a_window_before_the_history_begins_is_unverifiable() {
        let mut record = record(wednesday(), this_week_runs());
        if let Some(history) = record.history.as_mut() {
            history.covered_from = day(2026, 9, 5);
        }
        // The calendar week (from 09-07) is held, the last 7 days (from 09-03)
        // are not: 22 km agrees with the reading we hold...
        assert_eq!(
            status("You ran 22 km this week.", &record),
            ClaimStatus::Supported
        );
        // ...and a figure that misses it may be the reading we cannot see.
        assert_eq!(
            status("You ran 35 km this week.", &record),
            ClaimStatus::Unverifiable
        );
    }

    #[test]
    fn a_block_resolves_from_the_plan_phase() {
        let mut record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("You averaged 50 km per week this block.", &record),
            ClaimStatus::Unverifiable,
            "no plan phase covers today, so the block has no start"
        );
        if let Some(history) = record.history.as_mut() {
            history.block_start = Some(day(2026, 8, 31));
        }
        // Block 08-31..09-09 holds all 50 km over 10 days touching 2 weeks:
        // 35 km per 7 days, 25 km per calendar week.
        assert_eq!(
            status("You averaged 25 km per week this block.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You averaged 35 km per week this block.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You averaged 45 km per week this block.", &record),
            ClaimStatus::Contradicted
        );
    }

    #[test]
    fn a_per_session_average_divides_by_sessions() {
        let record = record(wednesday(), this_week_runs());
        // Last 7 days: 50 km over 4 runs = 12.5 km; calendar week 11 km.
        assert_eq!(
            status("You averaged 11 km per run this week.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status(
                "Tu as couru en moyenne 12,5 km par sortie cette semaine.",
                &record
            ),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You averaged 20 km per run this week.", &record),
            ClaimStatus::Contradicted
        );
    }

    #[test]
    fn an_average_that_never_says_per_what_is_unverifiable() {
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("This month you averaged 40 km.", &record),
            ClaimStatus::Unverifiable
        );
    }

    #[test]
    fn an_average_pace_is_not_an_average_of_the_distance() {
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status(
                "You ran 22 km this week at an average pace of 5:20 min/km.",
                &record
            ),
            ClaimStatus::Supported
        );
    }

    #[test]
    fn hours_rounded_to_the_hour_agree() {
        // Calendar week: 55 + 65 = 120 min; last 7 days 275 min.
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("You trained 2 hours this week.", &record),
            ClaimStatus::Supported
        );
        // 5 h is 300 min: 25 min off 275, inside the half-hour rounding.
        assert_eq!(
            status("You trained 5 hours this week.", &record),
            ClaimStatus::Supported
        );
        // 7 h is 420 min, far from either.
        assert_eq!(
            status("You trained 7 hours this week.", &record),
            ClaimStatus::Contradicted
        );
    }

    #[test]
    fn one_session_of_the_period_is_a_reading_of_an_unworded_figure() {
        let record = record(wednesday(), this_week_runs());
        assert_eq!(
            status("Great 12 km run this week, nicely paced.", &record),
            ClaimStatus::Supported
        );
        // Said to be the total, 12 km is wrong whatever single run it equals.
        assert_eq!(
            status("You ran 12 km in total this week.", &record),
            ClaimStatus::Contradicted
        );
    }

    #[test]
    fn a_figure_beside_a_correct_total_is_not_contradicted() {
        let record = record(wednesday(), this_week_runs());
        // 22 km agrees; 3 km matches nothing (a stretch the layer does not
        // model). No part-of-session word declines it, so this is the mix.
        let outcome = verdict_for(
            "You ran 22 km this week, 3 km of it with the club.",
            &record,
        );
        assert_eq!(outcome.status, ClaimStatus::Unverifiable);
        assert!(
            outcome.explanation.contains("part of it agrees"),
            "{}",
            outcome.explanation
        );
    }

    #[test]
    fn goals_and_comparisons_are_left_to_the_per_activity_check() {
        let record = record(wednesday(), this_week_runs());
        for text in [
            "You ran 22 km this week, 8 km short of your 30 km goal.",
            "You ran 5 km more this week.",
            "Tu as couru 22 km cette semaine, il te reste 8 km pour ton objectif.",
        ] {
            let figures = super::super::extract_figures(text);
            assert!(
                check(text, &figures, &record).is_none(),
                "a goal or a difference is not a period total: {text}"
            );
        }
    }

    #[test]
    fn two_periods_are_left_alone() {
        let record = record(wednesday(), this_week_runs());
        let text = "You ran 30 km last week and 22 km this week.";
        let figures = super::super::extract_figures(text);
        assert!(check(text, &figures, &record).is_none());
    }

    #[test]
    fn without_a_history_only_worded_aggregates_are_settled_here() {
        let mut record = record(wednesday(), this_week_runs());
        record.history = None;
        let unworded = "You ran 22 km this week.";
        let figures = super::super::extract_figures(unworded);
        assert!(
            check(unworded, &figures, &record).is_none(),
            "an unworded figure falls through to the per-activity check"
        );
        assert_eq!(
            status("You ran 22 km in total this week.", &record),
            ClaimStatus::Unverifiable
        );
    }

    #[test]
    fn last_month_reads_as_the_calendar_month_and_the_month_ending_today() {
        let sessions = vec![
            session(day(2026, 8, 5), SportType::Run, 10.0, 50.0),
            session(day(2026, 8, 20), SportType::Run, 15.0, 80.0),
            session(day(2026, 9, 2), SportType::Run, 7.0, 40.0),
        ];
        let record = record(wednesday(), sessions);
        // August: 25 km. 08-09..09-09: 22 km.
        assert_eq!(
            status("You ran 25 km last month.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You ran 22 km last month.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You ran 40 km last month.", &record),
            ClaimStatus::Contradicted
        );
        // This month: 7 km.
        assert_eq!(
            status("You ran 7 km this month.", &record),
            ClaimStatus::Supported
        );
    }

    /// The same calendar-week total in each shipped locale, right and wrong:
    /// an unworded figure above both readings, a stated total between them.
    #[test]
    fn every_shipped_locale_names_its_periods() {
        let record = record(wednesday(), this_week_runs());
        for (right, wrong) in [
            ("You ran 22 km this week.", "You ran 60 km this week."),
            (
                "Tu as couru 22 km cette semaine.",
                "Tu as couru 60 km cette semaine.",
            ),
            (
                "Esta semana corriste 22 km en total.",
                "Esta semana corriste 35 km en total.",
            ),
            (
                "Diese Woche bist du insgesamt 22 km gelaufen.",
                "Diese Woche bist du insgesamt 35 km gelaufen.",
            ),
            (
                "Nesta semana correste 22 km ao todo.",
                "Nesta semana correste 35 km ao todo.",
            ),
        ] {
            assert_eq!(status(right, &record), ClaimStatus::Supported, "{right}");
            assert_eq!(status(wrong, &record), ClaimStatus::Contradicted, "{wrong}");
        }
        for text in [
            "Over the last 7 days you ran 50 km.",
            "Ces 7 derniers jours, tu as couru 50 km.",
            "En los últimos 7 días corriste 50 km.",
            "In den letzten 7 Tagen bist du 50 km gelaufen.",
            "Nos últimos 7 dias correste 50 km.",
        ] {
            assert_eq!(status(text, &record), ClaimStatus::Supported, "{text}");
        }
    }

    /// A figure that measures part of a session, one intensity, a pace, a
    /// subset of the week or one day of it is not the period's total, and no
    /// total over the week settles it. Each of these is true of a week of a
    /// 10 km and a 12 km run, and none may be contradicted.
    #[test]
    fn a_part_of_a_session_is_never_read_as_the_period_total() {
        let record = record(
            wednesday(),
            vec![
                session(day(2026, 9, 7), SportType::Run, 10.0, 70.0),
                session(day(2026, 9, 8), SportType::Run, 12.0, 75.0),
            ],
        );
        for text in [
            "You ran 45 minutes at threshold this week.",
            "You logged 40 minutes in zone 2 this week.",
            "You ran 8 km at tempo pace this week.",
            "You ran 6 x 800 m repeats this week.",
            "You ran 6x800m this week, strong work.",
            "You did 3 km of strides and warm-up this week.",
            "If you ran 30 km this week, take Sunday off.",
            "Tu as couru 20 minutes au seuil cette semaine.",
            "Tu as couru 6×800 m cette semaine.",
            "Si tu as couru 30 km cette semaine, repose-toi dimanche.",
            "Esta semana corriste 30 minutos en zona 2.",
            "Si corriste 30 km esta semana, descansa el domingo.",
            "Diese Woche bist du 20 Minuten an der Schwelle gelaufen.",
            "Wenn du diese Woche 30 km gelaufen bist, ruh dich am Sonntag aus.",
            "Nesta semana correste 20 minutos no limiar.",
            "Se você correu 30 km nesta semana, descanse no domingo.",
            // A pace or a speed, and a figure at one
            "You ran 8 km at an easy pace this week.",
            "You ran at 5 min/km this week.",
            "You rode at 28 km/h this week.",
            "You ran 8 km at 5:20/km this week.",
            "Tu as couru à 5 min/km cette semaine.",
            "Tu as roulé à 28 km/h cette semaine.",
            "Tu as couru 8 km en allure facile cette semaine.",
            "Esta semana corriste a 5 min/km.",
            "Esta semana pedaleaste a 28 km/h.",
            "Esta semana corriste 8 km a ritmo suave.",
            "Diese Woche bist du mit 5 min/km gelaufen.",
            "Diese Woche bist du mit 28 km/h geradelt.",
            "Diese Woche bist du 8 km in lockerem Tempo gelaufen.",
            "Nesta semana correste a 5 min/km.",
            "Nesta semana pedalaste a 28 km/h.",
            "Nesta semana correste 8 km em ritmo leve.",
            // One intensity or a subset of the week's sessions
            "You logged 18 km of easy running this week.",
            "You ran 18 km at easy pace this week.",
            "Your long run this week was 18 km.",
            "You ran 18 km on trails this week.",
            "You ran 18 km on the treadmill this week.",
            "Tu as couru 18 km en endurance fondamentale cette semaine.",
            "Tu as fait 18 km en sortie longue cette semaine.",
            "Tu as couru 18 km sur tapis cette semaine.",
            "Esta semana corriste 18 km suaves.",
            "Esta semana hiciste una tirada larga de 18 km.",
            "Diese Woche bist du 18 km locker gelaufen.",
            "Diese Woche bist du 18 km auf dem Laufband gelaufen.",
            "Nesta semana correste 18 km leves.",
            "Nesta semana correste 18 km na esteira.",
            // One day of the week, or a sum of some days
            "You ran 18 km on Monday and Wednesday this week.",
            "You ran 18 km yesterday, your biggest day this week.",
            "Tu as couru 18 km lundi et mercredi cette semaine.",
            "Esta semana corriste 18 km entre el lunes y el miércoles.",
            "Diese Woche bist du am Montag und Mittwoch 18 km gelaufen.",
            "Nesta semana correste 18 km na segunda-feira e na quarta-feira.",
        ] {
            let figures = super::super::extract_figures(text);
            assert!(
                check(text, &figures, &record).is_none(),
                "a part of a session is left to the per-activity check: {text}"
            );
            assert_ne!(
                status(text, &record),
                ClaimStatus::Contradicted,
                "a true sentence about part of a session: {text}"
            );
        }
    }

    /// A tool that reads one provider adds up that provider's week. When the
    /// sources hold different sessions, that total is a reading too.
    #[test]
    fn one_sources_own_total_is_a_reading() {
        let strava = vec![
            session(day(2026, 9, 7), SportType::Run, 10.0, 55.0),
            session(day(2026, 9, 8), SportType::VirtualRide, 30.0, 60.0),
        ];
        let garmin = vec![
            session(day(2026, 9, 7), SportType::Run, 10.0, 55.0),
            session(day(2026, 9, 8), SportType::StrengthTraining, 0.0, 45.0),
        ];
        let merged = vec![
            session(day(2026, 9, 7), SportType::Run, 10.0, 55.0),
            session(day(2026, 9, 8), SportType::VirtualRide, 30.0, 60.0),
            session(day(2026, 9, 8), SportType::StrengthTraining, 0.0, 45.0),
        ];
        let mut record = record(wednesday(), merged);
        if let Some(history) = record.history.as_mut() {
            history.by_source = vec![
                SourceSessions {
                    source: "strava".to_owned(),
                    sessions: strava,
                },
                SourceSessions {
                    source: "garmin".to_owned(),
                    sessions: garmin,
                },
            ];
        }
        // Merged: 160 minutes. Strava alone: 115. Garmin alone: 100.
        let outcome = verdict_for("You trained 115 minutes in total this week.", &record);
        assert_eq!(outcome.status, ClaimStatus::Supported);
        assert!(
            outcome.explanation.contains("from strava"),
            "the explanation names the source: {}",
            outcome.explanation
        );
        assert_eq!(
            status("You trained 100 minutes in total this week.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You trained 160 minutes in total this week.", &record),
            ClaimStatus::Supported
        );
        assert_eq!(
            status("You trained 240 minutes in total this week.", &record),
            ClaimStatus::Contradicted
        );
    }

    /// Runs of 8, 10 and 8 km this week: 26 km on either reading.
    fn three_runs() -> AthleteRecord {
        record(
            wednesday(),
            vec![
                session(day(2026, 9, 7), SportType::Run, 8.0, 45.0),
                session(day(2026, 9, 8), SportType::Run, 10.0, 55.0),
                session(day(2026, 9, 9), SportType::Run, 8.0, 45.0),
            ],
        )
    }

    /// 18 km is two of the three runs. A sentence may pick them out in words
    /// no vocabulary lists — the club, the last two runs — and a subset never
    /// exceeds the whole, so an unworded figure below the total is left
    /// unsettled, in every shipped locale.
    #[test]
    fn an_unworded_figure_below_the_total_may_be_some_of_the_sessions() {
        let record = three_runs();
        for text in [
            "You ran 18 km with the club this week.",
            "You ran 18 km over your last two runs this week.",
            "Tu as couru 18 km lors de tes deux dernières sorties cette semaine.",
            "Tu as couru 18 km avec le club cette semaine.",
            "Esta semana corriste 18 km con el club.",
            "Esta semana corriste 18 km en tus dos últimas salidas.",
            "Diese Woche bist du 18 km mit dem Verein gelaufen.",
            "Diese Woche bist du bei deinen letzten zwei Läufen 18 km gelaufen.",
            "Nesta semana correste 18 km com o clube.",
            "Nesta semana correste 18 km nas tuas duas últimas saídas.",
        ] {
            let outcome = verdict_for(text, &record);
            assert_eq!(
                outcome.status,
                ClaimStatus::Unverifiable,
                "{text} -> {}",
                outcome.explanation
            );
            assert!(
                outcome
                    .explanation
                    .contains("only some of the period's sessions")
                    && outcome.explanation.contains("26 km"),
                "the explanation names the subset reading and the total: {}",
                outcome.explanation
            );
        }
    }

    /// Above the total, an unworded figure is no part of the week, whatever
    /// its sentence picks out.
    #[test]
    fn an_unworded_figure_above_the_total_is_contradicted() {
        let record = three_runs();
        for text in [
            "You ran 40 km this week.",
            "You ran 40 km with the club this week.",
            "Tu as couru 40 km cette semaine.",
            "Esta semana corriste 40 km.",
            "Diese Woche bist du 40 km gelaufen.",
            "Nesta semana correste 40 km.",
        ] {
            let outcome = verdict_for(text, &record);
            assert_eq!(
                outcome.status,
                ClaimStatus::Contradicted,
                "{text} -> {}",
                outcome.explanation
            );
            assert!(
                outcome.explanation.contains("26 km"),
                "{}",
                outcome.explanation
            );
        }
    }

    /// Said to be the total or an average, a figure below the record is as
    /// wrong as one above it, in every shipped locale. The total is 26 km and
    /// the average 8.7 km per run.
    #[test]
    fn a_stated_total_or_average_below_the_record_is_contradicted() {
        let record = three_runs();
        for text in [
            "You ran 18 km in total this week.",
            "Tu as couru 18 km en tout cette semaine.",
            "Esta semana corriste 18 km en total.",
            "Diese Woche bist du insgesamt 18 km gelaufen.",
            "Nesta semana correste 18 km ao todo.",
            "You averaged 5 km per run this week.",
            "Tu as couru en moyenne 5 km par sortie cette semaine.",
            "Esta semana corriste en promedio 5 km por salida.",
            "Diese Woche bist du im Schnitt 5 km pro Lauf gelaufen.",
            "Nesta semana correste em média 5 km por saída.",
        ] {
            let outcome = verdict_for(text, &record);
            assert_eq!(
                outcome.status,
                ClaimStatus::Contradicted,
                "{text} -> {}",
                outcome.explanation
            );
        }
    }
}
