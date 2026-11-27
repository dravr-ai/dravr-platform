// ABOUTME: Whether a coached athlete missed planned sessions on consecutive planned days: each session judged, then the run
// ABOUTME: Pure: provider pairings, Dravr plan days and recorded activities in; a run of missed planned days out (carnet#1792)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Missed planned sessions
//!
//! Coach Home's `planned_sessions_missed` reason compares what the athlete's
//! plan asked for with what they recorded, against the athlete's own plan
//! and never a population threshold. This module holds the comparison. The
//! calendar and plan reads that feed it live with the roster.
//!
//! - **Who says a session was done.** In order: the provider's own pairing
//!   (intervals.icu pairs a planned event with the activity that did it,
//!   and TrainingPeaks does the same), the provider's pairing of an event
//!   Dravr pushed there, and only for a Dravr plan day no calendar can pair
//!   an activity of the same sport within a day either side
//!   ([`Pairing`]).
//! - **When a session is judged.** A session already done is done at once.
//!   One not done is missed only once the day after it has ended, so a
//!   session moved to the next day, or a late upload, is not a miss.
//! - **What a missed day is.** A planned day is missed when none of the
//!   sessions planned on it was done (Phil, 2026-11-26): one done session
//!   of a double day keeps the day.
//! - **The run.** The planned days are walked back from the latest judged
//!   one. Rest days between them don't break the run, and a done day ends
//!   it. The reason is raised once the run reaches the threshold
//!   ([`missed_run`]); one isolated missed day never raises it.
//! - **A read that failed is never a miss.** The caller reports a calendar
//!   it could not read as [`PlannedVsDone::Unreadable`] and judges nothing
//!   for that athlete: an absent completion is never the answer to a read
//!   that did not happen.
//!
//! Counts and context only: never a percentage, never a guess at why.

use std::collections::{BTreeMap, BTreeSet};

use dravr_cageux::models::sport_alias::sport_family_head;
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::models::SportType;

/// Who decides whether a planned session was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pairing {
    /// The calendar `provider` pairs its planned sessions with activities
    /// itself; `completed` is whether it paired this one.
    Provider {
        /// The provider, as the athlete names it (`intervals_icu`).
        provider: String,
        /// The provider paired an activity with the session.
        completed: bool,
    },
    /// No calendar pairs it: an activity of the same sport on the day or a
    /// day either side does.
    Activity,
}

/// One planned session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSession {
    /// The athlete's calendar day it was planned for.
    pub date: CivilDate,
    /// Its sport.
    pub sport: SportType,
    /// Who decides whether it was done.
    pub pairing: Pairing,
}

/// One recorded activity, on the athlete's own calendar day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneActivity {
    /// The athlete's calendar day it started on.
    pub date: CivilDate,
    /// Its sport.
    pub sport: SportType,
}

/// What a planned session came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It was done.
    Done,
    /// It was not done, and the day after it has ended.
    Missed,
    /// Not done yet, and still inside its day or the next.
    Pending,
}

/// A planned session and what it came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedSession {
    /// The day it was planned for.
    pub date: CivilDate,
    /// Its sport.
    pub sport: SportType,
    /// The calendar that paired it, when a provider did; `None` for a Dravr
    /// plan day judged on recorded activities.
    pub provider: Option<String>,
    /// What it came to.
    pub verdict: Verdict,
}

/// A run of consecutive missed planned days long enough to raise the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedRun {
    /// The missed planned days, oldest first.
    pub days: Vec<CivilDate>,
    /// The sessions planned on them, every one missed, oldest first.
    pub sessions: Vec<JudgedSession>,
    /// The first judged planned day of the window read.
    pub since: CivilDate,
    /// Sessions judged since then.
    pub planned: u32,
    /// Of those, the missed ones.
    pub missed: u32,
}

/// The athlete's last wellness note, as their coach may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WellnessNote {
    /// The day the note is about.
    pub date: CivilDate,
    /// What the athlete wrote.
    pub text: String,
}

/// The missed-session check of one athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedVsDone {
    /// The athlete switched the check off for this coach.
    Off,
    /// A calendar could not be read, so nothing was judged.
    Unreadable,
    /// Read and judged.
    Read {
        /// When the calendars were read.
        read_at: Timestamp,
        /// The run of missed planned days, when it reached the threshold.
        run: Option<MissedRun>,
        /// The athlete's last wellness note in the window, read only with a
        /// run and only under their wellness grant to the coach.
        wellness: Option<WellnessNote>,
    },
}

/// A sport's wire name, the one the clients label (`run`, `trail_running`);
/// a provider's own type keeps its own name.
#[must_use]
pub(crate) fn sport_name(sport: &SportType) -> String {
    match sport {
        SportType::Other(name) => name.clone(),
        known => serde_json::to_value(known)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| known.display_name().to_owned()),
    }
}

/// Whether `activity` is of `planned`'s sport: the same, or the same family
/// (a trail run for a planned run, a virtual ride for a planned ride).
fn same_sport(activity: &SportType, planned: &SportType) -> bool {
    let head = |sport: &SportType| sport_family_head(sport).unwrap_or_else(|| sport.clone());
    activity == planned || head(activity) == head(planned)
}

/// The day after `date` has ended by `today`: the session's grace day is over.
fn grace_over(date: CivilDate, today: CivilDate) -> bool {
    today.days_since(date) >= 2
}

/// Each session judged, in `sessions`' order.
///
/// A provider's pairing is taken as it is. A Dravr plan day is matched to a
/// recorded activity of its sport, each activity doing for one session: the
/// same day first, then a day either side, the earliest session first.
#[must_use]
pub fn judge(
    sessions: &[PlannedSession],
    activities: &[DoneActivity],
    today: CivilDate,
) -> Vec<JudgedSession> {
    let mut used = vec![false; activities.len()];
    let mut matched: BTreeSet<usize> = BTreeSet::new();
    let mut order: Vec<usize> = (0..sessions.len())
        .filter(|i| sessions[*i].pairing == Pairing::Activity)
        .collect();
    order.sort_by_key(|i| sessions[*i].date);
    for reach in [0_i64, 1] {
        for &i in &order {
            if matched.contains(&i) {
                continue;
            }
            let session = &sessions[i];
            let found = (0..activities.len()).find(|&j| {
                !used[j]
                    && session.date.days_since(activities[j].date).abs() == reach
                    && same_sport(&activities[j].sport, &session.sport)
            });
            if let Some(j) = found {
                used[j] = true;
                matched.insert(i);
            }
        }
    }
    sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let (provider, done) = match &session.pairing {
                Pairing::Provider {
                    provider,
                    completed,
                } => (Some(provider.clone()), *completed),
                Pairing::Activity => (None, matched.contains(&i)),
            };
            let verdict = if done {
                Verdict::Done
            } else if grace_over(session.date, today) {
                Verdict::Missed
            } else {
                Verdict::Pending
            };
            JudgedSession {
                date: session.date,
                sport: session.sport.clone(),
                provider,
                verdict,
            }
        })
        .collect()
}

/// The run of consecutive missed planned days ending at the latest judged
/// one, when it holds at least `threshold` days; `None` otherwise.
///
/// A planned day with a pending session and nothing done is not judged yet
/// and is skipped; a day with any session done ends the run.
#[must_use]
pub fn missed_run(judged: &[JudgedSession], threshold: u32) -> Option<MissedRun> {
    let mut days: BTreeMap<CivilDate, Vec<&JudgedSession>> = BTreeMap::new();
    for session in judged {
        days.entry(session.date).or_default().push(session);
    }
    let decided: Vec<(CivilDate, bool)> = days
        .iter()
        .filter_map(|(date, sessions)| {
            let done = sessions.iter().any(|s| s.verdict == Verdict::Done);
            let pending = sessions.iter().any(|s| s.verdict == Verdict::Pending);
            (done || !pending).then_some((*date, !done))
        })
        .collect();
    let run: Vec<CivilDate> = decided
        .iter()
        .rev()
        .take_while(|(_, missed)| *missed)
        .map(|(date, _)| *date)
        .collect();
    let threshold = usize::try_from(threshold.max(1)).unwrap_or(usize::MAX);
    if run.len() < threshold {
        return None;
    }
    let since = decided.first().map(|(date, _)| *date)?;
    let counted: Vec<&JudgedSession> = judged
        .iter()
        .filter(|s| s.verdict != Verdict::Pending)
        .collect();
    let mut days_in_run = run;
    days_in_run.reverse();
    let mut sessions: Vec<JudgedSession> = judged
        .iter()
        .filter(|s| days_in_run.contains(&s.date))
        .cloned()
        .collect();
    sessions.sort_by_key(|s| s.date);
    Some(MissedRun {
        days: days_in_run,
        sessions,
        since,
        planned: count(counted.len()),
        missed: count(
            counted
                .iter()
                .filter(|s| s.verdict == Verdict::Missed)
                .count(),
        ),
    })
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> CivilDate {
        CivilDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Thursday 2026-11-26, the brief's morning.
    fn today() -> CivilDate {
        date("2026-11-26")
    }

    fn paired(day: &str, completed: bool) -> PlannedSession {
        PlannedSession {
            date: date(day),
            sport: SportType::Run,
            pairing: Pairing::Provider {
                provider: "intervals_icu".to_owned(),
                completed,
            },
        }
    }

    fn plan_day(day: &str, sport: SportType) -> PlannedSession {
        PlannedSession {
            date: date(day),
            sport,
            pairing: Pairing::Activity,
        }
    }

    fn did(day: &str, sport: SportType) -> DoneActivity {
        DoneActivity {
            date: date(day),
            sport,
        }
    }

    fn run_of(sessions: &[PlannedSession], activities: &[DoneActivity]) -> Option<MissedRun> {
        missed_run(&judge(sessions, activities, today()), 2)
    }

    #[test]
    fn one_isolated_missed_day_never_raises_it() {
        let sessions = [
            paired("2026-11-18", true),
            paired("2026-11-20", false),
            paired("2026-11-22", true),
            paired("2026-11-24", true),
        ];
        assert_eq!(
            run_of(&sessions, &[]),
            None,
            "bug: one missed planned day between done ones raised the reason"
        );
    }

    #[test]
    fn two_planned_days_missed_across_a_rest_day_raise_it() {
        // Planned Sat and Mon, rest on Sun between them, both missed.
        let sessions = [
            paired("2026-11-19", true),
            paired("2026-11-21", false),
            paired("2026-11-23", false),
        ];
        let run = run_of(&sessions, &[]).unwrap();
        assert_eq!(
            run.days,
            vec![date("2026-11-21"), date("2026-11-23")],
            "bug: the rest day between the two misses broke the run"
        );
        assert_eq!(
            run.sessions.len(),
            2,
            "bug: the run carries a session other than its two missed ones"
        );
        assert_eq!(
            run.since,
            date("2026-11-19"),
            "bug: the window does not start at its first judged day"
        );
        assert_eq!(
            (run.missed, run.planned),
            (2, 3),
            "bug: the window's counts are not two missed of three planned"
        );
    }

    #[test]
    fn a_done_day_after_the_misses_ends_the_run() {
        let sessions = [
            paired("2026-11-20", false),
            paired("2026-11-22", false),
            paired("2026-11-24", true),
        ];
        assert_eq!(
            run_of(&sessions, &[]),
            None,
            "bug: a done day after two misses did not end the run"
        );
    }

    #[test]
    fn a_session_is_not_missed_before_its_next_day_has_ended() {
        // Monday missed; Wednesday's session, unpaired, is still inside its
        // grace day on Thursday morning, so the run is one day.
        let sessions = [paired("2026-11-23", false), paired("2026-11-25", false)];
        let judged = judge(&sessions, &[], today());
        assert_eq!(
            judged[1].verdict,
            Verdict::Pending,
            "bug: a session was judged missed inside its grace day"
        );
        assert_eq!(
            missed_run(&judged, 2),
            None,
            "bug: a pending day counted toward the run"
        );
    }

    #[test]
    fn a_session_done_yesterday_counts_at_once() {
        let sessions = [
            paired("2026-11-21", false),
            paired("2026-11-23", false),
            paired("2026-11-25", true),
        ];
        assert_eq!(
            run_of(&sessions, &[]),
            None,
            "bug: yesterday's done session did not end the run at once"
        );
    }

    #[test]
    fn one_done_session_keeps_a_double_day() {
        let mut swim = paired("2026-11-23", true);
        swim.sport = SportType::Swim;
        let sessions = [
            paired("2026-11-21", false),
            paired("2026-11-23", false),
            swim,
        ];
        assert_eq!(
            run_of(&sessions, &[]),
            None,
            "bug: a double day with one session done was read as missed"
        );
    }

    #[test]
    fn a_plan_day_moved_by_a_day_is_not_a_miss() {
        // Planned Fri and Sun runs, done Sat and Mon: each a day late.
        let sessions = [
            plan_day("2026-11-20", SportType::Run),
            plan_day("2026-11-22", SportType::Run),
        ];
        let activities = [
            did("2026-11-21", SportType::TrailRunning),
            did("2026-11-23", SportType::Run),
        ];
        let judged = judge(&sessions, &activities, today());
        assert!(
            judged.iter().all(|s| s.verdict == Verdict::Done),
            "bug: a plan day done a day late was judged missed: {judged:?}"
        );
    }

    #[test]
    fn a_plan_day_two_days_off_or_of_another_sport_is_missed() {
        let sessions = [
            plan_day("2026-11-20", SportType::Run),
            plan_day("2026-11-22", SportType::Ride),
        ];
        let activities = [
            did("2026-11-18", SportType::Run),
            did("2026-11-22", SportType::Swim),
        ];
        let run = run_of(&sessions, &activities).unwrap();
        assert_eq!(
            run.days.len(),
            2,
            "bug: an activity two days off or of another sport matched a plan day"
        );
        assert!(
            run.sessions.iter().all(|s| s.provider.is_none()),
            "bug: a Dravr plan day was credited to a calendar: {run:?}"
        );
    }

    #[test]
    fn one_activity_does_for_one_planned_session() {
        // Two runs planned on consecutive days, one run done on the second:
        // the same-day match takes it, and the first day stays missed.
        let sessions = [
            plan_day("2026-11-22", SportType::Run),
            plan_day("2026-11-23", SportType::Run),
        ];
        let judged = judge(&sessions, &[did("2026-11-23", SportType::Run)], today());
        assert_eq!(
            judged[0].verdict,
            Verdict::Missed,
            "bug: one activity matched two planned sessions"
        );
        assert_eq!(
            judged[1].verdict,
            Verdict::Done,
            "bug: the same-day match did not take the activity"
        );
    }

    #[test]
    fn the_threshold_is_the_run_length() {
        let sessions = [
            paired("2026-11-19", false),
            paired("2026-11-21", false),
            paired("2026-11-23", false),
        ];
        let judged = judge(&sessions, &[], today());
        assert!(
            missed_run(&judged, 3).is_some(),
            "bug: a run of three missed days did not reach a threshold of three"
        );
        assert!(
            missed_run(&judged, 4).is_none(),
            "bug: a run of three missed days reached a threshold of four"
        );
    }
}
