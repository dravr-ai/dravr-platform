// ABOUTME: Which civil days the athlete's record holds in full, from the activity cache's own coverage signals
// ABOUTME: Each live source's latest head read bounds the end, its current capture's backfill depth bounds the start

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Which days the record holds in full
//!
//! A period total is contradicted only over days the record holds every
//! session of. The rows themselves cannot say so: a day with no row reads the
//! same whether the athlete rested or the cache never received the session.
//! So the span is taken from the signals the activity cache keeps for that
//! same question, read per live source:
//!
//! - **Up to when.** The source's latest successful read of its list head
//!   (`ActivityCacheRepository::latest_activity_sync`, per provider). The span
//!   ends at the *oldest* of these across the athlete's live sources: a fresh
//!   Strava read says nothing about a Garmin connection last read days ago,
//!   whose sessions since are missing from the cache while a tool reading
//!   Garmin directly still sees them. A head read vouches for its own day only
//!   up to the moment it ran, so today counts only when that read is at most
//!   [`TODAY_SYNC_RECENCY`] old, which in practice is a read this turn made.
//!   Otherwise the span ends the day before the read: a run finished after a
//!   07:00 sync is not in the cache at 19:00.
//! - **From when.** How deep a completed backfill of the source's *current*
//!   capture reached (`ActivityCacheRepository::get_backfill_coverage`), read
//!   with the historical gate's own rule (`historical_depth_covered` in the
//!   tool runtime), so a window starts inside the span exactly when
//!   `get_activities` would itself serve it from the cache as complete. The
//!   span never starts before the cache read behind the record reaches, nor on
//!   a day that read's row cap may have cut.
//!
//! A live source that was never fetched, or whose current capture has no
//! backfill on record, vouches for nothing, and the history is absent. A
//! lapsed connection, one the athlete must reconnect, does not bound the span:
//! no session can arrive from it until they do, and what any tool reads of it
//! meanwhile is the cache itself.

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};

use super::super::RecordedActivity;
use super::Window;

/// How old the latest head read may be for today to count as held.
///
/// The tools a turn calls read the provider's head moments before the reply
/// is checked, so a read that recent is this turn's. An older one left the
/// rest of the day open: whatever the athlete finished since may be on the
/// provider, and in the reply, without being in the cache.
const TODAY_SYNC_RECENCY: TimeDelta = TimeDelta::minutes(10);

/// How deep a source's history is vouched for, as the historical gate reads
/// its backfill coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillDepth {
    /// No backfill of the source's current capture is on record, so no day is
    /// vouched for: rows alone may be only the recent slice of a deeper
    /// window.
    Unrecorded,
    /// A backfill of the current capture reached back to this instant.
    Reached(DateTime<Utc>),
    /// The provider reported its feed exhausted: nothing older exists.
    FeedEnd,
}

/// What one of the athlete's live activity sources vouches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceCoverage {
    /// When the source's list head was last read successfully; `None` when
    /// it never was.
    pub last_sync: Option<DateTime<Utc>>,
    /// How far back its history is held.
    pub depth: BackfillDepth,
}

/// How far back the athlete-data layer looks when matching a claim.
///
/// The chat stage reads the activity cache this far behind every verified
/// reply, so a [`CacheRead`] it builds starts this long before the turn.
///
/// An agent discussing "last month" or "the past few weeks" is the common case;
/// a quarter covers those without pulling a whole history into a per-reply
/// check.
pub const ATHLETE_RECORD_WINDOW: TimeDelta = TimeDelta::days(90);

/// What the activity-cache read behind the record reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheRead {
    /// The earliest instant the read asked for.
    pub from: DateTime<Utc>,
    /// When the read filled its row cap, the civil day of its oldest row: the
    /// cap fell inside that day, so only the days after it are whole.
    pub cut_day: Option<NaiveDate>,
}

/// One source's own sessions, merged within that source only.
///
/// A tool that reads one provider adds up that provider's sessions. When the
/// athlete's sources disagree (a Zwift ride only Strava holds, a strength
/// session only Garmin holds), that total differs from the merged one, and a
/// figure read off it is the record of that source, not an invention.
#[derive(Debug, Clone)]
pub struct SourceSessions {
    /// The source's slug, as its cached rows carry it.
    pub source: String,
    /// Its sessions.
    pub sessions: Vec<RecordedActivity>,
}

/// Everything [`PeriodHistory::vouched`] builds a history from.
#[derive(Debug, Clone)]
pub struct HistoryInputs<Z: TimeZone> {
    /// One entry per workout, across every source: the copies several
    /// providers hold of one session merged, so a total counts it once.
    pub sessions: Vec<RecordedActivity>,
    /// Each source's own sessions, when more than one source holds any.
    pub by_source: Vec<SourceSessions>,
    /// What the cache read reached.
    pub read: CacheRead,
    /// The athlete's live activity sources: connected, not lapsed, and able
    /// to deliver activities.
    pub sources: Vec<SourceCoverage>,
    /// When the record was read. The record's `today` is this instant's day
    /// on the athlete's clock.
    pub now: DateTime<Utc>,
    /// The athlete's zone.
    pub zone: Z,
    /// The first day of the training block the athlete is in, the phase of
    /// their active plan covering today, when they have one.
    pub block_start: Option<NaiveDate>,
}

/// What the record vouches for when a claim sums or averages over a period.
///
/// Separate from [`super::super::AthleteRecord::activities`] because a total
/// needs two things a single-activity match does not: each workout counted
/// once, and the days the record is complete for. Built only through
/// [`Self::vouched`], so the days it vouches for are always derived the same
/// way.
#[derive(Debug, Clone)]
pub struct PeriodHistory {
    /// One entry per workout across every source.
    pub(super) sessions: Vec<RecordedActivity>,
    /// Each source's own sessions, when more than one source holds any.
    pub(super) by_source: Vec<SourceSessions>,
    /// The first civil day every live source holds in full.
    pub(super) covered_from: NaiveDate,
    /// The last civil day every live source holds in full.
    pub(super) synced_through: NaiveDate,
    /// The first day of the current training block, when there is one.
    pub(super) block_start: Option<NaiveDate>,
}

impl PeriodHistory {
    /// The history the record vouches for, or `None` when it vouches for no
    /// day: no live source, a live source never fetched, or one whose current
    /// capture has no backfill on record.
    #[must_use]
    pub fn vouched<Z: TimeZone>(inputs: HistoryInputs<Z>) -> Option<Self> {
        let HistoryInputs {
            sessions,
            by_source,
            read,
            sources,
            now,
            zone,
            block_start,
        } = inputs;
        // The athlete's clock, as `civil_time::clock_date` reads it: every
        // instant here is a real moment, never a date-only row.
        let day = |instant: DateTime<Utc>| instant.with_timezone(&zone).date_naive();

        let oldest_sync = sources
            .iter()
            .map(|source| source.last_sync)
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .min()?;
        let synced_through = if now.signed_duration_since(oldest_sync) <= TODAY_SYNC_RECENCY {
            day(now)
        } else {
            day(oldest_sync).pred_opt()?
        };

        // The read's first instant, and a backfill's floor, can fall inside a
        // day; only the day after is whole.
        let mut covered_from = day(read.from).succ_opt()?;
        if let Some(cut) = read.cut_day {
            covered_from = covered_from.max(cut.succ_opt()?);
        }
        for source in &sources {
            match source.depth {
                BackfillDepth::Unrecorded => return None,
                BackfillDepth::Reached(floor) => {
                    covered_from = covered_from.max(day(floor).succ_opt()?);
                }
                BackfillDepth::FeedEnd => {}
            }
        }

        Some(Self {
            sessions,
            by_source,
            covered_from,
            synced_through,
            block_start,
        })
    }

    /// Whether the record holds every day of `window`.
    pub(super) fn covers(&self, window: &Window) -> bool {
        window.start >= self.covered_from && window.end <= self.synced_through
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::athlete_data::{check, AthleteRecord};
    use crate::claim_extractor::{ClaimSource, ExtractedClaim};
    use pierre_core::civil_time::resolve_zone;
    use pierre_core::models::SportType;
    use pierre_memory::{ClaimCategory, ClaimStatus};

    fn at(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, d, h, m, 0).unwrap()
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, d).unwrap()
    }

    fn run(date: NaiveDate, km: f64) -> RecordedActivity {
        RecordedActivity {
            date,
            sport: SportType::Run,
            name: String::new(),
            distance_km: Some(km),
            duration_min: km * 6.0,
            elevation_m: None,
        }
    }

    fn source(last_sync: Option<DateTime<Utc>>, depth: BackfillDepth) -> SourceCoverage {
        SourceCoverage { last_sync, depth }
    }

    /// Wednesday 2026-09-09 at 19:00 UTC, read back 90 days, block-less.
    fn inputs(sources: Vec<SourceCoverage>) -> HistoryInputs<Utc> {
        let now = at(9, 19, 0);
        HistoryInputs {
            sessions: vec![run(day(7), 10.0), run(day(8), 12.0)],
            by_source: Vec::new(),
            read: CacheRead {
                from: now - TimeDelta::days(90),
                cut_day: None,
            },
            sources,
            now,
            zone: Utc,
            block_start: None,
        }
    }

    fn deep() -> BackfillDepth {
        BackfillDepth::Reached(at(1, 0, 0) - TimeDelta::days(200))
    }

    #[test]
    fn a_sync_this_turn_holds_today() {
        let history =
            PeriodHistory::vouched(inputs(vec![source(Some(at(9, 18, 55)), deep())])).unwrap();
        assert_eq!(history.synced_through, day(9));
    }

    #[test]
    fn a_morning_sync_leaves_today_open() {
        let history =
            PeriodHistory::vouched(inputs(vec![source(Some(at(9, 7, 0)), deep())])).unwrap();
        assert_eq!(
            history.synced_through,
            day(8),
            "a session finished after 07:00 may be missing, so today is not held"
        );
    }

    /// The reviewer's case end to end: synced at 07:00, a 10 km run at 18:00
    /// that never synced, and a coach reply at 19:00 that counts it. The cache
    /// sums 22 km; the reply's 32 km is right, so it must not be contradicted.
    #[test]
    fn an_evening_session_after_a_morning_sync_is_not_contradicted() {
        let outcome_when_synced_at = |sync: DateTime<Utc>| {
            let record = AthleteRecord {
                has_provider: true,
                today: day(9),
                locale: "en".to_owned(),
                activities: vec![run(day(7), 10.0), run(day(8), 12.0)],
                history: PeriodHistory::vouched(inputs(vec![source(Some(sync), deep())])),
            };
            let claim = ExtractedClaim {
                text: "That brings you to 32 km this week.".to_owned(),
                category: ClaimCategory::AthleteData,
                source: ClaimSource::Reply,
            };
            check(&claim, &record).unwrap()
        };
        let morning = outcome_when_synced_at(at(9, 7, 0));
        assert_eq!(
            morning.status,
            ClaimStatus::Unverifiable,
            "{}",
            morning.explanation
        );
        // Read this turn, the same 22 km is the whole week, and 32 km is wrong.
        let this_turn = outcome_when_synced_at(at(9, 18, 55));
        assert_eq!(
            this_turn.status,
            ClaimStatus::Contradicted,
            "{}",
            this_turn.explanation
        );
    }

    /// Two providers: Strava read this turn, Garmin last read three days ago.
    /// The fresh one must not vouch for the stale one's days.
    #[test]
    fn the_stalest_live_source_bounds_the_span() {
        let history = PeriodHistory::vouched(inputs(vec![
            source(Some(at(9, 18, 58)), deep()),
            source(Some(at(6, 12, 0)), deep()),
        ]))
        .unwrap();
        assert_eq!(history.synced_through, day(5));
    }

    #[test]
    fn a_live_source_never_fetched_vouches_for_nothing() {
        assert!(PeriodHistory::vouched(inputs(vec![
            source(Some(at(9, 18, 58)), deep()),
            source(None, deep()),
        ]))
        .is_none());
    }

    #[test]
    fn no_live_source_vouches_for_nothing() {
        assert!(PeriodHistory::vouched(inputs(Vec::new())).is_none());
    }

    #[test]
    fn rows_without_a_backfill_on_record_vouch_for_nothing() {
        assert!(PeriodHistory::vouched(inputs(vec![
            source(Some(at(9, 18, 58)), deep()),
            source(Some(at(9, 18, 58)), BackfillDepth::Unrecorded),
        ]))
        .is_none());
    }

    #[test]
    fn the_span_starts_where_every_backfill_and_the_read_reach() {
        // The read starts 2026-06-11 19:00, so 06-12 is its first whole day.
        let read_bound = PeriodHistory::vouched(inputs(vec![source(
            Some(at(9, 18, 58)),
            BackfillDepth::FeedEnd,
        )]))
        .unwrap();
        assert_eq!(
            read_bound.covered_from,
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        );

        // A backfill that reached 08-20 10:00 holds from 08-21; the deeper
        // source does not lend the shallower one its depth.
        let shallow = BackfillDepth::Reached(Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap());
        let backfill_bound = PeriodHistory::vouched(inputs(vec![
            source(Some(at(9, 18, 58)), deep()),
            source(Some(at(9, 18, 58)), shallow),
        ]))
        .unwrap();
        assert_eq!(
            backfill_bound.covered_from,
            NaiveDate::from_ymd_opt(2026, 8, 21).unwrap()
        );
    }

    #[test]
    fn a_capped_read_holds_only_the_days_after_its_cut() {
        let mut capped = inputs(vec![source(Some(at(9, 18, 58)), deep())]);
        capped.read.cut_day = Some(day(1));
        let history = PeriodHistory::vouched(capped).unwrap();
        assert_eq!(history.covered_from, day(2));
    }

    /// Days are the athlete's: a 23:30 sync in Toronto on Tuesday is 03:30
    /// Wednesday in UTC, and holds Monday, not Tuesday.
    #[test]
    fn the_sync_day_is_on_the_athletes_clock() {
        let toronto = resolve_zone(Some("America/Toronto"));
        let now = at(9, 19, 0);
        let history = PeriodHistory::vouched(HistoryInputs {
            sessions: Vec::new(),
            by_source: Vec::new(),
            read: CacheRead {
                from: now - TimeDelta::days(90),
                cut_day: None,
            },
            sources: vec![source(Some(at(9, 3, 30)), deep())],
            now,
            zone: toronto,
            block_start: None,
        })
        .unwrap();
        assert_eq!(history.synced_through, day(7));
    }
}
