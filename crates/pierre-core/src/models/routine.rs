// ABOUTME: A routine — when Dravr speaks first: what it does, for whom, on which trigger, and its next slot
// ABOUTME: Pure schedule math on the recipient's own clock: weekdays plus an hour, never between 00:00 and 05:00

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routines (ADR-036, carnet#1790).
//!
//! A routine is everything that makes the platform speak without being asked:
//! a trigger, the action it runs and who it is for. Built-in routines — the
//! check-in the athlete chose a cadence for — and routines people build with
//! the agent are the same object, so one engine decides *when* every one of
//! them fires and `PlatformPost` keeps deciding *how*.
//!
//! A scheduled routine fires on chosen weekdays at one hour of the
//! recipient's own wall clock. Nothing fires between midnight and
//! [`NO_WAKE_BEFORE_HOUR`]: a routine set for 02:00 fires at 05:00. Hours,
//! never minutes: routines due in the same hour share one wake of the
//! server, which is what keeps a scale-to-zero environment cheap.

use std::fmt;

use dravr_tronc::dates::{CivilDate, Elapsed, LocalResolution, Timestamp, Weekday, Zone};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::check_in::{CheckInCadence, CHECK_IN_HOUR, CHECK_IN_WINDOW_HOURS};
use super::TenantId;

/// The first local hour a routine may fire at. A schedule set earlier fires
/// at this hour instead: nobody is woken by a coach between midnight and 05:00.
pub const NO_WAKE_BEFORE_HOUR: u32 = 5;

/// How far ahead a schedule is searched for its next slot. Two weeks covers a
/// weekly schedule across any clock change.
const SEARCH_DAYS: i64 = 15;

/// The local hour a coach's morning brief fires at until they change it.
pub const MORNING_BRIEF_HOUR: u32 = 7;

/// How long after its slot a morning brief may still go out: a brief that
/// could not go out by mid-morning is no longer a morning brief.
const MORNING_BRIEF_WINDOW_HOURS: u32 = 3;

/// What a routine does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    /// The coach's check-in on the cadence the athlete chose (carnet#831).
    CheckIn,
    /// A coach's morning brief of the athletes their To handle queue holds
    /// (carnet#1791).
    MorningBrief,
}

impl RoutineKind {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::CheckIn, Self::MorningBrief];

    /// The stored spelling. Persisted: rows carry it, so it never changes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CheckIn => "check_in",
            Self::MorningBrief => "morning_brief",
        }
    }

    /// Read the stored spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// How long after its slot a firing may still go out. Past it the slot is
    /// skipped rather than sent late: a morning check-in is never an evening one.
    #[must_use]
    pub fn window(self) -> Elapsed {
        match self {
            Self::CheckIn => Elapsed::hours(i64::from(CHECK_IN_WINDOW_HOURS)),
            Self::MorningBrief => Elapsed::hours(i64::from(MORNING_BRIEF_WINDOW_HOURS)),
        }
    }
}

impl fmt::Display for RoutineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whom a routine speaks to and about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineScope {
    /// Its owner, about themselves.
    Me,
    /// Its owner (a coach), about every athlete they coach.
    Roster,
}

impl RoutineScope {
    /// Every scope.
    pub const ALL: [Self; 2] = [Self::Me, Self::Roster];

    /// The stored spelling. Persisted: rows carry it, so it never changes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Roster => "roster",
        }
    }

    /// Read the stored spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == value)
    }
}

/// Whether a routine fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineState {
    /// It fires on its trigger.
    Active,
    /// It does not fire until resumed. A check-in set to `off` is paused.
    Paused,
}

impl RoutineState {
    /// The stored spelling. Persisted: rows carry it, so it never changes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
        }
    }

    /// Read the stored spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Active, Self::Paused]
            .into_iter()
            .find(|state| state.as_str() == value)
    }
}

/// A set of weekdays, stored as a bit mask: bit 0 is Monday, bit 6 Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Weekdays(u8);

impl Weekdays {
    /// Every day of the week.
    pub const EVERY_DAY: Self = Self(0b111_1111);

    /// No day: a schedule over it never fires.
    pub const NONE: Self = Self(0);

    /// The set holding exactly `days`.
    #[must_use]
    pub fn of(days: &[Weekday]) -> Self {
        Self(
            days.iter()
                .fold(0, |mask, day| mask | (1 << day.num_days_from_monday())),
        )
    }

    /// The set a stored mask names, or `None` for bits past Sunday.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::EVERY_DAY.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The stored mask.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `day` is in the set.
    #[must_use]
    pub const fn contains(self, day: Weekday) -> bool {
        self.0 & (1 << day.num_days_from_monday()) != 0
    }

    /// Whether the set holds no day.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The days a check-in cadence posts on.
    #[must_use]
    pub fn of_cadence(cadence: CheckInCadence) -> Self {
        Self::of(
            &[
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ]
            .into_iter()
            .filter(|day| cadence.posts_on(*day))
            .collect::<Vec<_>>(),
        )
    }
}

/// When a scheduled routine fires: chosen weekdays at one hour of the
/// recipient's own clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Schedule {
    days: Weekdays,
    hour: u32,
}

impl Schedule {
    /// A schedule over `days` at `hour` (0–23). `None` for an hour past 23 or
    /// an empty set of days: a schedule that can never fire is not one.
    #[must_use]
    pub const fn try_new(days: Weekdays, hour: u32) -> Option<Self> {
        if hour < 24 && !days.is_empty() {
            Some(Self { days, hour })
        } else {
            None
        }
    }

    /// The weekdays it fires on.
    #[must_use]
    pub const fn days(self) -> Weekdays {
        self.days
    }

    /// The hour it was set for, before [`Self::firing_hour`] moves an early
    /// one: what its owner sees and edits.
    #[must_use]
    pub const fn hour(self) -> u32 {
        self.hour
    }

    /// The check-in's schedule for `cadence`: its days at
    /// [`CHECK_IN_HOUR`]. `None` for [`CheckInCadence::Off`].
    #[must_use]
    pub fn of_check_in(cadence: CheckInCadence) -> Option<Self> {
        Self::try_new(Weekdays::of_cadence(cadence), CHECK_IN_HOUR)
    }

    /// The hour it fires at: the hour it was set for, moved to
    /// [`NO_WAKE_BEFORE_HOUR`] when earlier.
    #[must_use]
    pub const fn firing_hour(self) -> u32 {
        if self.hour < NO_WAKE_BEFORE_HOUR {
            NO_WAKE_BEFORE_HOUR
        } else {
            self.hour
        }
    }

    /// The slot on `date` of `zone`'s calendar, when the schedule fires that
    /// day. A firing hour the clocks skip (a spring change at that hour) has
    /// no slot that day; one they show twice fires at the first.
    #[must_use]
    pub fn slot_on(self, zone: &Zone, date: CivilDate) -> Option<Timestamp> {
        if !self.days.contains(date.weekday()) {
            return None;
        }
        match zone.resolve(date.and_hms(self.firing_hour(), 0, 0).ok()?) {
            LocalResolution::Single(at) | LocalResolution::Ambiguous(at, _) => Some(at),
            LocalResolution::None => None,
        }
    }

    /// The first slot strictly after `after`, on `zone`'s clock.
    #[must_use]
    pub fn next_after(self, zone: &Zone, after: Timestamp) -> Option<Timestamp> {
        let today = zone.date_at(after);
        (0..SEARCH_DAYS)
            .filter_map(|offset| today.checked_add_days(offset))
            .filter_map(|date| self.slot_on(zone, date))
            .find(|slot| *slot > after)
    }

    /// The slot a firing at `now` belongs to: the latest slot still open at
    /// `now` (it began no more than `window` ago), else the next one.
    #[must_use]
    pub fn current_or_next(
        self,
        zone: &Zone,
        now: Timestamp,
        window: Elapsed,
    ) -> Option<Timestamp> {
        let open_since = now - window;
        self.next_after(zone, open_since)
            .filter(|slot| *slot <= now)
            .or_else(|| self.next_after(zone, now))
    }

    /// Whether `slot` is one of this schedule's slots on `zone`'s clock. A
    /// slot computed before the recipient moved zones is not.
    #[must_use]
    pub fn holds_slot(self, zone: &Zone, slot: Timestamp) -> bool {
        self.slot_on(zone, zone.date_at(slot)) == Some(slot)
    }
}

/// What makes a routine fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineTrigger {
    /// Weekdays at an hour of the recipient's clock.
    Schedule(Schedule),
}

/// One routine, as the engine reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    /// The routine's id.
    pub id: Uuid,
    /// The owner's tenant.
    pub tenant_id: TenantId,
    /// Who it belongs to; for [`RoutineScope::Me`], also whom it speaks to.
    pub owner_user_id: Uuid,
    /// What it does.
    pub kind: RoutineKind,
    /// Whom it speaks about.
    pub scope: RoutineScope,
    /// What makes it fire; `None` for a paused routine that has no schedule
    /// (a check-in set to `off`).
    pub trigger: Option<RoutineTrigger>,
    /// Whether it fires.
    pub state: RoutineState,
    /// The schedule slot it fires for next, once the engine scheduled it.
    pub next_slot: Option<Timestamp>,
    /// When it fires next: the slot, or later while a firing is deferred.
    pub next_fire: Option<Timestamp>,
    /// How many times the pending firing was deferred because the recipient
    /// was mid-conversation.
    pub deferrals: u32,
    /// When it last fired.
    pub last_fired_at: Option<Timestamp>,
    /// What a morning brief leaves out; empty for every other kind.
    pub brief: BriefSettings,
}

/// What a coach's morning brief leaves out: the attention reasons it does not
/// check, and the athletes it does not list. Stored on the routine as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BriefSettings {
    /// Attention reasons (their stored spelling, `nothing_received`, ...) the
    /// brief does not check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons_off: Vec<String>,
    /// Athletes the brief does not list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted: Vec<Uuid>,
}

impl BriefSettings {
    /// Whether the brief checks reasons of the stored spelling `reason`.
    #[must_use]
    pub fn checks(&self, reason: &str) -> bool {
        !self.reasons_off.iter().any(|off| off == reason)
    }

    /// Whether the brief lists `athlete`.
    #[must_use]
    pub fn lists(&self, athlete: Uuid) -> bool {
        !self.muted.contains(&athlete)
    }
}

/// What one firing of a routine did, as its run records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// It posted, and its reply was stored.
    Posted,
    /// Its turn ran and its reply was withheld.
    Withheld,
    /// It found nothing to say and posted nothing.
    Empty,
    /// A gate held it: nothing was posted for the slot.
    Held,
    /// It had nothing to run on (no data, a closed account) and moved on.
    Skipped,
}

impl RunOutcome {
    /// Every outcome.
    pub const ALL: [Self; 5] = [
        Self::Posted,
        Self::Withheld,
        Self::Empty,
        Self::Held,
        Self::Skipped,
    ];

    /// The stored spelling. Persisted: rows carry it, so it never changes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Posted => "posted",
            Self::Withheld => "withheld",
            Self::Empty => "empty",
            Self::Held => "held",
            Self::Skipped => "skipped",
        }
    }

    /// Read the stored spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == value)
    }
}

/// One firing of a routine, as the Routines screen reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRun {
    /// The schedule slot it fired for.
    pub slot: Timestamp,
    /// When it ran.
    pub ran_at: Timestamp,
    /// What it did.
    pub outcome: RunOutcome,
    /// How many items it listed (a morning brief's athletes); 0 otherwise.
    pub items: u32,
    /// Whom it could not judge (a morning brief's); none otherwise.
    pub left_out: LeftOut,
}

/// The athletes a morning brief could not judge: never listed, always named
/// as a count, so a missing signal never reads as all fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LeftOut {
    /// They don't share with the coach, or the coach-read gate refused them.
    pub not_sharing: u32,
    /// They share, and nothing of their training could be read.
    pub unreadable: u32,
    /// Their missed sessions were not checked: a calendar could not be read
    /// (carnet#1792). They may still be listed for another reason.
    pub missed_unreadable: u32,
    /// Their missed sessions were not checked: they switched the check off
    /// for this coach. They may still be listed for another reason.
    pub missed_off: u32,
}

impl Routine {
    /// The schedule it fires on, when it is active and scheduled.
    #[must_use]
    pub fn schedule(&self) -> Option<Schedule> {
        match (self.state, self.trigger) {
            (RoutineState::Active, Some(RoutineTrigger::Schedule(schedule))) => Some(schedule),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn montreal() -> Result<Zone, Box<dyn Error>> {
        Ok(Zone::from_name("America/Montreal")?)
    }

    fn local(
        zone: &Zone,
        y: i32,
        m: u32,
        d: u32,
        h: u32,
        min: u32,
    ) -> Result<Timestamp, Box<dyn Error>> {
        let civil = CivilDate::from_ymd(y, m, d)?.and_hms(h, min, 0)?;
        match zone.resolve(civil) {
            LocalResolution::Single(at) => Ok(at),
            other => Err(format!("ambiguous test instant: {other:?}").into()),
        }
    }

    #[test]
    fn stored_spellings_read_back() {
        for kind in RoutineKind::ALL {
            assert_eq!(
                RoutineKind::parse(kind.as_str()),
                Some(kind),
                "bug: a stored kind does not read back"
            );
        }
        for scope in RoutineScope::ALL {
            assert_eq!(
                RoutineScope::parse(scope.as_str()),
                Some(scope),
                "bug: a stored scope does not read back"
            );
        }
        for state in [RoutineState::Active, RoutineState::Paused] {
            assert_eq!(
                RoutineState::parse(state.as_str()),
                Some(state),
                "bug: a stored state does not read back"
            );
        }
        for outcome in RunOutcome::ALL {
            assert_eq!(
                RunOutcome::parse(outcome.as_str()),
                Some(outcome),
                "bug: a stored run outcome does not read back"
            );
        }
        assert_eq!(
            RoutineKind::parse("cron"),
            None,
            "bug: an unknown kind was read as a known one"
        );
        assert_eq!(
            RunOutcome::parse("deferred"),
            None,
            "bug: an unknown run outcome was read as a known one"
        );
    }

    #[test]
    fn weekdays_round_trip_their_mask_and_refuse_bits_past_sunday() {
        let mwf = Weekdays::of(&[Weekday::Mon, Weekday::Wed, Weekday::Fri]);
        assert_eq!(
            mwf.bits(),
            0b001_0101,
            "bug: Monday is not bit 0 or Sunday not bit 6"
        );
        assert_eq!(
            Weekdays::from_bits(mwf.bits()),
            Some(mwf),
            "bug: a stored mask does not read back"
        );
        assert!(
            mwf.contains(Weekday::Wed) && !mwf.contains(Weekday::Tue),
            "bug: membership does not follow the mask"
        );
        assert_eq!(
            Weekdays::from_bits(0b1000_0000),
            None,
            "bug: a bit past Sunday was read as a day"
        );
        assert_eq!(
            Weekdays::of_cadence(CheckInCadence::Daily),
            Weekdays::EVERY_DAY,
            "bug: a daily cadence misses a day"
        );
        assert_eq!(
            Weekdays::of_cadence(CheckInCadence::ThreePerWeek),
            mwf,
            "bug: three a week is not Monday-Wednesday-Friday"
        );
        assert_eq!(
            Weekdays::of_cadence(CheckInCadence::Off),
            Weekdays::NONE,
            "bug: `off` posts on a day"
        );
    }

    #[test]
    fn a_schedule_that_can_never_fire_is_refused() {
        assert_eq!(
            Schedule::try_new(Weekdays::NONE, 8),
            None,
            "bug: a schedule with no day was built"
        );
        assert_eq!(
            Schedule::try_new(Weekdays::EVERY_DAY, 24),
            None,
            "bug: a schedule at hour 24 was built"
        );
        assert_eq!(
            Schedule::of_check_in(CheckInCadence::Off),
            None,
            "bug: `off` has a check-in schedule"
        );
    }

    #[test]
    fn nothing_fires_between_midnight_and_five() -> TestResult {
        let zone = montreal()?;
        for hour in 0..NO_WAKE_BEFORE_HOUR {
            let schedule = Schedule::try_new(Weekdays::EVERY_DAY, hour).ok_or("no schedule")?;
            let after = local(&zone, 2026, 11, 24, 12, 0)?;
            let next = schedule.next_after(&zone, after).ok_or("no next slot")?;
            assert_eq!(
                next,
                local(&zone, 2026, 11, 25, 5, 0)?,
                "bug: a routine set for {hour}:00 fires before 05:00"
            );
        }
        let seven = Schedule::try_new(Weekdays::EVERY_DAY, 7).ok_or("no schedule")?;
        assert_eq!(
            seven.firing_hour(),
            7,
            "bug: an hour past the quiet window was moved"
        );
        Ok(())
    }

    #[test]
    fn the_next_slot_is_on_the_recipients_own_clock_and_weekdays() -> TestResult {
        let zone = montreal()?;
        let monday_check_in = Schedule::of_check_in(CheckInCadence::Weekly).ok_or("no schedule")?;
        // Tuesday 2026-11-24, noon: the next Monday is 2026-11-30.
        let after = local(&zone, 2026, 11, 24, 12, 0)?;
        assert_eq!(
            monday_check_in.next_after(&zone, after),
            Some(local(&zone, 2026, 11, 30, 8, 0)?),
            "bug: the next slot is not the next Monday at 08:00 local"
        );
        // Strictly after: a firing at the slot looks for the following one.
        let slot = local(&zone, 2026, 11, 30, 8, 0)?;
        assert_eq!(
            monday_check_in.next_after(&zone, slot),
            Some(local(&zone, 2026, 12, 7, 8, 0)?),
            "bug: the slot itself was returned as the one after it"
        );
        Ok(())
    }

    #[test]
    fn a_slot_still_inside_its_window_is_current_and_one_past_it_is_not() -> TestResult {
        let zone = montreal()?;
        let daily = Schedule::of_check_in(CheckInCadence::Daily).ok_or("no schedule")?;
        let window = RoutineKind::CheckIn.window();
        let at_nine = local(&zone, 2026, 11, 24, 9, 30)?;
        assert_eq!(
            daily.current_or_next(&zone, at_nine, window),
            Some(local(&zone, 2026, 11, 24, 8, 0)?),
            "bug: a slot still inside its window is not the current one"
        );
        let at_noon = local(&zone, 2026, 11, 24, 12, 0)?;
        assert_eq!(
            daily.current_or_next(&zone, at_noon, window),
            Some(local(&zone, 2026, 11, 25, 8, 0)?),
            "bug: a slot past its window is still current"
        );
        Ok(())
    }

    #[test]
    fn a_slot_from_before_a_zone_move_is_not_held() -> TestResult {
        let montreal = montreal()?;
        let paris = Zone::from_name("Europe/Paris")?;
        let daily = Schedule::of_check_in(CheckInCadence::Daily).ok_or("no schedule")?;
        let slot = local(&montreal, 2026, 11, 24, 8, 0)?;
        assert!(
            daily.holds_slot(&montreal, slot),
            "bug: a slot computed on the recipient's clock is not held"
        );
        assert!(
            !daily.holds_slot(&paris, slot),
            "bug: a slot from the zone the recipient left is still held"
        );
        Ok(())
    }

    #[test]
    fn a_routine_set_inside_the_skipped_spring_hour_fires_at_five() -> TestResult {
        // 2026-03-08: Montreal skips 02:00–03:00; a 02:00 routine fires at 05:00 anyway.
        let zone = montreal()?;
        let two = Schedule::try_new(Weekdays::EVERY_DAY, 2).ok_or("no schedule")?;
        let date = CivilDate::from_ymd(2026, 3, 8)?;
        assert_eq!(
            two.slot_on(&zone, date),
            Some(local(&zone, 2026, 3, 8, 5, 0)?),
            "bug: the spring change dropped a slot that fires at 05:00"
        );
        Ok(())
    }
}
