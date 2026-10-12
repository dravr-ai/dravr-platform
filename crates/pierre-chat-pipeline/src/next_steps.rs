// ABOUTME: Next steps — at most three catalogue entries a reply hands the athlete, so the move after it is a tap
// ABOUTME: The transition table that picks them, and the one way any reply attaches them: live actions and stored row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Next steps (carnet#830).
//!
//! A guided walk, a command and an analysis each end somewhere the athlete
//! could go next. [`StepsAfter`] is the transition table that says where: a
//! row per ending, naming use-case catalogue entries in priority order —
//! the pillars walk leads to the season or a calibration, the season to the
//! fortnight, a saved fortnight to the calendar or today's session, a first
//! analysis to a week-on-week comparison. Every candidate is then filtered
//! through its catalogue `requires:` against the athlete's
//! [`AthleteState`], so no step is offered before its prerequisite holds,
//! and a `once` entry the athlete already tapped is not offered again. A post
//! the coach makes on its own has no such ending: it closes on the use cases
//! ranked for the athlete the way a welcome's starters are (carnet#831).
//!
//! Whatever was picked reaches the athlete one way: as the reply's actions —
//! buttons where the surface draws them, lines of text where it does not —
//! and as one `{"type":"actions"}` entry on the stored row, so a reload shows
//! the buttons the live turn did. Each step posts `nx:<position>:<id>`, which
//! [`crate::suggestions`] resolves into the command or prompt the entry
//! stands for and reports as a next step rather than a starter.
//!
//! A step starts something — a walk, a question — and never answers it: what
//! the athlete says once it has started is still theirs to say. So no step
//! rides a reply while a guided walk owns the thread, and none is offered in
//! a shared room, where one athlete's state would be read back to everyone.

use chrono::Utc;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_contremaitre::use_case_catalogue::{Repeat, UseCase, UseCaseCatalogue, UseCaseRun};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::ConversationRecord;
use pierre_database::repositories::UseCaseExposure;
use pierre_database::RepositoryRegistry;
use pierre_services::athlete_state::AthleteState;
use pierre_services::use_case_rank::worded;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::envelope::{ActionKind, TurnAction};
use crate::stages::command_persistence::actions_block;
use crate::stages::onboarding::season_laid_out;
use crate::stages::platform_grounding;
use crate::suggestions::{emit_shown, Postback, SuggestionEvent, SURFACE_NEXT_STEP};
use crate::surface_profile::{SurfaceId, SurfaceProfile};
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// The catalogue entry whose answer is a workout analysis: the athlete's
/// first tap of it is their first analysis ([`StepsAfter::FirstAnalysis`]).
const ANALYSIS_USE_CASE: &str = "last_workout";

/// The most next steps one reply offers: one row of buttons on a phone, and
/// as many as `WhatsApp` shows as reply buttons before it folds them into a
/// list.
pub const MAX_NEXT_STEPS: usize = 3;

/// What a reply's next steps follow: one row of the transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepsAfter {
    /// The pillars walk had nothing left to ask.
    PillarsWalk,
    /// The season walk ended, or the season it led to was laid out.
    SeasonWalk,
    /// The fortnight rail's re-check found the two weeks on the plan.
    FortnightSaved,
    /// The athlete's first tapped workout analysis was answered.
    FirstAnalysis,
    /// `/status` answered.
    StatusCommand,
    /// `/plan` answered.
    PlanCommand,
    /// `/fortnight` answered without starting the rail: it declined.
    FortnightCommand,
}

impl StepsAfter {
    /// The catalogue entries offered after this ending, in priority order,
    /// before the athlete's state filters them.
    ///
    /// The guided chain the coach guide recommends — `/pillars`, `/calibrate`,
    /// `/season`, `/fortnight`, then the calendar — read one link at a time.
    /// The calibration wrap-up has no row on purpose: it closes on a question
    /// (rebuild the plan, or build one) the athlete answers in their own
    /// words, and a button would answer it for them.
    #[must_use]
    pub const fn candidates(self) -> &'static [&'static str] {
        match self {
            Self::PillarsWalk => &["season", "harder"],
            Self::SeasonWalk => &["fortnight"],
            Self::FortnightSaved => &["calendar", "today"],
            Self::FirstAnalysis => &["compare_week"],
            // What an athlete reading their account does next: connect a
            // source, tell the coach about themselves, plan the season.
            Self::StatusCommand => &["connect", "about_me", "season"],
            // `/plan` already shows today: the season when there is none to
            // show, else the next two weeks and the calendar.
            Self::PlanCommand => &["season", "fortnight", "calendar"],
            // `/fortnight` declines when there is no season to write against,
            // or the next two weeks are already on the plan.
            Self::FortnightCommand => &["season", "today", "calendar"],
        }
    }

    /// The row the answer to the athlete's first tap of the catalogue entry
    /// `id` offers: a first workout analysis leads to the weekly comparison.
    /// `None` for every other entry, whose answer is an ordinary coaching
    /// turn and offers no step.
    #[must_use]
    pub fn on_first_tap(id: &str) -> Option<Self> {
        (id == ANALYSIS_USE_CASE).then_some(Self::FirstAnalysis)
    }

    /// The row for a command's reply, by the command definition's `name:`
    /// id; `None` for a command whose reply offers no next step. `/help`,
    /// `/agent` and `/discover` carry their own controls, a walk's opener is
    /// the walk's first question, and account, privacy and group commands are
    /// not a step in the athlete's training.
    #[must_use]
    pub fn for_command(name: &str) -> Option<Self> {
        match name {
            "status" => Some(Self::StatusCommand),
            "plan" => Some(Self::PlanCommand),
            "fortnight" => Some(Self::FortnightCommand),
            _ => None,
        }
    }
}

/// The catalogue entries `after` offers an athlete in `state`, at most
/// [`MAX_NEXT_STEPS`], in the row's order.
///
/// An entry is dropped when the catalogue no longer lists it, when one of its
/// requirements does not hold, when it is a `once` entry the athlete already
/// tapped, or when it runs `ran` — the command whose reply this is.
#[must_use]
pub(crate) fn pick<'a>(
    catalogue: &'a UseCaseCatalogue,
    after: StepsAfter,
    state: &AthleteState,
    exposures: &[UseCaseExposure],
    ran: Option<&str>,
) -> Vec<&'a UseCase> {
    after
        .candidates()
        .iter()
        .filter_map(|id| catalogue.get(id))
        .filter(|entry| entry.eligible(|predicate| state.holds(predicate)))
        .filter(|entry| !tapped_once(entry, exposures))
        .filter(|entry| !runs(entry, ran))
        .take(MAX_NEXT_STEPS)
        .collect()
}

/// Whether `entry` is a `once` entry the athlete already tapped.
fn tapped_once(entry: &UseCase, exposures: &[UseCaseExposure]) -> bool {
    entry.repeat == Repeat::Once
        && exposures
            .iter()
            .any(|exposure| exposure.use_case_id == entry.id && exposure.tapped_at.is_some())
}

/// Whether `entry` runs the command `ran` the athlete just ran.
fn runs(entry: &UseCase, ran: Option<&str>) -> bool {
    match (&entry.run, ran) {
        (UseCaseRun::Command(command), Some(ran)) => command.trim() == ran.trim(),
        _ => false,
    }
}

/// One step: the entry it stands for, in the slot it is shown in, and the
/// label its button carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    postback: Postback,
    label: String,
}

/// The next steps a reply offers, at most [`MAX_NEXT_STEPS`], in the order
/// they were picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NextSteps {
    steps: Vec<Step>,
}

/// Who a reply's next steps are for, and where they are read.
pub struct StepsFor<'a> {
    /// The athlete the reply answers.
    pub user_id: Uuid,
    /// The athlete's own tenant, where their state and taps live.
    pub tenant_id: TenantId,
    /// Locale the steps' labels are written in.
    pub locale: &'a str,
    /// The command whose reply this is, as typed; its own entry is not
    /// offered back.
    pub ran: Option<&'a str>,
}

impl NextSteps {
    /// The steps `after` offers the athlete `target` names, labelled in their
    /// language.
    ///
    /// Best-effort: an athlete whose state cannot be read is offered nothing,
    /// and the reply goes out without steps rather than with ones that may not
    /// fit. Read under the caller's derivation, so a reply whose steps were
    /// chosen from first-party-only data is stamped with it.
    pub async fn after(
        repos: &RepositoryRegistry,
        strings: &MessagingStringsRegistry,
        after: StepsAfter,
        target: &StepsFor<'_>,
    ) -> Self {
        let state = match AthleteState::read(repos, target.tenant_id, target.user_id, Utc::now())
            .await
        {
            Ok((state, _)) => state,
            Err(e) => {
                warn!(error = %e, after = ?after, "athlete state unreadable; the reply offers no next step");
                return Self::default();
            }
        };
        let exposures = repos
            .use_case_exposures
            .use_case_exposures(target.tenant_id, target.user_id)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, after = ?after, "use-case taps unreadable; next steps picked as if none were tapped");
                Vec::new()
            });
        let picked = pick(
            UseCaseCatalogue::pinned(),
            after,
            &state,
            &exposures,
            target.ran,
        );
        Self::labelled(&picked, strings, target.locale)
    }

    /// `entries` as steps, slot by slot; an entry with no words in `locale` —
    /// as [`worded`] reads a starter's — is skipped and gives its slot to the
    /// next.
    #[must_use]
    pub(crate) fn labelled(
        entries: &[&UseCase],
        strings: &MessagingStringsRegistry,
        locale: &str,
    ) -> Self {
        let steps = entries
            .iter()
            .filter_map(|entry| worded(entry, strings, locale))
            .enumerate()
            .map(|(position, worded)| Step {
                postback: Postback::NextStep {
                    position,
                    id: worded.use_case.id.clone(),
                },
                label: worded.label,
            })
            .collect();
        Self { steps }
    }

    /// Whether there is nothing to offer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Report each step shown, on the surfaces that draw it as a button.
    ///
    /// A messaging channel lists the steps as text, where a step typed back
    /// is the athlete's own words, so only the app's surfaces count — the
    /// rule the agent welcome's starters follow. A next step is not counted
    /// toward retiring a welcome's starter: that counts welcomes.
    pub(crate) fn report_shown(&self, user_id: &str, tenant_id: TenantId, surface: SurfaceId) {
        if !matches!(surface, SurfaceId::Web | SurfaceId::Mobile) {
            return;
        }
        let event = SuggestionEvent {
            user_id,
            tenant_id,
            surface: SURFACE_NEXT_STEP,
            channel: surface.as_str(),
        };
        for step in &self.steps {
            emit_shown(&event, &step.postback);
        }
    }

    /// The steps as the live reply's controls — what
    /// [`crate::TurnState::actions`] carries into the envelope.
    #[must_use]
    pub fn into_actions(self) -> Vec<TurnAction> {
        self.steps
            .into_iter()
            .map(|step| TurnAction {
                label: step.label,
                kind: ActionKind::Postback,
                value: step.postback.encode(),
            })
            .collect()
    }

    /// The row's `content_blocks` with these steps appended as one actions
    /// entry.
    ///
    /// `stored` is what the reply's other stages already wrote to the column —
    /// a chart's specs, a workout plan — or `None` for a reply with no other
    /// block. With no steps it comes back unchanged, so a reply that offers
    /// nothing keeps the `NULL` column of any other prose row.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `stored` is not a JSON array or
    /// the entry cannot be encoded. The caller then persists the reply without
    /// its steps rather than losing the reply.
    pub fn stored_blocks(&self, stored: Option<&str>) -> Result<Option<String>, serde_json::Error> {
        if self.steps.is_empty() {
            return Ok(stored.map(ToOwned::to_owned));
        }
        let mut entries: Vec<Value> = stored
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let actions = self.clone().into_actions();
        entries.push(serde_json::to_value(actions_block(None, &actions))?);
        serde_json::to_string(&entries).map(Some)
    }
}

/// The next steps the agent's reply to `input` offers.
///
/// A post the platform composed on its own ends on the catalogue's use cases
/// ranked for the athlete ([`platform_grounding::closing_steps`],
/// carnet#831). A walk that ended on this turn picked them already
/// (`closing`). Otherwise a reply offers steps only after a transition the
/// turn itself completed:
/// the athlete's first tapped workout analysis ([`TurnInput::steps_after`]),
/// or the season the season walk offered, laid out on the turn after it.
/// Never in a shared room, where one athlete's state would be read back to
/// everyone; an ordinary coaching turn offers none.
pub(crate) async fn for_reply(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    profile: &SurfaceProfile,
    conv: &ConversationRecord,
    closing: NextSteps,
    plan_was_saved: bool,
) -> NextSteps {
    if !input.origin.persists_user_row() {
        return platform_grounding::closing_steps(ctx, input, profile, conv).await;
    }
    if !input.is_direct_message {
        return NextSteps::default();
    }
    if !closing.is_empty() {
        return closing;
    }
    let laid_out = season_laid_out(conv.onboarding_state.as_deref(), Utc::now(), plan_was_saved);
    let Some(after) = input
        .steps_after
        .or_else(|| laid_out.then_some(StepsAfter::SeasonWalk))
    else {
        return closing;
    };
    let Ok(user_id) = parse_uuid(&input.user_id) else {
        return closing;
    };
    NextSteps::after(
        &ctx.repos,
        &ctx.messaging_strings_registry,
        after,
        &StepsFor {
            user_id,
            tenant_id: input.tool_tenant_id,
            locale: &profile.locale,
            ran: None,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_contremaitre::use_case_catalogue::Stage;
    use pierre_core::models::{PersistedAction, PersistedReplyBlock};
    use pierre_services::athlete_state::{ActivityHistory, DossierCoverage};
    use serde_json::json;

    use super::*;

    /// The rows' entries as the catalogue writes them, plus one no row names.
    const CATALOGUE: &str = "
- {id: connect, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [any], repeat: once}
- {id: about_me, run: command, command: /pillars, requires: [dossier_empty], stage: first_session, domains: [any], repeat: once}
- {id: season, run: command, command: /season, requires: [pillars_done, no_season], stage: first_week, domains: [training], repeat: once}
- {id: harder, run: command, command: /calibrate, requires: [plan_active], stage: first_week, domains: [training], repeat: once}
- {id: today, run: command, command: /plan today, requires: [plan_active], stage: any, domains: [training], repeat: recurring}
- {id: fortnight, run: command, command: /fortnight, requires: [season_set], stage: any, domains: [training], repeat: recurring}
- {id: calendar, run: prompt, command: null, requires: [season_set, calendar_writable], stage: any, domains: [training], repeat: once}
- {id: compare_week, run: prompt, command: null, requires: [weeks_of_data>=2], stage: any, domains: [analysis, training], repeat: recurring}
- {id: zone2, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [training], repeat: once}
";

    fn catalogue() -> UseCaseCatalogue {
        UseCaseCatalogue::parse(CATALOGUE).expect("the fixture parses")
    }

    /// An athlete with nothing connected, nothing told and no plan.
    const fn newcomer() -> AthleteState {
        AthleteState {
            stage: Stage::FirstSession,
            has_provider: false,
            activities: ActivityHistory::Absent,
            weeks_of_data: 0,
            dossier: DossierCoverage {
                covered: 0,
                complete: false,
                empty: true,
            },
            season_set: false,
            plan_active: false,
            recovery_source: false,
            calendar_writable: false,
        }
    }

    /// Every pillar told, a provider with three weeks of rides, no season yet.
    const fn pillars_done() -> AthleteState {
        AthleteState {
            has_provider: true,
            activities: ActivityHistory::Present,
            weeks_of_data: 3,
            dossier: DossierCoverage {
                covered: 6,
                complete: true,
                empty: false,
            },
            ..newcomer()
        }
    }

    /// A season laid out, this week on the plan, a calendar that takes it.
    const fn planned() -> AthleteState {
        AthleteState {
            season_set: true,
            plan_active: true,
            calendar_writable: true,
            ..pillars_done()
        }
    }

    fn ids(entries: &[&UseCase]) -> Vec<String> {
        entries.iter().map(|entry| entry.id.clone()).collect()
    }

    fn picked(after: StepsAfter, state: &AthleteState) -> Vec<String> {
        ids(&pick(&catalogue(), after, state, &[], None))
    }

    fn tapped(id: &str) -> UseCaseExposure {
        UseCaseExposure {
            use_case_id: id.to_owned(),
            shown_count: 0,
            last_shown_at: None,
            tapped_at: Some(Utc::now()),
        }
    }

    #[test]
    fn the_pillars_walk_leads_to_the_season_or_a_calibration_as_the_athlete_stands() {
        assert_eq!(picked(StepsAfter::PillarsWalk, &pillars_done()), ["season"]);
        let with_plan = AthleteState {
            plan_active: true,
            ..pillars_done()
        };
        assert_eq!(
            picked(StepsAfter::PillarsWalk, &with_plan),
            ["season", "harder"]
        );
        assert_eq!(picked(StepsAfter::PillarsWalk, &planned()), ["harder"]);
        let gaps_left = AthleteState {
            dossier: DossierCoverage {
                covered: 4,
                complete: false,
                empty: false,
            },
            ..pillars_done()
        };
        assert!(
            picked(StepsAfter::PillarsWalk, &gaps_left).is_empty(),
            "no season before every pillar is told, no calibration without a plan"
        );
    }

    #[test]
    fn the_season_leads_to_the_fortnight_only_once_it_is_laid_out() {
        assert!(picked(StepsAfter::SeasonWalk, &pillars_done()).is_empty());
        assert_eq!(picked(StepsAfter::SeasonWalk, &planned()), ["fortnight"]);
    }

    #[test]
    fn a_saved_fortnight_leads_to_the_calendar_where_one_takes_it_and_to_today() {
        assert_eq!(
            picked(StepsAfter::FortnightSaved, &planned()),
            ["calendar", "today"]
        );
        let no_calendar = AthleteState {
            calendar_writable: false,
            ..planned()
        };
        assert_eq!(picked(StepsAfter::FortnightSaved, &no_calendar), ["today"]);
    }

    #[test]
    fn a_first_analysis_leads_to_the_weekly_comparison_with_two_weeks_of_data() {
        assert_eq!(
            picked(StepsAfter::FirstAnalysis, &pillars_done()),
            ["compare_week"]
        );
        let one_week = AthleteState {
            weeks_of_data: 1,
            ..pillars_done()
        };
        assert!(picked(StepsAfter::FirstAnalysis, &one_week).is_empty());
    }

    #[test]
    fn a_command_reply_offers_its_row_and_never_the_command_that_ran() {
        assert_eq!(
            picked(StepsAfter::StatusCommand, &newcomer()),
            ["connect", "about_me"]
        );
        assert_eq!(picked(StepsAfter::PlanCommand, &pillars_done()), ["season"]);
        assert_eq!(
            picked(StepsAfter::PlanCommand, &planned()),
            ["fortnight", "calendar"]
        );
        let catalogue = catalogue();
        let after_today = pick(
            &catalogue,
            StepsAfter::FortnightCommand,
            &planned(),
            &[],
            Some("/plan today"),
        );
        assert_eq!(ids(&after_today), ["calendar"]);
        assert_eq!(
            StepsAfter::for_command("plan"),
            Some(StepsAfter::PlanCommand)
        );
        assert_eq!(
            StepsAfter::on_first_tap("last_workout"),
            Some(StepsAfter::FirstAnalysis)
        );
        assert_eq!(StepsAfter::on_first_tap("recovered"), None);
        for silent in [
            "help",
            "agent-add",
            "discover",
            "pillars",
            "season",
            "logout",
        ] {
            assert_eq!(StepsAfter::for_command(silent), None, "{silent}");
        }
    }

    #[test]
    fn a_tapped_once_entry_is_not_offered_again_and_a_recurring_one_is() {
        let used = [tapped("calendar"), tapped("today")];
        let catalogue = catalogue();
        let left = pick(
            &catalogue,
            StepsAfter::FortnightSaved,
            &planned(),
            &used,
            None,
        );
        assert_eq!(ids(&left), ["today"]);
    }

    #[test]
    fn every_row_names_an_entry_the_pinned_catalogue_lists() {
        let pinned = UseCaseCatalogue::pinned();
        assert!(
            pinned.get(ANALYSIS_USE_CASE).is_some(),
            "the analysis a first tap is read from"
        );
        for after in [
            StepsAfter::PillarsWalk,
            StepsAfter::SeasonWalk,
            StepsAfter::FortnightSaved,
            StepsAfter::FirstAnalysis,
            StepsAfter::StatusCommand,
            StepsAfter::PlanCommand,
            StepsAfter::FortnightCommand,
        ] {
            assert!(after.candidates().len() <= MAX_NEXT_STEPS, "{after:?}");
            for id in after.candidates() {
                assert!(
                    pinned.get(id).is_some(),
                    "{after:?} names {id}, which the pinned catalogue does not list"
                );
            }
        }
    }

    fn step(position: usize, id: &str, label: &str) -> Step {
        Step {
            postback: Postback::NextStep {
                position,
                id: id.to_owned(),
            },
            label: label.to_owned(),
        }
    }

    fn steps(entries: &[(&str, &str)]) -> NextSteps {
        NextSteps {
            steps: entries
                .iter()
                .enumerate()
                .map(|(position, (id, label))| step(position, id, label))
                .collect(),
        }
    }

    fn persisted(label: &str, value: &str) -> PersistedAction {
        PersistedAction {
            label: label.to_owned(),
            action_type: "postback".to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_step_shows_its_label_and_posts_its_slot_and_id() {
        let strings = MessagingStringsRegistry::new();
        let catalogue = UseCaseCatalogue::pinned();
        let entries: Vec<&UseCase> = ["season", "fortnight"]
            .iter()
            .filter_map(|id| catalogue.get(id))
            .collect();
        let actions = NextSteps::labelled(&entries, &strings, "en").into_actions();
        let values: Vec<&str> = actions.iter().map(|a| a.value.as_str()).collect();
        assert_eq!(values, ["nx:0:season", "nx:1:fortnight"]);
        for (action, entry) in actions.iter().zip(&entries) {
            assert_eq!(action.kind, ActionKind::Postback);
            assert_eq!(action.label, strings.get(&entry.label_key(), "en"));
            assert!(!action.label.is_empty());
        }
    }

    #[test]
    fn an_entry_with_no_label_gives_its_slot_to_the_next() {
        let strings = MessagingStringsRegistry::new();
        let fixture = catalogue();
        let unknown = UseCaseCatalogue::parse(
            "- {id: no_words_here, run: prompt, command: null, requires: [], stage: any, domains: [any], repeat: once}",
        )
        .expect("parses");
        let entries = [
            unknown.get("no_words_here").expect("listed"),
            fixture.get("season").expect("listed"),
        ];
        let values: Vec<String> = NextSteps::labelled(&entries, &strings, "en")
            .into_actions()
            .into_iter()
            .map(|action| action.value)
            .collect();
        assert_eq!(values, ["nx:0:season"]);
    }

    #[test]
    fn a_reply_with_no_other_block_stores_one_actions_entry() {
        let stored = steps(&[("season", "Season"), ("today", "Today")])
            .stored_blocks(None)
            .unwrap()
            .expect("steps are stored");
        let blocks: Vec<PersistedReplyBlock> = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            blocks,
            [PersistedReplyBlock::Actions {
                title: None,
                actions: vec![
                    persisted("Season", "nx:0:season"),
                    persisted("Today", "nx:1:today")
                ],
            }]
        );
    }

    #[test]
    fn steps_follow_the_blocks_the_reply_already_stored() {
        let chart = json!({ "type": "line", "title": "Load" });
        let stored = serde_json::to_string(&[&chart]).unwrap();
        let merged = steps(&[("today", "Today")])
            .stored_blocks(Some(&stored))
            .unwrap()
            .expect("steps are stored");
        let entries: Vec<Value> = serde_json::from_str(&merged).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], chart, "the chart is kept, first");
        assert_eq!(
            serde_json::from_value::<PersistedReplyBlock>(entries[1].clone()).unwrap(),
            PersistedReplyBlock::Actions {
                title: None,
                actions: vec![persisted("Today", "nx:0:today")],
            }
        );
    }

    #[test]
    fn no_steps_leave_the_column_as_it_was() {
        let none = NextSteps::default();
        assert_eq!(none.stored_blocks(None).unwrap(), None);
        let chart = r#"[{"type":"line"}]"#;
        assert_eq!(
            none.stored_blocks(Some(chart)).unwrap().as_deref(),
            Some(chart)
        );
    }

    #[test]
    fn a_column_that_is_not_an_array_is_refused_rather_than_overwritten() {
        let one = steps(&[("season", "Season")]);
        assert!(one.stored_blocks(Some(r#"{"type":"line"}"#)).is_err());
        assert!(one.stored_blocks(Some("not json")).is_err());
    }
}
