// ABOUTME: A coach's day editor in their thread about one athlete — the family catalogue, a built session's preview, the draft-day writes
// ABOUTME: Every built session is the save path's own: instantiate_template_day on the athlete's anchors, projected through the plan card's day

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The coach's family editor (carnet#1515)
//!
//! In a coach's thread about one athlete, each day of the plan card opens an
//! editor that builds the day from a workout family: a family, one of its
//! presets, a sport and the preset's values. The subject-plan routes call
//! the three entry points here, after running the coaching gates:
//!
//! - [`thread_families`] — the families the thread's agent offers, as the
//!   coach's save resolves them ([`PackagedCatalogue`] of the thread's
//!   writer), each preset with the evidence ranges per sport of the values
//!   its shape reads ([`WorkoutTemplate::read_ranges`], carnet#1710): a
//!   value the preset's steps never read is no knob of the coach's;
//! - [`preview_session`] — the session a build makes, generated on the
//!   athlete's stored anchors exactly as the save generates it
//!   ([`instantiate_template_day`]) and projected through the plan card's
//!   own day ([`plan_card::day_card`]), so the chip and warning the preview
//!   shows are the ones the card shows once the day is saved; its findings
//!   and anchor read in the envelope notice's sentences ([`envelope_words`]).
//!   Nothing is written;
//! - [`save_draft_day`] — the build saved as that date of the coach's draft,
//!   opened from the live plan when the coach holds none. The day keeps no
//!   steps of its own: the save path generates them, so the draft holds what
//!   the preview showed.
//!
//! The same editor opens on the day's own session, which the coach edits in
//! place or makes a rest day ([`save_draft_hand_day`]), and puts a day they
//! edited back as it stood ([`restore_draft_day`]) (carnet#1714). Every day
//! save of the coach's keeps the day as it stood before their first edit in
//! the draft, which is what restoring writes back.
//!
//! A date holds several sessions (carnet#1716). The coach adds one
//! ([`add_draft_session`]), which restoring removes, and removes one of
//! several ([`remove_draft_day`]); the last one on a date is made a rest day
//! instead. A session is addressed by its date and position, and every write
//! names it as the client last saw it ([`SessionAt`]): a write whose session
//! moved writes nothing.
//!
//! No number is derived here: the kernel builds the steps, measures them and
//! derives the values; this module chooses which kernel call and words them.
//! A value outside the preset's range is built and measured, never refused
//! (founder decision D3); only the bounds a written session keeps
//! ([`check_template_params`]) refuse a value.
//!
//! What either reads of the athlete — their anchors, their plan — runs under
//! the athlete's own AI consents, never the coach's (carnet#1419).
//!
//! [`envelope_words`]: crate::envelope_words

use std::collections::BTreeMap;
use std::sync::Arc;

use dravr_tronc::dates::CivilDate;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_contremaitre::messaging_strings::{envelope_purpose_key, MessagingStringsRegistry};
use pierre_contremaitre::session_view;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{
    DerivedValues, EnvelopeFinding, FindingCode, Instance, ParamRange, SportFamily, WorkoutFilter,
    WorkoutParams, WorkoutPreset, WorkoutPurpose, WorkoutTemplate,
};
use pierre_core::models::{HeldAnchors, UnitSystem, WorkoutStep};
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanWeekInput, SaveDraftParams, SavedDraftBundle};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use pierre_providers::ai_scope;
use pierre_services::agent_package::{load_agent_package, PackagedCatalogue};
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget};
use pierre_services::plan_calendar_push::plan_sport;
use pierre_services::plan_card::{self, DayEvidence, TemplateValues};
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_services::provider_notice;
use pierre_services::units::resolve_user_units;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use super::plan_scope::PlanWriter;
use super::training_plan_vision::{check_template_params, instantiate_template_day};
use crate::envelope_words::{anchor_sentence, finding_sentence, stated_param};
use crate::runtime::ToolRuntime;

mod day_edits;
mod sessions;

pub use day_edits::{
    add_draft_session, remove_draft_day, restore_draft_day, save_draft_hand_day, AddedSession,
    HandDay,
};
use sessions::{placed, stamp_edit, written_index};
pub use sessions::{SessionAt, SessionExpect};

/// A session as the coach builds it: what the preview measures and the
/// draft-day write saves.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionBuild {
    /// The family's slug.
    pub family: String,
    /// The preset's key; the family's default preset when absent.
    #[serde(default)]
    pub preset: Option<String>,
    /// The sport the session is built for.
    pub sport: SportFamily,
    /// The values the coach stated; each one left out takes the preset's
    /// default.
    #[serde(default)]
    pub params: Option<TemplateValues>,
}

/// Whose thread the editor works in: the coach's draft of the athlete's plan,
/// and the coach writing it with the thread's agent.
#[derive(Debug)]
pub struct EditorThread {
    /// The athlete, under their own tenant, and the coach whose draft it is.
    pub target: CoachDraftTarget,
    /// The coach and the agent the thread answers as: whose package the
    /// families resolve through, and the author the draft records.
    pub writer: PlanWriter,
}

/// The reader's language for the words the editor shows.
#[derive(Debug, Clone, Copy)]
pub struct EditorWords<'words> {
    /// The server strings.
    pub strings: &'words MessagingStringsRegistry,
    /// The coach's locale.
    pub locale: &'words str,
}

/// One workout family the thread's agent offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogueFamily {
    /// The family's slug.
    pub slug: String,
    /// The family's name, as the catalogue writes it.
    pub name: String,
    /// The kernel's purpose word.
    pub purpose: WorkoutPurpose,
    /// The purpose in the reader's words, when the strings have it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose_label: Option<String>,
    /// The sports any of its presets has a form for.
    pub sports: Vec<SportFamily>,
    /// The preset a session takes when none is named.
    pub default_preset: String,
    /// Its presets, in the order the family writes them.
    pub presets: Vec<CataloguePreset>,
}

/// One preset of a family, with what the coach may state on it in each sport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CataloguePreset {
    /// The preset's key.
    pub key: String,
    /// Its name, as the catalogue writes it.
    pub name: String,
    /// One line on what it is for, when the catalogue has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The sports it has a form for.
    pub sports: Vec<SportFamily>,
    /// Per sport it has a form for, the values its shape reads, which are the
    /// ones the coach may state, and their evidence ranges: the sport's form,
    /// else the preset, else the family's axes, key by key, as the kernel
    /// builds and measures the session.
    pub params: BTreeMap<SportFamily, PresetRanges>,
}

/// The evidence ranges of the six values a coach states; a key left out is
/// not one of the preset's values in that sport.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PresetRanges {
    /// Sets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sets: Option<ParamRange>,
    /// Reps per set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reps: Option<ParamRange>,
    /// Work bout, seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_seconds: Option<ParamRange>,
    /// Rest between reps, seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest_seconds: Option<ParamRange>,
    /// Rest between sets, seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_rest_seconds: Option<ParamRange>,
    /// Whole-session duration, minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<ParamRange>,
}

impl PresetRanges {
    /// The six values' ranges out of the ranges the kernel says a preset
    /// reads.
    fn of(ranges: WorkoutParams) -> Self {
        Self {
            sets: ranges.sets,
            reps: ranges.reps,
            work_seconds: ranges.work_seconds,
            rest_seconds: ranges.rest_seconds,
            set_rest_seconds: ranges.set_rest_seconds,
            duration_minutes: ranges.duration_minutes,
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

/// What a draft-day write came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftDayOutcome {
    /// The day is in the coach's draft.
    Saved,
    /// The athlete has no live plan and the coach no draft: there is no week
    /// to write the day into. Nothing was written.
    NoPlan,
    /// The coach holds no draft to restore a day of, and a restore never
    /// opens one. Nothing was written.
    NoDraft,
    /// No week of the plan holds the date. Nothing was written.
    NoPlanWeek,
    /// The day carries no snapshot to put back and is no session the coach
    /// added: the coach never edited it in this draft. Nothing was written.
    NothingToRestore,
    /// The position the write named no longer holds the session the client
    /// saw there (carnet#1716). Nothing was written.
    SessionMoved,
    /// The session is the only entry on its date, which is made a rest day
    /// rather than emptied. Nothing was written.
    LastSession,
    /// The date already holds as many entries as a date holds. Nothing was
    /// written.
    DateFull,
}

/// A build the catalogue cannot make, with the `details.reason` a client
/// branches on.
fn refused(reason: &str, message: String) -> AppError {
    let mut error = AppError::invalid_input(message);
    error.details = Some(Box::new(json!({ "reason": reason })));
    error
}

/// The catalogue the thread's writer resolves families through: their
/// agent's package over the registry, as the coach's save resolves it.
async fn writer_catalogue<'a>(
    runtime: &'a Arc<dyn ToolRuntime>,
    writer: &PlanWriter,
) -> AppResult<PackagedCatalogue<'a>> {
    let package = load_agent_package(
        runtime.repos(),
        writer.tenant,
        writer.user_id,
        writer.agent_id.as_deref(),
    )
    .await?;
    Ok(PackagedCatalogue::new(
        runtime.training_catalogue(),
        package,
    ))
}

/// The workout families the thread's agent offers, in the catalogue's order,
/// each preset with its evidence ranges per sport.
///
/// # Errors
///
/// Returns the repository error from the package read.
pub async fn thread_families(
    runtime: &Arc<dyn ToolRuntime>,
    writer: &PlanWriter,
    words: EditorWords<'_>,
) -> AppResult<Vec<CatalogueFamily>> {
    let catalogue = writer_catalogue(runtime, writer).await?;
    Ok(families(&catalogue, words))
}

/// Every family `catalogue` holds, the package's first.
fn families(catalogue: &PackagedCatalogue<'_>, words: EditorWords<'_>) -> Vec<CatalogueFamily> {
    let templates = catalogue.workouts_matching(&WorkoutFilter::default());
    templates
        .iter()
        .filter_map(|template| catalogue_family(template, words))
        .collect()
}

/// `template` as the editor lists it; `None` for a template with no presets.
fn catalogue_family(template: &WorkoutTemplate, words: EditorWords<'_>) -> Option<CatalogueFamily> {
    let default_preset = template.default_preset()?.key.clone();
    let presets: Vec<CataloguePreset> = template
        .presets
        .iter()
        .map(|preset| catalogue_preset(template, preset))
        .collect();
    let mut sports: Vec<SportFamily> = presets
        .iter()
        .flat_map(|preset| preset.sports.iter().copied())
        .collect();
    sports.sort_unstable();
    sports.dedup();
    let label = words.strings.get(
        &envelope_purpose_key(template.purpose.as_str()),
        words.locale,
    );
    Some(CatalogueFamily {
        slug: template.slug.clone(),
        name: template.name.clone(),
        purpose: template.purpose,
        purpose_label: (!label.is_empty()).then_some(label),
        sports,
        default_preset,
        presets,
    })
}

/// One preset with, in each sport it has a form for, the ranges of the
/// values its shape reads.
fn catalogue_preset(template: &WorkoutTemplate, preset: &WorkoutPreset) -> CataloguePreset {
    let sports: Vec<SportFamily> = preset.families().collect();
    let params = sports
        .iter()
        .map(|sport| {
            let ranges = template.read_ranges(preset, *sport);
            (*sport, PresetRanges::of(ranges))
        })
        .collect();
    CataloguePreset {
        key: preset.key.clone(),
        name: preset.name.clone(),
        summary: preset.summary.clone(),
        sports,
        params,
    }
}

/// A build made into the day the save stores, with the kernel's instance of
/// its session.
#[derive(Debug)]
struct BuiltDay {
    /// The day: steps, duration and envelope as the save stamps them.
    day: PlannedDay,
    /// The preset the session resolved to.
    preset: String,
    /// What the kernel derives from the session.
    derived: Option<DerivedValues>,
}

/// Make `build` into the family day the save stores on `date` at `position`
/// among its entries, its sport written `sport_label`, generated on `anchors`
/// by the save's own path ([`instantiate_template_day`]).
///
/// The day names its family, preset and values and writes no steps, so the
/// save path generates them; its session name and intensity are the
/// catalogue's for that preset and sport ([`session_view`]).
///
/// # Errors
///
/// Invalid input, with its `details.reason`, for a family the catalogue
/// does not offer (`unknown_family`), a preset it does not hold
/// (`unknown_preset`), a sport the preset has no form for
/// (`sport_not_offered`), a value past the bounds a written session keeps
/// (`params_out_of_bounds`), and values whose session leaves those bounds
/// once built (`session_out_of_bounds`).
fn build_day(
    catalogue: &PackagedCatalogue<'_>,
    build: &SessionBuild,
    date: CivilDate,
    position: u32,
    sport_label: String,
    anchors: Option<&HeldAnchors>,
) -> AppResult<BuiltDay> {
    let (template, tier) = catalogue
        .workout(&build.family)
        .filter(|(template, _)| template.is_family())
        .ok_or_else(|| {
            refused(
                "unknown_family",
                format!(
                    "'{}' is not a workout family this thread's agent offers",
                    build.family
                ),
            )
        })?;
    let preset = build
        .preset
        .as_deref()
        .map_or_else(|| template.default_preset(), |key| template.preset(key))
        .ok_or_else(|| {
            refused(
                "unknown_preset",
                format!(
                    "'{}' is not a preset of {}",
                    build.preset.as_deref().unwrap_or_default(),
                    template.slug
                ),
            )
        })?;
    if preset.realization(build.sport).is_none() {
        return Err(refused(
            "sport_not_offered",
            format!("{} has no {} form", preset.name, build.sport),
        ));
    }
    let params = build
        .params
        .map(TemplateValues::template_params)
        .filter(|params| !params.is_empty());
    if let Some(params) = params.as_ref() {
        check_template_params("params", params).map_err(|error| {
            let message = error.message;
            refused("params_out_of_bounds", message)
        })?;
    }
    let mut day = PlannedDay {
        date: date.to_string(),
        sport: sport_label,
        workout: String::new(),
        duration_min: None,
        intensity: String::new(),
        steps: Vec::new(),
        fueling: None,
        template_slug: Some(template.slug.clone()),
        template_params: params,
        template_source: Some(tier.template_source()),
        template_preset: Some(preset.key.clone()),
        envelope: None,
        coach_original: None,
        coach_added: false,
    };
    let Some(instance) = instantiate_template_day(catalogue, &mut day, position, anchors)? else {
        let why = day.envelope.and_then(|envelope| envelope.not_generated);
        return Err(why.map_or_else(
            || {
                AppError::internal(format!(
                    "family {} generated no session for {}",
                    template.slug, preset.key
                ))
            },
            |why| refused("session_out_of_bounds", why),
        ));
    };
    let Instance {
        generated, report, ..
    } = instance;
    let session = session_view(&template, generated);
    day.workout = session.name;
    // The catalogue's intensity for this sport, in the intensity grammar —
    // else its label in the coach's own voice, else its distribution's name.
    day.intensity = session
        .params
        .intensity
        .get(&build.sport.sport())
        .or(session.params.intensity_label.as_ref())
        .cloned()
        .unwrap_or_else(|| session.intensity_distribution.as_str().to_owned());
    Ok(BuiltDay {
        day,
        preset: preset.key.clone(),
        derived: report.derived,
    })
}

/// The preview of a built day: the plan card's own projection of it, the
/// kernel's derived values, and the findings and anchor in the envelope
/// notice's words, the anchor named as a coach's thread names it and a
/// speed written in the coach's `units`.
fn session_preview(
    built: BuiltDay,
    sport: SportFamily,
    words: EditorWords<'_>,
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
        .and_then(|anchor| {
            anchor_sentence(
                words.strings,
                words.locale,
                AnchorReader::Coach,
                units,
                anchor,
            )
        });
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

/// The session `build` makes on `date` for the thread's athlete, generated
/// on their stored anchors as the save generates it, read under their AI
/// consents. Nothing is written.
///
/// # Errors
///
/// The refusals of [`build_day`], and the repository error from the
/// package, anchor or unit reads.
pub async fn preview_session(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    build: &SessionBuild,
    words: EditorWords<'_>,
) -> AppResult<SessionPreview> {
    let repos = runtime.repos();
    let target = &thread.target;
    let preview = async {
        // What follows reads the athlete's own data (carnet#1174).
        ai_scope::note_member_read(target.athlete());
        let catalogue = writer_catalogue(runtime, &thread.writer).await?;
        let anchors = stored_athlete_anchors(repos, target.tenant(), target.athlete()).await?;
        let label = build.sport.as_str().to_owned();
        // A preview is no entry of the week: it is built as the date's first.
        let day = build_day(&catalogue, build, date, 0, label, anchors.as_ref())?;
        let units = resolve_user_units(repos.unit_preferences.as_ref(), target.coach()).await;
        Ok(session_preview(day, build.sport, words, units))
    };
    provider_notice::under_subject_ai_consent(
        repos,
        target.tenant().as_uuid(),
        target.athlete(),
        preview,
    )
    .await
}

/// Save `build` as a session on `date` of the coach's draft of the athlete's plan.
///
/// It goes over the session `at` names, or, with no `at`, over the date's
/// first entry — a new one where the date holds none — as the released app
/// builds write it (carnet#1515).
///
/// The week holding `date` is read from the draft — over the live plan when
/// the coach holds none, which the save then opens it from — and that
/// session replaced by the family day the save path generates on the
/// athlete's anchors; the week's other days stay as they are
/// ([`save_days_into_week`]). The athlete sees nothing until the coach
/// publishes.
///
/// A day whose sport the build's sport family covers keeps its own label (a
/// gravel ride built as a ride stays gravel); any other takes the family's.
///
/// # Errors
///
/// The refusals of [`build_day`], `UnavailableOverTransport` for a plan an
/// external caller may not read, and the repository errors of the reads and
/// the save.
pub async fn save_draft_day(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    at: Option<&SessionAt>,
    build: &SessionBuild,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        ai_scope::note_member_read(target.athlete());
        let week = match week_holding(repos, target, date, WeekFrom::DraftOrLive).await? {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let dated = date.to_string();
        let index = match written_index(&week.days, &dated, at) {
            Ok(index) => index,
            Err(outcome) => return Ok(outcome),
        };
        let catalogue = writer_catalogue(runtime, &thread.writer).await?;
        let anchors = stored_athlete_anchors(repos, target.tenant(), target.athlete()).await?;
        let replaced = index.and_then(|index| week.days.get(index));
        let position = at.map_or(0, |at| at.position);
        let label = sport_label(replaced, build.sport);
        let mut made = build_day(&catalogue, build, date, position, label, anchors.as_ref())?;
        stamp_edit(&mut made.day, replaced);
        let days = placed(&week.days, index, made.day)?;
        let saved = save_days_into_week(repos, thread, &week, &days).await?;
        info!(
            athlete_user_id = %target.athlete(),
            coach_user_id = %target.coach(),
            draft_id = %saved.draft.plan.id,
            opened = saved.opened,
            date = %dated,
            position,
            family = %build.family,
            preset = %made.preset,
            "coach saved a family day into their draft of a coached athlete's plan"
        );
        Ok(DraftDayOutcome::Saved)
    };
    // The draft is stamped from what the write read: the plan's weeks and
    // the anchors the day was generated on (carnet#769).
    let tracked = Box::pin(ai_scope::tracking(ai_scope::Provenance::new(), write));
    provider_notice::under_subject_ai_consent(
        repos,
        target.tenant().as_uuid(),
        target.athlete(),
        tracked,
    )
    .await
}

/// Which plan a day write reads the week it changes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WeekFrom {
    /// The coach's draft, else the live plan the write opens the draft from.
    DraftOrLive,
    /// The coach's draft only: the write never opens one.
    Draft,
}

/// The week holding `date` that a write on that day of the coach's draft
/// changes, read as `from` says — the draft's weeks are the live ones with
/// each week the coach drafted in its place — or the outcome that leaves
/// nothing written: `NoPlan` or `NoDraft` when there is no plan to read it
/// from, `NoPlanWeek` when no week holds the date.
///
/// # Errors
///
/// The errors of [`coach_plan_view`].
async fn week_holding(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    date: CivilDate,
    from: WeekFrom,
) -> AppResult<Result<PlanWeek, DraftDayOutcome>> {
    let view = coach_plan_view(repos, target).await?;
    let weeks = match (view.draft, view.live, from) {
        (Some(draft), _, _) => draft.weeks,
        (None, Some(live), WeekFrom::DraftOrLive) => live.weeks,
        (None, None, WeekFrom::DraftOrLive) => return Ok(Err(DraftDayOutcome::NoPlan)),
        (None, _, WeekFrom::Draft) => return Ok(Err(DraftDayOutcome::NoDraft)),
    };
    Ok(weeks
        .into_iter()
        .find(|week| holds(week, date))
        .ok_or(DraftDayOutcome::NoPlanWeek))
}

/// Save `days` as `week` of the coach's draft, and only that week, its
/// other fields as they stand. The draft opens from the live plan when the
/// coach holds none ([`SaveDraftParams`] with no outline), authored by the
/// thread's agent and stamped from what the write read.
///
/// # Errors
///
/// The repository error of the save.
async fn save_days_into_week(
    repos: &RepositoryRegistry,
    thread: &EditorThread,
    week: &PlanWeek,
    days: &[PlannedDay],
) -> AppResult<SavedDraftBundle> {
    repos
        .training_plans
        .save_draft_bundle(&SaveDraftParams {
            owner: thread.target.owner(),
            author: PlanAuthor::from_agent(thread.writer.agent_id.as_deref()),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: &week.week_start,
                focus: &week.focus,
                days,
                adjustment_reason: &week.adjustment_reason,
                phase_index: week.phase_index,
            }],
            transport_policy: ai_scope::derived_policy(),
        })
        .await
}

/// Whether `week` holds `date`: its first day or one of the six after it.
fn holds(week: &PlanWeek, date: CivilDate) -> bool {
    parse_plan_date(&week.week_start).is_some_and(|start| {
        start <= date && start.checked_add_days(7).is_some_and(|end| date < end)
    })
}

/// The label a built day's sport is written with: the replaced day's own
/// when the build's sport family covers it, else the family's.
fn sport_label(replaced: Option<&PlannedDay>, sport: SportFamily) -> String {
    replaced
        .filter(|day| SportFamily::of(&plan_sport(&day.sport)) == Some(sport))
        .map_or_else(|| sport.as_str().to_owned(), |day| day.sport.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implementations::tests::{
        catalogue_with_family, catalogue_with_shapes, FAMILY_SLUG, SHAPES_SLUG,
    };
    use dravr_tronc::dates::Timestamp;
    use pierre_contremaitre::TrainingCatalogueRegistry;
    use pierre_memory::training_plans::WeekStatus;
    use serde_json::Value;

    fn date(text: &str) -> CivilDate {
        parse_plan_date(text).unwrap()
    }

    fn build(value: Value) -> SessionBuild {
        serde_json::from_value(value).unwrap()
    }

    fn built(registry: &TrainingCatalogueRegistry, value: Value) -> AppResult<BuiltDay> {
        let catalogue = PackagedCatalogue::catalogue_only(registry);
        build_day(
            &catalogue,
            &build(value),
            date("2026-11-18"),
            0,
            "ride".to_owned(),
            None,
        )
    }

    fn reason(error: &AppError) -> &str {
        error
            .details
            .as_ref()
            .and_then(|details| details["reason"].as_str())
            .unwrap_or_default()
    }

    #[test]
    fn the_catalogue_lists_each_preset_with_its_ranges_per_sport() {
        let registry = catalogue_with_family();
        let catalogue = PackagedCatalogue::catalogue_only(&registry);
        let strings = MessagingStringsRegistry::new();
        let words = EditorWords {
            strings: &strings,
            locale: "en",
        };
        let listed = families(&catalogue, words);
        assert!(
            listed.iter().all(|family| !family.presets.is_empty()),
            "bug: a template with no presets is listed as a family"
        );
        let family = listed
            .iter()
            .find(|family| family.slug == FAMILY_SLUG)
            .expect("the fixture family is listed");
        assert_eq!(
            family.default_preset, "fx_30_15",
            "bug: the default preset is the family's first"
        );
        assert_eq!(
            family.sports,
            [SportFamily::Ride, SportFamily::Run],
            "bug: the family lists every sport a preset has a form for"
        );
        assert_eq!(
            family.purpose_label.as_deref(),
            Some("short-interval VO2max work"),
            "bug: the purpose reads in the reader's words"
        );
        let wire = serde_json::to_value(&family.presets[0]).unwrap();
        assert_eq!(
            wire["params"]["ride"]["reps"],
            json!({ "min": 10, "max": 13, "default": 13 }),
            "bug: a key the preset ranges is listed with the preset's own range: {wire}"
        );
        assert_eq!(
            wire["params"]["ride"]["set_rest_seconds"],
            json!({ "min": 120, "max": 180, "default": 180 }),
            "bug: a key the preset leaves open is listed with the family's axis: {wire}"
        );
        assert!(
            wire["params"]["ride"].get("duration_minutes").is_none(),
            "bug: a value no layer ranges is not one of the preset's: {wire}"
        );
        assert!(
            wire["params"].get("swim").is_none(),
            "bug: a sport the preset has no form for carries ranges: {wire}"
        );
    }

    #[test]
    fn a_preset_lists_only_the_values_its_shape_reads() {
        let registry = catalogue_with_shapes();
        let catalogue = PackagedCatalogue::catalogue_only(&registry);
        let strings = MessagingStringsRegistry::new();
        let words = EditorWords {
            strings: &strings,
            locale: "en",
        };
        let listed = families(&catalogue, words);
        let family = listed
            .iter()
            .find(|family| family.slug == SHAPES_SLUG)
            .expect("the fixture family is listed");
        let ride = |key: &str| {
            let preset = family
                .presets
                .iter()
                .find(|preset| preset.key == key)
                .expect("the preset is listed");
            serde_json::to_value(preset).unwrap()["params"]["ride"].clone()
        };
        let steady = ride("steady");
        assert_eq!(
            steady,
            json!({ "duration_minutes": { "min": 20, "max": 240, "default": 60 } }),
            "bug: a continuous session offers its duration alone, not the axes' reps, work and rest: {steady}"
        );
        let openers = ride("openers");
        assert_eq!(
            openers,
            json!({}),
            "bug: literal steps scale with nothing, so they offer no value, not even a ranged duration: {openers}"
        );
    }

    #[test]
    fn a_build_is_generated_as_the_save_generates_it_and_named_from_the_catalogue() {
        let registry = catalogue_with_family();
        let day = built(&registry, json!({ "family": FAMILY_SLUG, "sport": "ride" }))
            .unwrap()
            .day;
        // Warm-up, 3 × (work, float), two set rests, cool-down: 61 minutes.
        assert_eq!(
            day.steps.len(),
            10,
            "bug: the day holds the steps the save path generates: {:?}",
            day.steps
        );
        assert_eq!(
            day.duration_min,
            Some(61),
            "bug: the day takes the generated session's length"
        );
        assert_eq!(
            day.template_preset.as_deref(),
            Some("fx_30_15"),
            "bug: the day records the preset it resolved to"
        );
        assert_eq!(
            day.workout, "VO2max — short intervals — 30/15 (Rønnestad)",
            "bug: the session is named as the catalogue names the preset"
        );
        assert_eq!(
            day.intensity, "100-110% MAP",
            "bug: the intensity is the catalogue's for the sport"
        );
        let envelope = day.envelope.as_ref().expect("the save stamps the envelope");
        assert!(envelope.generated, "bug: the save path generated the steps");
        assert!(
            envelope.findings.is_empty(),
            "bug: a build at the preset's defaults earns no finding: {:?}",
            envelope.findings
        );
    }

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

    #[test]
    fn a_build_the_catalogue_cannot_make_is_refused_with_its_reason() {
        let registry = catalogue_with_family();
        let plain = registry
            .workouts()
            .into_iter()
            .find(|w| !w.is_family())
            .unwrap()
            .slug;
        for (value, expected) in [
            (
                json!({ "family": "no_such_family", "sport": "ride" }),
                "unknown_family",
            ),
            (
                json!({ "family": plain, "sport": "ride" }),
                "unknown_family",
            ),
            (
                json!({ "family": FAMILY_SLUG, "preset": "fx_99", "sport": "ride" }),
                "unknown_preset",
            ),
            (
                json!({ "family": FAMILY_SLUG, "sport": "swim" }),
                "sport_not_offered",
            ),
            (
                json!({ "family": FAMILY_SLUG, "sport": "ride", "params": { "sets": 51 } }),
                "params_out_of_bounds",
            ),
            (
                json!({ "family": FAMILY_SLUG, "sport": "ride",
                        "params": { "work_seconds": 21_600 } }),
                "session_out_of_bounds",
            ),
        ] {
            let refused = built(&registry, value.clone()).unwrap_err();
            assert_eq!(
                reason(&refused),
                expected,
                "bug: a build the catalogue cannot make is refused by its reason: {value}: {refused}"
            );
        }
    }

    #[test]
    fn a_week_holds_its_seven_days_and_a_label_its_sport_family() {
        let week = PlanWeek {
            id: "week".to_owned(),
            tenant_id: "tenant".to_owned(),
            user_id: "user".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: "2026-11-16".to_owned(),
            focus: String::new(),
            phase_index: None,
            days: Vec::new(),
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        };
        assert!(
            holds(&week, date("2026-11-16")),
            "bug: a week holds its first day"
        );
        assert!(
            holds(&week, date("2026-11-22")),
            "bug: a week holds its seventh day"
        );
        assert!(
            !holds(&week, date("2026-11-23")),
            "bug: a week does not hold the next Monday"
        );
        assert!(
            !holds(&week, date("2026-11-15")),
            "bug: a week does not hold the day before it"
        );

        let gravel: PlannedDay = serde_json::from_value(
            json!({ "date": "2026-11-18", "sport": "gravel", "workout": "x" }),
        )
        .unwrap();
        assert_eq!(
            sport_label(Some(&gravel), SportFamily::Ride),
            "gravel",
            "bug: a ride built over a gravel day keeps the gravel label"
        );
        assert_eq!(
            sport_label(Some(&gravel), SportFamily::Run),
            "run",
            "bug: a run built over a gravel day is labelled a run"
        );
        assert_eq!(
            sport_label(None, SportFamily::Swim),
            "swim",
            "bug: a day built where none was takes the family's label"
        );
    }
}
