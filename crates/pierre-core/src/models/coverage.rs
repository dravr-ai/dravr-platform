// ABOUTME: Whether a served window is the whole truth for the window that was asked
// ABOUTME: One Coverage type for every window a tool serves: completeness, span, the vouched day, what is pending

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Window coverage.
//!
//! A count of activities is only as true as the window it was counted over.
//! The live replay of 2026-11-09 counted 43 trail runs since 2020 from a
//! 151-row cached window served while the live read ran on, and 599 four
//! minutes later from the 2,000 rows the read stopped at; each was stated as
//! fact. Every served window therefore says whether it is complete, and when
//! it is not, why.
//!
//! [`Coverage`] is the one shape every reader serves that answer in: the
//! `get_activities` sidecar, the training-history series, the volume weeks and
//! a provider's aggregate totals. It says whether the window is complete
//! ([`Completeness`]), what it spans ([`CoverageWindow`]), the first day the
//! stored history stands behind (`trustworthy_from`), and the walk still owed
//! for it ([`Pending`]): the history walk that copies the athlete's history
//! in, or the detail walk that reads its segment efforts. On the wire it flattens to `complete`,
//! `incomplete_reason`, `window`, `trustworthy_from`, `pending` and the `note`
//! a model reads ([`Coverage::note`]).

use std::borrow::Cow;
use std::fmt::Write as _;
use std::time::Duration;

use chrono::NaiveDate;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Why a served window is not the whole truth for the window asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    /// The live read outlived the turn's budget; the cached window answered
    /// and the read runs on.
    LiveReadRunning,
    /// The read stopped at its row cap, so older activities were not read.
    ReadCapped,
    /// The provider could not be read; what it last synced answered.
    ProviderUnavailable,
    /// The athlete's history walk has not reached the start of the window
    /// yet; the store answered with what it holds so far.
    HistoryWalkRunning,
    /// The stored history begins too late to stand behind the whole window:
    /// only `[trustworthy_from, asked_to]` is answered.
    ShallowHistory,
    /// Nothing stored covers the window, so nothing was computed.
    NoStoredActivities,
    /// The window's activities are stored, but the segment efforts of some
    /// were never read; the detail walk reading them runs on
    /// ([`Pending::DetailWalkRunning`]).
    SegmentDetailWalking,
    /// A detail walk ended over the window, and the segment efforts of
    /// `unreadable` of its activities could not be read: the backend
    /// answered nothing for them, or they are held only in the athlete's
    /// own export, which no walk reads.
    SegmentDetailPartial {
        /// Activities of the window whose segment efforts were not read.
        unreadable: u64,
    },
    /// No backend the athlete holds reads segment efforts: they come from
    /// Strava's API, which this athlete has not connected.
    SegmentDetailUnavailable,
}

impl IncompleteReason {
    /// The sentence a model reads after "This window is INCOMPLETE:", when
    /// the reason has one of its own. `None` for a reason whose note is a
    /// whole sentence of its own ([`Self::ShallowHistory`],
    /// [`Self::NoStoredActivities`]). English, like the rest of the note:
    /// the model renders it in the athlete's language.
    #[must_use]
    pub const fn explanation(self) -> Option<&'static str> {
        match self {
            Self::LiveReadRunning => Some(
                "the live read of the provider is still running, so only the activities already synced are shown; the rest lands when it finishes",
            ),
            Self::ReadCapped => Some(
                "the read stopped at its row cap, so activities older than the oldest shown were not read",
            ),
            Self::ProviderUnavailable => {
                Some("the provider could not be read, so only what it last synced is shown")
            }
            Self::HistoryWalkRunning => Some(
                "the athlete's history is still being copied in, so only the activities stored so far are shown; older ones land as the walk reaches them",
            ),
            Self::SegmentDetailWalking => Some(
                "the segment efforts of some of its activities are still being read in the background, so the efforts here are not final; the rest land as the walk reaches them",
            ),
            Self::SegmentDetailPartial { .. } => Some(
                "the segment efforts of some of its activities could not be read, so the efforts here are not all the athlete's",
            ),
            Self::SegmentDetailUnavailable => Some(
                "segment efforts can't be read for this athlete yet; they come from Strava's API, which this athlete hasn't connected",
            ),
            Self::ShallowHistory | Self::NoStoredActivities => None,
        }
    }
}

/// Whether a served window is the whole truth for the window asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every activity of the asked window was read.
    Complete,
    /// The served window is partial, for this reason.
    Incomplete(IncompleteReason),
}

impl Completeness {
    /// Whether the window is complete.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    /// Why the window is incomplete, when it is.
    #[must_use]
    pub const fn reason(self) -> Option<IncompleteReason> {
        match self {
            Self::Complete => None,
            Self::Incomplete(reason) => Some(reason),
        }
    }
}

/// Which activities a window spans.
///
/// Serialized with a `kind` tag. A tool that answers the window its caller
/// asked for (`get_activities`, the training history) is [`Self::Asked`]; a
/// provider's aggregate totals (`get_stats`) say which sample they were summed
/// over, so a sample is never read as a lifetime figure (carnet#919).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowScope {
    /// The window the caller asked for, `[asked_from, asked_to]`.
    #[default]
    Asked,
    /// The athlete's whole history, as the provider's aggregate reports it.
    Lifetime,
    /// Only the provider's most recent activities: `total` of them, the
    /// oldest on `oldest`.
    RecentActivities,
    /// Only the activities of the last `days` days, ending now.
    LastDays {
        /// Length of the window, in days.
        days: u32,
    },
    /// The window the caller asked for, `[asked_from, asked_to]`, counted in
    /// segment reads: `total` is every activity in it, `returned` those whose
    /// segment efforts were read (`get_segment_history`).
    SegmentsRead,
}

/// Count, distance, duration and climb summed over a whole window, taken
/// before a display limit truncates it.
///
/// A year-against-year question is answered from these, not from the shown
/// slice: with a display limit of 20, a 63-ride 2011 read as its last three
/// December rides (carnet#874). Serialized with distance and elevation to one
/// decimal and the duration exact, which the agent quotes and derives pace
/// from (carnet#1015).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct WindowTotals {
    /// Activities in the window.
    pub count: u64,
    /// Summed distance in kilometres, over the activities that report one.
    pub distance_km: f64,
    /// Summed duration in seconds, exact.
    pub duration_seconds: u64,
    /// Summed elevation gain in metres, over the activities that report one.
    pub elevation_gain_m: f64,
}

impl Serialize for WindowTotals {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let one_decimal = |value: f64| (value * 10.0).round() / 10.0;
        let mut totals = serializer.serialize_struct("WindowTotals", 4)?;
        totals.serialize_field("count", &self.count)?;
        totals.serialize_field("distance_km", &one_decimal(self.distance_km))?;
        totals.serialize_field("duration_seconds", &self.duration_seconds)?;
        totals.serialize_field("elevation_gain_m", &one_decimal(self.elevation_gain_m))?;
        totals.end()
    }
}

/// What a served window spans.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct CoverageWindow {
    /// Which activities the window spans.
    #[serde(default)]
    pub scope: WindowScope,
    /// First day asked for, `YYYY-MM-DD`; absent when the ask had no start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_from: Option<NaiveDate>,
    /// Last day asked for, `YYYY-MM-DD`; absent when the ask ran to now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_to: Option<NaiveDate>,
    /// The oldest day the window holds an activity on, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<NaiveDate>,
    /// The newest day the window holds an activity on, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<NaiveDate>,
    /// Activities the window holds, before any display limit; absent when
    /// the reader does not count them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Whether `total` is only a lower bound: the read filled its row cap, so
    /// the window may hold more.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub total_is_lower_bound: bool,
    /// Entries this payload returns: activities, or days of a daily series.
    #[serde(default)]
    pub returned: u64,
    /// The window's totals, when the reader summed them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<WindowTotals>,
}

/// What is still owed for a window: the history walk that would fill it with
/// activities, or the detail walk that reads the segment efforts of the
/// activities it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    /// No walk of the athlete's history has run yet; one has been asked for.
    HistoryNotWalked,
    /// A walk of the athlete's history is running.
    HistoryWalkRunning {
        /// Months of history walked so far.
        months_done: u32,
        /// Months the walk has to cover; absent while its floor is not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        months_total: Option<u32>,
        /// Seconds until the walk reaches its floor at the pace its steps
        /// measured; absent until a step measured one or the floor is known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eta_secs: Option<u64>,
    },
    /// A detail walk is reading the segment efforts of the window's
    /// activities, newest first.
    DetailWalkRunning {
        /// Activities the walk has read so far.
        units_done: u32,
        /// Activities the walk has to read; absent while not counted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        units_total: Option<u32>,
        /// Seconds until the walk has read them all at its pace; absent
        /// until it has a total and a pace.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eta_secs: Option<u64>,
    },
}

impl Pending {
    /// A running walk's state from its recorded progress: `months_done` of
    /// `months_total`, each month measured to take `secs_per_month`.
    ///
    /// The ETA is the months left times the measured pace, and needs both;
    /// a walk past its total has none left.
    #[must_use]
    pub fn running(
        months_done: u32,
        months_total: Option<u32>,
        secs_per_month: Option<f64>,
    ) -> Self {
        Self::HistoryWalkRunning {
            months_done,
            months_total,
            eta_secs: eta_secs(months_done, months_total, secs_per_month),
        }
    }

    /// A running detail walk's state from its recorded progress:
    /// `units_done` of `units_total` activities read, each taking
    /// `secs_per_unit`, the pace the walk goes at (the reader takes the
    /// slower of what its steps measured and what its budget allows).
    ///
    /// The ETA is the activities left times that pace, and needs both.
    #[must_use]
    pub fn detail_walk_running(
        units_done: u32,
        units_total: Option<u32>,
        secs_per_unit: Option<f64>,
    ) -> Self {
        Self::DetailWalkRunning {
            units_done,
            units_total,
            eta_secs: eta_secs(units_done, units_total, secs_per_unit),
        }
    }
}

/// Seconds until a walk at `pace` seconds a unit covers the units it has
/// left: `total` less `done`, none once past its total. `None` without a
/// total or a pace.
fn eta_secs(done: u32, total: Option<u32>, pace: Option<f64>) -> Option<u64> {
    total.zip(pace).and_then(|(total, pace)| {
        let left = f64::from(total.saturating_sub(done));
        // A pace that is negative, not a number or past any duration is
        // no measurement, and gives no ETA.
        Duration::try_from_secs_f64((left * pace).ceil())
            .ok()
            .map(|eta| eta.as_secs())
    })
}

/// Whether a served window is the whole truth for the window asked, what it
/// spans, the first day it stands behind, and what is still owed for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    /// Whether the window is complete, and why not when it is not.
    pub completeness: Completeness,
    /// What the window spans.
    pub window: CoverageWindow,
    /// The first day the stored history stands behind, when it starts later
    /// than the window asked.
    pub trustworthy_from: Option<NaiveDate>,
    /// The walk still owed for the window, when one is.
    pub pending: Option<Pending>,
}

impl Coverage {
    /// A complete window spanning `window`, with nothing pending.
    #[must_use]
    pub const fn complete(window: CoverageWindow) -> Self {
        Self {
            completeness: Completeness::Complete,
            window,
            trustworthy_from: None,
            pending: None,
        }
    }

    /// A provider's aggregate over the athlete's whole history, of `total`
    /// activities.
    #[must_use]
    pub fn lifetime(total: u64) -> Self {
        Self::complete(CoverageWindow {
            scope: WindowScope::Lifetime,
            total: Some(total),
            returned: total,
            ..CoverageWindow::default()
        })
    }

    /// Whether the window is complete.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.completeness.is_complete()
    }

    /// What a model must and must not claim from this window, or `None` when
    /// the window answers its ask with nothing to qualify.
    ///
    /// A daily series the stored history cannot warm says which days have no
    /// rows and forbids estimating them; a window of activities states its
    /// whole count and span, so the shown slice is never read as the
    /// athlete's history, and an incomplete one forbids stating a count as
    /// fact. A provider's aggregate totals carry no note: the `get_stats`
    /// description says how to read their scope.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self.completeness.reason() {
            Some(IncompleteReason::NoStoredActivities) => self.nothing_stored_note(),
            Some(IncompleteReason::ShallowHistory) => self.shallow_history_note(),
            _ if self.window.scope == WindowScope::SegmentsRead => self.segments_read_note(),
            _ if self.window.scope == WindowScope::Asked => self.activities_note(),
            _ => None,
        }
    }

    /// The sentence a pending walk adds to a daily series' note.
    fn fetching(&self) -> &'static str {
        if self.pending.is_some() {
            " Their history is being fetched in the background; say so."
        } else {
            ""
        }
    }

    fn nothing_stored_note(&self) -> Option<String> {
        let (from, to) = (self.window.asked_from?, self.window.asked_to?);
        Some(format!(
            "No stored activities cover {from} to {to}, so no CTL/ATL/TSB rows were \
             computed and no fitness figure exists for this athlete yet. Do not \
             estimate one and do not describe their chronic load. Answer from \
             the activities you can actually see.{}",
            self.fetching()
        ))
    }

    fn shallow_history_note(&self) -> Option<String> {
        let (from, vouched) = (self.window.asked_from?, self.trustworthy_from?);
        Some(format!(
            "Stored history only reaches back far enough to compute from {vouched} \
             onward, so days between {from} and {vouched} have no rows and no \
             fitness figure. Cite CTL/ATL/TSB only for {vouched} onward, and do not \
             infer values for the earlier part of the window.{}",
            self.fetching()
        ))
    }

    fn activities_note(&self) -> Option<String> {
        let total = self.window.total?;
        let returned = self.window.returned;
        let reason = self.completeness.reason();
        let total_phrase =
            if self.window.total_is_lower_bound || reason == Some(IncompleteReason::ReadCapped) {
                format!("at least {total}")
            } else {
                total.to_string()
            };
        let span_phrase = match (self.window.oldest, self.window.newest) {
            (Some(oldest), Some(newest)) => format!(" spanning {oldest} to {newest}"),
            _ => String::new(),
        };
        let mut note = if total > returned {
            format!(
                "This window holds {total_phrase} activities{span_phrase}; only {returned} are shown below. Frame your reply around the full count ({total_phrase}) and span — do not imply the user's history is limited to the activities shown."
            )
        } else {
            format!("This window holds {total_phrase} activities{span_phrase}.")
        };
        if let Some(reason) = reason {
            match reason.explanation() {
                Some(why) => {
                    note.push_str(" This window is INCOMPLETE: ");
                    note.push_str(why);
                    note.push('.');
                }
                None => note.push_str(" This window is INCOMPLETE."),
            }
            note.push_str(" Do not state a count, a total or \"nothing before\" for the period asked as fact: say which dates are covered and that the rest is not loaded.");
        }
        if self.window.totals.is_some() {
            note.push_str(
                " For totals or a comparison between periods, use window.totals, which covers \
                 the whole window.",
            );
        }
        Some(note)
    }

    /// A segments window's note ([`WindowScope::SegmentsRead`]): how many of
    /// the window's activities had their segment efforts read, and, when the
    /// window is incomplete, why and what may not be claimed from it. It
    /// counts the activities read, never the efforts listed: those are every
    /// effort of the read activities, and there is no slice of them to
    /// frame.
    fn segments_read_note(&self) -> Option<String> {
        let total = self.window.total?;
        let read = self.window.returned;
        let span_phrase = match (self.window.asked_from, self.window.asked_to) {
            (Some(from), Some(to)) => format!(" from {from} to {to}"),
            _ => String::new(),
        };
        let mut note = format!(
            "{read} of {total} activities in this window{span_phrase} have their segment efforts read."
        );
        let Some(reason) = self.completeness.reason() else {
            return Some(note);
        };
        note.push_str(" This window is INCOMPLETE");
        if let Some(why) = reason.explanation() {
            note.push_str(": ");
            note.push_str(why);
        }
        note.push('.');
        if let IncompleteReason::SegmentDetailPartial { unreadable } = reason {
            let _ = write!(
                note,
                " {unreadable} of its activities could not have their segment efforts read."
            );
        }
        note.push_str(" Do not state an effort count, a best time or that the athlete never rode a segment in the period as fact: say how many activities were read and that the rest were not.");
        Some(note)
    }
}

/// [`Coverage`] as it travels: completeness flattened to the `complete` and
/// `incomplete_reason` keys the turn's readers key on, and the note a model
/// reads beside the structure.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Coverage")]
struct CoverageWire {
    /// Whether every activity of the asked window was read.
    complete: bool,
    /// Why the window is incomplete; absent when it is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    incomplete_reason: Option<IncompleteReason>,
    /// What the window spans.
    window: CoverageWindow,
    /// The first day the stored history stands behind, `YYYY-MM-DD`, when it
    /// starts later than the window asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trustworthy_from: Option<NaiveDate>,
    /// The history walk still owed for the window, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Pending>,
    /// What may and may not be claimed from the window; absent when nothing
    /// qualifies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl Serialize for Coverage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CoverageWire {
            complete: self.completeness.is_complete(),
            incomplete_reason: self.completeness.reason(),
            window: self.window.clone(),
            trustworthy_from: self.trustworthy_from,
            pending: self.pending,
            note: self.note(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Coverage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let wire = CoverageWire::deserialize(deserializer)?;
        let completeness = match (wire.complete, wire.incomplete_reason) {
            (true, None) => Completeness::Complete,
            (false, Some(reason)) => Completeness::Incomplete(reason),
            (true, Some(_)) => {
                return Err(D::Error::custom(
                    "a complete coverage names no incomplete_reason",
                ))
            }
            (false, None) => {
                return Err(D::Error::custom(
                    "an incomplete coverage names its incomplete_reason",
                ))
            }
        };
        // The note is derived, never stored: it is rebuilt from the rest.
        Ok(Self {
            completeness,
            window: wire.window,
            trustworthy_from: wire.trustworthy_from,
            pending: wire.pending,
        })
    }
}

impl JsonSchema for Coverage {
    fn schema_name() -> Cow<'static, str> {
        CoverageWire::schema_name()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        CoverageWire::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use serde_json::json;

    type TestResult = Result<(), Box<dyn Error>>;

    fn day(y: i32, m: u32, d: u32) -> Result<NaiveDate, &'static str> {
        NaiveDate::from_ymd_opt(y, m, d).ok_or("not a day")
    }

    fn history(from: NaiveDate, to: NaiveDate) -> CoverageWindow {
        CoverageWindow {
            asked_from: Some(from),
            asked_to: Some(to),
            ..CoverageWindow::default()
        }
    }

    #[test]
    fn a_running_walk_serializes_snake_case_with_its_eta() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::ShallowHistory),
            window: history(day(2026, 1, 1)?, day(2026, 11, 12)?),
            trustworthy_from: Some(day(2026, 6, 1)?),
            pending: Some(Pending::running(12, Some(48), Some(10.0))),
        };
        let wire = serde_json::to_value(&coverage)?;
        assert_eq!(
            wire["complete"], false,
            "bug: a shallow window is incomplete"
        );
        assert_eq!(
            wire["incomplete_reason"], "shallow_history",
            "bug: the reason travels snake_case"
        );
        assert_eq!(
            wire["trustworthy_from"], "2026-06-01",
            "bug: the vouched day travels as YYYY-MM-DD"
        );
        assert_eq!(
            wire["window"]["scope"],
            json!({ "kind": "asked" }),
            "bug: an asked window's scope is tagged `asked`"
        );
        assert_eq!(
            wire["window"]["asked_from"], "2026-01-01",
            "bug: the window names the first day asked"
        );
        assert_eq!(
            wire["pending"],
            json!({
                "kind": "history_walk_running",
                "months_done": 12,
                "months_total": 48,
                "eta_secs": 360,
            }),
            "bug: 36 months left at 10 s a month"
        );
        assert!(
            wire["note"]
                .as_str()
                .is_some_and(|note| note.contains("being fetched")),
            "bug: a pending walk is named in the note: {wire}"
        );
        let back: Coverage = serde_json::from_value(wire)?;
        assert_eq!(back, coverage, "bug: the wire shape reads back as written");
        Ok(())
    }

    #[test]
    fn a_walk_not_yet_walked_has_no_progress_fields() -> TestResult {
        assert_eq!(
            serde_json::to_value(Pending::HistoryNotWalked)?,
            json!({ "kind": "history_not_walked" }),
            "bug: a walk not yet run carries only its kind"
        );
        assert_eq!(
            Pending::running(3, None, Some(5.0)),
            Pending::HistoryWalkRunning {
                months_done: 3,
                months_total: None,
                eta_secs: None,
            },
            "bug: no floor, no ETA"
        );
        assert_eq!(
            Pending::running(40, Some(36), Some(5.0)),
            Pending::HistoryWalkRunning {
                months_done: 40,
                months_total: Some(36),
                eta_secs: Some(0),
            },
            "bug: a walk past its total has nothing left"
        );
        Ok(())
    }

    #[test]
    fn a_complete_window_omits_every_optional_key() -> TestResult {
        let coverage = Coverage::complete(CoverageWindow {
            scope: WindowScope::Lifetime,
            total: Some(150),
            returned: 150,
            ..CoverageWindow::default()
        });
        let wire = serde_json::to_value(&coverage)?;
        assert_eq!(
            wire,
            json!({
                "complete": true,
                "window": { "scope": { "kind": "lifetime" }, "total": 150, "returned": 150 },
            }),
            "bug: lifetime totals carry no note"
        );
        assert_eq!(
            serde_json::to_value(WindowScope::LastDays { days: 90 })?,
            json!({ "kind": "last_days", "days": 90 }),
            "bug: a last-days scope is tagged and names its days"
        );
        Ok(())
    }

    #[test]
    fn a_contradictory_completeness_is_refused() {
        let wire = json!({
            "complete": false,
            "window": { "returned": 0 },
        });
        assert!(
            serde_json::from_value::<Coverage>(wire).is_err(),
            "bug: an incomplete coverage without its reason is refused"
        );
    }

    #[test]
    fn nothing_stored_names_the_window_and_forbids_an_estimate() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::NoStoredActivities),
            window: history(day(2026, 8, 1)?, day(2026, 11, 1)?),
            trustworthy_from: None,
            pending: None,
        };
        let note = coverage.note().ok_or("no note")?;
        assert!(
            note.contains("2026-08-01 to 2026-11-01"),
            "bug: the note names the window: {note}"
        );
        assert!(
            note.contains("Do not estimate"),
            "bug: the note forbids an estimate: {note}"
        );
        assert!(
            !note.contains("being fetched"),
            "bug: nothing pending is named: {note}"
        );
        Ok(())
    }

    #[test]
    fn a_shallow_history_cites_only_the_vouched_days() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::ShallowHistory),
            window: history(day(2026, 8, 1)?, day(2026, 11, 1)?),
            trustworthy_from: Some(day(2026, 9, 15)?),
            pending: Some(Pending::HistoryNotWalked),
        };
        let note = coverage.note().ok_or("no note")?;
        assert!(
            note.contains("compute from 2026-09-15 onward"),
            "bug: the note cites only the vouched days: {note}"
        );
        assert!(
            note.contains("between 2026-08-01 and 2026-09-15"),
            "bug: the note names the days with no rows: {note}"
        );
        assert!(
            note.contains("being fetched"),
            "bug: the pending walk is named: {note}"
        );
        Ok(())
    }

    #[test]
    fn a_truncated_window_frames_the_full_count_and_its_bound() -> TestResult {
        let mut window = CoverageWindow {
            oldest: Some(day(2024, 1, 15)?),
            newest: Some(day(2024, 12, 28)?),
            total: Some(552),
            returned: 200,
            ..CoverageWindow::default()
        };
        let note = Coverage::complete(window.clone()).note().ok_or("no note")?;
        assert!(
            note.contains("552 activities spanning 2024-01-15 to 2024-12-28"),
            "bug: the note frames the full count and span: {note}"
        );
        assert!(
            note.contains("only 200 are shown"),
            "bug: the note says how many are shown: {note}"
        );
        assert!(!note.contains("at least"), "bug: an exact count: {note}");

        window.total_is_lower_bound = true;
        let capped = Coverage::complete(window).note().ok_or("no note")?;
        assert!(
            capped.contains("at least 552"),
            "bug: a capped count is a lower bound: {capped}"
        );
        Ok(())
    }

    #[test]
    fn an_incomplete_window_forbids_stating_a_count() -> TestResult {
        let window = CoverageWindow {
            total: Some(43),
            returned: 43,
            ..CoverageWindow::default()
        };
        let live = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::LiveReadRunning),
            window: window.clone(),
            trustworthy_from: None,
            pending: None,
        };
        let note = live.note().ok_or("no note")?;
        assert!(
            note.contains("INCOMPLETE: the live read of the provider is still running"),
            "bug: a running live read is named: {note}"
        );
        assert!(
            note.contains("Do not state a count"),
            "bug: an incomplete window forbids a count: {note}"
        );

        let walking = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
            window,
            trustworthy_from: None,
            pending: Some(Pending::running(36, None, None)),
        };
        let note = walking.note().ok_or("no note")?;
        assert!(
            note.contains(
                "This window is INCOMPLETE: the athlete's history is still being copied in, \
                 so only the activities stored so far are shown; older ones land as the walk \
                 reaches them. Do not state a count"
            ),
            "bug: a running walk is named and forbids a count: {note}"
        );
        Ok(())
    }

    #[test]
    fn totals_point_the_model_at_the_whole_window() -> TestResult {
        let totals = WindowTotals {
            count: 13,
            distance_km: 120.04,
            duration_seconds: 46_800,
            elevation_gain_m: 1_499.96,
        };
        assert_eq!(
            serde_json::to_value(&totals)?,
            json!({
                "count": 13,
                "distance_km": 120.0,
                "duration_seconds": 46_800,
                "elevation_gain_m": 1500.0,
            }),
            "bug: distance and climb to one decimal, duration exact"
        );
        let note = Coverage::complete(CoverageWindow {
            total: Some(13),
            returned: 13,
            totals: Some(totals),
            ..CoverageWindow::default()
        })
        .note()
        .ok_or("no note")?;
        assert!(
            note.contains("use window.totals"),
            "bug: the note points at the whole window's totals: {note}"
        );
        Ok(())
    }

    fn segments(total: u64, read: u64) -> Result<CoverageWindow, &'static str> {
        Ok(CoverageWindow {
            scope: WindowScope::SegmentsRead,
            asked_from: Some(day(2016, 1, 1)?),
            asked_to: Some(day(2026, 11, 13)?),
            total: Some(total),
            returned: read,
            ..CoverageWindow::default()
        })
    }

    #[test]
    fn a_complete_segments_window_counts_the_activities_read() -> TestResult {
        let note = Coverage::complete(segments(12, 12)?)
            .note()
            .ok_or("no note")?;
        assert_eq!(
            note,
            "12 of 12 activities in this window from 2016-01-01 to 2026-11-13 have their segment efforts read.",
            "bug: a complete segments window counts the activities read"
        );
        assert!(
            !note.contains("shown below"),
            "bug: a segments note frames no shown slice: {note}"
        );
        Ok(())
    }

    #[test]
    fn a_walking_segments_window_forbids_a_final_count() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::SegmentDetailWalking),
            window: segments(40, 12)?,
            trustworthy_from: None,
            pending: Some(Pending::detail_walk_running(12, Some(40), Some(1_728.0))),
        };
        let wire = serde_json::to_value(&coverage)?;
        assert_eq!(
            wire["incomplete_reason"], "segment_detail_walking",
            "bug: the reason travels snake_case"
        );
        assert_eq!(
            wire["pending"],
            json!({
                "kind": "detail_walk_running",
                "units_done": 12,
                "units_total": 40,
                "eta_secs": 48_384,
            }),
            "bug: 28 activities left at 1728 s an activity"
        );
        let note = wire["note"].as_str().ok_or("no note")?;
        assert!(
            note.starts_with("12 of 40 activities in this window"),
            "bug: the note counts the activities read: {note}"
        );
        assert!(
            note.contains(
                "INCOMPLETE: the segment efforts of some of its activities are still being read"
            ),
            "bug: the running walk is named: {note}"
        );
        assert!(
            note.contains("Do not state an effort count"),
            "bug: an incomplete segments window forbids a final count: {note}"
        );
        assert!(
            !note.contains("shown below"),
            "bug: a segments note frames no shown slice: {note}"
        );
        let back: Coverage = serde_json::from_value(wire)?;
        assert_eq!(back, coverage, "bug: the wire shape reads back as written");
        Ok(())
    }

    #[test]
    fn a_partial_segments_window_names_what_could_not_be_read() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::SegmentDetailPartial {
                unreadable: 3,
            }),
            window: segments(40, 37)?,
            trustworthy_from: None,
            pending: None,
        };
        let wire = serde_json::to_value(&coverage)?;
        assert_eq!(
            wire["incomplete_reason"],
            json!({ "segment_detail_partial": { "unreadable": 3 } }),
            "bug: the partial reason carries how many could not be read"
        );
        let note = wire["note"].as_str().ok_or("no note")?;
        assert!(
            note.contains("3 of its activities could not have their segment efforts read"),
            "bug: the note names the unreadable count: {note}"
        );
        let back: Coverage = serde_json::from_value(wire)?;
        assert_eq!(back, coverage, "bug: the wire shape reads back as written");
        Ok(())
    }

    #[test]
    fn segments_nobody_can_read_promise_no_connect_step() -> TestResult {
        let coverage = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::SegmentDetailUnavailable),
            window: segments(40, 0)?,
            trustworthy_from: None,
            pending: None,
        };
        let note = coverage.note().ok_or("no note")?;
        assert!(
            note.contains("they come from Strava's API, which this athlete hasn't connected"),
            "bug: the note says where efforts come from: {note}"
        );
        assert!(
            !note.to_lowercase().contains("connect strava"),
            "bug: the note promises no connect step: {note}"
        );
        Ok(())
    }

    #[test]
    fn a_detail_walk_without_a_total_or_a_pace_has_no_eta() {
        assert_eq!(
            Pending::detail_walk_running(5, None, Some(1_728.0)),
            Pending::DetailWalkRunning {
                units_done: 5,
                units_total: None,
                eta_secs: None,
            },
            "bug: no total, no ETA"
        );
        assert_eq!(
            Pending::detail_walk_running(5, Some(9), None),
            Pending::DetailWalkRunning {
                units_done: 5,
                units_total: Some(9),
                eta_secs: None,
            },
            "bug: no pace, no ETA"
        );
    }

    #[test]
    fn a_sample_of_totals_carries_no_note() -> TestResult {
        let coverage = Coverage::complete(CoverageWindow {
            scope: WindowScope::RecentActivities,
            oldest: Some(day(2026, 9, 1)?),
            total: Some(30),
            returned: 30,
            ..CoverageWindow::default()
        });
        assert_eq!(
            coverage.note(),
            None,
            "bug: a provider's sample of totals carries no note"
        );
        Ok(())
    }
}
