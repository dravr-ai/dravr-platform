// ABOUTME: Who the day editor works for — a coach in their thread, or the athlete on their own plan — and a built session's preview
// ABOUTME: The preview is the save path's own build, worded for the reader: a coach reads the athlete's anchors, the athlete their own

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;
use std::sync::Arc;

use dravr_tronc::dates::CivilDate;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_core::errors::AppResult;
use pierre_core::models::periodization::{
    DerivedValues, EnvelopeFinding, FindingCode, SportFamily,
};
use pierre_core::models::{TenantId, UnitSystem, WorkoutStep};
use pierre_database::RepositoryRegistry;
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_services::plan_card::{self, DayEvidence};
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_services::provider_notice;
use pierre_services::units::resolve_user_units;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use super::{
    build_day, day_edits, writer_catalogue, BuiltDay, EditorThread, EditorWords, SessionBuild,
};
use crate::envelope_words::{anchor_sentence, finding_sentence, stated_param};
use crate::implementations::plan_scope::PlanWriter;
use crate::runtime::ToolRuntime;

/// The athlete on their own plan (carnet#1841): no coach, no group, no
/// coaching gate. The families are the catalogue's own, since no agent's
/// turn names a package, and a day the athlete writes names no agent.
#[derive(Debug, Clone, Copy)]
pub struct OwnPlan {
    /// The athlete's tenant, where their plan lives.
    pub tenant: TenantId,
    /// The athlete.
    pub athlete: Uuid,
}

impl OwnPlan {
    /// The athlete's own overlay of their plan, where their agent's unasked
    /// changes wait.
    #[must_use]
    pub fn target(&self) -> CoachDraftTarget {
        CoachDraftTarget::own(self.tenant, self.athlete)
    }

    /// The athlete as their own writer, with no agent.
    #[must_use]
    pub const fn writer(&self) -> PlanWriter {
        PlanWriter {
            agent_id: None,
            tenant: self.tenant,
            user_id: self.athlete,
        }
    }
}

/// Whose plan the day editor works on, and who works it: a coach in their
/// thread about one athlete, or the athlete on their own plan (carnet#1841).
#[derive(Debug)]
pub enum PlanEditor {
    /// A group's human coach, on their draft of a coached athlete's plan.
    Coach(EditorThread),
    /// The athlete, on their own plan.
    Own(OwnPlan),
}

impl PlanEditor {
    /// The athlete whose plan it is, under their tenant.
    fn athlete(&self) -> (TenantId, Uuid) {
        match self {
            Self::Coach(thread) => (thread.target.tenant(), thread.target.athlete()),
            Self::Own(own) => (own.tenant, own.athlete),
        }
    }

    /// Whose package the families resolve through.
    fn writer(&self) -> PlanWriter {
        match self {
            Self::Coach(thread) => thread.writer.clone(),
            Self::Own(own) => own.writer(),
        }
    }

    /// Who reads what the editor words about the athlete's anchors, and whose
    /// units a speed is written in.
    fn reader(&self) -> (AnchorReader, Uuid) {
        match self {
            Self::Coach(thread) => (AnchorReader::Coach, thread.target.coach()),
            Self::Own(own) => (AnchorReader::Athlete, own.athlete),
        }
    }

    /// Run `read`, a read of the athlete's own data: under the athlete's AI
    /// consents, and on a coach's call noted as the coach's read of them
    /// (carnet#1174, carnet#1419).
    async fn reading<F: Future>(&self, repos: &RepositoryRegistry, read: F) -> F::Output {
        match self {
            Self::Coach(thread) => {
                day_edits::note_coach_read(thread);
                let target = &thread.target;
                provider_notice::under_subject_ai_consent(
                    repos,
                    target.tenant().as_uuid(),
                    target.athlete(),
                    read,
                )
                .await
            }
            Self::Own(own) => {
                provider_notice::under_ai_consent(repos, own.tenant.as_uuid(), own.athlete, read)
                    .await
            }
        }
    }
}

/// The session a build makes for one of the athlete's days, measured on
/// their anchors.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionPreview {
    /// The family's slug.
    pub family: String,
    /// The preset the session resolved to.
    pub preset: String,
    /// The sport it was built for.
    pub sport: SportFamily,
    /// Its length in whole minutes.
    pub duration_min: u32,
    /// Its structure, the targets written on the athlete's stored anchors.
    pub steps: Vec<WorkoutStep>,
    /// What the kernel derives from it, when it derives anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived: Option<SessionDerived>,
    /// What the envelope check found, each in the reader's words; empty when
    /// the session sits inside its family's evidence.
    pub findings: Vec<SessionFinding>,
    /// The plan card's "outside the evidence" warning, when a finding fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_evidence: Option<String>,
    /// The plan card's evidence chip, when the evidence is weaker than an
    /// unqualified trial in this sport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<DayEvidence>,
    /// "Counts as <purpose>", when a rule relabelled the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_as: Option<String>,
    /// The anchor sentence, when the targets rest on one the athlete holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// What the kernel derives from a built session, as it derives it.
/// Percentages are of the athlete's aerobic maximum (`aerobic_max`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionDerived {
    /// Sets of the main cycle.
    pub sets: u32,
    /// Reps per set of the main cycle.
    pub reps: u32,
    /// Work bout, seconds.
    pub work_seconds: u32,
    /// Everything else in one cycle, seconds.
    pub rest_seconds: u32,
    /// Work over rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f32>,
    /// Work target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_pct: Option<f32>,
    /// Mean rest target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest_pct: Option<f32>,
    /// Time-weighted mean of one cycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_mean_pct: Option<f32>,
    /// Work minus mean rest, in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amplitude_pct: Option<f32>,
    /// Work time across the main cycle, minutes.
    pub on_time_minutes: f32,
    /// The whole session, minutes.
    pub total_minutes: f32,
    /// What the aerobic maximum is: `MAP` or `vVO2max`.
    pub aerobic_max: String,
}

impl SessionDerived {
    /// The kernel's derived values, as the editor shows them.
    fn of(derived: DerivedValues) -> Self {
        Self {
            sets: derived.sets,
            reps: derived.reps,
            work_seconds: derived.work_seconds,
            rest_seconds: derived.rest_seconds,
            ratio: derived.ratio,
            work_pct: derived.work_pct,
            rest_pct: derived.rest_pct,
            cycle_mean_pct: derived.cycle_mean_pct,
            amplitude_pct: derived.amplitude_pct,
            on_time_minutes: derived.on_time_minutes,
            total_minutes: derived.total_minutes,
            aerobic_max: derived.aerobic_max,
        }
    }
}

/// One envelope finding, worded in the reader's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionFinding {
    /// The kernel's finding code.
    pub code: FindingCode,
    /// The sentence.
    pub text: String,
    /// The stated value the finding is about, when it is one outside its
    /// preset's range: the editor marks that field from it, so the kernel
    /// alone decides what is out of range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<&'static str>,
}

/// The preview of a built day: the plan card's own projection of it, the
/// kernel's derived values, and the findings and anchor in the envelope
/// notice's words, the anchor named as `reader` reads it and a speed written
/// in the reader's `units`.
fn session_preview(
    built: BuiltDay,
    sport: SportFamily,
    words: EditorWords<'_>,
    reader: AnchorReader,
    units: UnitSystem,
) -> SessionPreview {
    // The day carries no fuelling protocol — the editor builds none — so no
    // disclosure has anything to decide on it, and the athlete's medical
    // flag is not read for it.
    let fueling = FuelingDisclosure::from_gate(None);
    let card = plan_card::day_card(&built.day, &fueling, words.strings, words.locale);
    let envelope = built.day.envelope.as_ref();
    let findings = envelope
        .map_or_default(|envelope| envelope.findings.as_slice())
        .iter()
        .map(|finding| SessionFinding {
            code: finding.code,
            text: finding_text(words, finding),
            param: stated_param(finding),
        })
        .collect();
    let anchor = envelope
        .and_then(|envelope| envelope.anchor.as_ref())
        .and_then(|anchor| anchor_sentence(words.strings, words.locale, reader, units, anchor));
    SessionPreview {
        family: built.day.template_slug.clone().unwrap_or_default(),
        preset: built.preset,
        sport,
        duration_min: card.duration_min.unwrap_or_default(),
        steps: card.steps,
        derived: built.derived.map(SessionDerived::of),
        findings,
        outside_evidence: card.outside_evidence,
        evidence: card.evidence,
        counts_as: card.counts_as,
        anchor,
    }
}

/// A finding in the envelope notice's sentence; the kernel's own message
/// when the strings have none for it, since a warning is never dropped (D3).
fn finding_text(words: EditorWords<'_>, finding: &EnvelopeFinding) -> String {
    finding_sentence(words.strings, words.locale, finding).unwrap_or_else(|| {
        warn!(
            code = finding.code.as_str(),
            locale = words.locale,
            "family editor: a finding has no sentence; its kernel message is shown"
        );
        finding.message.clone()
    })
}

/// The session `build` makes on `date` for the editor's athlete, generated
/// on their stored anchors as the save generates it, read under their AI
/// consents, and worded for whoever edits: a coach reads the athlete's
/// anchors in their own units, the athlete their own (carnet#1841). Nothing
/// is written.
///
/// # Errors
///
/// The refusals of [`build_day`], and the repository error from the
/// package, anchor or unit reads.
pub async fn preview_session(
    runtime: &Arc<dyn ToolRuntime>,
    editor: &PlanEditor,
    date: CivilDate,
    build: &SessionBuild,
    words: EditorWords<'_>,
) -> AppResult<SessionPreview> {
    let repos = runtime.repos();
    let (tenant, athlete) = editor.athlete();
    let (reader, units_of) = editor.reader();
    let writer = editor.writer();
    let preview = async {
        let catalogue = writer_catalogue(runtime, &writer).await?;
        let anchors = stored_athlete_anchors(repos, tenant, athlete).await?;
        let label = build.sport.as_str().to_owned();
        // A preview is no entry of the week: it is built as the date's first.
        let day = build_day(&catalogue, build, date, 0, label, anchors.as_ref())?;
        let units = resolve_user_units(repos.unit_preferences.as_ref(), units_of).await;
        Ok(session_preview(day, build.sport, words, reader, units))
    };
    editor.reading(repos, preview).await
}

#[cfg(test)]
mod tests {
    use super::super::tests::built;
    use super::*;
    use crate::implementations::tests::{catalogue_with_family, FAMILY_SLUG};
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use serde_json::json;

    #[test]
    fn a_value_outside_the_preset_is_built_and_worded_as_the_notice_words_it() {
        let registry = catalogue_with_family();
        let day = built(
            &registry,
            json!({ "family": FAMILY_SLUG, "preset": "fx_30_15", "sport": "ride",
                    "params": { "reps": 20 } }),
        )
        .unwrap();
        assert!(
            day.day.steps.iter().any(|step| step.repeat == 20),
            "bug: the value is built as written (D3): {:?}",
            day.day.steps
        );
        let strings = MessagingStringsRegistry::new();
        let preview = session_preview(
            day,
            SportFamily::Ride,
            EditorWords {
                strings: &strings,
                locale: "en",
            },
            AnchorReader::Coach,
            UnitSystem::Metric,
        );
        let finding = preview
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::ParamOutOfRange)
            .expect("the out-of-range value earns a finding");
        assert_eq!(
            finding.text, "20 reps per set is outside this format's tested range (limit 13).",
            "bug: the finding reads as the envelope notice words it"
        );
        assert!(
            preview.outside_evidence.is_some(),
            "bug: the card's warning goes with a finding"
        );
        assert!(
            preview.derived.as_ref().is_some_and(|d| d.reps == 20),
            "bug: the derived values are the kernel's for what was built: {:?}",
            preview.derived
        );
        assert_eq!(
            preview.preset, "fx_30_15",
            "bug: the preview names the preset it resolved to"
        );
        assert_eq!(
            preview.family, FAMILY_SLUG,
            "bug: the preview names the family it built"
        );
    }
}
