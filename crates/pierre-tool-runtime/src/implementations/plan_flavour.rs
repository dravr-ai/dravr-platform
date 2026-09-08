// ABOUTME: recommend_plan_flavour — the athlete's profile through the catalogue's selection rule, and the season it implies
// ABOUTME: Reads the stored profile and active plan for what they hold; the questionnaire answers arrive as arguments
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `recommend_plan_flavour`
//!
//! The one chat-callable surface over the periodization kernel. The coach
//! calls it after `/season` and `/calibrate` have run, passes what the athlete
//! answered, and receives a ranked verdict — what they can run, what they
//! cannot and why, and how sure the rule is — plus the season laid backward
//! from their goal on the skeleton that fits.
//!
//! Two things are read from storage rather than asked again: the profile,
//! for the devices the athlete already has thresholds for and the training
//! age they stated; and the active plan, for the goal race the season is
//! aimed at. Everything the tool resolved and where it came from is echoed
//! back as `inputs`, so the coach can confirm before saving the outcome
//! through `save_training_plan`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;
use tracing::info;

use super::data_helpers::read_only_annotations;
use super::plan_scope::{resolve_plan_scope, PlanScopeRequest};
use super::training_plan_telemetry::athlete_today;
use super::training_plans::load_conversation;
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response,
};
use crate::implementations::plan_flavour_output::{
    FlavourInputsEcho, InputSource, LabelledExclusion, LabelledFlavour, LabelledVerdict,
    LaidPhaseReport, PlanFlavourResult, SeasonReport, ShareRange,
};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{
    build_skeleton, select_flavour, EventClass, FlavourFamily, FlavourInputs, FlavourVerdict,
    HoursTier, InjuryLoad, IntervalExperience, LaidPhase, Measurement, RecoverySpeed, SeasonLayout,
    SeasonPhase, SkeletonTemplate, SportMix, TrainingAge,
};
use pierre_core::models::{SportFamily, SportType, TenantId, UserPhysiologicalProfile};
use pierre_mcp_schema::PropertySchema;
use pierre_memory::training_plans::parse_plan_date;
use pierre_services::locale::resolve_user_locale;
use pierre_tools_core::ToolResult;

/// Upper bound on weekly hours a payload may claim; above this it is a typo.
const MAX_HOURS_PER_WEEK: f32 = 60.0;
/// Upper bound on weekly sessions.
const MAX_SESSIONS_PER_WEEK: u8 = 21;

/// The tool.
pub struct RecommendPlanFlavourTool;

/// What the coach passes: the questionnaire's answers, each optional where
/// storage can fill it in.
#[derive(Deserialize)]
struct Payload {
    hours_per_week: f32,
    sessions_per_week: u8,
    #[serde(default)]
    training_age: Option<TrainingAge>,
    #[serde(default)]
    event_class: Option<EventClass>,
    #[serde(default)]
    weeks_to_goal: Option<u32>,
    #[serde(default)]
    measurements: Option<Vec<Measurement>>,
    #[serde(default)]
    recovery_speed: Option<RecoverySpeed>,
    #[serde(default)]
    injury_load: Option<InjuryLoad>,
    #[serde(default)]
    interval_experience: Option<IntervalExperience>,
    #[serde(default)]
    sport_mix: Option<SportMix>,
    #[serde(default)]
    season_phase: Option<SeasonPhase>,
    #[serde(default)]
    coach_preference: Option<String>,
    #[serde(default)]
    athlete: Option<String>,
}

/// Where each resolved input came from, so the coach can confirm it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Argument,
    Profile,
    Plan,
    Default,
}

impl Source {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Argument => "argument",
            Self::Profile => "profile",
            Self::Plan => "plan",
            Self::Default => "default",
        }
    }
}

/// The inputs with their provenance — what the plan calls `inputs_snapshot`.
struct Resolved {
    inputs: FlavourInputs,
    sources: Vec<(&'static str, Source)>,
    goal_date: Option<NaiveDate>,
}

/// A string property.
fn string_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// A number property.
fn number_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "number".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// An integer property.
fn integer_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "integer".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// A string property over a closed vocabulary. The words are named in the
/// description rather than as `enum_values`, the convention every tool with
/// a closed set follows; the payload's typed deserialize is the validation.
fn vocab_prop(description: &str, values: &[&str]) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(format!("{description} One of: {}.", values.join(", "))),
        ..Default::default()
    }
}

/// An array of vocabulary strings.
fn vocab_array_prop(description: &str, values: &[&str]) -> PropertySchema {
    PropertySchema {
        property_type: "array".to_owned(),
        description: Some(format!(
            "{description} Entries from: {}.",
            values.join(", ")
        )),
        items: Some(Box::new(PropertySchema {
            property_type: "string".to_owned(),
            description: Some("One entry.".to_owned()),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// The athlete-facing name of a flavour, in one locale.
///
/// Plain words — "mostly easy with two hard days", never "polarized" —
/// from the string catalogue under `messaging.flavour.<id>`, the hyphens
/// of the id folded to underscores. A flavour the catalogue has no words
/// for (a coach package's house flavour) is named by its id, which is at
/// least honest.
struct FlavourLabels<'a> {
    registry: &'a MessagingStringsRegistry,
    locale: &'a str,
}

impl FlavourLabels<'_> {
    fn of(&self, id: &str) -> String {
        let key = format!("messaging.flavour.{}", id.replace('-', "_"));
        let label = self.registry.get(&key, self.locale);
        if label.is_empty() {
            id.to_owned()
        } else {
            label
        }
    }
}

impl RecommendPlanFlavourTool {
    fn properties() -> HashMap<String, PropertySchema> {
        let words = |all: &[&'static str]| all.to_vec();
        let mut p = HashMap::new();
        p.insert(
            "hours_per_week".to_owned(),
            number_prop(
                "Hours the athlete trains in a typical week, from /calibrate Availability.",
            ),
        );
        p.insert(
            "sessions_per_week".to_owned(),
            integer_prop("Sessions in a typical week."),
        );
        p.insert(
            "training_age".to_owned(),
            vocab_prop(
                "Years of structured training as a band. Omit to derive from the stored profile.",
                &words(
                    &TrainingAge::ALL
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>(),
                ),
            ),
        );
        p.insert(
            "event_class".to_owned(),
            vocab_prop(
                "The goal event. Omit to derive from the active plan's goal race.",
                &words(
                    &EventClass::ALL
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>(),
                ),
            ),
        );
        p.insert(
            "weeks_to_goal".to_owned(),
            integer_prop("Weeks until the goal race. Omit to derive from the active plan."),
        );
        p.insert(
            "measurements".to_owned(),
            vocab_array_prop(
                "What the athlete can steer intensity by. Omit to derive from stored thresholds; an athlete with none is read as effort.",
                &words(&Measurement::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>()),
            ),
        );
        p.insert(
            "recovery_speed".to_owned(),
            vocab_prop(
                "From /calibrate RecoverySpeed. Masters-style loading triggers on this, never on age.",
                &words(&RecoverySpeed::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>()),
            ),
        );
        p.insert(
            "injury_load".to_owned(),
            vocab_prop(
                "From /calibrate Injury.",
                &words(
                    &InjuryLoad::ALL
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>(),
                ),
            ),
        );
        p.insert(
            "interval_experience".to_owned(),
            vocab_prop(
                "Structured interval history. Omitted means none — the safe reading for an athlete about whom nothing is known.",
                &words(&IntervalExperience::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>()),
            ),
        );
        p.insert(
            "sport_mix".to_owned(),
            vocab_prop(
                "The sports trained. Omit to derive from the profile's primary sport.",
                &words(&SportMix::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>()),
            ),
        );
        p.insert(
            "season_phase".to_owned(),
            vocab_prop(
                "Where in the season the athlete stands, if known.",
                &words(
                    &SeasonPhase::ALL
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>(),
                ),
            ),
        );
        p.insert(
            "coach_preference".to_owned(),
            string_prop("A flavour id the coach's package pins. Outranks the table when the athlete can run it."),
        );
        p.insert(
            "athlete".to_owned(),
            string_prop(
                "A human coach reading a consenting athlete's profile from their own direct chat.",
            ),
        );
        p
    }

    fn payload(args: &Value) -> AppResult<Payload> {
        let payload: Payload = serde_json::from_value(args.clone())
            .map_err(|e| AppError::invalid_input(format!("recommend_plan_flavour: {e}")))?;
        if !payload.hours_per_week.is_finite()
            || !(0.0..=MAX_HOURS_PER_WEEK).contains(&payload.hours_per_week)
        {
            return Err(AppError::invalid_input(format!(
                "hours_per_week must be between 0 and {MAX_HOURS_PER_WEEK}, got {}",
                payload.hours_per_week
            )));
        }
        if payload.sessions_per_week > MAX_SESSIONS_PER_WEEK {
            return Err(AppError::invalid_input(format!(
                "sessions_per_week must be at most {MAX_SESSIONS_PER_WEEK}, got {}",
                payload.sessions_per_week
            )));
        }
        if let Some(weeks) = payload.weeks_to_goal {
            if weeks > 156 {
                return Err(AppError::invalid_input(format!(
                    "weeks_to_goal {weeks} is more than three years out"
                )));
            }
        }
        Ok(payload)
    }

    /// Fill each input from the argument, else storage, else the default —
    /// recording which, so the reply can say what it assumed.
    fn resolve(
        payload: &Payload,
        profile: Option<&UserPhysiologicalProfile>,
        goal: Option<(EventClass, NaiveDate)>,
        today: NaiveDate,
    ) -> Resolved {
        let mut sources = Vec::new();
        let mut take = |name: &'static str, src: Source| sources.push((name, src));

        let training_age = if let Some(v) = payload.training_age {
            take("training_age", Source::Argument);
            v
        } else if let Some(level) = profile.map(|p| p.fitness_level) {
            take("training_age", Source::Profile);
            training_age_from_level(level)
        } else {
            take("training_age", Source::Default);
            TrainingAge::Novice
        };
        // The catalogue gates on years as a number and on the band as a word,
        // and both must agree. A stored count wins; otherwise the band stands
        // in, at the fewest years that band means — so a coach who says
        // "trained" is not refused every flavour for an unstated zero.
        let training_age_years = profile
            .and_then(|p| p.training_experience_years)
            .map_or_else(|| years_the_band_means(training_age), f32::from);

        let (event_class, goal_date) = match (payload.event_class, goal) {
            (Some(ec), Some((_, date))) => {
                take("event_class", Source::Argument);
                (Some(ec), Some(date))
            }
            (Some(ec), None) => {
                take("event_class", Source::Argument);
                (Some(ec), None)
            }
            (None, Some((ec, date))) => {
                take("event_class", Source::Plan);
                (Some(ec), Some(date))
            }
            (None, None) => (None, None),
        };

        let mut goal_date = goal_date;
        let weeks_to_goal = match payload.weeks_to_goal {
            Some(w) => {
                take("weeks_to_goal", Source::Argument);
                // A stated runway fixes the goal on the calendar too, so the
                // season can be laid even before an outline names the race.
                if goal_date.is_none() {
                    goal_date = Some(today + chrono::Duration::weeks(i64::from(w)));
                }
                Some(w)
            }
            None => goal_date.map(|d| {
                take("weeks_to_goal", Source::Plan);
                let days = (d - today).num_days().max(0);
                u32::try_from(days / 7).unwrap_or(u32::MAX)
            }),
        };

        let measurements: BTreeSet<Measurement> = if let Some(list) = &payload.measurements {
            take("measurements", Source::Argument);
            list.iter().copied().collect()
        } else {
            let mut set = BTreeSet::new();
            if let Some(p) = profile {
                if p.ftp_watts.is_some() {
                    set.insert(Measurement::Power);
                }
                if p.threshold_pace_sec_per_km.is_some() {
                    set.insert(Measurement::Pace);
                }
                if p.max_hr.is_some() {
                    set.insert(Measurement::Hr);
                }
            }
            take(
                "measurements",
                if set.is_empty() {
                    Source::Default
                } else {
                    Source::Profile
                },
            );
            set
        };

        let sport_mix = if let Some(v) = payload.sport_mix {
            take("sport_mix", Source::Argument);
            v
        } else if let Some(p) = profile {
            take("sport_mix", Source::Profile);
            sport_mix_from(&p.primary_sport)
        } else {
            take("sport_mix", Source::Default);
            SportMix::Mixed
        };

        macro_rules! or_default {
            ($field:ident, $default:expr) => {
                match payload.$field {
                    Some(v) => {
                        take(stringify!($field), Source::Argument);
                        v
                    }
                    None => {
                        take(stringify!($field), Source::Default);
                        $default
                    }
                }
            };
        }
        let recovery_speed = or_default!(recovery_speed, RecoverySpeed::Typical);
        let injury_load = or_default!(injury_load, InjuryLoad::None);
        // None is the safe reading for an athlete about whom nothing is known:
        // it refuses every flavour that needs interval history, which is the
        // error a coach can recover from in one question.
        let interval_experience = or_default!(interval_experience, IntervalExperience::None);
        if payload.season_phase.is_some() {
            take("season_phase", Source::Argument);
        }

        Resolved {
            inputs: FlavourInputs {
                hours_per_week: payload.hours_per_week,
                sessions_per_week: payload.sessions_per_week,
                training_age_years,
                training_age,
                event_class,
                weeks_to_goal,
                measurements,
                recovery_speed,
                injury_load,
                interval_experience,
                sport_mix,
                season_phase: payload.season_phase,
                coach_preference: payload.coach_preference.clone(),
            },
            sources,
            goal_date,
        }
    }

    /// The skeleton the catalogue offers this event at this weekly volume.
    fn skeleton_for(
        skeletons: &[SkeletonTemplate],
        event: EventClass,
        tier: HoursTier,
    ) -> Option<SkeletonTemplate> {
        skeletons
            .iter()
            .find(|s| s.event_classes.contains(&event) && s.hours_tiers.contains(&tier))
            .or_else(|| skeletons.iter().find(|s| s.event_classes.contains(&event)))
            .cloned()
    }

    /// The verdict, with every ranked and excluded flavour also named in the
    /// athlete's own words and locale.
    ///
    /// The label is what the coach says out loud while the id stays the
    /// coach's name for it. Every other field is the kernel's own, and an
    /// unknown field is ignored on the way back in, so the coach can still
    /// pass this to `save_training_plan.flavour.verdict` verbatim and have
    /// the stored snapshot deserialize into `FlavourVerdict`.
    fn verdict_json(v: &FlavourVerdict, label: &FlavourLabels<'_>) -> LabelledVerdict {
        LabelledVerdict {
            ranked: v
                .ranked
                .iter()
                .map(|s| LabelledFlavour {
                    id: s.id.clone(),
                    label: label.of(&s.id),
                    score: s.score,
                    reasons: s.reasons.clone(),
                })
                .collect(),
            excluded: v
                .excluded
                .iter()
                .map(|e| LabelledExclusion {
                    id: e.id.clone(),
                    label: label.of(&e.id),
                    reasons: e.reasons.clone(),
                })
                .collect(),
            confidence: v.confidence,
            missing_inputs: v.missing_inputs.clone(),
            coach_pinned: v.coach_pinned.clone(),
        }
    }

    /// The inputs the verdict was reached on, and where each came from.
    fn inputs_json(resolved: &Resolved, tier: HoursTier) -> FlavourInputsEcho {
        FlavourInputsEcho {
            hours_per_week: resolved.inputs.hours_per_week,
            hours_tier: tier.as_str().to_owned(),
            sessions_per_week: resolved.inputs.sessions_per_week,
            training_age: resolved.inputs.training_age,
            training_age_years: resolved.inputs.training_age_years,
            event_class: resolved.inputs.event_class,
            weeks_to_goal: resolved.inputs.weeks_to_goal,
            // The EFFECTIVE set, not the raw one: what the athlete can
            // actually measure is what decides whether a flavour is runnable.
            measurements: resolved.inputs.effective_measurements(),
            recovery_speed: resolved.inputs.recovery_speed,
            injury_load: resolved.inputs.injury_load,
            interval_experience: resolved.inputs.interval_experience,
            sport_mix: resolved.inputs.sport_mix,
            season_phase: resolved.inputs.season_phase,
            coach_preference: resolved.inputs.coach_preference.clone(),
            sources: resolved
                .sources
                .iter()
                .map(|(k, s)| InputSource {
                    input: (*k).to_owned(),
                    from: s.as_str().to_owned(),
                })
                .collect(),
        }
    }

    fn phase_json(p: &LaidPhase) -> LaidPhaseReport {
        LaidPhaseReport {
            kind: p.kind.as_str().to_owned(),
            start: p.start.format("%Y-%m-%d").to_string(),
            weeks: p.weeks,
            purpose: p.purpose.clone(),
            volume_share_of_peak: ShareRange {
                min: p.volume_share_of_peak.min,
                max: p.volume_share_of_peak.max,
            },
            flavour_override: p
                .flavour_override
                .map(|f| FlavourFamily::as_str(f).to_owned()),
            key_sessions: p
                .key_sessions
                .iter()
                .map(|k| k.as_str().to_owned())
                .collect(),
            loading_pattern: p.loading_pattern.to_string(),
            peak: p.peak.format("%Y-%m-%d").to_string(),
        }
    }

    fn season_json(layout: Option<&SeasonLayout>, skeleton_id: Option<&str>) -> SeasonReport {
        let base = SeasonReport {
            status: String::new(),
            skeleton_id: skeleton_id.map(str::to_owned),
            note: None,
            needs_weeks: None,
            has_weeks: None,
            phases: Vec::new(),
            shrunk: Vec::new(),
        };
        match layout {
            None => SeasonReport {
                status: "no_goal".to_owned(),
                skeleton_id: None,
                note: Some("no goal race is known; pass event_class and weeks_to_goal, or save an outline with a goal race first".to_owned()),
                ..base
            },
            Some(SeasonLayout::NotEnoughRunway {
                needs_weeks,
                has_weeks,
            }) => SeasonReport {
                status: "not_enough_runway".to_owned(),
                needs_weeks: Some(*needs_weeks),
                has_weeks: Some(*has_weeks),
                note: Some("the skeleton cannot be compressed into this runway; move the goal or choose another skeleton rather than squeezing every phase".to_owned()),
                ..base
            },
            Some(SeasonLayout::Laid { phases, shrunk }) => SeasonReport {
                status: "laid".to_owned(),
                phases: phases.iter().map(Self::phase_json).collect(),
                shrunk: shrunk.iter().map(|k| k.as_str().to_owned()).collect(),
                ..base
            },
        }
    }
}

/// The fewest years of structured training a band means, for when the
/// profile stores the band but not the count.
const fn years_the_band_means(band: TrainingAge) -> f32 {
    match band {
        TrainingAge::Novice => 0.0,
        TrainingAge::Recreational => 1.0,
        TrainingAge::Trained => 3.0,
        TrainingAge::Elite => 8.0,
    }
}

/// The catalogue's training-age band for a stored fitness level.
const fn training_age_from_level(level: FitnessLevel) -> TrainingAge {
    match level {
        FitnessLevel::Beginner => TrainingAge::Novice,
        FitnessLevel::Recreational => TrainingAge::Recreational,
        FitnessLevel::Intermediate | FitnessLevel::Advanced => TrainingAge::Trained,
        FitnessLevel::Elite | FitnessLevel::Professional => TrainingAge::Elite,
    }
}

/// The sport mix a primary sport implies. Anything the catalogue does not
/// name a row for is `mixed`, which is the honest reading rather than a guess.
fn sport_mix_from(sport: &SportType) -> SportMix {
    match SportFamily::of(sport) {
        SportFamily::Running => SportMix::Running,
        SportFamily::Cycling => SportMix::Cycling,
        SportFamily::Swimming => SportMix::Swimming,
        SportFamily::Other => SportMix::Mixed,
    }
}

/// The event class a saved goal race's discipline names, when it does.
fn event_class_from_discipline(discipline: &str) -> Option<EventClass> {
    let d = discipline.trim().to_ascii_lowercase();
    EventClass::ALL
        .iter()
        .copied()
        .find(|e| e.as_str() == d || e.as_str().replace('_', " ") == d)
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for RecommendPlanFlavourTool {
    fn definition(&self) -> Tool {
        let schema = object_schema(
            Self::properties(),
            Some(vec![
                "hours_per_week".to_owned(),
                "sessions_per_week".to_owned(),
            ]),
        );
        answers_with::<PlanFlavourResult>(tool_definition(
            "recommend_plan_flavour",
            "Choose the training flavour an athlete should run this season and lay the season out. Call it once /season and /calibrate have run, passing what the athlete answered — weekly hours and sessions at minimum. The stored profile fills in training age, the devices they have thresholds for and their primary sport; the active plan fills in the goal race. Returns every flavour they can run ranked with the reasons and evidence behind each, every flavour they cannot run with the reason stated, how confident the rule is, and the season's phases laid backward from the goal on the skeleton that fits. Present the verdict in your own voice, confirm the inputs it echoes back, and save the outcome through save_training_plan, passing `verdict` and `inputs` through exactly as returned here, with selected_by rule for the first-ranked flavour or coach/athlete plus the reason for any other. This only recommends; it writes nothing.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        // Reads the stored profile — training age, thresholds, primary sport —
        // and echoes what it found, so the call discloses identity data and
        // must carry profile:read. Declared from the first commit rather than
        // patched in later, which is what carnet#363 was about.
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::PROFILE,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
            let requester = context.user_id;
            let payload = Self::payload(&args)?;
            let repos = context.resources.repos();

            let conversation = load_conversation(
                repos,
                context.conversation_id.as_deref(),
                requester_tenant,
                &requester.to_string(),
            )
            .await?;
            // Whose profile and plan: the caller's own, or — for the group's
            // human coach, from a direct chat, with the athlete's consent — a
            // coached athlete's. The same gate get_training_plan reads under.
            let scope = match resolve_plan_scope(PlanScopeRequest {
                context: &context,
                requester_tenant,
                conversation: conversation.as_ref(),
                arg_coach: None,
                athlete: payload.athlete.as_deref(),
                tool_name: "recommend_plan_flavour",
            })
            .await?
            {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
            let tenant_id = scope.tenant;
            let user_id = scope.user_id;
            let coach = scope.coach_slug.as_deref();

            let profile = repos
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let plan = repos
                .training_plans
                .get_active_plan(&tenant_id.to_string(), &user_id.to_string(), coach)
                .await?;
            let goal = plan.as_ref().and_then(|p| {
                let ec = event_class_from_discipline(&p.goal_race.discipline)?;
                let date = parse_plan_date(&p.goal_race.date)?;
                Some((ec, date))
            });

            let today = athlete_today(repos, &user_id.to_string()).await;
            let resolved = Self::resolve(&payload, profile.as_ref(), goal, today);

            let catalogue = state.training_catalogue();
            let table = catalogue.selection().ok_or_else(|| {
                AppError::internal("the training catalogue carries no selection table")
            })?;
            let flavours = catalogue.flavours();
            let verdict = select_flavour(&resolved.inputs, &table, &flavours);

            // The season: only when a goal is known, on the skeleton the
            // catalogue offers this event at this volume.
            let tier = resolved.inputs.hours_tier();
            let skeleton = resolved
                .inputs
                .event_class
                .and_then(|ec| Self::skeleton_for(&catalogue.skeletons(), ec, tier));
            let layout = match (&skeleton, resolved.goal_date) {
                (Some(sk), Some(goal_date)) => Some(build_skeleton(
                    sk,
                    &[goal_date],
                    today,
                    resolved.inputs.recovery_speed == RecoverySpeed::Limited,
                )),
                _ => None,
            };

            // The verdict speaks the athlete's language: the labels are what
            // the coach says, resolved the way the memory tool resolves them.
            let locale = resolve_user_locale(repos.users.as_ref(), user_id).await;
            let labels = FlavourLabels {
                registry: context.resources.messaging_strings_registry(),
                locale: &locale,
            };

            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                hours_tier = tier.as_str(),
                event_class = resolved.inputs.event_class.map_or("none", EventClass::as_str),
                top = verdict.top().map_or("none", |s| s.id.as_str()),
                excluded = verdict.excluded.len(),
                confidence = ?verdict.confidence,
                "recommended a plan flavour"
            );

            ok_typed(
                "recommend_plan_flavour",
                PlanFlavourResult {
                    athlete: scope.acting_for.clone(),
                    verdict: Self::verdict_json(&verdict, &labels),
                    season: Self::season_json(
                        layout.as_ref(),
                        skeleton.as_ref().map(|s| s.id.as_str()),
                    ),
                    inputs: Self::inputs_json(&resolved, tier),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}
// A read of the athlete's own profile and plan, then pure computation over
// the compiled-in catalogue. Nothing is written, nothing leaves the process,
// and the response carries only catalogue text and the athlete's own data.
crate::declare_security!(RecommendPlanFlavourTool => empty);
