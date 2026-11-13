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
//!
//! A plan write stores whole weeks, so it also reports every day of them it
//! found nothing on, as a clean day: a day saved again clean in the same turn
//! replaces what an earlier save reported for it, and the notice never warns
//! about a session the agent already fixed.

use chrono::Days;
use pierre_core::models::periodization::{EnvelopeFinding, WorkoutPurpose};
use pierre_core::models::TargetAnchor;
use pierre_memory::training_plans;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::guardian::TurnKey;
use crate::turn_store::{Stamp, TurnStore};
use pierre_llm::FunctionResponse;

/// The plan write that reports the days it saved.
const SAVE_TOOL: &str = "save_training_plan";

/// The coach's publish, which reports the days that went live.
const PUBLISH_TOOL: &str = "publish_training_plan";

/// The one-session push, which reports the session it pushed.
const PRESCRIBE_TOOL: &str = "prescribe_workout";

/// The days a stored plan week holds, from its `week_start`: a save refuses a
/// day outside them.
const DAYS_PER_WEEK: u64 = 7;

/// Which write reported a session, and so which earlier report it replaces.
///
/// A plan day and a prescription on one date are two sessions: a
/// prescription is its own calendar entry, which no plan write touches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NoticeSource {
    /// A day of a week a plan save or publish stored, known by its date.
    #[default]
    PlanDay,
    /// The session a `prescribe_workout` put on the athlete's calendar,
    /// known by its prescription whatever its date.
    Prescription {
        /// The prescription's id.
        id: String,
        /// The prescription it replaced in place, when it did.
        replaces: Option<String>,
    },
}

/// One session a write reported: what the envelope check found on it, or
/// nothing on a session a write stored clean, which shows no line and
/// replaces the earlier reports it supersedes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NoticeDay {
    /// The session's day, `YYYY-MM-DD`.
    pub date: String,
    /// What the check found; every finding warns. Empty on a clean day.
    #[serde(default)]
    pub findings: Vec<EnvelopeFinding>,
    /// The purpose the session trains when an edit moved it to another
    /// family.
    #[serde(default)]
    pub relabelled_purpose: Option<WorkoutPurpose>,
    /// The athlete's anchor the session's targets are written on.
    #[serde(default)]
    pub anchor: Option<TargetAnchor>,
    /// The write that reported it, set from the tool, never read off the
    /// payload: it decides which earlier report this one replaces.
    #[serde(skip)]
    pub source: NoticeSource,
}

impl NoticeDay {
    /// A day a plan write stored with nothing found on it.
    fn clean(date: String) -> Self {
        Self {
            date,
            findings: Vec::new(),
            relabelled_purpose: None,
            anchor: None,
            source: NoticeSource::PlanDay,
        }
    }

    /// Whether this report replaces `held`: a plan day replaces the same
    /// date's plan day; a prescription replaces its own earlier report and the
    /// prescription it replaced in place, whatever their dates.
    fn replaces(&self, held: &Self) -> bool {
        match (&self.source, &held.source) {
            (NoticeSource::PlanDay, NoticeSource::PlanDay) => self.date == held.date,
            (
                NoticeSource::Prescription { id, replaces },
                NoticeSource::Prescription { id: held_id, .. },
            ) => id == held_id || replaces.as_ref() == Some(held_id),
            _ => false,
        }
    }
}

/// The sessions a `tool_name` result `payload` reports; empty for any other
/// tool.
///
/// A plan write reports its days with findings under `envelope.days`, and
/// every other day of the weeks it stored as clean. A day the save could not
/// build carries a reason for the agent and no finding, so it reads as clean:
/// the notice leaves it to the agent. A prescription reports its one session:
/// its `envelope`, `envelope_anchor` and `scheduled_for`, clean when the check
/// found nothing or did not measure it, so a clean replacement clears the
/// prescription it replaced; it stores no plan day, so it clears none. A
/// session that does not read as one is logged and costs only its own
/// notice, never the other days theirs.
#[must_use]
pub fn envelope_in_payload(tool_name: &str, payload: &Value) -> Vec<NoticeDay> {
    match tool_name {
        SAVE_TOOL | PUBLISH_TOOL => plan_write(tool_name, payload),
        PRESCRIBE_TOOL => prescribed_session(payload).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Every day one plan write stored: the days it reported, then the rest of
/// the weeks it wrote as clean. A reported day that did not parse is neither
/// shown nor marked clean, so an earlier report of it stands: a stale warning
/// costs less than a lost one.
fn plan_write(tool_name: &str, payload: &Value) -> Vec<NoticeDay> {
    let reported: &[Value] = payload
        .get("envelope")
        .and_then(|envelope| envelope.get("days"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut days: Vec<NoticeDay> = reported
        .iter()
        .filter_map(|day| parsed(tool_name, day.clone()))
        .collect();
    let reported_dates: Vec<&str> = reported
        .iter()
        .filter_map(|day| day.get("date").and_then(Value::as_str))
        .collect();
    days.extend(
        days_written(tool_name, payload)
            .into_iter()
            .filter(|date| !reported_dates.contains(&date.as_str()))
            .map(NoticeDay::clean),
    );
    days
}

/// The seven days of every week a plan write stored, `YYYY-MM-DD`: a save
/// names its weeks under `weeks[].week_start`, a publish under
/// `weeks_published`. A week whose start does not read as a plan date is
/// logged and its days keep any earlier report.
fn days_written(tool_name: &str, payload: &Value) -> Vec<String> {
    let starts: Vec<&str> = match tool_name {
        SAVE_TOOL => payload
            .get("weeks")
            .and_then(Value::as_array)
            .map(|weeks| {
                weeks
                    .iter()
                    .filter_map(|week| week.get("week_start").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default(),
        PUBLISH_TOOL => payload
            .get("weeks_published")
            .and_then(Value::as_array)
            .map(|weeks| weeks.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    starts
        .into_iter()
        .filter_map(|start| {
            let monday = training_plans::parse_plan_date(start);
            if monday.is_none() {
                warn!(
                    tool_name,
                    "envelope notice: a written week's start is not a plan date; its days keep any earlier notice"
                );
            }
            monday
        })
        .flat_map(|monday| {
            (0..DAYS_PER_WEEK).filter_map(move |offset| monday.checked_add_days(Days::new(offset)))
        })
        .map(|day| day.to_string())
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

/// The one session a prescription reports, known by its `prescription_id`;
/// `None` when the result names no prescription or its findings do not read,
/// so an earlier report of it stands.
fn prescribed_session(payload: &Value) -> Option<NoticeDay> {
    let id = payload.get("prescription_id")?.as_str()?.to_owned();
    let envelope = payload.get("envelope");
    let findings = match envelope.and_then(|envelope| envelope.get("findings")) {
        Some(findings) => parsed(PRESCRIBE_TOOL, findings.clone())?,
        None => Vec::new(),
    };
    Some(NoticeDay {
        date: payload.get("scheduled_for")?.as_str()?.to_owned(),
        findings,
        relabelled_purpose: envelope
            .and_then(|envelope| envelope.get("relabelled_purpose"))
            .and_then(|purpose| serde_json::from_value(purpose.clone()).ok()),
        anchor: payload
            .get("envelope_anchor")
            .and_then(|anchor| serde_json::from_value(anchor.clone()).ok()),
        source: NoticeSource::Prescription {
            id,
            replaces: payload
                .get("replaced_prescription_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    })
}

/// The sessions a round of tool responses reports, in the order they ran.
#[must_use]
pub fn envelope_in_responses(responses: &[FunctionResponse]) -> Vec<NoticeDay> {
    responses
        .iter()
        .flat_map(|resp| envelope_in_payload(&resp.name, &resp.response))
        .collect()
}

/// Fold `later` sessions into `days`, each replacing the earlier reports it
/// supersedes.
///
/// A second save of the same plan day is what was stored, a clean day
/// included, which clears it; a prescription replaces its own earlier report
/// and the one it replaced in place. A plan day and a prescription on one
/// date are two sessions, so neither replaces the other.
pub fn merge_notice_days(days: &mut Vec<NoticeDay>, later: Vec<NoticeDay>) {
    for day in later {
        days.retain(|held| !day.replaces(held));
        days.push(day);
    }
}

/// Process-wide store of the sessions one headless turn's writes reported.
///
/// Keyed by the turn as the reconnect offer is (carnet#1404): one athlete can
/// run overlapping turns, and a one-shot MCP call records nothing.
#[derive(Debug, Default)]
pub struct EnvelopeNotices {
    /// A turn takes its notice when it returns, so the store only grows when
    /// turns die mid-flight. A notice the store's overflow drops costs that
    /// turn its notice, never an answer.
    notices: TurnStore<Vec<NoticeDay>>,
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
        if days.is_empty() {
            return;
        }
        self.notices.record(key, Stamp::Latest, |held| {
            let mut held = held.unwrap_or_default();
            merge_notice_days(&mut held, days);
            held
        });
    }

    /// Consume and return what `key`'s turn recorded.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> Vec<NoticeDay> {
        self.notices.take(key).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::implementations::endurance_workouts_output::PrescribeWorkoutResult;
    use crate::implementations::training_plans_output::{
        PlanEnvelopeReport, PublishStatus, PublishTrainingPlanResult, SavedWeek,
    };
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
            source: NoticeSource::PlanDay,
        }
    }

    /// Prescription `id` put on `date`, replacing `replaces` when given, its
    /// envelope carrying `findings`.
    fn prescription(id: &str, replaces: Option<&str>, date: &str, findings: &Value) -> Value {
        json!({
            "prescription_id": id,
            "replaced_prescription_id": replaces,
            "scheduled_for": date,
            "envelope": { "family": "vo2max_short", "sport": "ride", "purpose": "vo2max_short",
                          "anchor_basis": "", "findings": findings }
        })
    }

    /// Prescription `id` on 2026-07-14, replacing nothing.
    fn prescription_on_the_14th(id: &str, findings: &Value) -> Value {
        prescription(id, None, "2026-07-14", findings)
    }

    /// The days of `days` that would show a notice line.
    fn warned(days: &[NoticeDay]) -> Vec<&str> {
        days.iter()
            .filter(|day| !day.findings.is_empty())
            .map(|day| day.date.as_str())
            .collect()
    }

    /// A save of the week of Monday 2026-07-13, reporting `days`.
    fn save_of_week(days: &Value) -> Value {
        json!({ "envelope": { "days": days },
                "weeks": [{ "week_start": "2026-07-13", "week_id": "w-1", "superseded": false }] })
    }

    #[test]
    fn a_plan_write_reports_its_days_and_a_day_it_could_not_build_reads_clean() {
        let payload = json!({ "envelope": { "days": [
            { "date": "2026-07-14", "template_slug": "vo2max_short", "relabelled_purpose": "threshold",
              "findings": [finding("cycle_mean_floor")] },
            { "date": "2026-07-15", "template_slug": "endurance", "findings": [],
              "not_generated": "the steps left a session's bounds" }
        ]}});
        let days = envelope_in_payload(SAVE_TOOL, &payload);
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: only the day with a finding warns; a day the save could not build is the agent's"
        );
        assert_eq!(
            days.len(),
            2,
            "bug: the day with no finding still rides, as clean, so it replaces an earlier report"
        );
        assert_eq!(
            days[0].relabelled_purpose,
            Some(WorkoutPurpose::Threshold),
            "bug: the relabel rides with the day"
        );
        assert_eq!(
            warned(&envelope_in_payload(PUBLISH_TOOL, &payload)),
            vec!["2026-07-14"],
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
            "prescription_id": "p-20",
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
        let unmeasured = envelope_in_payload(
            PRESCRIBE_TOOL,
            &json!({ "prescription_id": "p-21", "scheduled_for": "2026-07-20" }),
        );
        assert_eq!(
            unmeasured.len(),
            1,
            "bug: a prescription the check did not measure still rides, so it can clear the one it replaced"
        );
        assert_eq!(
            warned(&unmeasured),
            Vec::<&str>::new(),
            "bug: a prescription with no envelope warns about nothing"
        );
        assert_eq!(
            envelope_in_payload(PRESCRIBE_TOOL, &json!({ "scheduled_for": "2026-07-20" })),
            Vec::<NoticeDay>::new(),
            "bug: a result that names no prescription reports no session"
        );
    }

    /// The reader takes what the tools actually serialize, not a hand-written
    /// copy of it: a stored day through `PlanEnvelopeReport`, and a
    /// `PrescribeWorkoutResult` and the one that replaced it, each read back as
    /// the session it reports.
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
        let week = SavedWeek {
            week_start: "2026-07-13".to_owned(),
            week_id: "w-1".to_owned(),
            superseded: false,
        };
        let saved = envelope_in_payload(
            SAVE_TOOL,
            &json!({ "envelope": report, "weeks": [serde_json::to_value(&week)?] }),
        );
        assert_eq!(
            warned(&saved),
            vec!["2026-07-14"],
            "bug: the one day with a finding"
        );
        assert_eq!(
            saved.len(),
            7,
            "bug: the save's stored week reads off SavedWeek: its six other days ride clean"
        );
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

        let published = PublishTrainingPlanResult {
            status: PublishStatus::Published,
            athlete: "Ana".to_owned(),
            draft_id: "d-1".to_owned(),
            message: String::new(),
            plan_id: Some("p-1".to_owned()),
            superseded_plan_id: None,
            weeks_published: vec!["2026-07-13".to_owned()],
            athlete_notified: Some(true),
            conflict: None,
            safety: None,
            envelope: PlanEnvelopeReport::from_days(&days),
            calendar_push: None,
            calendar: None,
        };
        let live = envelope_in_payload(PUBLISH_TOOL, &serde_json::to_value(&published)?);
        assert_eq!(
            warned(&live),
            vec!["2026-07-14"],
            "bug: the one day with a finding the publish put live"
        );
        assert_eq!(
            live.len(),
            7,
            "bug: the publish's live week reads off weeks_published: its six other days ride clean"
        );

        let first = Uuid::new_v4();
        let mut prescribed = PrescribeWorkoutResult {
            prescription_id: first,
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
        let mut pushed = envelope_in_payload(PRESCRIBE_TOOL, &serde_json::to_value(&prescribed)?);
        assert_eq!(pushed.len(), 1, "bug: the session the prescription pushed");
        assert_eq!(
            pushed[0].source,
            NoticeSource::Prescription {
                id: first.to_string(),
                replaces: None
            },
            "bug: the session is known by the prescription_id the tool wrote"
        );
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

        prescribed.replaced_prescription_id = Some(first);
        prescribed.prescription_id = Uuid::new_v4();
        prescribed.scheduled_for = "2026-07-21".to_owned();
        prescribed.envelope = None;
        prescribed.envelope_anchor = None;
        merge_notice_days(
            &mut pushed,
            envelope_in_payload(PRESCRIBE_TOOL, &serde_json::to_value(&prescribed)?),
        );
        assert_eq!(
            warned(&pushed),
            Vec::<&str>::new(),
            "bug: the replacement reads replaced_prescription_id off the result and clears the warning it replaced"
        );
        assert_eq!(
            pushed.len(),
            1,
            "bug: the replacement rides clean in the replaced prescription's place"
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
    fn a_week_saved_again_clean_clears_its_earlier_warning() {
        let mut days = envelope_in_payload(
            SAVE_TOOL,
            &save_of_week(&json!([
                { "date": "2026-07-14", "findings": [finding("ratio_ceiling")] }
            ])),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: the first save warns on its day"
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(SAVE_TOOL, &save_of_week(&json!([]))),
        );
        assert_eq!(
            warned(&days),
            Vec::<&str>::new(),
            "bug: a day the agent fixed in the same turn must not keep warning"
        );
    }

    #[test]
    fn a_publish_clears_the_weeks_it_put_live() {
        let mut days = vec![day("ratio_ceiling")];
        merge_notice_days(
            &mut days,
            envelope_in_payload(PUBLISH_TOOL, &json!({ "weeks_published": ["2026-07-13"] })),
        );
        assert_eq!(
            warned(&days),
            Vec::<&str>::new(),
            "bug: a publish that put the week live clean replaces the draft's warning"
        );
    }

    #[test]
    fn a_clean_prescription_clears_only_the_one_it_replaced() {
        let mut days = vec![day("ratio_ceiling")];
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                PRESCRIBE_TOOL,
                &prescription_on_the_14th("p-1", &json!([finding("cycle_mean_floor")])),
            ),
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(PRESCRIBE_TOOL, &prescription_on_the_14th("p-2", &json!([]))),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14", "2026-07-14"],
            "bug: a clean prescription clears neither the plan day's warning nor another prescription's"
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                PRESCRIBE_TOOL,
                &prescription("p-3", Some("p-1"), "2026-07-16", &json!([])),
            ),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: a clean replacement clears the prescription it replaced, even moved to another day"
        );
    }

    #[test]
    fn a_replacement_whose_findings_do_not_read_keeps_the_earlier_warning() {
        let mut days = envelope_in_payload(
            PRESCRIBE_TOOL,
            &prescription_on_the_14th("p-1", &json!([finding("ratio_ceiling")])),
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                PRESCRIBE_TOOL,
                &prescription("p-2", Some("p-1"), "2026-07-14", &json!("not a list")),
            ),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: a replacement whose findings do not read is not clean; the earlier warning stands"
        );
    }

    #[test]
    fn two_prescriptions_on_one_day_keep_both_warnings() {
        let mut days = envelope_in_payload(
            PRESCRIBE_TOOL,
            &prescription_on_the_14th("p-1", &json!([finding("ratio_ceiling")])),
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                PRESCRIBE_TOOL,
                &prescription_on_the_14th("p-2", &json!([finding("cycle_mean_floor")])),
            ),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14", "2026-07-14"],
            "bug: two prescriptions are two calendar entries; the second hides nothing of the first"
        );
    }

    #[test]
    fn a_plan_write_never_clears_a_prescriptions_warning() {
        let mut days = envelope_in_payload(
            PRESCRIBE_TOOL,
            &prescription_on_the_14th("p-1", &json!([finding("ratio_ceiling")])),
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(SAVE_TOOL, &save_of_week(&json!([]))),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: a prescription is its own calendar entry; a plan week stored clean leaves its warning"
        );
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                SAVE_TOOL,
                &save_of_week(&json!([
                    { "date": "2026-07-14", "findings": [finding("cycle_mean_floor")] }
                ])),
            ),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14", "2026-07-14"],
            "bug: a plan day and a prescription on one date are two sessions, each with its warning"
        );
    }

    #[test]
    fn a_week_start_that_is_not_a_plan_date_marks_nothing_clean() {
        let payload = json!({ "weeks": [{ "week_start": "2026-7-13" }] });
        assert_eq!(
            envelope_in_payload(SAVE_TOOL, &payload),
            Vec::<NoticeDay>::new(),
            "bug: a week whose start does not read must not clear days it may not hold"
        );
    }

    #[test]
    fn a_malformed_reported_day_keeps_its_earlier_warning() {
        let mut days = vec![day("ratio_ceiling")];
        merge_notice_days(
            &mut days,
            envelope_in_payload(
                SAVE_TOOL,
                &save_of_week(&json!([{ "date": "2026-07-14", "findings": "not a list" }])),
            ),
        );
        assert_eq!(
            warned(&days),
            vec!["2026-07-14"],
            "bug: a day this save reported but did not parse is not clean; the earlier warning stands"
        );
    }

    #[test]
    fn the_store_clears_a_day_a_later_write_stored_clean() {
        let store = EnvelopeNotices::new();
        let turn = TurnKey::new(None, "turn-fixed");
        store.record(
            &turn,
            envelope_in_payload(
                SAVE_TOOL,
                &save_of_week(&json!([
                    { "date": "2026-07-14", "findings": [finding("ratio_ceiling")] }
                ])),
            ),
        );
        store.record(
            &turn,
            envelope_in_payload(SAVE_TOOL, &save_of_week(&json!([]))),
        );
        assert_eq!(
            warned(&store.take(&turn)),
            Vec::<&str>::new(),
            "bug: the headless loop clears a fixed day as the API loop does"
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
}
