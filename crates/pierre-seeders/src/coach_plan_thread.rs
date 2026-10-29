// ABOUTME: Seeds a human coach's plan thread about one athlete, its card showing the coach's open draft, for the plan-thread flows
// ABOUTME: The group the coach coaches the athlete in, the athlete's live season, the coach's draft over it and the reply a turn leaves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach plan thread seeder (carnet#1069/#1070).
//!
//! A group's human coach taps "Work on <name>'s plan" in Group info and lands
//! in their thread about that athlete, where the plan card a turn left shows
//! the coach's draft with Publish and Discard. A turn puts the card on its
//! reply only when the model saved or read the plan, so a stack with no model
//! behind it (the mobile E2E lanes) has none. This seeder writes what such a
//! turn leaves, so the mobile Maestro flow
//! `coach-plan/01-publish-draft-from-athlete-thread.yaml` has a draft to
//! publish:
//!
//! - a group named [`GROUP_NAME`], owned by the athlete — its one member, who
//!   shares with its coach — and coached by the coach, who holds no
//!   membership row, as a coach invite leaves them; the coach's group thread
//!   is filed, which is how they reach Group info;
//! - the athlete's live season, aimed at [`LIVE_RACE`], with this week's
//!   sessions built around [`LIVE_WORKOUT`];
//! - the coach's thread about the athlete, opened as Group info opens it;
//! - the coach's open draft, re-laying the season toward [`DRAFT_RACE`] and
//!   redrafting this week around [`DRAFT_WORKOUT`], through the draft store;
//! - the coach's ask, [`QUESTION`], and the reply carrying the card: the
//!   athlete's plan as the coach works on it, with the athlete and where the
//!   draft stands beside it, from the card builder a turn uses.
//!
//! Before anything is written to the thread the coaching gates are asked as
//! the route that opens it asks them, so a seed the flow's taps would be
//! refused on fails here, naming the gate. Publishing the draft consumes it,
//! which is why the Maestro flow asks its control process for a new pair of
//! accounts seeded this way on every attempt.
//!
//! Run it after `user create` has made both accounts. It is idempotent: a
//! coach who already coaches a group of that name is left as they are.
//!
//! ```bash
//! pierre-cli seed coach-plan-thread
//! ```

use chrono::{Datelike, Days, NaiveDate};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::GroupRole;
use pierre_core::models::{AddMessageParams, ConversationSubject};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{
    NewSubjectConversation, PlanAuthor, PlanOutlineInput, PlanWeekInput, SaveDraftParams,
    SavePlanBundleParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{GoalRace, PlannedDay, RacePriority};
use pierre_middleware::mask_email;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use pierre_services::locale::user_locale;
use pierre_services::plan_card::{try_load_subject_plan_card, PlanSubject, PLAN_CARD_SOURCE_TOOL};
use tracing::info;

use crate::accounts::{account, Account};
use crate::coached_group::{coaches_group_named, seed_coached_group, GroupSpec};

/// The seeded group's name, which the coach's group thread is titled after.
pub const GROUP_NAME: &str = "Tempo Plan Squad";

/// The race the athlete's live season is aimed at.
pub const LIVE_RACE: &str = "Spring Half Marathon";

/// The race the coach's draft re-lays the season toward.
pub const DRAFT_RACE: &str = "Autumn Marathon";

/// The session the athlete's live week is built around.
pub const LIVE_WORKOUT: &str = "Steady run, 50 min";

/// The session the coach's draft rebuilds the week around.
pub const DRAFT_WORKOUT: &str = "Tempo 3 x 10 min";

/// What the coach asked in the thread, on the turn that saved the draft.
pub const QUESTION: &str = "Draft the next training block.";

/// How far past the athlete's today each race is.
const LIVE_RACE_DAYS_AHEAD: u64 = 120;
const DRAFT_RACE_DAYS_AHEAD: u64 = 180;

/// The seeded group, owned by the athlete and coached by the coach.
const GROUP: GroupSpec<'static> = GroupSpec {
    name: GROUP_NAME,
    description: "A runner and the coach who plans their season",
    agent_description: "The agent of the seeded coach plan group",
    agent_prompt: "You coach a runner whose human coach plans their season with you.",
};

/// CLI arguments for the coach plan thread seeder.
#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    /// The group's human coach, who drafts the athlete's plan
    #[arg(long, default_value = "plancoach@pierre.dev")]
    pub coach_email: String,

    /// The athlete whose plan the coach drafts: the group's owner and only member
    #[arg(long, default_value = "planathlete@pierre.dev")]
    pub athlete_email: String,

    /// The model the threads are created with. `pierre-cli` fills it, when
    /// not given, with the model every new conversation gets from the
    /// configured LLM provider
    #[arg(long)]
    pub model: Option<String>,
}

/// Seed the group, the athlete's season, the coach's thread about them, the
/// coach's draft and the reply whose card shows it.
///
/// # Errors
///
/// Returns a configuration error when no model was resolved or an account is
/// missing, the coaching gate's own refusal when the pair would not pass it,
/// or an error if any repository write fails.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    let model = args.model.ok_or_else(|| {
        AppError::config(
            "No model for the seeded threads: pass --model, or configure the LLM provider the \
             server creates conversations with",
        )
    })?;
    let coach = account(repos, &args.coach_email).await?;
    let athlete = account(repos, &args.athlete_email).await?;
    if coaches_group_named(repos, &coach, GROUP_NAME).await? {
        return Ok(());
    }

    let group = seed_coached_group(
        repos,
        &GROUP,
        &athlete,
        &coach,
        &[(&athlete, GroupRole::Owner)],
    )
    .await?;
    // A coach who redeems a coach invite gets a group thread, and that thread
    // is how they reach Group info and its roster; the seeded coach is
    // attached directly, so it is filed here the same way.
    let coach_id = coach.id.to_string();
    let group_id = group.id.to_string();
    repos
        .chat
        .create_conversation(
            &coach_id,
            coach.tenant,
            GROUP_NAME,
            &model,
            Some(&group.agent_id),
            Some(&group_id),
        )
        .await?;

    // The gates "Work on <name>'s plan" and every request in the thread ask,
    // asked the same way: the athlete's tenant and roster name come from them.
    let access = coached_athlete_access(repos, coach.id, athlete.id, group.id)
        .await?
        .map_err(CoachedAthleteRefusal::into_error)?;
    let today = athlete_today(repos, athlete.id).await;
    let week_start = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    seed_live_season(repos, &access, &group.agent_id, today, week_start).await?;

    let thread = repos
        .chat
        .open_subject_conversation(&NewSubjectConversation {
            user_id: &coach_id,
            tenant_id: coach.tenant,
            title: &access.display_name,
            model: &model,
            agent_id: &group.agent_id,
            subject: ConversationSubject {
                athlete_user_id: athlete.id,
                group_id: group.id,
            },
        })
        .await?
        .conversation;
    let target = CoachDraftTarget::new(access.tenant, athlete.id, coach.id);
    seed_draft(
        repos,
        &target,
        &group.agent_id,
        &thread.id,
        today,
        week_start,
    )
    .await?;
    seed_card_reply(repos, &coach, &thread.id, &model, &target, access, today).await?;

    info!(
        "Seeded {GROUP_NAME}: {} drafts {}'s plan in their thread",
        mask_email(&args.coach_email),
        mask_email(&args.athlete_email)
    );
    Ok(())
}

/// The athlete's live season: aimed at [`LIVE_RACE`], with this week laid. An
/// athlete who already has a season keeps it, and the draft is laid over it.
async fn seed_live_season(
    repos: &RepositoryRegistry,
    access: &CoachedAthleteAccess,
    agent_id: &str,
    today: NaiveDate,
    week_start: NaiveDate,
) -> AppResult<()> {
    let tenant_id = access.tenant.to_string();
    let athlete_id = access.athlete_user_id.to_string();
    if repos
        .training_plans
        .get_active_plan(&tenant_id, &athlete_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let race = race(LIVE_RACE, today, LIVE_RACE_DAYS_AHEAD);
    let days = week_days(week_start, LIVE_WORKOUT, "Z2");
    let week_start = ymd(week_start);
    repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant_id,
            user_id: &athlete_id,
            author: PlanAuthor::agent(agent_id),
            goal_fact_id: None,
            outline: Some(outline(&race, "Aerobic base toward the spring half", None)),
            replace_season: false,
            weeks: &[PlanWeekInput {
                week_start: &week_start,
                focus: "Aerobic base",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await?;
    Ok(())
}

/// The coach's draft: the season re-laid toward [`DRAFT_RACE`] and this week
/// redrafted, saved where the coach's `save_training_plan` saves it.
async fn seed_draft(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    agent_id: &str,
    thread_id: &str,
    today: NaiveDate,
    week_start: NaiveDate,
) -> AppResult<()> {
    let race = race(DRAFT_RACE, today, DRAFT_RACE_DAYS_AHEAD);
    let days = week_days(week_start, DRAFT_WORKOUT, "Z3");
    let week_start = ymd(week_start);
    repos
        .training_plans
        .save_draft_bundle(&SaveDraftParams {
            owner: target.owner(),
            author: PlanAuthor::agent(agent_id),
            goal_fact_id: None,
            outline: Some(outline(
                &race,
                "Build toward the autumn marathon with one tempo session a week",
                Some(thread_id),
            )),
            weeks: &[PlanWeekInput {
                week_start: &week_start,
                focus: "Tempo week",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await?;
    Ok(())
}

/// The coach's ask and the reply carrying the card of the athlete's plan as
/// the coach works on it — the coach's draft over the live weeks, with the
/// athlete and the draft's standing beside it — in the coach's locale.
async fn seed_card_reply(
    repos: &RepositoryRegistry,
    coach: &Account,
    thread_id: &str,
    model: &str,
    target: &CoachDraftTarget,
    access: CoachedAthleteAccess,
    today: NaiveDate,
) -> AppResult<()> {
    let locale = user_locale(repos.users.get_global(coach.id).await?.as_ref());
    let name = access.display_name;
    let subject = PlanSubject {
        user_id: access.athlete_user_id,
        display_name: name.clone(),
    };
    let card = try_load_subject_plan_card(
        repos,
        target,
        subject,
        today,
        &MessagingStringsRegistry::new(),
        &locale,
    )
    .await?
    .ok_or_else(|| AppError::internal("The seeded season and draft read back as no plan"))?;
    let block = card
        .card
        .as_subject_block(PLAN_CARD_SOURCE_TOOL, &card.draft)
        .and_then(|block| serde_json::to_string(&[block]))
        .map_err(|e| AppError::internal(format!("The seeded plan card did not serialise: {e}")))?;
    // The prose names neither race nor session, so whatever the flow reads of
    // the season is read off the card.
    let reply = format!(
        "I drafted the next block for {name}, with this week rebuilt around a tempo session. \
         {name} sees none of it until you publish it."
    );

    let coach_id = coach.id.to_string();
    for (role, content, content_blocks) in [
        ("user", QUESTION, None),
        ("assistant", reply.as_str(), Some(block.as_str())),
    ] {
        let assistant = role == "assistant";
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: coach.tenant,
                conversation_id: thread_id,
                user_id: &coach_id,
                role,
                content,
                token_count: None,
                finish_reason: assistant.then_some("stop"),
                prompt_tokens: None,
                model: assistant.then_some(model),
                content_blocks,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await?;
    }
    Ok(())
}

/// A race `days_ahead` days past `today`, the season's A race.
fn race(name: &str, today: NaiveDate, days_ahead: u64) -> GoalRace {
    GoalRace {
        name: name.to_owned(),
        date: ymd(today + Days::new(days_ahead)),
        discipline: "road".to_owned(),
        priority: RacePriority::A,
    }
}

/// An outline aimed at `race`, with no phases laid yet.
fn outline<'a>(
    race: &'a GoalRace,
    strategy: &'a str,
    source_conversation_id: Option<&'a str>,
) -> PlanOutlineInput<'a> {
    PlanOutlineInput {
        goal_race: race,
        races: None,
        strategy,
        flavour: None,
        season_start: None,
        season_end: None,
        phases: &[],
        source_conversation_id,
    }
}

/// The week starting `week_start`: `key_session` at `key_intensity` on
/// Tuesday, an easy run on Thursday and a long run on Saturday.
fn week_days(week_start: NaiveDate, key_session: &str, key_intensity: &str) -> Vec<PlannedDay> {
    [
        (1, key_session, 50, key_intensity),
        (3, "Easy run, 40 min", 40, "Z2"),
        (5, "Long run, 90 min", 90, "Z2"),
    ]
    .into_iter()
    .map(|(offset, workout, minutes, intensity)| PlannedDay {
        date: ymd(week_start + Days::new(offset)),
        sport: "run".to_owned(),
        workout: workout.to_owned(),
        duration_min: Some(minutes),
        intensity: intensity.to_owned(),
        steps: vec![],
        fueling: None,
        fuel_product_ids: Vec::new(),
        template_slug: None,
        template_params: None,
        template_source: None,
    })
    .collect()
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}
