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
//! stored history stands behind (`trustworthy_from`), and the history walk
//! still owed for it ([`Pending`]). On the wire it flattens to `complete`,
//! `incomplete_reason`, `window`, `trustworthy_from`, `pending` and the `note`
//! a model reads ([`Coverage::note`]).

use std::borrow::Cow;
use std::time::Duration;

use chrono::NaiveDate;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as DeError;
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
}

impl IncompleteReason {
    /// The sentence a model reads after "This window is INCOMPLETE:", when
    /// the reason has one of its own. `None` for a reason whose note is a
    /// whole sentence of its own ([`Self::ShallowHistory`],
    /// [`Self::NoStoredActivities`]) or whose wording is not authored yet
    /// ([`Self::HistoryWalkRunning`], which the structured `pending` carries).
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
            Self::HistoryWalkRunning | Self::ShallowHistory | Self::NoStoredActivities => None,
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
}

/// Count, distance, duration and climb summed over a whole window, taken
/// before a display limit truncates it.
///
/// A year-against-year question is answered from these, not from the shown
/// slice: with a display limit of 20, a 63-ride 2011 read as its last three
/// December rides (carnet#874). Serialized with distance and elevation to one
/// decimal and the duration exact, which the agent quotes and derives pace
/// from (carnet#1015).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowTotals {
    /// Activities in the window.
    pub count: u64,
    /// Summed distance in kilometres, over the activities that report one.
    #[serde(serialize_with = "one_decimal")]
    pub distance_km: f64,
    /// Summed duration in seconds, exact.
    pub duration_seconds: u64,
    /// Summed elevation gain in metres, over the activities that report one.
    #[serde(serialize_with = "one_decimal")]
    pub elevation_gain_m: f64,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes the field by reference
fn one_decimal<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64((value * 10.0).round() / 10.0)
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

/// What is still owed for a window: the history walk that would fill it.
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
        let eta_secs = months_total.zip(secs_per_month).and_then(|(total, pace)| {
            let left = f64::from(total.saturating_sub(months_done));
            // A pace that is negative, not a number or past any duration is
            // no measurement, and gives no ETA.
            Duration::try_from_secs_f64((left * pace).ceil())
                .ok()
                .map(|eta| eta.as_secs())
        });
        Self::HistoryWalkRunning {
            months_done,
            months_total,
            eta_secs,
        }
    }
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
    /// The history walk still owed for the window, when one is.
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
        let wire = CoverageWire::deserialize(deserializer)?;
        let completeness = match (wire.complete, wire.incomplete_reason) {
            (true, None) => Completeness::Complete,
            (false, Some(reason)) => Completeness::Incomplete(reason),
            (true, Some(_)) => {
                return Err(DeError::custom(
                    "a complete coverage names no incomplete_reason",
                ))
            }
            (false, None) => {
                return Err(DeError::custom(
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
        assert_eq!(wire["complete"], false);
        assert_eq!(wire["incomplete_reason"], "shallow_history");
        assert_eq!(wire["trustworthy_from"], "2026-06-01");
        assert_eq!(wire["window"]["scope"], json!({ "kind": "asked" }));
        assert_eq!(wire["window"]["asked_from"], "2026-01-01");
        assert_eq!(
            wire["pending"],
            json!({
                "kind": "history_walk_running",
                "months_done": 12,
                "months_total": 48,
                "eta_secs": 360,
            }),
            "36 months left at 10 s a month"
        );
        assert!(
            wire["note"]
                .as_str()
                .is_some_and(|note| note.contains("being fetched")),
            "a pending walk is named in the note: {wire}"
        );
        let back: Coverage = serde_json::from_value(wire)?;
        assert_eq!(back, coverage);
        Ok(())
    }

    #[test]
    fn a_walk_not_yet_walked_has_no_progress_fields() -> TestResult {
        assert_eq!(
            serde_json::to_value(Pending::HistoryNotWalked)?,
            json!({ "kind": "history_not_walked" })
        );
        assert_eq!(
            Pending::running(3, None, Some(5.0)),
            Pending::HistoryWalkRunning {
                months_done: 3,
                months_total: None,
                eta_secs: None,
            },
            "no floor, no ETA"
        );
        assert_eq!(
            Pending::running(40, Some(36), Some(5.0)),
            Pending::HistoryWalkRunning {
                months_done: 40,
                months_total: Some(36),
                eta_secs: Some(0),
            },
            "a walk past its total has nothing left"
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
            "lifetime totals carry no note"
        );
        assert_eq!(
            serde_json::to_value(WindowScope::LastDays { days: 90 })?,
            json!({ "kind": "last_days", "days": 90 })
        );
        Ok(())
    }

    #[test]
    fn a_contradictory_completeness_is_refused() {
        let wire = json!({
            "complete": false,
            "window": { "returned": 0 },
        });
        assert!(serde_json::from_value::<Coverage>(wire).is_err());
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
        assert!(note.contains("2026-08-01 to 2026-11-01"), "{note}");
        assert!(note.contains("Do not estimate"), "{note}");
        assert!(!note.contains("being fetched"), "nothing pending: {note}");
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
        assert!(note.contains("compute from 2026-09-15 onward"), "{note}");
        assert!(note.contains("between 2026-08-01 and 2026-09-15"), "{note}");
        assert!(note.contains("being fetched"), "{note}");
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
            "{note}"
        );
        assert!(note.contains("only 200 are shown"), "{note}");
        assert!(!note.contains("at least"), "an exact count: {note}");

        window.total_is_lower_bound = true;
        let capped = Coverage::complete(window).note().ok_or("no note")?;
        assert!(capped.contains("at least 552"), "{capped}");
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
            "{note}"
        );
        assert!(note.contains("Do not state a count"), "{note}");

        let walking = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
            window,
            trustworthy_from: None,
            pending: Some(Pending::running(36, None, None)),
        };
        let note = walking.note().ok_or("no note")?;
        assert!(
            note.contains("This window is INCOMPLETE. Do not state a count"),
            "{note}"
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
            "distance and climb to one decimal, duration exact"
        );
        let note = Coverage::complete(CoverageWindow {
            total: Some(13),
            returned: 13,
            totals: Some(totals),
            ..CoverageWindow::default()
        })
        .note()
        .ok_or("no note")?;
        assert!(note.contains("use window.totals"), "{note}");
        Ok(())
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
        assert_eq!(coverage.note(), None);
        Ok(())
    }
}
