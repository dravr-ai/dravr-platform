// ABOUTME: The envelope findings this turn's plan saves, publishes and prescriptions reported, read off their results
// ABOUTME: One reader for every loop, and a per-turn store for the headless loop, whose payloads only the dispatch sees

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the warn-only envelope check found on this turn's sessions (carnet#1495).
//!
//! `save_training_plan`, `publish_training_plan` and `prescribe_workout` carry
//! the check's findings in their results, and the chat pipeline renders them
//! itself under the reply, from fixed translated sentences, so the warning
//! reaches the athlete whatever the model writes. This module reads them: the
//! API and text loops through [`envelope_in_responses`] over the payloads they
//! hold, the headless loop through [`EnvelopeNotices`], filled at the dispatch
//! chokepoint, since its tools run on the tool-host surface's own executor.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use pierre_core::models::periodization::{EnvelopeFinding, WorkoutPurpose};
use pierre_core::models::TargetAnchor;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::guardian::state::TURN_TTL;
use crate::guardian::TurnKey;
use pierre_llm::FunctionResponse;

/// The plan write that reports the days it saved.
const SAVE_TOOL: &str = "save_training_plan";

/// The coach's publish, which reports the days that went live.
const PUBLISH_TOOL: &str = "publish_training_plan";

/// The one-session push, which reports the session it pushed.
const PRESCRIBE_TOOL: &str = "prescribe_workout";

/// How many turns the store holds. A turn takes its notice when it returns,
/// so the store only grows past this when turns die mid-flight. Overflow drops
/// the entries past [`TURN_TTL`] first, every entry only if that was not
/// enough, and costs those turns their notice, never an answer.
const NOTICE_CAP: usize = 10_000;

/// One session the envelope check found something on.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NoticeDay {
    /// The session's day, `YYYY-MM-DD`.
    pub date: String,
    /// What the check found; every finding warns.
    #[serde(default)]
    pub findings: Vec<EnvelopeFinding>,
    /// The purpose the session trains when an edit moved it to another
    /// family.
    #[serde(default)]
    pub relabelled_purpose: Option<WorkoutPurpose>,
    /// The athlete's anchor the session's targets are written on.
    #[serde(default)]
    pub anchor: Option<TargetAnchor>,
}

/// The sessions with findings a `tool_name` result `payload` reports; empty
/// for any other tool and for a result with nothing found.
///
/// A plan write reports its days under `envelope.days`; a day the save could
/// not build carries a reason for the agent and no finding, and is left out.
/// A prescription reports one session: its `envelope`, `envelope_anchor` and
/// `scheduled_for`. A session that does not read as one is logged and costs
/// only its own notice, never the other days theirs.
#[must_use]
pub fn envelope_in_payload(tool_name: &str, payload: &Value) -> Vec<NoticeDay> {
    let days: Vec<NoticeDay> = match tool_name {
        SAVE_TOOL | PUBLISH_TOOL => payload
            .get("envelope")
            .and_then(|envelope| envelope.get("days"))
            .and_then(Value::as_array)
            .map(|days| {
                days.iter()
                    .filter_map(|day| parsed(tool_name, day.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        PRESCRIBE_TOOL => prescribed_session(payload).into_iter().collect(),
        _ => Vec::new(),
    };
    days.into_iter()
        .filter(|day| !day.findings.is_empty())
        .collect()
}

/// `value` read as a `T`, or `None` with a warning naming the tool whose
/// result did not carry the shape this reader expects.
fn parsed<T: DeserializeOwned>(tool_name: &str, value: Value) -> Option<T> {
    match serde_json::from_value(value) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            warn!(
                tool_name,
                %error,
                "envelope notice: a reported session did not parse; its notice is not shown"
            );
            None
        }
    }
}

/// The one session a prescription reports, when its result carries an
/// envelope.
fn prescribed_session(payload: &Value) -> Option<NoticeDay> {
    let envelope = payload.get("envelope")?;
    Some(NoticeDay {
        date: payload.get("scheduled_for")?.as_str()?.to_owned(),
        findings: parsed(PRESCRIBE_TOOL, envelope.get("findings")?.clone())?,
        relabelled_purpose: envelope
            .get("relabelled_purpose")
            .and_then(|purpose| serde_json::from_value(purpose.clone()).ok()),
        anchor: payload
            .get("envelope_anchor")
            .and_then(|anchor| serde_json::from_value(anchor.clone()).ok()),
    })
}

/// The sessions with findings across a round of tool responses.
#[must_use]
pub fn envelope_in_responses(responses: &[FunctionResponse]) -> Vec<NoticeDay> {
    responses
        .iter()
        .flat_map(|resp| envelope_in_payload(&resp.name, &resp.response))
        .collect()
}

/// Fold `later` sessions into `days`: a day reported again replaces the
/// earlier report, since a second save of the same day is what was stored.
pub fn merge_notice_days(days: &mut Vec<NoticeDay>, later: Vec<NoticeDay>) {
    for day in later {
        days.retain(|held| held.date != day.date);
        days.push(day);
    }
}

/// Process-wide store of the sessions with findings one headless turn wrote.
///
/// Keyed by the turn as the reconnect offer is (carnet#1404): one athlete can
/// run overlapping turns, and a one-shot MCP call records nothing.
#[derive(Debug, Default)]
pub struct EnvelopeNotices {
    notices: Mutex<HashMap<TurnKey, (Instant, Vec<NoticeDay>)>>,
}

impl EnvelopeNotices {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold `days` into what `key`'s turn has recorded. A poisoned lock is
    /// skipped: the turn then carries no notice of its own, never a panic.
    pub fn record(&self, key: &TurnKey, days: Vec<NoticeDay>) {
        self.record_at(key, days, Instant::now());
    }

    /// [`Self::record`] as of `now`.
    fn record_at(&self, key: &TurnKey, days: Vec<NoticeDay>, now: Instant) {
        if days.is_empty() {
            return;
        }
        if let Ok(mut notices) = self.notices.lock() {
            if notices.len() >= NOTICE_CAP && !notices.contains_key(key) {
                notices.retain(|_, (at, _)| now.duration_since(*at) < TURN_TTL);
                if notices.len() >= NOTICE_CAP {
                    notices.clear();
                }
            }
            let (at, held) = notices
                .entry(key.clone())
                .or_insert_with(|| (now, Vec::new()));
            *at = now;
            merge_notice_days(held, days);
        }
    }

    /// Consume and return what `key`'s turn recorded.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> Vec<NoticeDay> {
        self.notices
            .lock()
            .ok()
            .and_then(|mut notices| notices.remove(key))
            .map(|(_, days)| days)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use super::*;
    use crate::implementations::endurance_workouts_output::PrescribeWorkoutResult;
    use crate::implementations::training_plans_output::PlanEnvelopeReport;
    use pierre_memory::training_plans::PlannedDay;
    use serde_json::json;
    use uuid::Uuid;

    type TestResult = Result<(), Box<dyn Error>>;

    fn finding(code: &str) -> Value {
        json!({ "code": code, "action": "warn", "message": "English, for the model." })
    }

    fn day(code: &str) -> NoticeDay {
        NoticeDay {
            date: "2026-07-14".to_owned(),
            findings: serde_json::from_value(json!([finding(code)]))
                .expect("a finding the kernel's own shape reads"),
            relabelled_purpose: None,
            anchor: None,
        }
    }

    #[test]
    fn a_plan_write_reports_only_its_days_with_findings() {
        let payload = json!({ "envelope": { "days": [
            { "date": "2026-07-14", "template_slug": "vo2max_short", "relabelled_purpose": "threshold",
              "findings": [finding("cycle_mean_floor")] },
            { "date": "2026-07-15", "template_slug": "endurance", "findings": [],
              "not_generated": "the steps left a session's bounds" }
        ]}});
        let days = envelope_in_payload(SAVE_TOOL, &payload);
        assert_eq!(
            days.len(),
            1,
            "bug: a day with no finding is the agent's, not a notice"
        );
        assert_eq!(days[0].date, "2026-07-14", "bug: the day with the finding");
        assert_eq!(
            days[0].relabelled_purpose,
            Some(WorkoutPurpose::Threshold),
            "bug: the relabel rides with the day"
        );
        assert_eq!(
            envelope_in_payload(PUBLISH_TOOL, &payload).len(),
            1,
            "bug: a publish reports its days as a save does"
        );
        assert_eq!(
            envelope_in_payload("get_training_plan", &payload),
            Vec::<NoticeDay>::new(),
            "bug: only a tool that writes sessions raises a notice"
        );
    }

    #[test]
    fn a_prescription_reports_its_one_session_with_its_anchor() {
        let payload = json!({
            "scheduled_for": "2026-07-20",
            "envelope": { "family": "vo2max_short", "sport": "ride", "purpose": "vo2max_short",
                          "anchor_basis": "", "findings": [finding("map_without_definition")] },
            "envelope_anchor": { "sport": "ride", "anchor": "map", "value": 380.0, "origin": "athlete",
                                 "confidence": "low", "own_conversions": false }
        });
        let days = envelope_in_payload(PRESCRIBE_TOOL, &payload);
        assert_eq!(days.len(), 1, "bug: a prescription reports one session");
        assert_eq!(days[0].date, "2026-07-20", "bug: the day it was put on");
        assert!(
            days[0].anchor.is_some(),
            "bug: the anchor rides with the session"
        );
        assert_eq!(
            envelope_in_payload(PRESCRIBE_TOOL, &json!({ "scheduled_for": "2026-07-20" })),
            Vec::<NoticeDay>::new(),
            "bug: a prescription with no envelope raises no notice"
        );
    }

    /// The reader takes what the tools actually serialize, not a hand-written
    /// copy of it: a stored day through `PlanEnvelopeReport`, and a
    /// `PrescribeWorkoutResult`, each read back as the session it reports.
    #[test]
    fn the_reader_takes_the_shapes_the_tools_write() -> TestResult {
        let anchor = json!({ "sport": "ride", "anchor": "ftp", "value": 285.0,
                             "origin": "intervals.icu", "confidence": "medium", "own_conversions": false });
        let days: Vec<PlannedDay> = serde_json::from_value(json!([
            { "date": "2026-07-14", "sport": "ride", "workout": "20/40", "template_slug": "vo2max_short",
              "envelope": { "relabelled_purpose": "threshold", "findings": [finding("cycle_mean_floor")],
                            "anchor": anchor } },
            { "date": "2026-07-15", "sport": "ride", "workout": "easy", "template_slug": "endurance",
              "envelope": { "findings": [] } }
        ]))?;
        let report = PlanEnvelopeReport::from_days(&days).ok_or("a report")?;
        let saved = envelope_in_payload(SAVE_TOOL, &json!({ "envelope": report }));
        assert_eq!(saved.len(), 1, "bug: the one day with a finding");
        assert_eq!(saved[0].date, "2026-07-14", "bug: its day");
        assert_eq!(
            saved[0].relabelled_purpose,
            Some(WorkoutPurpose::Threshold),
            "bug: the relabel the save stored"
        );
        assert!(
            saved[0].anchor.is_some(),
            "bug: the anchor the save stored rides with the day"
        );

        let prescribed = PrescribeWorkoutResult {
            prescription_id: Uuid::nil(),
            provider: "intervals_icu".to_owned(),
            provider_event_id: "evt-1".to_owned(),
            replaced_prescription_id: None,
            template_slug: "vo2max_short".to_owned(),
            name: "VO2max short".to_owned(),
            duration_minutes: 60,
            scheduled_for: "2026-07-20".to_owned(),
            status: "pushed".to_owned(),
            calendar_note: None,
            envelope: Some(serde_json::from_value(json!({
                "family": "vo2max_short", "sport": "ride", "purpose": "vo2max_short",
                "anchor_basis": "", "relabelled_purpose": "threshold",
                "findings": [finding("cycle_mean_floor")]
            }))?),
            envelope_anchor: Some(serde_json::from_value(anchor)?),
        };
        let pushed = envelope_in_payload(PRESCRIBE_TOOL, &serde_json::to_value(&prescribed)?);
        assert_eq!(pushed.len(), 1, "bug: the session the prescription pushed");
        assert_eq!(pushed[0].date, "2026-07-20", "bug: the day it was put on");
        assert_eq!(
            pushed[0].relabelled_purpose,
            Some(WorkoutPurpose::Threshold),
            "bug: the relabel the envelope carried"
        );
        assert!(
            pushed[0].anchor.is_some(),
            "bug: the envelope anchor rides with the session"
        );
        Ok(())
    }

    #[test]
    fn a_day_that_does_not_parse_costs_only_its_own_notice() {
        let payload = json!({ "envelope": { "days": [
            { "date": "2026-07-14", "findings": [finding("ratio_ceiling")] },
            { "date": "2026-07-15", "findings": "not a list" }
        ]}});
        let dates: Vec<String> = envelope_in_payload(SAVE_TOOL, &payload)
            .into_iter()
            .map(|day| day.date)
            .collect();
        assert_eq!(
            dates,
            vec!["2026-07-14".to_owned()],
            "bug: one malformed day must not drop the others"
        );
    }

    #[test]
    fn a_day_saved_twice_keeps_its_latest_report() {
        let mut days = vec![day("ratio_ceiling")];
        merge_notice_days(&mut days, vec![day("true_tabata")]);
        assert_eq!(days.len(), 1, "bug: one day, reported once");
        assert_eq!(
            days[0].findings.len(),
            1,
            "bug: the later report replaces the earlier, never adds to it"
        );
        assert_eq!(
            days[0].findings[0].code.as_str(),
            "true_tabata",
            "bug: the later report is the one kept"
        );
    }

    #[test]
    fn the_store_hands_each_turn_its_own_days_once() {
        let store = EnvelopeNotices::new();
        let turn = TurnKey::new(None, "turn-a");
        let other = TurnKey::new(None, "turn-b");
        let days = envelope_in_payload(
            SAVE_TOOL,
            &json!({ "envelope": { "days": [
                { "date": "2026-07-14", "findings": [finding("ratio_ceiling")] }
            ]}}),
        );
        store.record(&turn, days);
        assert_eq!(
            store.take(&other),
            Vec::<NoticeDay>::new(),
            "bug: another turn's notice is not this one's"
        );
        assert_eq!(
            store.take(&turn).len(),
            1,
            "bug: the turn takes what it recorded"
        );
        assert_eq!(
            store.take(&turn),
            Vec::<NoticeDay>::new(),
            "bug: a notice is taken once"
        );
    }

    /// Ages the entries by reading them from a later `now`, because an `Instant`
    /// cannot reach back past a freshly booted machine's clock start.
    #[test]
    fn overflow_drops_abandoned_notices_before_a_live_one() {
        let store = EnvelopeNotices::new();
        let later = Instant::now() + TURN_TTL + Duration::from_secs(1);
        let live = TurnKey::new(None, "turn-live");
        store.record_at(&live, vec![day("ratio_ceiling")], later);
        for turn in 0..NOTICE_CAP {
            store.notices.lock().expect("lock").insert(
                TurnKey::new(None, format!("turn-{turn}")),
                (Instant::now(), vec![day("true_tabata")]),
            );
        }

        let next = TurnKey::new(None, "turn-next");
        store.record_at(&next, vec![day("true_tabata")], later);

        assert_eq!(
            store.notices.lock().expect("lock").len(),
            2,
            "bug: only the abandoned entries went"
        );
        assert_eq!(
            store.take(&live).len(),
            1,
            "bug: a live turn keeps its notice through an overflow"
        );
        assert_eq!(
            store.take(&next).len(),
            1,
            "bug: the turn that overflowed the store keeps its own"
        );
    }
}
