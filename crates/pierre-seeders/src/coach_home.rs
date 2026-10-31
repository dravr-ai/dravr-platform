// ABOUTME: Seeds a human coach holding 45 athletes in three groups, every Coach Home attention reason and season state among them
// ABOUTME: Sharing or not, dead and quiet sources, a deep-fatigue block, drafts, a new member, ended plans, A races ahead, no plan
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach Home seeder (carnet#1252, #1250, #1253).
//!
//! Coach Home's queue, roster table and season view are walked live with a
//! coach holding more than 40 athletes. This seeder gives the coach
//! (`coachhome@pierre.dev` by default, granted `manages_roster`) three groups —
//! [`GROUP_NAMES`] — and 45 athletes whose states cover every attention
//! reason and both season sections:
//!
//! - **today**: two revoke their sharing; two get an
//!   agent reply in the group citing a figure their data does not hold;
//!   three lost their Strava connection eight days ago; three end a ten-day
//!   loading block just inside the deep-fatigue band;
//! - **this week**: four have gone 6 to 12 days without a session on live
//!   sources; two have only the coach's draft and one a draft over a live
//!   plan; three joined this week with no plan; two finished their plan with
//!   its A race and one ran out of plan after a C race;
//! - **calm** and **season**: the rest run a plan toward an A race ahead —
//!   a 70.3 or a full distance, a marathon or a fondo by group, two weeks to
//!   eight months out, some in transition after a race last week, some with a
//!   B or C race on the way — two a plan with no A race, two long-standing members have no plan at all,
//!   one shares with no source connected and one never shared (a standing
//!   state the row's status names, no reason).
//!
//! Each athlete with a source is seeded as a dev-fixture Strava athlete
//! (synthetic activities the fixture serves, its token) and their activity
//! cache is warmed with the same sessions, so the roster reads them at once.
//! Every athlete signs in with the demo password. The seeder creates the
//! coach too, as a plain account (never an admin, who would land on the admin
//! console). It is idempotent: a coach who already coaches the first group is
//! left as they are.
//!
//! ```bash
//! pierre-cli seed coach-home
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Days, Duration, NaiveDate, NaiveTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{GroupRole, NewGroupTranscriptEntry, TranscriptSpeaker};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{Activity, ActivityBuilder, AddMessageParams, SportType};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{
    DraftOwner, InsertClaimVerdictParams, PlanAuthor, PlanOutlineInput, PlanWeekInput,
    SaveDraftParams, SavePlanBundleParams,
};
use pierre_database::seed_models::SeedSyntheticActivity;
use pierre_database::RepositoryRegistry;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_memory::training_plans::{GoalRace, PlanPhase, RacePriority};
use pierre_middleware::mask_email;
use tracing::info;
use uuid::Uuid;

use crate::accounts::{account, create_account, hash_password, Account, NewAccount};
use crate::coached_group::{add_member, coaches_group_named, seed_coached_group, GroupSpec};
use crate::demo_data::DEMO_USER_PASSWORD;
use crate::synthetic_activities::register_provider_connection;

/// The coach's three groups.
pub const GROUP_NAMES: [&str; 3] = [
    "Tri — longue distance",
    "Course — marathon",
    "Route — Gran Fondo",
];

/// The provider every seeded source is.
const PROVIDER: &str = "strava";

/// Days of steady training before the latest stretch.
const BASE_DAYS: i64 = 55;

/// The days of the loading block a fatigue athlete ends on.
const FATIGUE_BLOCK_DAYS: i64 = 10;

/// What one seeded athlete's situation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    /// Training to plan toward an A race ahead.
    Calm,
    /// A plan running with no A race on it.
    NoARace,
    /// Revoked their sharing with the coach.
    Revoked,
    /// Never shared with the coach.
    NeverShared,
    /// An agent reply in the group cites a figure their data does not hold.
    UnmatchedFigure,
    /// Their Strava connection died eight days ago.
    DeadSource,
    /// A ten-day loading block on top of the base: just inside deep fatigue.
    Fatigue,
    /// No session for this many days, sources live.
    Quiet(i64),
    /// Only the coach's draft of a season.
    DraftOnly,
    /// A live plan and the coach's draft over it.
    DraftOverPlan,
    /// Joined this week, no plan.
    NewMember,
    /// A long-standing member with no plan.
    NoPlan,
    /// Their plan ended with its A race two weeks ago.
    RaceRun,
    /// Their plan ran out after a C race.
    PlanRanOut,
    /// Shares, no source connected.
    NoSource,
}

/// The 45 athletes, in order.
const SCENARIOS: [Scenario; 45] = {
    use Scenario::{
        Calm, DeadSource, DraftOnly, DraftOverPlan, Fatigue, NeverShared, NewMember, NoARace,
        NoPlan, NoSource, PlanRanOut, Quiet, RaceRun, Revoked, UnmatchedFigure,
    };
    [
        Fatigue,
        DeadSource,
        UnmatchedFigure,
        Revoked,
        DraftOnly,
        NewMember,
        Quiet(6),
        RaceRun,
        Calm,
        Calm,
        Calm,
        NoARace,
        NoPlan,
        Calm,
        Calm,
        Fatigue,
        DeadSource,
        UnmatchedFigure,
        Revoked,
        DraftOverPlan,
        NewMember,
        Quiet(9),
        PlanRanOut,
        Calm,
        Calm,
        Calm,
        NoSource,
        Calm,
        Calm,
        Calm,
        Fatigue,
        DeadSource,
        NeverShared,
        DraftOnly,
        NewMember,
        Quiet(12),
        Quiet(7),
        RaceRun,
        NoARace,
        NoPlan,
        Calm,
        Calm,
        Calm,
        Calm,
        Calm,
    ]
};

/// The athletes' names, one per scenario.
const NAMES: [&str; 45] = [
    "Léa Gagnon",
    "Julien Côté",
    "Félix Morin",
    "Maude Poirier",
    "Mathieu Roy",
    "Camille Bouchard",
    "Kevin Ouellet",
    "Olivier Fortin",
    "Sarah Nguyen",
    "Émilie Girard",
    "Thomas Bélanger",
    "Rosalie Dubé",
    "Noémie Lavoie",
    "Hugo Lemieux",
    "Antoine Pelletier",
    "Chloé Tremblay",
    "Samuel Gauthier",
    "Laurence Caron",
    "William Leblanc",
    "Juliette Cloutier",
    "Gabriel Paquette",
    "Florence Martel",
    "Raphaël Bergeron",
    "Alice Fournier",
    "Louis Lefebvre",
    "Charlotte Simard",
    "Nathan Boucher",
    "Béatrice Lapointe",
    "Xavier Dufour",
    "Clara Girouard",
    "Zoé Mercier",
    "Jacob Vachon",
    "Éloïse Rousseau",
    "Liam Dion",
    "Mia Bédard",
    "Adam Thibault",
    "Rose Lambert",
    "Elliot Nadeau",
    "Jeanne Hébert",
    "Arthur Lachance",
    "Lily Demers",
    "Victor Savard",
    "Anaïs Perron",
    "Léo Champagne",
    "Inès Ménard",
];

/// The coach's group at `index`.
fn group_spec(index: usize) -> GroupSpec<'static> {
    GroupSpec {
        name: GROUP_NAMES[index],
        description: "A squad Coach Home is walked with",
        agent_description: "The agent of a seeded Coach Home squad",
        agent_prompt: "You coach the athletes of this squad beside their human coach.",
        max_members: 30,
    }
}

/// CLI arguments for the Coach Home seeder.
#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    /// The human coach of the three groups
    #[arg(long, default_value = "coachhome@pierre.dev")]
    pub coach_email: String,

    /// The model the agent replies' conversations are created with.
    /// `pierre-cli` fills it, when not given, with the model every new
    /// conversation gets from the configured LLM provider
    #[arg(long)]
    pub model: Option<String>,
}

/// Seed the coach's three groups and their 45 athletes.
///
/// # Errors
///
/// Returns a configuration error when no model was resolved or the password
/// cannot be hashed, or the repository error from any write.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    let model = args.model.ok_or_else(|| {
        AppError::config(
            "No model for the seeded replies: pass --model, or configure the LLM provider the \
             server creates conversations with",
        )
    })?;
    let coach = coach_account(repos, &args.coach_email).await?;
    if coaches_group_named(repos, &coach, GROUP_NAMES[0]).await? {
        return Ok(());
    }
    repos.users.set_manages_roster(coach.id, true).await?;
    returning_coach(repos, &coach).await?;

    let mut groups = Vec::with_capacity(GROUP_NAMES.len());
    for index in 0..GROUP_NAMES.len() {
        groups.push(seed_coached_group(repos, &group_spec(index), &coach, &coach, &[]).await?);
    }
    let now = Utc::now();
    let password_hash = hash_password(DEMO_USER_PASSWORD)?;
    for (index, (scenario, name)) in SCENARIOS.iter().zip(NAMES).enumerate() {
        let athlete = athlete_account(repos, index, name, &password_hash).await?;
        let group = &groups[index % groups.len()];
        let seeded = Seeded {
            repos,
            athlete: &athlete,
            group_id: group.id,
            coach: &coach,
            slot: Slot {
                group: index % groups.len(),
                index,
            },
            now,
        };
        seeded.seed(*scenario, index, &model).await?;
    }
    info!(
        "Seeded {} athletes in {} groups coached by {}",
        SCENARIOS.len(),
        groups.len(),
        mask_email(&coach.email)
    );
    Ok(())
}

/// The seeded coach's dev-fixture password, as pinned by the web e2e and the
/// Maestro walk; never a real credential.
const COACH_PASSWORD: &str = "CoachHome1234";

/// The coach's account, created when a previous run has not. It is a plain
/// athlete account with `manages_roster`, never an admin: an admin lands on
/// the admin console, not on Coach Home.
async fn coach_account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
    if repos.seeder.seed_check_user_exists(email).await?.is_none() {
        create_account(
            repos,
            &NewAccount {
                email,
                display_name: "Coach Home",
                password_hash: &hash_password(COACH_PASSWORD)?,
                tier: "professional",
                status: "active",
            },
        )
        .await?;
    }
    account(repos, email).await
}

/// Onboarding steps a returning coach who also trains has answered: the
/// profile question, the coach's group, and the athlete steps put off. The
/// agent proposal is answered by the provider connection the setup script
/// seeds next (`seed synthetic-activities`).
const RETURNING_COACH_STEPS: [(&str, &str); 6] = [
    ("profile_type", "complete"),
    ("about_you", "skipped"),
    ("parq", "skipped"),
    ("check_in", "skipped"),
    ("coach_group", "complete"),
    ("messaging_channel", "skipped"),
];

/// Make the coach a returning one, so a sign-in lands on Coach Home rather
/// than on the onboarding a new account meets.
async fn returning_coach(repos: &RepositoryRegistry, coach: &Account) -> AppResult<()> {
    repos.users.set_coaches_others(coach.id, true).await?;
    let (user, tenant) = (coach.id.to_string(), coach.tenant.to_string());
    for (step, status) in RETURNING_COACH_STEPS {
        repos
            .user_onboarding
            .set_onboarding_step(&user, step, status, None, Some(&tenant))
            .await?;
    }
    Ok(())
}

/// The athlete account at `index`, created when a previous run has not.
async fn athlete_account(
    repos: &RepositoryRegistry,
    index: usize,
    name: &str,
    password_hash: &str,
) -> AppResult<Account> {
    let email = format!("coach-home-{index:02}@pierre.dev");
    if repos.seeder.seed_check_user_exists(&email).await?.is_none() {
        create_account(
            repos,
            &NewAccount {
                email: &email,
                display_name: name,
                password_hash,
                tier: "professional",
                status: "active",
            },
        )
        .await?;
    }
    account(repos, &email).await
}

/// One athlete being seeded into one of the coach's groups.
struct Seeded<'a> {
    repos: &'a RepositoryRegistry,
    athlete: &'a Account,
    group_id: Uuid,
    coach: &'a Account,
    slot: Slot,
    now: DateTime<Utc>,
}

impl Seeded<'_> {
    async fn seed(&self, scenario: Scenario, index: usize, model: &str) -> AppResult<()> {
        let shares = scenario != Scenario::NeverShared;
        add_member(
            self.repos,
            self.group_id,
            self.athlete,
            GroupRole::Member,
            shares,
        )
        .await?;
        // Joined weeks apart, the new members this week.
        let joined_days_ago = if scenario == Scenario::NewMember {
            3
        } else {
            30 + i64::try_from(index).unwrap_or(0)
        };
        self.repos
            .seeder
            .seed_backdate_membership(
                &self.athlete.tenant.to_string(),
                self.group_id,
                self.athlete.id,
                self.now - Duration::days(joined_days_ago),
            )
            .await?;
        if scenario != Scenario::NoSource {
            self.seed_sessions(&sessions(scenario)).await?;
        }
        self.seed_situation(scenario, model).await
    }

    async fn seed_situation(&self, scenario: Scenario, model: &str) -> AppResult<()> {
        let today = self.now.date_naive();
        match scenario {
            Scenario::Revoked => {
                self.repos
                    .groups
                    .update_coach_sharing_consent(
                        &self.group_id.to_string(),
                        self.athlete.id,
                        false,
                    )
                    .await?;
            }
            Scenario::DeadSource => {
                self.repos
                    .provider_connections
                    .mark_needs_reauth(
                        self.athlete.id,
                        self.athlete.tenant,
                        PROVIDER,
                        Some("invalid_grant"),
                        // After the connection seed_sessions just made.
                        Utc::now(),
                    )
                    .await?;
                self.live_plan(&a_race_season(self.slot, today)).await?;
            }
            Scenario::UnmatchedFigure => {
                self.agent_reply(model).await?;
                self.live_plan(&a_race_season(self.slot, today)).await?;
            }
            Scenario::Calm | Scenario::Fatigue | Scenario::Quiet(_) => {
                self.live_plan(&a_race_season(self.slot, today)).await?;
            }
            Scenario::NoARace => self.live_plan(&b_race_season(self.slot, today)).await?,
            Scenario::RaceRun => {
                self.live_plan(&ended_season(self.slot, today, RacePriority::A))
                    .await?;
            }
            Scenario::PlanRanOut => {
                self.live_plan(&ended_season(self.slot, today, RacePriority::C))
                    .await?;
            }
            Scenario::DraftOnly => {
                self.draft(Some(&a_race_season(self.slot, today)), today)
                    .await?;
            }
            Scenario::DraftOverPlan => {
                self.live_plan(&a_race_season(self.slot, today)).await?;
                self.draft(None, today).await?;
            }
            Scenario::NeverShared | Scenario::NewMember | Scenario::NoPlan | Scenario::NoSource => {
            }
        }
        Ok(())
    }

    /// The sessions the dev fixture serves, the Strava connection and token
    /// it reads them through, and the activity cache warmed with them.
    async fn seed_sessions(&self, sessions: &[(i64, u64)]) -> AppResult<()> {
        let tenant = self.athlete.tenant.as_uuid();
        let mut cached: Vec<Activity> = Vec::with_capacity(sessions.len());
        let offset = session_offset(self.now);
        for &(days_ago, seconds) in sessions {
            let id = Uuid::new_v4();
            let start = self.now - Duration::days(days_ago) - offset;
            #[allow(clippy::cast_precision_loss)]
            let metres = seconds as f64 * 30_000.0 / 3_600.0;
            let duration = i64::try_from(seconds).unwrap_or(i64::MAX);
            self.repos
                .seeder
                .seed_insert_synthetic_activity(&SeedSyntheticActivity {
                    id,
                    user_id: self.athlete.id,
                    tenant_id: tenant,
                    name: "Sortie vélo".to_owned(),
                    sport_type: "ride".to_owned(),
                    start_date: start,
                    duration_seconds: duration,
                    distance_meters: Some(metres),
                    elevation_gain: None,
                    average_heart_rate: 140,
                    max_heart_rate: 165,
                    average_speed: Some(30_000.0 / 3_600.0),
                    max_speed: None,
                    calories: None,
                    city: "Montreal".to_owned(),
                    region: "Quebec".to_owned(),
                    country: "Canada".to_owned(),
                    start_latitude: None,
                    start_longitude: None,
                    created_at: self.now,
                    updated_at: self.now,
                })
                .await?;
            cached.push(
                ActivityBuilder::new(
                    id.to_string(),
                    "Sortie vélo".to_owned(),
                    SportType::Ride,
                    start,
                    seconds,
                    PROVIDER.to_owned(),
                )
                .distance_meters(metres)
                .average_heart_rate(140)
                .build(),
            );
        }
        register_provider_connection(self.repos, self.athlete.id, tenant, PROVIDER).await?;
        self.repos
            .activity_cache
            .upsert_activities(self.athlete.id, &self.athlete.tenant, PROVIDER, &cached)
            .await?;
        Ok(())
    }

    async fn live_plan(&self, season: &Season) -> AppResult<()> {
        let (tenant, athlete) = (self.athlete.tenant.to_string(), self.athlete.id.to_string());
        self.repos
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &tenant,
                user_id: &athlete,
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(season.outline()),
                weeks: &[],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        Ok(())
    }

    /// The coach's draft: a season of its own (`season`), or one week over
    /// the athlete's live plan.
    async fn draft(&self, season: Option<&Season>, today: NaiveDate) -> AppResult<()> {
        let (tenant, athlete, coach) = (
            self.athlete.tenant.to_string(),
            self.athlete.id.to_string(),
            self.coach.id.to_string(),
        );
        let week_start = ymd(today);
        let weeks = [PlanWeekInput {
            week_start: &week_start,
            focus: "base",
            days: &[],
            adjustment_reason: "coach draft",
            phase_index: None,
        }];
        self.repos
            .training_plans
            .save_draft_bundle(&SaveDraftParams {
                owner: DraftOwner {
                    tenant_id: &tenant,
                    user_id: &athlete,
                    coach_user_id: &coach,
                },
                author: PlanAuthor::none(),
                goal_fact_id: None,
                outline: season.map(Season::outline),
                weeks: if season.is_some() { &[] } else { &weeks },
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        Ok(())
    }

    /// An agent reply to the athlete in the group, citing a power figure the
    /// checker could not match to their data.
    async fn agent_reply(&self, model: &str) -> AppResult<()> {
        const REPLY: &str = "Ton seuil est à 285 W : vise 240 W pour les blocs de samedi.";
        const FIGURE: &str = "ton seuil est à 285 W";
        let repos = self.repos;
        let user = self.athlete.id.to_string();
        let conversation = repos
            .chat
            .create_conversation(
                &user,
                self.athlete.tenant,
                "Groupe",
                model,
                None,
                Some(&self.group_id.to_string()),
            )
            .await?;
        let message = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: self.athlete.tenant,
                conversation_id: &conversation.id,
                user_id: &user,
                role: "assistant",
                content: REPLY,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await?;
        repos
            .groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &self.group_id.to_string(),
                tenant_id: &self.athlete.tenant.to_string(),
                author_user_id: self.athlete.id,
                speaker: TranscriptSpeaker::Coach,
                content: REPLY,
                source_conversation_id: Some(&conversation.id),
                source_message_id: Some(&message.id),
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        repos
            .claim_verdicts
            .insert_claim_verdict(&InsertClaimVerdictParams {
                tenant_id: self.athlete.tenant,
                user_id: &user,
                agent_id: None,
                conversation_id: Some(&conversation.id),
                message_id: Some(&message.id),
                claim_text: FIGURE,
                category: ClaimCategory::AthleteData,
                status: ClaimStatus::Unverifiable,
                evidence_strength: EvidenceStrength::None,
                confidence: 0.0,
                layer_fired: VerdictLayer::AthleteData,
                explanation: None,
                evidence_refs: None,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        Ok(())
    }
}

/// How long before `now`'s time of day a seeded session starts: three
/// hours, but never back across midnight UTC. A session `n` days ago is then
/// both `n` whole days old (the roster's days without a session) and on the
/// calendar day `n` days back (the day the training load counts it on), at
/// any hour the seeder runs.
fn session_offset(now: DateTime<Utc>) -> Duration {
    let since_midnight = now - now.date_naive().and_time(NaiveTime::MIN).and_utc();
    since_midnight.min(Duration::hours(3))
}

/// The sessions a scenario trains: `(days ago, seconds)`.
fn sessions(scenario: Scenario) -> Vec<(i64, u64)> {
    match scenario {
        // An hour a day, then ten days of 1 h 40: form ends just inside the
        // deep-fatigue band (about -31 % to -33 % of their own fitness) for
        // the last four mornings, measured against the snapshot's own load
        // calculation; the band's edge is cageux's.
        Scenario::Fatigue => (FATIGUE_BLOCK_DAYS + 1..=BASE_DAYS)
            .map(|day| (day, 3_600))
            .chain((1..=FATIGUE_BLOCK_DAYS).map(|day| (day, 6_000)))
            .collect(),
        // Nothing since the connection died.
        Scenario::DeadSource => every_other_day(8),
        Scenario::Quiet(days) => every_other_day(days),
        _ => every_other_day(1),
    }
}

/// An hour every other day from `latest` days ago back through the base.
fn every_other_day(latest: i64) -> Vec<(i64, u64)> {
    (latest..=BASE_DAYS)
        .step_by(2)
        .map(|day| (day, 3_600))
        .collect()
}

/// A season: its goal race, the races beside it and its phases.
struct Season {
    goal: GoalRace,
    races: Vec<GoalRace>,
    phases: Vec<PlanPhase>,
}

impl Season {
    fn outline(&self) -> PlanOutlineInput<'_> {
        PlanOutlineInput {
            goal_race: &self.goal,
            races: Some(&self.races),
            strategy: "Construire la base, puis l'intensité, puis affûter.",
            phases: &self.phases,
            source_conversation_id: None,
            flavour: None,
            season_start: None,
            season_end: None,
        }
    }
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn shifted(today: NaiveDate, days: i64) -> NaiveDate {
    let magnitude = Days::new(days.unsigned_abs());
    if days < 0 {
        today.checked_sub_days(magnitude)
    } else {
        today.checked_add_days(magnitude)
    }
    .unwrap_or(today)
}

fn phase(kind: PhaseKind, start: NaiveDate, weeks: u8) -> PlanPhase {
    PlanPhase {
        kind,
        start: ymd(start),
        weeks,
        purpose: String::new(),
        intent: "Le bloc de la saison.".to_owned(),
        target_hours: None,
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: None,
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    }
}

/// Phases laid back to back from `start`; a zero-week block is left out.
fn phases(start: NaiveDate, blocks: &[(PhaseKind, u8)]) -> Vec<PlanPhase> {
    let mut at = start;
    blocks
        .iter()
        .filter(|(_, weeks)| *weeks > 0)
        .map(|&(kind, weeks)| {
            let laid = phase(kind, at, weeks);
            at = shifted(at, i64::from(weeks) * 7);
            laid
        })
        .collect()
}

/// What each group trains for.
struct Discipline {
    /// The `discipline` its races carry.
    name: &'static str,
    /// The A races its athletes aim at.
    a_races: [&'static str; 4],
    /// The tune-up races beside them.
    tune_ups: [&'static str; 2],
}

/// One discipline per group, in [`GROUP_NAMES`] order.
const DISCIPLINES: [Discipline; 3] = [
    Discipline {
        name: "triathlon",
        a_races: [
            "IM 70.3 Indian Wells",
            "Ironman Cozumel",
            "Ironman Lake Placid",
            "IM 70.3 Mont-Tremblant",
        ],
        tune_ups: ["Triathlon de Magog", "Duathlon de Granby"],
    },
    Discipline {
        name: "marathon",
        a_races: [
            "Marathon de Philadelphie",
            "Marathon de Boston",
            "Marathon de Montréal",
            "Marathon de Québec",
        ],
        tune_ups: ["Demi-marathon de Lévis", "10 km de Laval"],
    },
    Discipline {
        name: "road",
        a_races: [
            "Gran Fondo Mont-Tremblant",
            "Paris-Roubaix Challenge",
            "L'Étape du Tour",
            "Gran Fondo Whistler",
        ],
        tune_ups: ["Classique de Charlevoix", "Critérium de Sherbrooke"],
    },
];

/// Weeks from today to the A race, spread from three weeks to eight months
/// so different phases are current: the shortest are in their taper.
const WEEKS_OUT: [i64; 11] = [2, 3, 5, 8, 11, 14, 18, 22, 26, 30, 34];

/// A seeded athlete's place: their group and their position on the roster.
#[derive(Debug, Clone, Copy)]
struct Slot {
    group: usize,
    index: usize,
}

impl Slot {
    fn discipline(self) -> &'static Discipline {
        &DISCIPLINES[self.group % DISCIPLINES.len()]
    }

    fn race(self, name: &str, date: NaiveDate, priority: RacePriority) -> GoalRace {
        GoalRace {
            name: name.to_owned(),
            date: ymd(date),
            discipline: self.discipline().name.to_owned(),
            priority,
        }
    }

    /// The athlete's A race, picked by their position.
    fn a_race_name(self) -> &'static str {
        let races = &self.discipline().a_races;
        races[(self.index / DISCIPLINES.len()) % races.len()]
    }
}

/// The blocks of a `total`-week build to a race, laid from the race back:
/// two weeks of taper, up to four of specialty, up to six of build, the rest
/// of base.
fn build_to_race(total: i64) -> [(PhaseKind, u8); 4] {
    let mut left = total.max(0);
    let mut take = |most: i64| {
        let weeks = left.min(most);
        left -= weeks;
        u8::try_from(weeks).unwrap_or(u8::MAX)
    };
    let taper = take(2);
    let specialty = take(4);
    let build = take(6);
    let base = take(i64::from(u8::MAX));
    [
        (PhaseKind::Base, base),
        (PhaseKind::Build, build),
        (PhaseKind::Specialty, specialty),
        (PhaseKind::Taper, taper),
    ]
}

/// A plan toward an A race between two weeks and eight months out, entered a
/// few weeks ago. One athlete in nine is in transition after an A race run
/// last week; one in six has a B race on the way, one in seven a C race.
fn a_race_season(slot: Slot, today: NaiveDate) -> Season {
    let index = i64::try_from(slot.index).unwrap_or(0);
    let weeks_out = WEEKS_OUT[slot.index % WEEKS_OUT.len()];
    let race_day = shifted(today, weeks_out * 7 - 1);
    let goal = slot.race(slot.a_race_name(), race_day, RacePriority::A);
    let tune_up = slot.discipline().tune_ups[slot.index % 2];
    let mut races = Vec::new();

    let (start, mut blocks) = if slot.index % 9 == 4 && weeks_out >= 4 {
        // Raced an A race eight days ago; two weeks of transition, then the
        // build to the next.
        let past = slot.discipline().a_races[(slot.index + 1) % 4];
        races.push(slot.race(past, shifted(today, -8), RacePriority::A));
        let start = shifted(today, -7);
        let mut blocks = vec![(PhaseKind::Transition, 2)];
        // From the transition's end, a week from now, to the race.
        blocks.extend(build_to_race(weeks_out - 1));
        (start, blocks)
    } else {
        let elapsed = 2 + index % 5;
        (
            shifted(today, -elapsed * 7),
            build_to_race(weeks_out + elapsed).to_vec(),
        )
    };
    if slot.index.is_multiple_of(6) && weeks_out > 6 {
        races.push(slot.race(
            tune_up,
            shifted(today, (weeks_out / 2) * 7 + 5),
            RacePriority::B,
        ));
    }
    if slot.index % 7 == 3 && weeks_out > 4 {
        races.push(slot.race(tune_up, shifted(today, 10), RacePriority::C));
    }
    races.sort_by(|a, b| a.date.cmp(&b.date));
    blocks.retain(|(_, weeks)| *weeks > 0);
    Season {
        goal,
        races,
        phases: phases(start, &blocks),
    }
}

/// A base and a build toward a B race, no A race.
fn b_race_season(slot: Slot, today: NaiveDate) -> Season {
    let start = shifted(today, -7);
    let tune_up = slot.discipline().tune_ups[slot.index % 2];
    Season {
        goal: slot.race(tune_up, shifted(start, 8 * 7 - 1), RacePriority::B),
        races: Vec::new(),
        phases: phases(start, &[(PhaseKind::Base, 4), (PhaseKind::Build, 4)]),
    }
}

/// A ten-week plan that ended two weeks ago with its goal race.
fn ended_season(slot: Slot, today: NaiveDate, priority: RacePriority) -> Season {
    let start = shifted(today, -12 * 7);
    let name = if priority == RacePriority::A {
        slot.a_race_name()
    } else {
        slot.discipline().tune_ups[0]
    };
    Season {
        goal: slot.race(name, shifted(start, 10 * 7 - 1), priority),
        races: Vec::new(),
        phases: phases(start, &[(PhaseKind::Base, 6), (PhaseKind::Build, 4)]),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_reason_and_season_state_has_athletes_and_the_roster_passes_forty() {
        assert!(SCENARIOS.len() > 40);
        for wanted in [
            Scenario::Revoked,
            Scenario::NeverShared,
            Scenario::UnmatchedFigure,
            Scenario::DeadSource,
            Scenario::Fatigue,
            Scenario::DraftOnly,
            Scenario::DraftOverPlan,
            Scenario::NewMember,
            Scenario::NoPlan,
            Scenario::RaceRun,
            Scenario::PlanRanOut,
            Scenario::NoARace,
            Scenario::NoSource,
            Scenario::Calm,
        ] {
            assert!(SCENARIOS.contains(&wanted), "{wanted:?} has no athlete");
        }
        assert!(SCENARIOS.iter().any(|s| matches!(s, Scenario::Quiet(_))));
    }

    #[test]
    fn the_seasons_end_where_their_scenario_needs() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let slot = Slot { group: 2, index: 7 };
        let ended = ended_season(slot, today, RacePriority::A);
        let last = ended.phases.last().unwrap().end_exclusive().unwrap();
        assert!(last <= today, "the ended plan is over: {last}");
        assert_eq!(ended.goal.date, ymd(last.pred_opt().unwrap()));
    }

    #[test]
    fn a_race_plans_vary_and_each_ends_on_its_race_with_today_inside() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let mut goals = BTreeSet::new();
        let mut current = BTreeSet::new();
        for index in 0..SCENARIOS.len() {
            let slot = Slot {
                group: index % GROUP_NAMES.len(),
                index,
            };
            let season = a_race_season(slot, today);
            let race_day = NaiveDate::parse_from_str(&season.goal.date, "%Y-%m-%d").unwrap();
            assert!(race_day > today, "{index}: the A race is ahead");
            assert!(
                race_day <= shifted(today, 8 * 31),
                "{index}: within eight months"
            );
            let end = season.phases.last().unwrap().end_exclusive().unwrap();
            assert_eq!(
                end,
                race_day.succ_opt().unwrap(),
                "{index}: ends on the race"
            );
            let now = season
                .phases
                .iter()
                .find(|p| p.covers(today))
                .unwrap_or_else(|| panic!("{index}: no phase covers today"));
            current.insert(now.kind.as_str());
            goals.insert((season.goal.name.clone(), season.goal.date.clone()));
            assert_eq!(
                season.goal.discipline, DISCIPLINES[slot.group].name,
                "{index}: the group's discipline"
            );
        }
        assert!(goals.len() > 20, "plans differ: {} distinct", goals.len());
        for kind in ["base", "build", "specialty", "taper", "transition"] {
            assert!(
                current.contains(kind),
                "{kind} is current for someone: {current:?}"
            );
        }
    }
}
