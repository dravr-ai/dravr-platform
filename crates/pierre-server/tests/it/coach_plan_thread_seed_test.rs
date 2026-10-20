// ABOUTME: The coach plan thread seeder leaves a coach's thread about one athlete whose card shows a draft the coach can publish
// ABOUTME: The group, both threads, the live season and the draft as the routes read them, the publish, a rerun, and the refusals

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The seeded state is read back through the routes the mobile plan-thread
//! flow drives — the subject-plan read, the history read a reloaded thread
//! draws its card from, and the publish behind the card's button — so a seed
//! the app could not act on fails here rather than on a device.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::errors::ErrorCode;
use pierre_core::models::groups::GroupRole;
use pierre_core::models::ConversationSummary;
use pierre_core::transport::TransportPolicy;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_seeders::coach_plan_thread::{
    run, SeedArgs, DRAFT_RACE, DRAFT_WORKOUT, GROUP_NAME, LIVE_RACE, LIVE_WORKOUT, QUESTION,
};
use serde_json::{json, Value};
use uuid::Uuid;

const COACH: &str = "coach@plan-seed.test";
const ATHLETE: &str = "athlete@plan-seed.test";
const ATHLETE_NAME: &str = "Maya Tempo";

fn args() -> SeedArgs {
    SeedArgs {
        coach_email: COACH.to_owned(),
        athlete_email: ATHLETE.to_owned(),
        model: Some("seed-model".to_owned()),
    }
}

/// The two accounts `user create` would have made, the athlete named.
struct Accounts {
    res: Arc<ServerContext>,
    coach: Uuid,
    coach_auth: String,
    athlete: Uuid,
    athlete_tenant: String,
}

impl Accounts {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let database = &res.agent.database;
        let (coach, coach_user, _) = create_test_user_with_plan(database, COACH, "professional")
            .await
            .unwrap();
        let (athlete, _, athlete_tenant) =
            create_test_user_with_plan(database, ATHLETE, "professional")
                .await
                .unwrap();
        res.common
            .repos
            .users
            .update_display_name(athlete, ATHLETE_NAME)
            .await
            .unwrap();
        let coach_auth = format!("Bearer {}", generate_test_token(&res, &coach_user).await);
        Self {
            res,
            coach,
            coach_auth,
            athlete,
            athlete_tenant: athlete_tenant.to_string(),
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// Every conversation the coach is in.
    async fn coach_threads(&self) -> Vec<ConversationSummary> {
        let coach = self
            .repos()
            .users
            .get_global(self.coach)
            .await
            .unwrap()
            .unwrap();
        let tenant = self
            .repos()
            .tenants
            .list_for_user(coach.id)
            .await
            .unwrap()
            .first()
            .expect("the coach's tenant")
            .id;
        self.repos()
            .chat
            .list_conversations(
                &self.coach.to_string(),
                tenant,
                50,
                0,
                TransportPolicy::AnyTransport,
            )
            .await
            .unwrap()
            .items
    }

    /// The coach's thread about the athlete.
    async fn plan_thread(&self) -> ConversationSummary {
        let threads = self.coach_threads().await;
        let about: Vec<_> = threads
            .into_iter()
            .filter(|thread| thread.subject_user_id == Some(self.athlete))
            .collect();
        assert_eq!(about.len(), 1, "one thread about the athlete");
        about.into_iter().next().unwrap()
    }

    /// `GET path` as the coach, through the chat routes.
    async fn get(&self, path: &str) -> Value {
        let resp = AxumTestRequest::get(path)
            .header("authorization", &self.coach_auth)
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "GET {path}");
        resp.json()
    }

    /// The athlete's live season: its goal race and every session of its weeks.
    async fn live_season(&self) -> (String, Vec<String>) {
        let athlete = self.athlete.to_string();
        let plan = self
            .repos()
            .training_plans
            .get_active_plan(&self.athlete_tenant, &athlete)
            .await
            .unwrap()
            .expect("the athlete's live season");
        let weeks = self
            .repos()
            .training_plans
            .list_plan_weeks(&self.athlete_tenant, &athlete, &plan.id, false)
            .await
            .unwrap();
        let sessions = weeks
            .iter()
            .flat_map(|week| week.days.iter().map(|day| day.workout.clone()))
            .collect();
        (plan.goal_race.name, sessions)
    }
}

/// The plan card of the newest reply carrying one, as a client reloading the
/// thread reads it: `GET …/messages`, whose stored blocks resolve into
/// `scene_blocks`. Also the thread's rows, for what was asked.
async fn history(accounts: &Accounts, thread: &str) -> (Value, Vec<Value>) {
    let page = accounts
        .get(&format!("/api/chat/conversations/{thread}/messages"))
        .await;
    let messages = page["messages"]
        .as_array()
        .expect("the thread's rows")
        .clone();
    let card = messages
        .iter()
        .rev()
        .filter_map(|message| message["scene_blocks"].as_str())
        .flat_map(|scenes| {
            serde_json::from_str::<Vec<Value>>(scenes).expect("scene_blocks is a JSON array")
        })
        .find(|block| block["kind"] == "workout_plan")
        .unwrap_or_else(|| panic!("a plan card in the history read: {page}"));
    (card, messages)
}

#[tokio::test]
async fn the_seeder_leaves_a_draft_the_coach_publishes_from_the_athletes_thread() {
    let accounts = Accounts::new().await;
    let repos = accounts.repos();

    run(args(), repos).await.unwrap();

    // The group: owned by the athlete, its one member, sharing with the coach,
    // who coaches it and holds no membership row.
    let coached: Vec<_> = repos
        .groups
        .list_groups_coached_by(accounts.coach)
        .await
        .unwrap()
        .into_iter()
        .filter(|group| group.name == GROUP_NAME)
        .collect();
    assert_eq!(coached.len(), 1, "one coached group of the seeded name");
    let group = &coached[0];
    assert!(group.is_active);
    assert_eq!(group.owner_id, accounts.athlete);
    let group_id = group.id.to_string();
    let members = repos.groups.list_members(&group_id).await.unwrap();
    assert_eq!(members.len(), 1, "the athlete is the one member");
    assert_eq!(members[0].user_id, accounts.athlete);
    assert_eq!(members[0].role, GroupRole::Owner);
    assert!(members[0].coach_sharing_consent);
    assert!(repos
        .groups
        .get_member(&group_id, accounts.coach)
        .await
        .unwrap()
        .is_none());

    // The coach's two threads: the group's, which reaches Group info, and the
    // one about the athlete, bound to the group and named after the athlete.
    let threads = accounts.coach_threads().await;
    assert_eq!(threads.len(), 2, "{threads:?}");
    let group_thread = threads
        .iter()
        .find(|thread| thread.group_id.as_deref() == Some(group_id.as_str()))
        .expect("the coach's group thread");
    assert_eq!(group_thread.title, GROUP_NAME);
    assert_eq!(group_thread.subject_user_id, None);
    let plan_thread = accounts.plan_thread().await;
    assert_eq!(plan_thread.subject_group_id, Some(group.id));
    assert_eq!(plan_thread.title, ATHLETE_NAME);
    assert_eq!(
        plan_thread.subject_display_name.as_deref(),
        Some(ATHLETE_NAME)
    );
    assert_eq!(
        plan_thread.agent_id.as_deref(),
        Some(group.agent_id.as_str())
    );

    // The athlete's live season is theirs until the coach publishes.
    let (live_race, live_sessions) = accounts.live_season().await;
    assert_eq!(live_race, LIVE_RACE);
    assert!(
        live_sessions.iter().any(|s| s == LIVE_WORKOUT),
        "{live_sessions:?}"
    );
    assert!(!live_sessions.iter().any(|s| s == DRAFT_WORKOUT));

    // The subject-plan read the card's controls judge it against: the live
    // season, the coach's draft over it, and a draft that overwrites nothing.
    let read = accounts
        .get(&format!(
            "/api/chat/conversations/{}/subject-plan",
            plan_thread.id
        ))
        .await;
    assert_eq!(read["athlete"]["display_name"], ATHLETE_NAME);
    assert_eq!(read["live"]["goal_race"]["name"], LIVE_RACE, "{read}");
    assert_eq!(read["draft"]["goal_race"]["name"], DRAFT_RACE, "{read}");
    assert!(read["draft"].to_string().contains(DRAFT_WORKOUT), "{read}");
    let draft_state = &read["draft_state"];
    assert!(draft_state["draft_id"].is_string(), "{read}");
    assert_eq!(draft_state["conflicts"], json!([]));
    assert_eq!(draft_state["season_relaid"], false);

    // The card a reload draws: the draft, naming the athlete, carrying the
    // read's own standing — the pair the client offers Publish on.
    let (card, messages) = history(&accounts, &plan_thread.id).await;
    assert_eq!(card["plan"]["goal_race"]["name"], DRAFT_RACE, "{card}");
    assert!(card["plan"].to_string().contains(DRAFT_WORKOUT), "{card}");
    assert_eq!(card["subject"]["user_id"], accounts.athlete.to_string());
    assert_eq!(card["subject"]["display_name"], ATHLETE_NAME);
    assert_eq!(&card["draft"], draft_state, "{card}");
    assert_eq!(card["source_tool"], "save_training_plan");
    let asked: Vec<_> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .collect();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["content"], QUESTION);

    // A rerun finds the group and seeds nothing twice.
    run(args(), repos).await.unwrap();
    assert_eq!(accounts.coach_threads().await.len(), 2);
    let (_, rerun_messages) = history(&accounts, &plan_thread.id).await;
    assert_eq!(rerun_messages.len(), messages.len());
    let reread = accounts
        .get(&format!(
            "/api/chat/conversations/{}/subject-plan",
            plan_thread.id
        ))
        .await;
    assert_eq!(&reread["draft_state"], draft_state, "the same draft");

    // The card's Publish: the draft becomes the athlete's live season.
    let resp = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/subject-plan/publish",
        plan_thread.id
    ))
    .header("authorization", &accounts.coach_auth)
    .json(&json!({ "overwrite_athlete_changes": false }))
    .send(ChatRoutes::routes(Arc::clone(&accounts.res)))
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let (published_race, published_sessions) = accounts.live_season().await;
    assert_eq!(published_race, DRAFT_RACE);
    assert!(
        published_sessions.iter().any(|s| s == DRAFT_WORKOUT),
        "{published_sessions:?}"
    );
    let after = accounts
        .get(&format!(
            "/api/chat/conversations/{}/subject-plan",
            plan_thread.id
        ))
        .await;
    assert_eq!(after["draft_state"], Value::Null, "{after}");
}

/// With no model resolved the seeder refuses before writing anything, and
/// names both ways to give it one.
#[tokio::test]
async fn the_seeder_refuses_without_a_model_and_writes_nothing() {
    let accounts = Accounts::new().await;
    let error = run(
        SeedArgs {
            model: None,
            ..args()
        },
        accounts.repos(),
    )
    .await
    .expect_err("no model to create the threads with");
    assert_eq!(error.code, ErrorCode::ConfigError, "{error}");
    assert!(error.message.contains("pass --model"), "{error}");
    assert!(accounts
        .repos()
        .groups
        .list_groups_coached_by(accounts.coach)
        .await
        .unwrap()
        .is_empty());
    assert!(accounts.coach_threads().await.is_empty());
}

/// An athlete nobody made is a configuration error naming the email, and the
/// coach is left with nothing.
#[tokio::test]
async fn the_seeder_names_a_missing_athlete() {
    let accounts = Accounts::new().await;
    let error = run(
        SeedArgs {
            athlete_email: "nobody@plan-seed.test".to_owned(),
            ..args()
        },
        accounts.repos(),
    )
    .await
    .expect_err("no such athlete");
    assert_eq!(error.code, ErrorCode::ConfigError, "{error}");
    assert!(error.message.contains("nobody@plan-seed.test"), "{error}");
    assert!(accounts.coach_threads().await.is_empty());
}
