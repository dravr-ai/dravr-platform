// ABOUTME: Overnight-recovery probes of the personalized layer — HRV, resting HR and sleep checked against the athlete's synced nights
// ABOUTME: Five-locale keyword probes with stated tolerances; the readings come from the stored health-sync rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Overnight recovery readings
//!
//! The rest of the personalized layer ([`crate::personalized`]) scores a claim
//! against values computed from the athlete's activity history. HRV, resting
//! heart rate and sleep duration are not computed: a watch or strap measures
//! them overnight, and health sync stores them as sleep sessions and recovery
//! readings. [`RecoveryReadings`] carries the athlete's recent values of each,
//! built by the caller from those rows, and the probes here rule on "your HRV
//! is 62 ms", "your HRV dropped 15% this week", "your resting HR is 48", "you
//! slept 8 hours" and "you've averaged 7 h of sleep" against them
//! (carnet#853).
//!
//! ## What a reading means
//!
//! Each metric holds the most recent night, the mean over the last seven days
//! and the mean of the nights before the most recent one ([`NightlyMetric`]).
//! A figure placed next to "last night" is scored against the night, one
//! placed next to an average or a week against the means, and one with
//! neither against all three: it is supported when any reading bears it out
//! and contradicted only when every one refutes it. A figure next to a week,
//! or next to nothing, may also be about one of the week's nights ("you
//! slept 5 h 40 on Wednesday"), so it is not contradicted while it sits
//! between the week's lowest and highest night. A percent change is the last
//! night against either mean.
//!
//! A sentence naming an earlier night — a weekday, "the night before last",
//! "three days ago" — may put any of its figures on that night, whichever
//! phrase sits nearest ("Tuesday's HRV dropped to 42 ms, but this morning it
//! was back to 60"). Its values are then not contradicted inside the week's
//! range whatever their scope, and its percent changes are not read: a
//! change on Tuesday is not last night's change, and the nights are not held
//! one by one.
//!
//! HRV and resting HR are held twice, because a night can carry two figures
//! for each: the sleep session's own and the next morning's recovery
//! reading. The sleep tools quote the first and the readiness tools the
//! second, so a reply faithful to either is scored against both, under the
//! same any-supports, all-refute rule.
//!
//! ## What is not a claim about the record
//!
//! The probes rule only on a sentence about the athlete — addressed to them,
//! or reporting a change that happened — and stay silent on advice ("aim for
//! 8 hours of sleep"), a hypothesis ("if you slept 5 hours"), a question, a
//! negation ("you didn't sleep 8 hours"), a difference ("5 bpm below your
//! average"), a bound ("more than 8 hours"), a sleep stage, a nap, and a
//! sentence naming a span longer than the week the readings cover ("over the
//! past month", "in 14 days"). Sleep is never matched against an activity
//! duration: these readings are nights, and the athlete-data layer still
//! declines sleep claims against activity records.

use chrono::{Duration, NaiveDate};

use crate::personalized::{verdict_from_call, PersonalizedContext, ToleranceCall};
use crate::verdict_engine::VerdictOutcome;

mod probes;
mod sentence;
mod vocab;

use probes::{reading_findings, sleep_findings, HRV_PROBE, RESTING_HR_PROBE};
use sentence::{Scope, Sentence};
use vocab::{ADVICE, NEGATIONS};

/// Days the weekly mean covers, ending with the athlete's current civil day.
pub const BASELINE_DAYS: i64 = 7;

/// Nights a mean needs before it is used.
///
/// A weekly HRV value is valid from three to four mornings a week (Plews et
/// al. 2014, `Methodology/Training Design/Individualization and
/// Readiness-Guided Adjustment`); with fewer, one odd night is most of the
/// mean.
const MIN_BASELINE_NIGHTS: usize = 3;

/// How old the most recent reading may be and still stand for "last night":
/// the night that ended today, or yesterday's when today's has not synced.
const LAST_NIGHT_MAX_AGE_DAYS: i64 = 1;

/// HRV band: ±5% of the reading.
///
/// cageux reads an athlete as fatigued only below −5% of their baseline
/// (`determine_hrv_recovery_status`), so a figure inside the band says nothing
/// about the athlete the stored reading does not, and rounding 62 ms to "about
/// 60" stays inside it.
const HRV_BAND_FRAC: f64 = 0.05;

/// Resting-HR band: ±3 bpm.
///
/// Where a provider gives no resting figure the sleep tools quote the night's
/// minimum heart rate as resting HR, and the two sit a few beats apart; the
/// max-HR probe uses the same width.
const RESTING_HR_BAND_BPM: f64 = 3.0;

/// Sleep band: ±30 min.
///
/// A reply rounds sleep to the hour ("about 7 hours" for 6 h 40); half an hour
/// either side keeps that rounding while still separating a 6-hour night
/// from an 8-hour one.
const SLEEP_BAND_HOURS: f64 = 0.5;

/// Percent-change band: ±5 percentage points of the mean, the rounding a
/// reply applies to a change ("about 15%" for 13%). Which mean the reply took
/// is settled by scoring against both.
const CHANGE_BAND: f64 = 0.05;

/// How many tokens from its keyword a figure may sit and still be read as
/// that metric's value, and a scope word as that figure's scope.
const WINDOW_TOKENS: usize = 8;

/// One metric's recent overnight readings.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NightlyMetric {
    /// The most recent reading, when it is from today or yesterday.
    pub last: Option<f64>,
    /// Mean over the last [`BASELINE_DAYS`] days, the most recent included,
    /// when at least three readings back it.
    pub week_mean: Option<f64>,
    /// Mean of the same days' readings before the most recent one, when at
    /// least three readings back it — the baseline a change is measured from.
    pub prior_mean: Option<f64>,
    /// The lowest and highest reading over the same days. A figure inside it
    /// may be about any one of those nights ("your HRV dipped to 42 ms on
    /// Tuesday"), which no mean stands for.
    pub range: Option<(f64, f64)>,
}

impl NightlyMetric {
    /// Build the metric from one reading per civil day, relative to `today`
    /// (the athlete's current civil day).
    ///
    /// Readings outside the last [`BASELINE_DAYS`] days, and any that are not
    /// a positive number, are ignored. `None` when neither a recent night nor
    /// a trusted mean remains.
    #[must_use]
    pub fn from_daily(readings: &[(NaiveDate, f64)], today: NaiveDate) -> Option<Self> {
        let first_day = today - Duration::days(BASELINE_DAYS - 1);
        let mut week: Vec<(NaiveDate, f64)> = readings
            .iter()
            .copied()
            .filter(|&(day, value)| {
                (first_day..=today).contains(&day) && value.is_finite() && value > 0.0
            })
            .collect();
        week.sort_by_key(|&(day, _)| day);
        let (&(last_day, last_value), earlier) = week.split_last()?;
        let all: Vec<f64> = week.iter().map(|&(_, value)| value).collect();
        let before: Vec<f64> = earlier.iter().map(|&(_, value)| value).collect();
        let range = all
            .iter()
            .fold((last_value, last_value), |(lo, hi), &value| {
                (lo.min(value), hi.max(value))
            });
        let metric = Self {
            last: ((today - last_day).num_days() <= LAST_NIGHT_MAX_AGE_DAYS).then_some(last_value),
            week_mean: trusted_mean(&all),
            prior_mean: trusted_mean(&before),
            range: Some(range),
        };
        (metric.last.is_some() || metric.week_mean.is_some()).then_some(metric)
    }

    /// The readings a figure in `scope` is scored against, each with the name
    /// the explanation gives it.
    fn references(&self, scope: Scope) -> Vec<(f64, &'static str)> {
        let last = self.last.map(|v| (v, "last night"));
        let week = self.week_mean.map(|v| (v, "7-day mean"));
        let prior = self.prior_mean.map(|v| (v, "mean of the nights before"));
        let picked = match scope {
            Scope::LastNight => vec![last],
            Scope::Average | Scope::Week => vec![week, prior],
            Scope::Either => vec![last, week, prior],
        };
        picked.into_iter().flatten().collect()
    }

    /// The week's lowest and highest night, when `scope` leaves a figure free
    /// to be about one of them rather than last night or a mean.
    fn nights_for(&self, scope: Scope) -> Option<(f64, f64)> {
        match scope {
            Scope::Week | Scope::Either => self.range,
            Scope::LastNight | Scope::Average => None,
        }
    }

    /// The last night as a ratio of each mean, each with the name the
    /// explanation gives it.
    fn change_ratios(&self) -> Vec<(f64, &'static str)> {
        let Some(last) = self.last else {
            return Vec::new();
        };
        [
            (self.week_mean, "the 7-day mean"),
            (self.prior_mean, "the nights before"),
        ]
        .into_iter()
        .filter_map(|(mean, name)| mean.filter(|m| *m > 0.0).map(|m| (last / m, name)))
        .collect()
    }
}

/// The mean of `values`, when enough of them back it.
fn trusted_mean(values: &[f64]) -> Option<f64> {
    (values.len() >= MIN_BASELINE_NIGHTS).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// The athlete's overnight readings: HRV, resting heart rate and sleep.
///
/// Built caller-side from the stored sleep sessions and recovery readings
/// health sync writes, read under each source's provider terms. A `None`
/// metric means claims about it fall through unverified.
///
/// A night can carry two HRV and resting-HR figures: the sleep session's own
/// (its average HRV, its lowest heart rate) and the next morning's recovery
/// reading. The readiness tools quote the morning's and the sleep tools the
/// night's, so a reply may state either; each metric is held both ways and a
/// figure either one bears out is supported.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecoveryReadings {
    /// Overnight HRV (rMSSD), in milliseconds: the morning reading's, or the
    /// night's own average when no morning reading carries one.
    pub hrv_ms: Option<NightlyMetric>,
    /// The same HRV as the sleep tools read it: the night's own average, or
    /// the morning reading's when the night carries none.
    pub hrv_ms_by_night: Option<NightlyMetric>,
    /// Resting heart rate, in beats per minute: the morning reading's, or the
    /// night's lowest heart rate when no morning reading carries one.
    pub resting_hr_bpm: Option<NightlyMetric>,
    /// The same resting heart rate as the sleep tools read it: the night's
    /// lowest heart rate, or the morning reading's when the night carries none.
    pub resting_hr_bpm_by_night: Option<NightlyMetric>,
    /// Time asleep per night, in hours.
    pub sleep_hours: Option<NightlyMetric>,
}

impl RecoveryReadings {
    /// True when at least one metric holds a reading.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        [
            self.hrv_ms,
            self.hrv_ms_by_night,
            self.resting_hr_bpm,
            self.resting_hr_bpm_by_night,
            self.sleep_hours,
        ]
        .iter()
        .any(Option::is_some)
    }
}

/// One way of reading a metric's nights, and the words the explanation adds
/// to its readings.
struct View {
    metric: NightlyMetric,
    label: &'static str,
}

/// A metric read morning-first and night-first; the night's reading is kept
/// only where it differs.
fn views(morning: Option<NightlyMetric>, night: Option<NightlyMetric>) -> Vec<View> {
    let night = night.filter(|n| Some(*n) != morning);
    [(morning, ""), (night, " (the night's own reading)")]
        .into_iter()
        .filter_map(|(metric, label)| metric.map(|metric| View { metric, label }))
        .collect()
}

/// What `read` yields from every view, each value once per reading name,
/// named for the explanation.
fn across_views(
    views: &[View],
    read: impl Fn(&NightlyMetric) -> Vec<(f64, &'static str)>,
) -> Vec<(f64, String)> {
    let mut seen: Vec<(f64, &'static str)> = Vec::new();
    let mut out = Vec::new();
    for view in views {
        for (value, name) in read(&view.metric) {
            if !seen.contains(&(value, name)) {
                seen.push((value, name));
                out.push((value, format!("{name}{}", view.label)));
            }
        }
    }
    out
}

/// The span of nights each view leaves open to a figure in `scope` — the
/// whole week whatever the scope once `earlier_night` says the sentence names
/// a night before the last ("Tuesday's HRV dropped to 42 ms, but this morning
/// it was back to 60"), where the phrase nearest a figure no longer says
/// which night it is about.
fn nights_across_views(views: &[View], scope: Scope, earlier_night: bool) -> Vec<(f64, f64)> {
    views
        .iter()
        .filter_map(|view| {
            if earlier_night {
                view.metric.range
            } else {
                view.metric.nights_for(scope)
            }
        })
        .collect()
}

/// Run the overnight-recovery probes over one claim.
///
/// `Some(verdict)` when the claim states a checkable HRV, resting-HR or sleep
/// figure about the athlete and every figure it states reaches a confident
/// call; `None` otherwise, and the claim continues to the next layer.
pub(crate) fn check(text: &str, ctx: &PersonalizedContext<'_>) -> Option<VerdictOutcome> {
    let readings = &ctx.metrics.recovery;
    if !readings.is_usable() || text.trim_end().ends_with('?') {
        return None;
    }
    let lower = text.to_lowercase();
    let sentence = Sentence::read(&lower);
    if !sentence.is_about_the_athlete()
        || sentence.has_phrase(ADVICE)
        || sentence.has_word(NEGATIONS)
        || sentence.names_a_longer_window()
    {
        return None;
    }
    let mut findings = reading_findings(
        &sentence,
        &views(readings.hrv_ms, readings.hrv_ms_by_night),
        &HRV_PROBE,
        ctx,
    );
    findings.extend(reading_findings(
        &sentence,
        &views(readings.resting_hr_bpm, readings.resting_hr_bpm_by_night),
        &RESTING_HR_PROBE,
        ctx,
    ));
    findings.extend(sleep_findings(
        &sentence,
        &views(readings.sleep_hours, None),
        ctx,
    ));
    settle(ctx, &findings)
}

/// One figure a probe scored: the tolerance call and the comparison behind it.
struct Finding {
    call: ToleranceCall,
    descr: String,
}

/// A sentence is contradicted when any figure in it is, and supported only
/// when every figure it states is.
fn settle(ctx: &PersonalizedContext<'_>, findings: &[Finding]) -> Option<VerdictOutcome> {
    if let Some(wrong) = findings
        .iter()
        .find(|f| f.call == ToleranceCall::Contradicted)
    {
        return verdict_from_call(ctx, ToleranceCall::Contradicted, &wrong.descr);
    }
    if findings.is_empty() || findings.iter().any(|f| f.call != ToleranceCall::Supported) {
        return None;
    }
    let descr = findings
        .iter()
        .map(|f| f.descr.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    verdict_from_call(ctx, ToleranceCall::Supported, &descr)
}

/// Supported when any reference supports the figure, contradicted when every
/// one refutes it, indeterminate otherwise.
fn assess_against(
    ctx: &PersonalizedContext<'_>,
    claimed: f64,
    ranges: impl Iterator<Item = (f64, f64)>,
) -> ToleranceCall {
    let calls: Vec<ToleranceCall> = ranges
        .map(|range| ctx.tolerance.assess(claimed, range))
        .collect();
    if calls.contains(&ToleranceCall::Supported) {
        ToleranceCall::Supported
    } else if !calls.is_empty() && calls.iter().all(|c| *c == ToleranceCall::Contradicted) {
        ToleranceCall::Contradicted
    } else {
        ToleranceCall::Indeterminate
    }
}

/// How wide the accepted band around a reading is.
#[derive(Clone, Copy)]
enum Band {
    /// A fraction of the reading either side.
    Relative(f64),
    /// A fixed amount either side, in the metric's unit.
    Absolute(f64),
}

impl Band {
    fn around(self, reading: f64) -> (f64, f64) {
        match self {
            Self::Relative(frac) => (reading * (1.0 - frac), reading * (1.0 + frac)),
            Self::Absolute(half) => (reading - half, reading + half),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::claim_extractor::{ClaimSource, ExtractedClaim};
    use crate::personalized::{check as personalized_check, AthleteMetrics, MIN_DATA_DAYS};
    use crate::{ConservativeStrategy, PersonalizedContext, TightStrategy, ToleranceStrategy};
    use pierre_memory::{ClaimCategory, ClaimStatus, VerdictLayer};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    /// Six nights of `earlier` from the 3rd, then `last` on the 9th (today).
    fn series(earlier: f64, last: f64) -> Option<NightlyMetric> {
        let mut daily: Vec<(NaiveDate, f64)> = (3..=8).map(|d| (day(d), earlier)).collect();
        daily.push((day(9), last));
        NightlyMetric::from_daily(&daily, day(9))
    }

    /// An athlete with a week of nights: HRV 60 ms then 51 last night, resting
    /// HR 48 then 52, sleep 7 h 30 then 6 h. No activity history, so only the
    /// overnight probes can fire.
    fn readings() -> RecoveryReadings {
        RecoveryReadings {
            hrv_ms: series(60.0, 51.0),
            resting_hr_bpm: series(48.0, 52.0),
            sleep_hours: series(7.5, 6.0),
            ..RecoveryReadings::default()
        }
    }

    fn check(text: &str) -> Option<ClaimStatus> {
        verdict(text).map(|v| v.status)
    }

    fn verdict(text: &str) -> Option<VerdictOutcome> {
        verdict_with(&readings(), &ConservativeStrategy::default(), text)
    }

    fn verdict_with(
        recovery: &RecoveryReadings,
        strategy: &dyn ToleranceStrategy,
        text: &str,
    ) -> Option<VerdictOutcome> {
        let metrics = AthleteMetrics {
            recovery: *recovery,
            ..AthleteMetrics::default()
        };
        let ctx = PersonalizedContext {
            metrics: &metrics,
            tolerance: strategy,
        };
        let claim = ExtractedClaim {
            text: text.to_owned(),
            category: ClaimCategory::Recovery,
            source: ClaimSource::Reply,
        };
        personalized_check(&claim, &ctx)
    }

    #[test]
    fn a_week_of_nights_yields_the_last_night_and_both_means() {
        let r = readings();
        let hrv = r.hrv_ms.unwrap();
        assert_eq!(hrv.last, Some(51.0));
        assert_eq!(hrv.prior_mean, Some(60.0));
        let week = hrv.week_mean.unwrap();
        assert!(
            (week - 411.0 / 7.0).abs() < 1e-9,
            "six nights of 60 and one of 51: {week}"
        );
        assert_eq!(
            hrv.range,
            Some((51.0, 60.0)),
            "the week's lowest and highest"
        );
    }

    /// A week of 7 h 30 nights but Wednesday the 7th at 5 h 36, and HRV
    /// 60 ms but Tuesday the 6th at 42 ms; last night (the 9th) is ordinary.
    fn a_week_with_one_odd_night() -> RecoveryReadings {
        let odd = |usual: f64, odd_day: u32, odd: f64| {
            let daily: Vec<(NaiveDate, f64)> = (3..=9)
                .map(|d| (day(d), if d == odd_day { odd } else { usual }))
                .collect();
            NightlyMetric::from_daily(&daily, day(9))
        };
        RecoveryReadings {
            sleep_hours: odd(7.5, 7, 5.6),
            hrv_ms: odd(60.0, 6, 42.0),
            ..RecoveryReadings::default()
        }
    }

    #[test]
    fn a_figure_about_one_of_the_weeks_nights_is_not_contradicted() {
        let conservative = ConservativeStrategy::default();
        let status = |text: &str| {
            verdict_with(&a_week_with_one_odd_night(), &conservative, text).map(|v| v.status)
        };
        // Faithful to Wednesday's and Tuesday's nights, which no mean and not
        // last night stands for: silent, never contradicted.
        for text in [
            "You slept 5 h 40 on Wednesday.",
            "On Wednesday you slept only 5 h 36.",
            "Your HRV dipped to 42 ms on Tuesday, then recovered.",
            "Your HRV dipped to 42 ms this week.",
        ] {
            assert_eq!(status(text), None, "{text:?}");
        }
        // Outside every night of the week, it is still contradicted.
        assert_eq!(
            status("You slept 3 hours on Wednesday."),
            Some(ClaimStatus::Contradicted)
        );
        assert_eq!(
            status("Your HRV dipped to 20 ms this week."),
            Some(ClaimStatus::Contradicted)
        );
        // Pinned to last night, or to the mean, the odd night rescues nothing.
        assert_eq!(
            status("Your HRV last night was 42 ms."),
            Some(ClaimStatus::Contradicted)
        );
        assert_eq!(
            status("Your HRV averaged 42 ms this week."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn a_change_about_an_earlier_night_is_not_read_as_last_nights() {
        let conservative = ConservativeStrategy::default();
        let status = |text: &str| {
            verdict_with(&a_week_with_one_odd_night(), &conservative, text).map(|v| v.status)
        };
        // Tuesday's 42 ms sits 30% under the week's 60; last night is back at
        // 60, a few percent above both means. A 30% drop read as last night's
        // is contradicted, in every shipped locale.
        for text in [
            "Your HRV dropped 30% this week.",
            "Ta VFC a chuté de 30 % cette semaine.",
            "Tu VFC bajó un 30 % esta semana.",
            "Deine HRV ist diese Woche um 30 % gefallen.",
            "Sua VFC caiu 30% esta semana.",
        ] {
            assert_eq!(status(text), Some(ClaimStatus::Contradicted), "{text:?}");
        }
        // Put on Tuesday, or on a night counted back from today, the same
        // drop is faithful: it is not read against last night.
        for text in [
            "Your HRV dropped 30% on Tuesday after the long run, then bounced back.",
            "Ta VFC a chuté de 30 % mardi après la sortie longue.",
            "Tu VFC bajó un 30 % el martes tras la tirada larga.",
            "Deine HRV ist am Dienstag um 30 % gefallen.",
            "Sua VFC caiu 30% na terça-feira depois do longão.",
            "Your HRV dropped 30% three days ago.",
            "Ta VFC a chuté de 30 % il y a trois jours.",
            "Tu VFC bajó un 30 % hace tres días.",
            "Deine HRV ist vor drei Tagen um 30 % gefallen.",
            "Sua VFC caiu 30% há três dias.",
            "Your HRV dropped 30% the night before last.",
            "Ta VFC a chuté de 30 % avant-hier.",
            "Tu VFC bajó un 30 % anteayer.",
            "Deine HRV ist vorgestern um 30 % gefallen.",
            "Sua VFC caiu 30% anteontem.",
        ] {
            assert_eq!(status(text), None, "{text:?}");
        }
        // "il y a" and "vor" count back only before a day, a night or a
        // week: "there is a drop", "before the long run" still read.
        for text in [
            "Il y a une baisse de 30 % de ta VFC cette semaine.",
            "Deine HRV ist vor dem langen Lauf um 30 % gefallen.",
        ] {
            assert_eq!(status(text), Some(ClaimStatus::Contradicted), "{text:?}");
        }
        // A change with no earlier night is still checked against the baseline.
        assert_eq!(
            check("Your HRV dropped 15% this week."),
            Some(ClaimStatus::Supported)
        );
    }

    #[test]
    fn a_figure_beside_an_earlier_night_may_be_about_it_whatever_sits_nearest() {
        let conservative = ConservativeStrategy::default();
        let status = |text: &str| {
            verdict_with(&a_week_with_one_odd_night(), &conservative, text).map(|v| v.status)
        };
        // "this morning" and "last night" sit nearest the first figure, but
        // the sentence puts it on Tuesday or Wednesday: faithful, so silent.
        for text in [
            "Tuesday's HRV dropped to 42 ms, but this morning it was back to 60 ms.",
            "Mardi, ta VFC est tombée à 42 ms ; ce matin elle était à 60 ms.",
            "On Wednesday you slept only 5 h 36, but last night 7 h 30.",
        ] {
            assert_eq!(status(text), None, "{text:?}");
        }
        // Outside every night of the week, the figure is still contradicted.
        for text in [
            "Tuesday's HRV dropped to 20 ms, but this morning it was back to 60 ms.",
            "Mardi, ta VFC est tombée à 20 ms ; ce matin elle était à 60 ms.",
            "On Wednesday you slept only 3 h, but last night 7 h 30.",
        ] {
            assert_eq!(status(text), Some(ClaimStatus::Contradicted), "{text:?}");
        }
        // With no earlier night named, last night is what the figure is about.
        assert_eq!(
            status("You slept only 5 h 36 last night."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn negations_and_spans_longer_than_the_week_are_not_read() {
        for text in [
            // A negated figure: true here, since last night was 6 h.
            "Tu n'as pas dormi 8 heures cette nuit.",
            "You haven't slept 8 hours a night this week.",
            "Du hast letzte Nacht nicht 8 Stunden geschlafen.",
            "Você não dormiu 8 horas esta noite.",
            "No dormiste nunca 8 horas esta semana.",
            // A span the seven nights do not cover.
            "You've averaged 5 hours of sleep over the past month.",
            "Your HRV averaged 40 ms over the last 14 days.",
            "Tu VFC promedió 40 ms este mes.",
            "Deine HRV lag in den letzten Wochen bei 40 ms.",
            "Ta VFC a baissé de 40 % en trois semaines.",
        ] {
            assert_eq!(check(text), None, "{text:?}");
        }
        // Without the negation, the same figure is read and contradicted.
        assert_eq!(
            check("Tu as dormi 8 heures cette nuit."),
            Some(ClaimStatus::Contradicted)
        );
        assert_eq!(
            check("Your HRV averaged 40 ms over the last 7 days."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn a_generality_in_french_is_not_read_as_addressed_to_the_athlete() {
        // "du" is French "of the", not the German "you".
        for text in [
            "Une VFC de 30 ms est typique du coureur fatigué.",
            "La VFC moyenne du sportif amateur est de 40 ms.",
        ] {
            assert_eq!(check(text), None, "{text:?}");
        }
        // German still reads the second person from its verb forms.
        assert_eq!(
            check("Du hast letzte Nacht 8 Stunden geschlafen."),
            Some(ClaimStatus::Contradicted)
        );
        assert_eq!(
            check("Du hast letzte Nacht 6 Stunden geschlafen."),
            Some(ClaimStatus::Supported)
        );
    }

    #[test]
    fn a_french_change_reads_its_direction_past_a_but() {
        assert_eq!(
            check("Ta VFC a baissé de 15 % cette semaine, mais rien d'inquiétant."),
            Some(ClaimStatus::Supported)
        );
        assert_eq!(
            check("Ta VFC a augmenté de 15 % cette semaine, mais rien d'inquiétant."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn a_stale_last_night_and_a_thin_week_are_not_trusted() {
        let today = day(9);
        // Three nights, the latest two days ago: a mean, but no "last night".
        let stale =
            NightlyMetric::from_daily(&[(day(5), 50.0), (day(6), 52.0), (day(7), 54.0)], today)
                .unwrap();
        assert_eq!(stale.last, None);
        assert_eq!(stale.week_mean, Some(52.0));
        // The 3 → 2 boundary: two nights make no mean, but last night stands.
        let thin = NightlyMetric::from_daily(&[(day(8), 50.0), (day(9), 52.0)], today).unwrap();
        assert_eq!(thin.last, Some(52.0));
        assert_eq!(thin.week_mean, None);
        assert_eq!(thin.prior_mean, None);
        // A night outside the seven days counts for nothing.
        assert_eq!(NightlyMetric::from_daily(&[(day(2), 50.0)], today), None);
        // Nor does a zero reading.
        assert_eq!(NightlyMetric::from_daily(&[(day(9), 0.0)], today), None);
    }

    #[test]
    fn an_hrv_figure_is_checked_in_every_shipped_locale() {
        for (text, expected) in [
            ("Your HRV last night was 51 ms.", ClaimStatus::Supported),
            ("Your HRV last night was 75 ms.", ClaimStatus::Contradicted),
            ("Ta VFC cette nuit était de 51 ms.", ClaimStatus::Supported),
            (
                "Ta VFC cette nuit était de 75 ms.",
                ClaimStatus::Contradicted,
            ),
            ("Tu VFC de anoche fue de 75 ms.", ClaimStatus::Contradicted),
            (
                "Deine HRV lag letzte Nacht bei 75 ms.",
                ClaimStatus::Contradicted,
            ),
            (
                "A tua VFC esta noite foi de 75 ms.",
                ClaimStatus::Contradicted,
            ),
        ] {
            assert_eq!(check(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn an_unscoped_hrv_figure_may_match_the_night_or_the_mean() {
        // 59 is the 7-day mean (58.7), not last night (51): either reading bears it out.
        assert_eq!(check("Your HRV is 59 ms."), Some(ClaimStatus::Supported));
        // 30 is far from every reading.
        assert_eq!(check("Your HRV is 30 ms."), Some(ClaimStatus::Contradicted));
        // Scoped to the night, the mean no longer rescues it.
        assert_eq!(
            check("Your HRV last night was 60 ms."),
            Some(ClaimStatus::Contradicted)
        );
        // Scoped to the average, last night no longer rescues 51.
        assert_eq!(
            check("Your 7-day HRV average is 45 ms."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn an_hrv_change_is_checked_against_the_baseline_in_every_shipped_locale() {
        // Last night 51 against 60 before it is −15%; against the 7-day mean −13%.
        for text in [
            "Your HRV dropped 15% this week.",
            "HRV dropped 15% this week.",
            "Ta VFC a baissé de 15 % cette semaine.",
            "Tu VFC bajó un 15 % esta semana.",
            "Deine HRV ist diese Woche um 15 % gesunken.",
            "A tua VFC caiu 15% esta semana.",
        ] {
            assert_eq!(check(text), Some(ClaimStatus::Supported), "{text:?}");
        }
        for text in [
            "Your HRV rose 15% this week.",
            "Ta VFC a augmenté de 15 % cette semaine.",
            "Tu VFC subió un 15 % esta semana.",
            "Deine HRV ist diese Woche um 15 % gestiegen.",
            "A tua VFC subiu 15% esta semana.",
            "Your HRV dropped 40% this week.",
        ] {
            assert_eq!(check(text), Some(ClaimStatus::Contradicted), "{text:?}");
        }
    }

    #[test]
    fn a_change_with_no_direction_is_not_read() {
        assert_eq!(check("Your HRV moved 15% this week."), None);
    }

    #[test]
    fn a_resting_hr_figure_is_checked_in_every_shipped_locale() {
        for (text, expected) in [
            (
                "Your resting heart rate this morning is 52 bpm.",
                ClaimStatus::Supported,
            ),
            ("Your resting HR is 49.", ClaimStatus::Supported),
            (
                "Your resting HR is 52 beats per minute.",
                ClaimStatus::Supported,
            ),
            (
                "Your resting HR this morning is 44 bpm.",
                ClaimStatus::Contradicted,
            ),
            (
                "Ta FC au repos ce matin est de 52 bpm.",
                ClaimStatus::Supported,
            ),
            ("Ta FC au repos est de 40.", ClaimStatus::Contradicted),
            (
                "Tu frecuencia cardíaca en reposo es de 40 lpm.",
                ClaimStatus::Contradicted,
            ),
            ("Dein Ruhepuls liegt bei 40.", ClaimStatus::Contradicted),
            (
                "A tua frequência cardíaca em repouso é de 40 bpm.",
                ClaimStatus::Contradicted,
            ),
        ] {
            assert_eq!(check(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn a_sleep_duration_is_checked_in_every_shipped_locale() {
        for (text, expected) in [
            ("You slept 6 hours last night.", ClaimStatus::Supported),
            ("You slept 8 hours last night.", ClaimStatus::Contradicted),
            (
                "You've averaged 7.5 hours of sleep this week.",
                ClaimStatus::Supported,
            ),
            (
                "You've averaged 5 hours of sleep this week.",
                ClaimStatus::Contradicted,
            ),
            ("Tu as dormi 6 h cette nuit.", ClaimStatus::Supported),
            ("Tu as dormi 8 h 30 cette nuit.", ClaimStatus::Contradicted),
            (
                "Tu dors en moyenne 7 h 30 par nuit.",
                ClaimStatus::Supported,
            ),
            (
                "Anoche dormiste 8 horas y media.",
                ClaimStatus::Contradicted,
            ),
            (
                "Du hast letzte Nacht 8,5 Stunden geschlafen.",
                ClaimStatus::Contradicted,
            ),
            ("Você dormiu 6 horas esta noite.", ClaimStatus::Supported),
            (
                "Dormiste 9 horas na última noite.",
                ClaimStatus::Contradicted,
            ),
        ] {
            assert_eq!(check(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn last_nights_sleep_is_not_rescued_by_the_weekly_mean() {
        // 7 h 30 is the week before, not last night's 6 h.
        assert_eq!(
            check("You slept 7.5 hours last night."),
            Some(ClaimStatus::Contradicted)
        );
        // With nothing scoping it, the week bears it out.
        assert_eq!(check("You slept 7.5 hours."), Some(ClaimStatus::Supported));
    }

    #[test]
    fn advice_questions_differences_and_bounds_are_not_read_as_the_record() {
        for text in [
            "Aim for 8 hours of sleep tonight.",
            "You should sleep 9 hours tonight.",
            "If you slept 4 hours, take it easy.",
            "Did you sleep 9 hours last night?",
            "Your resting HR is 10 bpm above your average.",
            "Your resting HR is 3 beats higher than usual.",
            "Your HRV is 5 points higher than last week.",
            "You slept more than 9 hours last night.",
            "You slept over 9 hours last night.",
            "Tu as dormi plus de 9 heures cette nuit.",
            "Your HRV dropped 20 ms this week.",
            "You got 2 hours of deep sleep, your sleep was short.",
            "An HRV of 30 ms signals fatigue in most athletes.",
            "Your HRV has been low for 3 days.",
            "Your HRV at 6:30 this morning was low.",
        ] {
            assert_eq!(check(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_value_landed_on_by_a_change_is_read() {
        // "dropped to 51" states the new value; 51 is last night.
        assert_eq!(
            check("Your HRV dropped to 51 ms last night."),
            Some(ClaimStatus::Supported)
        );
        assert_eq!(
            check("Your HRV dropped to 30 ms last night."),
            Some(ClaimStatus::Contradicted)
        );
        // "52 bpm, up from 48" states 52 as the value and 48 as the past.
        assert_eq!(
            check("Your resting HR is 52 bpm, up from 48."),
            Some(ClaimStatus::Supported)
        );
    }

    #[test]
    fn a_clock_time_is_not_a_reading() {
        // The 6:30 sits nearer the keyword than the 75 ms, and is not a figure.
        assert_eq!(
            check("Your HRV at 6:30 this morning was 75 ms."),
            Some(ClaimStatus::Contradicted)
        );
        assert_eq!(
            check("Your HRV at 6:30 this morning was 51 ms."),
            Some(ClaimStatus::Supported)
        );
    }

    /// The same week, where last night's sleep session and this morning's
    /// reading disagree: resting HR 52 by the morning, 44 by the night's
    /// lowest heart rate; HRV 51 ms by the morning, 58 by the night's average.
    fn readings_read_two_ways() -> RecoveryReadings {
        RecoveryReadings {
            hrv_ms_by_night: series(60.0, 58.0),
            resting_hr_bpm_by_night: series(44.0, 44.0),
            ..readings()
        }
    }

    #[test]
    fn a_figure_the_sleep_tools_quote_is_not_contradicted_by_the_morning_reading() {
        // Zero margin: a figure outside both bands is contradicted outright.
        let tight = TightStrategy;
        let status = |recovery: RecoveryReadings, text: &str| {
            verdict_with(&recovery, &tight, text).map(|v| v.status)
        };
        for text in [
            // The sleep tools' night: lowest heart rate 44, average HRV 58.
            "Your resting HR last night was 44 bpm.",
            "Your HRV last night was 58 ms.",
            // The readiness tools' morning: resting HR 52, HRV 51.
            "Your resting HR last night was 52 bpm.",
            "Your HRV last night was 51 ms.",
            "Ta FC au repos cette nuit était de 44 bpm.",
        ] {
            assert_eq!(
                status(readings_read_two_ways(), text),
                Some(ClaimStatus::Supported),
                "{text:?}"
            );
        }
        // Refuted by both readings of the night, it is still contradicted.
        assert_eq!(
            status(
                readings_read_two_ways(),
                "Your resting HR last night was 36 bpm."
            ),
            Some(ClaimStatus::Contradicted)
        );
        // Read the morning's way only, the night's 44 would have been
        // contradicted: the reply quoted a tool faithfully.
        let morning_only = RecoveryReadings {
            resting_hr_bpm_by_night: None,
            ..readings_read_two_ways()
        };
        assert_eq!(
            status(morning_only, "Your resting HR last night was 44 bpm."),
            Some(ClaimStatus::Contradicted)
        );
    }

    #[test]
    fn the_verdict_names_both_readings_of_a_night_only_when_they_differ() {
        let tight = TightStrategy;
        let v = verdict_with(
            &readings_read_two_ways(),
            &tight,
            "Your resting HR last night was 36 bpm.",
        )
        .unwrap();
        assert!(
            v.explanation.contains(
                "resting HR 36 bpm vs your 52 bpm last night, \
                 44 bpm last night (the night's own reading)"
            ),
            "{}",
            v.explanation
        );
        // The night and the morning agree: the reading is named once.
        let agreeing = RecoveryReadings {
            resting_hr_bpm_by_night: readings().resting_hr_bpm,
            ..readings()
        };
        let v = verdict_with(&agreeing, &tight, "Your resting HR last night was 36 bpm.").unwrap();
        assert!(!v.explanation.contains("night's own"), "{}", v.explanation);
    }

    #[test]
    fn an_athlete_with_trusted_physiology_still_has_their_nights_checked() {
        // Max HR and FTP backed by enough history: the activity probes run
        // first, and the overnight probes after them.
        let metrics = AthleteMetrics {
            max_hr: Some(190.0),
            ftp_watts: Some(250.0),
            data_days: MIN_DATA_DAYS,
            recovery: readings(),
            ..AthleteMetrics::default()
        };
        let strategy = ConservativeStrategy::default();
        let ctx = PersonalizedContext {
            metrics: &metrics,
            tolerance: &strategy,
        };
        let verdict = |text: &str| {
            let claim = ExtractedClaim {
                text: text.to_owned(),
                category: ClaimCategory::Recovery,
                source: ClaimSource::Reply,
            };
            personalized_check(&claim, &ctx).expect(text)
        };
        // The max-HR probe does not take a resting heart rate for its own.
        let v = verdict("Your resting heart rate this morning is 52 bpm.");
        assert_eq!(v.status, ClaimStatus::Supported);
        assert!(
            v.explanation.contains("resting HR 52 bpm"),
            "{}",
            v.explanation
        );
        let v = verdict("You slept 8 hours last night.");
        assert_eq!(v.status, ClaimStatus::Contradicted);
        assert!(v.explanation.contains("sleep 8 h"), "{}", v.explanation);
        // And a max-HR figure is still the max-HR probe's.
        let v = verdict("Your max heart rate is 150 bpm.");
        assert_eq!(v.status, ClaimStatus::Contradicted);
        assert!(v.explanation.contains("max HR"), "{}", v.explanation);
    }

    #[test]
    fn a_figure_belongs_to_the_metric_it_sits_by() {
        // The 15% is the HRV's drop, not the resting HR's (which rose 7%).
        assert_eq!(
            check("Your HRV dropped 15%, while your resting HR held at 52 bpm."),
            Some(ClaimStatus::Supported)
        );
    }

    #[test]
    fn every_figure_in_a_sentence_must_hold() {
        // The HRV holds, the sleep does not.
        assert_eq!(
            check("Your HRV is 51 ms and you slept 9 hours last night."),
            Some(ClaimStatus::Contradicted)
        );
        // Both hold.
        assert_eq!(
            check("Your HRV is 51 ms and you slept 6 hours last night."),
            Some(ClaimStatus::Supported)
        );
    }

    #[test]
    fn the_verdict_names_the_readings_it_was_checked_against() {
        let v = verdict("You slept 8 hours last night.").unwrap();
        assert_eq!(v.layer_fired, VerdictLayer::Personalized);
        assert!(
            v.explanation.contains("sleep 8 h vs your 6 h last night"),
            "{}",
            v.explanation
        );
        let v = verdict("Your HRV dropped 15% this week.").unwrap();
        assert!(
            v.explanation
                .contains("HRV change -15% vs your last night's -13% against the 7-day mean, -15% against the nights before"),
            "{}",
            v.explanation
        );
    }

    #[test]
    fn without_readings_the_probes_stay_silent() {
        let metrics = AthleteMetrics::default();
        let strategy = ConservativeStrategy::default();
        let ctx = PersonalizedContext {
            metrics: &metrics,
            tolerance: &strategy,
        };
        let claim = ExtractedClaim {
            text: "You slept 8 hours last night.".to_owned(),
            category: ClaimCategory::Recovery,
            source: ClaimSource::Reply,
        };
        assert!(personalized_check(&claim, &ctx).is_none());
    }
}
