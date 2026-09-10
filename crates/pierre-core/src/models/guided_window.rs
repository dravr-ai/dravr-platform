// ABOUTME: The profile-recorded window of an athlete's most recent guided walk — when it started, when it ended
// ABOUTME: A re-run supersedes the answers inside that window and nothing outside it, whichever flow wrote them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where a fixed-list walk remembers its last run.
//!
//! The window lives in the user profile rather than the conversation's flow
//! state because a re-run may well happen in a different conversation — a
//! first calibration over Telegram, the next one on the web — and the flow
//! state is per-conversation and cleared on completion.
//!
//! Both ends matter. The answers of one walk cannot be matched by kind and
//! pillar — calibration and the season walk both land `goal`, `physiology`
//! and `preference` facts in the training pillar — so the window is the only
//! thing that identifies them, and a window open at the far end would sweep
//! up every training answer the athlete gave *after* that walk, including
//! the other flow's. A walk that never completed keeps an open end and is
//! superseded up to the re-run, which is the pre-existing behaviour.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::onboarding::GuidedFlow;

/// Sub-key holding the RFC3339 start of the most recent walk.
const LAST_STARTED_AT: &str = "last_started_at";
/// Sub-key holding the RFC3339 completion of the most recent walk, absent
/// while it is running or if it was abandoned.
const LAST_COMPLETED_AT: &str = "last_completed_at";

/// The recorded window of the athlete's most recent run of `flow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidedWindow {
    /// When the walk started.
    pub started_at: DateTime<Utc>,
    /// When it completed; `None` for a walk still running or abandoned.
    pub completed_at: Option<DateTime<Utc>>,
}

impl GuidedFlow {
    /// The profile-JSON key this flow records its window under. Only the
    /// fixed-list flows keep one: the pillars walk supersedes by pillar and
    /// the intake writes its own record.
    #[must_use]
    pub const fn profile_key(self) -> Option<&'static str> {
        match self {
            Self::Calibration => Some("calibration"),
            Self::Season => Some("season"),
            // The pillars walk supersedes by pillar, the intake writes its
            // own record, and the fortnight probes nothing at all — there is
            // no run of it for a later run to supersede.
            Self::Pillars | Self::Intake | Self::Fortnight => None,
        }
    }
}

fn read_stamp(block: &Value, key: &str) -> Option<DateTime<Utc>> {
    block
        .get(key)?
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

impl GuidedWindow {
    /// The previous run's window, if this athlete has run `flow` before.
    /// A malformed start stamp reads as no window, so it is never passed to
    /// the expiry as a bound.
    #[must_use]
    pub fn last(profile: Option<&Value>, flow: GuidedFlow) -> Option<Self> {
        let block = profile?.get(flow.profile_key()?)?;
        Some(Self {
            started_at: read_stamp(block, LAST_STARTED_AT)?,
            completed_at: read_stamp(block, LAST_COMPLETED_AT),
        })
    }

    /// The profile with a new run of `flow` recorded as started, preserving
    /// every other key.
    ///
    /// `upsert_profile` replaces the whole document, so the existing profile
    /// is merged rather than overwritten — writing a bare `{"season": …}`
    /// would drop the athlete's nutrition and equipment blocks. A previous
    /// completion stamp is dropped with the previous start: the new run is
    /// open until it completes.
    #[must_use]
    pub fn record_start(profile: Option<Value>, flow: GuidedFlow, started_at: &str) -> Value {
        let mut doc = as_object(profile);
        if let (Some(map), Some(key)) = (doc.as_object_mut(), flow.profile_key()) {
            map.insert(key.to_owned(), json!({ LAST_STARTED_AT: started_at }));
        }
        doc
    }

    /// The profile with the current run of `flow` recorded as completed,
    /// preserving the start it was opened with and every other key.
    #[must_use]
    pub fn record_completion(
        profile: Option<Value>,
        flow: GuidedFlow,
        completed_at: &str,
    ) -> Value {
        let mut doc = as_object(profile);
        if let (Some(map), Some(key)) = (doc.as_object_mut(), flow.profile_key()) {
            let block = map.entry(key.to_owned()).or_insert_with(|| json!({}));
            if let Some(b) = block.as_object_mut() {
                b.insert(LAST_COMPLETED_AT.to_owned(), json!(completed_at));
            }
        }
        doc
    }
}

/// The stored document as an object, or an empty one — a profile column
/// holding a bare array or string is not something a walk should panic over.
fn as_object(profile: Option<Value>) -> Value {
    match profile {
        Some(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::GuidedWindow;
    use crate::models::GuidedFlow;
    use serde_json::json;

    const START: &str = "2026-07-28T12:00:00+00:00";
    const END: &str = "2026-07-28T12:20:00+00:00";

    #[test]
    fn a_first_run_has_no_window_to_supersede() {
        assert!(GuidedWindow::last(None, GuidedFlow::Season).is_none());
        assert!(GuidedWindow::last(Some(&json!({})), GuidedFlow::Season).is_none());
        assert!(GuidedWindow::last(
            Some(&json!({ "nutrition": { "carbs": 60 } })),
            GuidedFlow::Calibration
        )
        .is_none());
    }

    /// The window as `(start, end)` RFC3339 strings, for assertions.
    fn stamps(profile: &serde_json::Value, flow: GuidedFlow) -> Option<(String, Option<String>)> {
        GuidedWindow::last(Some(profile), flow).map(|w| {
            (
                w.started_at.to_rfc3339(),
                w.completed_at.map(|d| d.to_rfc3339()),
            )
        })
    }

    #[test]
    fn a_recorded_window_round_trips_and_completion_closes_it() {
        let started = GuidedWindow::record_start(None, GuidedFlow::Season, START);
        assert_eq!(
            stamps(&started, GuidedFlow::Season),
            Some((START.to_owned(), None)),
            "a running walk has an open end"
        );

        let done = GuidedWindow::record_completion(Some(started), GuidedFlow::Season, END);
        assert_eq!(
            stamps(&done, GuidedFlow::Season),
            Some((START.to_owned(), Some(END.to_owned()))),
            "completion keeps the start and closes the end"
        );
    }

    #[test]
    fn the_flows_keep_separate_windows_and_the_rest_of_the_profile() {
        // `upsert_profile` replaces the whole document: nutrition and
        // equipment must survive, and one flow's stamp must not touch the
        // other's.
        let existing = json!({
            "nutrition": { "carbs_per_hour": 60 },
            "calibration": { "last_started_at": START, "last_completed_at": END },
        });
        let merged = GuidedWindow::record_start(Some(existing), GuidedFlow::Season, END);
        assert_eq!(merged["nutrition"]["carbs_per_hour"], 60);
        assert_eq!(
            stamps(&merged, GuidedFlow::Calibration),
            Some((START.to_owned(), Some(END.to_owned()))),
            "the other flow's window is untouched"
        );
        assert_eq!(
            stamps(&merged, GuidedFlow::Season),
            Some((END.to_owned(), None))
        );
    }

    #[test]
    fn a_re_run_reopens_the_window() {
        let first = GuidedWindow::record_start(None, GuidedFlow::Calibration, START);
        let done = GuidedWindow::record_completion(Some(first), GuidedFlow::Calibration, END);
        let second = GuidedWindow::record_start(Some(done), GuidedFlow::Calibration, END);
        assert_eq!(
            stamps(&second, GuidedFlow::Calibration),
            Some((END.to_owned(), None)),
            "the next re-run supersedes the latest run, and a new run is open until it completes"
        );
    }

    #[test]
    fn a_non_object_profile_and_a_malformed_stamp_degrade_safely() {
        let merged =
            GuidedWindow::record_start(Some(json!(["unexpected"])), GuidedFlow::Season, START);
        assert!(GuidedWindow::last(Some(&merged), GuidedFlow::Season).is_some());
        let bad = json!({ "season": { "last_started_at": "not a timestamp" } });
        assert!(GuidedWindow::last(Some(&bad), GuidedFlow::Season).is_none());
        let half = json!({ "season": { "last_started_at": START, "last_completed_at": "nope" } });
        assert_eq!(
            stamps(&half, GuidedFlow::Season),
            Some((START.to_owned(), None)),
            "a malformed end reads as open, the safe direction"
        );
    }

    #[test]
    fn only_the_fixed_list_flows_keep_a_window() {
        assert_eq!(GuidedFlow::Pillars.profile_key(), None);
        assert_eq!(GuidedFlow::Intake.profile_key(), None);
        assert_eq!(GuidedFlow::Calibration.profile_key(), Some("calibration"));
        assert_eq!(GuidedFlow::Season.profile_key(), Some("season"));
    }
}
