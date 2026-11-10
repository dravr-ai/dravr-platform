// ABOUTME: The injection eval's per-scenario fixture: scraper stand-in, athlete, peer, group, catalogue coach, turn
// ABOUTME: A submodule of injection_corpus.rs, named by its callers as `injection_corpus::fixture`
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Everything one scenario attempt runs against, seeded fresh, and the web
//! turn that asks its question.

use crate::helpers::contremaitre_checkout::contremaitre_tree;
use crate::helpers::live_seed::{
    create_athlete, create_group, day_at, seed_onboarded, seed_recipes, weekly_slots,
    AthleteProfile, NewGroup,
};
use crate::helpers::sciotte_mock::seed_scrape_session;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnOrigin, TurnRequest,
};
use pierre_core::config::fitness::{FitnessConfig, IntelligenceConfig};
use pierre_core::models::groups::GroupRole;
use pierre_core::models::ConversationTurnId;
use pierre_core::models::{
    Activity, ActivityBuilder, ActivityComment, ConnectionType, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_seeders::agents::{self as agent_seeder, SeedArgs as AgentSeedArgs};
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_services::conversation_forge::selected_or_system_agent;
use pierre_services::default_agent::group_default;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::{
    Scenario, Surface, ATHLETE_CONNECTIONS, ATHLETE_DISPLAY, FITNESS_CONFIG_NAME, PEER_DISPLAY,
    PLAIN_RUN_NAME, SAVED_RECIPES, SEEDED_FTP_WATTS, SEEDED_MAX_HR,
};

// -----------------------------------------------------------------------
// Scraper stand-in
// -----------------------------------------------------------------------

/// The activity lists the scraper stand-in serves, by scrape session id.
///
/// One stand-in serves every fixture; each athlete's seeded session carries its
/// own id, which the provider sends as `X-Session-Id` on every scrape, so each
/// fixture reads its own history and nobody else's payload.
#[derive(Clone, Default)]
pub(crate) struct ScraperBook {
    lists: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    plans: Arc<Mutex<HashMap<String, Vec<Value>>>>,
}

impl ScraperBook {
    fn serve(&self, session_id: &str, activities: Vec<Value>) {
        self.lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session_id.to_owned(), activities);
    }

    fn plan(&self, session_id: &str, workouts: Vec<Value>) {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session_id.to_owned(), workouts);
    }

    fn list(&self, headers: &HeaderMap) -> Vec<Value> {
        Self::read(&self.lists, headers)
    }

    fn planned(&self, headers: &HeaderMap) -> Vec<Value> {
        Self::read(&self.plans, headers)
    }

    fn read(map: &Mutex<HashMap<String, Vec<Value>>>, headers: &HeaderMap) -> Vec<Value> {
        let session = headers
            .get("x-session-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        map.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
            .unwrap_or_default()
    }
}

/// Spawn the scraper stand-in; returns its base URL and the book it serves.
pub(crate) async fn spawn_injection_scraper() -> (String, ScraperBook) {
    let book = ScraperBook::default();
    let app =
        Router::new()
            .route(
                "/auth/import-session",
                post(|Json(body): Json<Value>| async move {
                    Json(json!({ "session_id": body["session"]["session_id"] }))
                }),
            )
            .route(
                "/api/athlete",
                get(|| async { Json(json!({ "display_name": ATHLETE_DISPLAY })) }),
            )
            .route(
                "/api/activities",
                get(
                    |State(book): State<ScraperBook>, headers: HeaderMap| async move {
                        let activities = book.list(&headers);
                        Json(json!({
                            "count": activities.len(),
                            "activities": activities,
                            "head_complete": true,
                        }))
                    },
                ),
            )
            .route(
                "/api/activities/{id}",
                get(
                    |State(book): State<ScraperBook>,
                     headers: HeaderMap,
                     Path(id): Path<String>| async move {
                        let found = book
                            .list(&headers)
                            .into_iter()
                            .find(|a| a["id"] == id.as_str())
                            .unwrap_or(Value::Null);
                        Json(found)
                    },
                ),
            )
            .route(
                "/api/planned-workouts",
                get(
                    |State(book): State<ScraperBook>, headers: HeaderMap| async move {
                        let workouts = book.planned(&headers);
                        Json(json!({ "count": workouts.len(), "planned_workouts": workouts }))
                    },
                ),
            )
            .with_state(book.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), book)
}

// -----------------------------------------------------------------------
// Fixture
// -----------------------------------------------------------------------

/// One seeded run, as both the scraper and the cache carry it.
struct SeededRun {
    id: String,
    name: String,
    start: DateTime<Utc>,
    seconds: u64,
    metres: f64,
    note: Option<String>,
}

impl SeededRun {
    fn scraper_json(&self) -> Value {
        let mut row = json!({
            "id": self.id,
            "name": self.name,
            "sport_type": "run",
            "start_date": self.start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "duration_seconds": self.seconds,
            "provider": "strava",
            "distance_meters": self.metres,
        });
        if let Some(note) = &self.note {
            row["comments"] = json!([{ "author": "athlete", "text": note }]);
        }
        row
    }

    fn activity(&self) -> Activity {
        let builder = ActivityBuilder::new(
            self.id.clone(),
            self.name.clone(),
            SportType::Run,
            self.start,
            self.seconds,
            "strava",
        )
        .distance_meters(self.metres);
        match &self.note {
            Some(note) => builder
                .comments(vec![ActivityComment {
                    author: Some("athlete".to_owned()),
                    text: note.clone(),
                    created_at: None,
                }])
                .build(),
            None => builder.build(),
        }
    }
}

/// Four weeks of runs on `days` of each week; yesterday's is named
/// `plant_name` and carries the note `plant_note` when given. Every other run
/// reads «Course».
fn runs(
    prefix: &str,
    now: DateTime<Utc>,
    days: &[(i64, u32, u32)],
    plant_name: Option<&str>,
    plant_note: Option<&str>,
) -> Vec<SeededRun> {
    let mut out = vec![SeededRun {
        id: format!("{prefix}-yesterday"),
        name: plant_name.unwrap_or(PLAIN_RUN_NAME).to_owned(),
        start: day_at(now, 1, 18, 5),
        seconds: 3_300,
        metres: 10_000.0,
        note: plant_note.map(str::to_owned),
    }];
    out.extend(
        weekly_slots(now, 4, days, 0, |days_ago| days_ago == 1)
            .into_iter()
            .map(|slot| SeededRun {
                id: format!("{prefix}-{}-{}", slot.week, slot.day),
                name: PLAIN_RUN_NAME.to_owned(),
                start: slot.start,
                seconds: 3_000 + u64::try_from(slot.day).unwrap_or(0) * 300,
                metres: (slot.day as f64).mul_add(1_000.0, 9_000.0),
                note: None,
            }),
    );
    out
}

/// Everything one scenario attempt runs against, seeded fresh.
pub(crate) struct InjectionFixture {
    pub athlete: Uuid,
    pub tenant: TenantId,
    /// The catalogue agent the question is answered by: the deployed default
    /// for the athlete's own thread, or the group's default in its thread.
    pub agent: Uuid,
    /// That agent's catalogue handle, the key its prompt is served under.
    pub agent_handle: String,
    /// Whether that agent declares a first-turn activity window, which is what
    /// puts the athlete's own activity names in the first request unasked.
    pub declares_window: bool,
    /// The conversation the question is asked in: the athlete's own, or the
    /// group's thread for a peer scenario.
    pub conversation_id: String,
    /// Whether [`Self::conversation_id`] is a direct conversation.
    pub direct: bool,
}

/// Seed a fresh athlete, peer, group and conversation for `scenario`.
///
/// Unique emails, tenant and scrape session per call, so one server can host
/// every attempt of every scenario and none reads another's data or payload.
/// `model` is the model the conversation is created under.
pub(crate) async fn seed_injection_fixture(
    resources: &Arc<ServerContext>,
    book: &ScraperBook,
    scenario: &Scenario,
    model: &str,
) -> InjectionFixture {
    let tag = Uuid::new_v4().simple().to_string();
    let now = Utc::now();
    let (athlete, tenant) = create_athlete(
        resources,
        &INJECTION_ATHLETE,
        &format!("injection-athlete-{tag}@dravr.test"),
        ATHLETE_DISPLAY,
    )
    .await;
    let (peer, _) = create_athlete(
        resources,
        &INJECTION_ATHLETE,
        &format!("injection-peer-{tag}@dravr.test"),
        PEER_DISPLAY,
    )
    .await;

    let payload = scenario.payload;
    let own_runs = runs(
        &format!("inj-{tag}"),
        now,
        &[(1, 18, 5), (3, 6, 40), (5, 12, 15)],
        payload.filter(|_| scenario.surface == Surface::OwnActivityName),
        payload.filter(|_| scenario.surface == Surface::ActivityNote),
    );
    let peer_runs = runs(
        &format!("inj-peer-{tag}"),
        now,
        &[(2, 7, 20), (4, 17, 45)],
        payload.filter(|_| scenario.surface == Surface::PeerActivityName),
        None,
    );

    // The athlete's history is served live by the scraper stand-in under this
    // fixture's own session, and cached identically, so a read through either
    // path sees the same rows.
    let session_id = format!("injection-{tag}");
    book.serve(
        &session_id,
        own_runs.iter().map(SeededRun::scraper_json).collect(),
    );
    // A TrainingPeaks calendar only for a planned-workout scenario, registered
    // first so the sciotte Strava capture stays the activity primary. Its
    // session serves no activities, only the plan.
    let planned = scenario.surface == Surface::PlannedWorkout;
    if planned {
        let plan_session = format!("injection-plan-{tag}");
        book.plan(
            &plan_session,
            vec![planned_workout(
                &tag,
                now,
                payload.unwrap_or("Easy 45 min."),
            )],
        );
        seed_scrape_session(resources, athlete, tenant, PLAN_CONNECTION, &plan_session).await;
    }
    seed_scrape_session(resources, athlete, tenant, "sciotte", &session_id).await;

    let registrations = planned
        .then_some((athlete, PLAN_CONNECTION))
        .into_iter()
        .chain(
            ATHLETE_CONNECTIONS
                .iter()
                .map(|provider| (athlete, *provider)),
        )
        .chain([(peer, "strava")]);
    for (user, provider) in registrations {
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user, tenant, provider, &ConnectionType::Manual, None)
            .await
            .unwrap();
    }

    let cache = &resources.common.repos.activity_cache;
    let own: Vec<Activity> = own_runs.iter().map(SeededRun::activity).collect();
    cache
        .upsert_activities(athlete, &tenant, "strava", &own)
        .await
        .unwrap();
    let theirs: Vec<Activity> = peer_runs.iter().map(SeededRun::activity).collect();
    cache
        .upsert_activities(peer, &tenant, "strava", &theirs)
        .await
        .unwrap();

    seed_recipes(resources, athlete, tenant, SAVED_RECIPES).await;
    seed_fitness_config(resources, athlete, tenant).await;
    seed_physiology(resources, athlete, tenant).await;

    // The agents an athlete actually gets, resolved the way production
    // resolves them: the athlete's own thread through the DM default
    // (`selected_or_system_agent`: no selection, so the first athlete-facing
    // catalogue agent), the group through `group_default`. Onboarding is
    // recorded first because the DM default reads it.
    seed_onboarded(resources, athlete, tenant).await;
    let repos = &resources.common.repos;
    let system = repos.agents.list_catalogue_system_agents().await.unwrap();
    let dm_agent = selected_or_system_agent(repos, tenant, athlete)
        .await
        .expect("the catalogue holds an athlete-facing default agent; seed it with seed_catalogue");
    let group_agent = group_default(&system, None).expect("the catalogue holds a group default");
    let group = create_group(
        resources,
        &NewGroup {
            tenant,
            name: "Injection Squad",
            agent: Uuid::parse_str(&group_agent).unwrap(),
            owner: athlete,
            channel: None,
            members: &[
                (athlete, GroupRole::Owner, ATHLETE_DISPLAY),
                (peer, GroupRole::Member, PEER_DISPLAY),
            ],
        },
    )
    .await;

    let direct = !scenario.surface.group_thread();
    let agent_id = if direct { dm_agent } else { group_agent };
    let agent = system
        .iter()
        .find(|a| a.id.to_string() == agent_id)
        .unwrap_or_else(|| panic!("agent {agent_id} is not a catalogue system agent"));
    let group_id = group.to_string();
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &athlete.to_string(),
            tenant,
            "Injection eval",
            model,
            Some(&agent_id),
            (!direct).then_some(group_id.as_str()),
        )
        .await
        .unwrap()
        .id;

    InjectionFixture {
        athlete,
        tenant,
        agent: agent.id,
        agent_handle: agent.handle.clone().unwrap_or_default(),
        declares_window: agent
            .data_requirements
            .as_ref()
            .is_some_and(|d| d.activities.is_some()),
        conversation_id,
        direct,
    }
}

/// Every athlete a fixture seeds: a plain user,
/// the cheapest password hash bcrypt accepts (nobody signs in as them, and a
/// default-cost hash per attempt in a debug build is seconds each), a tenant
/// named «Injection …».
const INJECTION_ATHLETE: AthleteProfile<'static> = AthleteProfile {
    role: UserRole::User,
    bcrypt_cost: 4,
    tenant_label: "Injection",
    tenant_slug_prefix: "injection",
};

async fn seed_fitness_config(resources: &Arc<ServerContext>, athlete: Uuid, tenant: TenantId) {
    let config = FitnessConfig {
        sport_types: HashMap::from([("run".to_owned(), "Running".to_owned())]),
        intelligence: IntelligenceConfig::default(),
        weather_api: None,
    };
    resources
        .common
        .repos
        .fitness_config
        .save_user_config(tenant, &athlete.to_string(), FITNESS_CONFIG_NAME, &config)
        .await
        .unwrap();
}

async fn seed_physiology(resources: &Arc<ServerContext>, athlete: Uuid, tenant: TenantId) {
    let mut profile = UserPhysiologicalProfile::new(athlete, SportType::Run);
    profile.ftp_watts = Some(SEEDED_FTP_WATTS);
    profile.max_hr = Some(SEEDED_MAX_HR);
    profile.resting_hr = Some(48);
    profile.weight = Some(72.0);
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, athlete, &profile)
        .await
        .unwrap();
}

/// The connection a planned-workout fixture reads its plan through.
const PLAN_CONNECTION: &str = "sciotte_trainingpeaks";

/// One workout planned for tomorrow on the athlete's `TrainingPeaks`
/// calendar, as the scraper serves it, its description `description`.
fn planned_workout(tag: &str, now: DateTime<Utc>, description: &str) -> Value {
    json!({
        "id": format!("900001:{}", 910_000 + tag.len()),
        "date": (now + ChronoDuration::days(1)).format("%Y-%m-%d").to_string(),
        "sport_type": "run",
        "title": "Footing",
        "description": description,
        "planned_duration_seconds": 2_700,
    })
}

/// Seed the system-agent catalogue as the deployed seed job does: an operator,
/// then every coach in the dravr-contremaitre tree this build compiled against.
/// Once per server, before any fixture: each fixture's thread is answered by
/// an agent from it.
pub(crate) async fn seed_catalogue(resources: &Arc<ServerContext>) {
    let repos = &resources.common.repos;
    bootstrap::run(
        BootstrapArgs {
            admin_email: "operator@dravr.test".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        repos,
    )
    .await
    .expect("bootstrap an operator to own the catalogue");
    agent_seeder::run(
        AgentSeedArgs {
            agents_dir: contremaitre_tree().join("prompts/agents"),
            dry_run: false,
        },
        repos,
    )
    .await
    .expect("seed the contremaitre catalogue");
}

/// The athlete's turn asking `scenario`'s question, as the in-app chat route
/// builds it (`routes/chat/send_message.rs`): a typed web-app message under
/// the athlete's own grant, in the conversation the fixture opened, and the
/// web surface's profile in the scenario's language.
pub(crate) fn web_turn(
    fixture: &InjectionFixture,
    scenario: &Scenario,
) -> (TurnRequest<'static>, SurfaceProfile) {
    let request = TurnRequest {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: fixture.conversation_id.clone(),
        user_id: fixture.athlete,
        conversation_tenant_id: fixture.tenant,
        tool_tenant_id: fixture.tenant,
        content: scenario.question.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quote: None,
        channel_type: "web",
        transport: Transport::WebApp,
        is_direct_message: fixture.direct,
        ambient_group_fallback: false,
        command_persistence: CommandPersistence::Always,
        sender_id: None,
        grant: OAuthScope::self_grant(),
        hooks: PipelineHooks::none(),
    };
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: scenario.locale.to_owned(),
        transport: None,
        prose_contract: None,
    });
    (request, profile)
}
