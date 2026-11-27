// ABOUTME: The break baseline of one coached athlete with nothing planned: is their current break unusual for them?
// ABOUTME: Read only under their routine-detection grant; scored by dravr-cageux's training_gap, never a population figure

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The break baseline
//!
//! Coach Home's `unusual_break` reason (Routines P3b, carnet#1792) measures a
//! quiet spell against the athlete's own last weeks of training, for an
//! athlete with nothing planned to miss. The scoring is dravr-cageux's
//! [`training_gap`](dravr_cageux::training_gap::training_gap); this module
//! holds what the roster read and when it applies.
//!
//! - **Only under the athlete's grant.** Learning someone's pattern to flag a
//!   deviation is profiling (Law 25 art. 8.1), so the check reads nothing
//!   until the athlete grants their coach routine detection
//!   ([`BreakCheck::NotGranted`]).
//! - **Only with nothing planned.** An athlete with a session planned in the
//!   window is judged against their plan by the missed-session check, so the
//!   two never both speak of one spell ([`BreakCheck::Planned`], Phil,
//!   2026-11-27).
//! - **A reading replaces the population threshold.** With a reading, the
//!   athlete's own pattern decides whether a quiet spell is raised: an
//!   unusual break is, from its third day; a usual one is not, even past
//!   `nothing_received`'s five days. Without one (no grant, too little
//!   history, an unreadable source) the five-day rule stands (Phil,
//!   2026-11-27).
//! - **A source down gives no finding.** With a dead source or stale data,
//!   the break may be sessions that never arrived, so the source reason
//!   stands alone and no unusual break is raised (Phil, 2026-11-27).
//! - **What cannot be read gives no reading.** A calendar or an activity read
//!   that failed or was cut is [`BreakCheck::Unreadable`]: the five-day rule
//!   then stands, so a missing signal never reads as a usual break.
//!
//! Counts and context only: never a percentage, never a guess at why.

use dravr_cageux::training_gap::{TrainingGap, TrainingGapReading, TrainingGapRule};
use dravr_tronc::dates::Timestamp;
use pierre_config::coach_home::{CoachHomeConfig, BASELINE_MIN_HISTORY_DAYS, BASELINE_WINDOW_DAYS};

use crate::missed_sessions::WellnessNote;

/// The break baseline of one athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakCheck {
    /// The athlete has not granted their coach routine detection.
    NotGranted,
    /// A session is planned for them in the window: their plan is the measure.
    Planned,
    /// A calendar or their activities could not be read in full.
    Unreadable,
    /// Their training days were read and scored.
    Read {
        /// When their activities were read.
        read_at: Timestamp,
        /// The scoring of their current break against their own window.
        gap: TrainingGap,
        /// The athlete's last wellness note since their last session, read
        /// only for an unusual break and only under their wellness grant.
        wellness: Option<WellnessNote>,
    },
}

impl BreakCheck {
    /// Whether the athlete's own pattern read their current break, so that it,
    /// and not the population threshold, decides whether a quiet spell is
    /// raised.
    #[must_use]
    pub const fn has_reading(&self) -> bool {
        matches!(
            self,
            Self::Read {
                gap: TrainingGap::Usual(_) | TrainingGap::Unusual(_),
                ..
            }
        )
    }

    /// The reading of an unusual break, with when it was read and the note
    /// that goes with it.
    #[must_use]
    pub const fn unusual(&self) -> Option<(&TrainingGapReading, Timestamp, Option<&WellnessNote>)> {
        match self {
            Self::Read {
                read_at,
                gap: TrainingGap::Unusual(reading),
                wellness,
            } => Some((reading, *read_at, wellness.as_ref())),
            _ => None,
        }
    }
}

/// The rule the baseline scores a break with.
///
/// The window and the history floor are the fixed ones the routine-detection
/// consent names; the shortest unusual break is a Coach Home threshold.
#[must_use]
pub const fn gap_rule(thresholds: &CoachHomeConfig) -> TrainingGapRule {
    TrainingGapRule {
        window_days: BASELINE_WINDOW_DAYS,
        min_history_days: BASELINE_MIN_HISTORY_DAYS,
        min_break_days: thresholds.baseline_min_break_days,
    }
}

#[cfg(test)]
mod tests {
    use dravr_cageux::training_gap::training_gap;
    use dravr_tronc::dates::CivilDate;

    use super::*;

    fn date(s: &str) -> CivilDate {
        CivilDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn reading() -> TrainingGapReading {
        TrainingGapReading {
            last_session: date("2026-11-20"),
            break_days: 6,
            longest_previous_break: 3,
            days_per_week: 4,
            weeks_of_history: 8,
        }
    }

    fn read(gap: TrainingGap) -> BreakCheck {
        BreakCheck::Read {
            read_at: Timestamp::now(),
            gap,
            wellness: None,
        }
    }

    #[test]
    fn only_a_usual_or_unusual_reading_replaces_the_population_threshold() {
        assert!(
            read(TrainingGap::Usual(reading())).has_reading(),
            "bug: a usual break left the five-day rule standing"
        );
        assert!(
            read(TrainingGap::Unusual(reading())).has_reading(),
            "bug: an unusual break left the five-day rule standing"
        );
        for check in [
            BreakCheck::NotGranted,
            BreakCheck::Planned,
            BreakCheck::Unreadable,
            read(TrainingGap::NotEnoughHistory {
                weeks_of_history: 2,
            }),
        ] {
            assert!(
                !check.has_reading(),
                "bug: {check:?} silenced the five-day rule without a reading"
            );
        }
    }

    #[test]
    fn only_an_unusual_break_is_one_to_raise() {
        let unusual = read(TrainingGap::Unusual(reading()));
        let (got, _, note) = unusual
            .unusual()
            .expect("bug: an unusual reading is not one to raise");
        assert_eq!(
            *got,
            reading(),
            "bug: the raised reading is not the one read"
        );
        assert!(
            note.is_none(),
            "bug: a note was made up for a reading without one"
        );
        assert!(
            read(TrainingGap::Usual(reading())).unusual().is_none(),
            "bug: a usual break is one to raise"
        );
        assert!(
            BreakCheck::Planned.unusual().is_none(),
            "bug: an athlete with a plan has a break to raise"
        );
    }

    /// The consent's words name "8 weeks" and "4 weeks" (dravr-contremaitre
    /// strings), so the default rule is pinned at its boundaries by what it
    /// reads, not by restating its numbers.
    #[test]
    fn the_default_rule_reads_eight_weeks_needs_four_and_flags_from_the_third_day() {
        let rule = gap_rule(&CoachHomeConfig::default());
        let today = date("2026-11-27");
        let ago = |days: i64| today.checked_sub_days(days).unwrap();
        // A session every day from `from` days ago through `to` days ago.
        let daily = |from: i64, to: i64| (to..=from).map(ago).collect::<Vec<_>>();
        let is_unusual = |gap: TrainingGap| matches!(gap, TrainingGap::Unusual(_));
        assert!(
            matches!(
                training_gap(&daily(27, 5), today, &rule),
                TrainingGap::NotEnoughHistory { .. }
            ),
            "bug: under four weeks of history gave a reading"
        );
        assert!(
            is_unusual(training_gap(&daily(28, 5), today, &rule)),
            "bug: four weeks of history gave no reading"
        );
        assert!(
            matches!(
                training_gap(&daily(40, 3), today, &rule),
                TrainingGap::Usual(_)
            ),
            "bug: a daily trainer's two days off read as unusual"
        );
        assert!(
            is_unusual(training_gap(&daily(40, 4), today, &rule)),
            "bug: a daily trainer's third day off is not unusual"
        );
        // A first session just outside the eight weeks is not read, so three
        // weeks of history remain; just inside, it is the athlete's history.
        let mut outside = daily(20, 4);
        outside.push(ago(57));
        assert!(
            matches!(
                training_gap(&outside, today, &rule),
                TrainingGap::NotEnoughHistory { .. }
            ),
            "bug: a session before the eight weeks was read"
        );
        let mut inside = daily(20, 4);
        inside.push(ago(56));
        assert!(
            matches!(training_gap(&inside, today, &rule), TrainingGap::Usual(_)),
            "bug: a session on the eight weeks' first day was not read"
        );
    }
}
