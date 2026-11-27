// ABOUTME: /api/me/plan — the athlete's own plan as their Plan tab reads it: live, their agent's proposals, what « Envoyer » would send
// ABOUTME: Pins the read's shape, the move pairing, the pending send per key and its reasons, the families, the preview and the own history

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1841. A self-coached athlete's Plan tab reads their own plan
//! through these routes, scoped by their token alone. Plans and the overlay
//! of their agent's unasked changes are seeded through the repository's own
//! saves, the way the plan tools write them; every assertion reads the wire
//! a client parses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::INTERVALS_ICU;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken, API_KEY_TOKEN_TYPE};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    DraftOwner, PlanChangeKind, PlanChangeSource, PlanOutlineInput, PlanWeekInput, SaveDraftParams,
    SavePlanBundleParams,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_memory::training_plans::{GoalRace, PlannedDay, RacePriority};
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_services::plan_change_history::{record_plan_change, PlanChange};
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

/// The agent that writes the athlete's season and its proposals.
const AGENT: &str = "plan-agent";

struct Athlete {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

struct Fixture {
    res: Arc<ServerContext>,
    athlete: Athlete,
}

/// Next week's Monday: every date of its week is in the athlete's future on
/// whatever day the suite runs, and the plan card shows the week.
fn monday() -> CivilDate {
    let mut day = Timestamp::now().utc_date() + Elapsed::days(1);
    while day.weekday().num_days_from_monday() != 0 {
        day += Elapsed::days(1);
    }
    day
}

fn on(offset: i64) -> String {
    (monday() + Elapsed::days(offset)).to_string()
}

fn session(date: &str, workout: &str) -> PlannedDay {
    serde_json::from_value(json!({
        "date": date, "sport": "run", "workout": workout, "duration_min": 45, "intensity": "Z2",
    }))
    .unwrap()
}

/// The athlete's live week: Tuesday tempo, Wednesday easy, Friday long.
fn live_week() -> Vec<PlannedDay> {
    vec![
        session(&on(1), "Tempo 40"),
        session(&on(2), "Easy 45"),
        session(&on(4), "Long 90"),
    ]
}

impl Fixture {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let email = format!("own-plan-{}@own-plan.test", Uuid::new_v4());
        let (id, user, tenant) =
            create_test_user_with_plan(&res.agent.database, &email, "professional")
                .await
                .unwrap();
        let token = generate_test_token(&res, &user).await;
        Self {
            res,
            athlete: Athlete {
                id,
                tenant,
                auth: format!("Bearer {token}"),
            },
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    async fn request(&self, request: AxumTestRequest) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", &self.athlete.auth)
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        let status = resp.status_code();
        (status, resp.json())
    }

    async fn plan(&self) -> Value {
        let (status, body) = self
            .request(AxumTestRequest::get("/api/me/plan?locale=en"))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The athlete's live season: one race, and `days` in the week of
    /// [`monday`].
    async fn lay_season(&self, days: &[PlannedDay]) {
        let race = GoalRace {
            name: "Spring Marathon".to_owned(),
            date: (monday() + Elapsed::days(120)).to_string(),
            discipline: "road".to_owned(),
            priority: RacePriority::A,
        };
        let week_start = monday().to_string();
        self.repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.athlete.tenant.to_string(),
                user_id: &self.athlete.id.to_string(),
                author: PlanAuthor::agent(AGENT),
                goal_fact_id: None,
                outline: Some(PlanOutlineInput {
                    goal_race: &race,
                    races: Some(&[]),
                    strategy: "build to the marathon",
                    flavour: None,
                    season_start: None,
                    season_end: None,
                    phases: &[],
                    source_conversation_id: None,
                }),
                replace_season: false,
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "build",
                    days,
                    adjustment_reason: "",
                    phase_index: None,
                }],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }

    /// The agent writes `days` for the week of [`monday`] into the athlete's
    /// own overlay, unasked.
    async fn agent_proposes(&self, days: &[PlannedDay]) {
        let tenant = self.athlete.tenant.to_string();
        let athlete = self.athlete.id.to_string();
        let week_start = monday().to_string();
        self.repos()
            .training_plans
            .save_draft_bundle(&SaveDraftParams {
                owner: DraftOwner {
                    tenant_id: &tenant,
                    user_id: &athlete,
                    coach_user_id: &athlete,
                },
                author: PlanAuthor::agent(AGENT),
                goal_fact_id: None,
                outline: None,
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "build",
                    days,
                    adjustment_reason: "the agent's proposal",
                    phase_index: None,
                }],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }

    /// The athlete's own Intervals.icu API key, with the connection a linked
    /// key registers: a calendar Dravr writes.
    async fn connect_intervals(&self) {
        let now = Timestamp::now();
        let repos = self.repos();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: self.athlete.id,
                tenant_id: self.athlete.tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: SecretString::from("own-key".to_owned()),
                refresh_token: None,
                token_type: API_KEY_TOKEN_TYPE.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some("i-own".to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                self.athlete.id,
                self.athlete.tenant,
                INTERVALS_ICU,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }
}

fn dates(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|date| date.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn with_no_plan_and_no_calendar_the_read_offers_nothing_to_send() {
    let fx = Fixture::new().await;
    let body = fx.plan().await;
    assert!(
        body["live"].is_null() && body["draft"].is_null(),
        "bug: an athlete with no plan reads none: {body}"
    );
    assert_eq!(body["proposed_dates"], json!([]), "{body}");
    assert_eq!(body["proposals"], json!([]), "{body}");
    let pending = &body["pending_send"];
    assert_eq!(
        pending["send_available"], false,
        "bug: with no calendar Dravr writes, « Envoyer » is not offered: {body}"
    );
    assert_eq!(pending["reason"], "no_provider", "{body}");
    assert!(pending["destination"].is_null(), "{body}");
    assert_eq!(pending["entries"], json!([]), "{body}");
    assert!(
        body["today"].as_str().is_some(),
        "bug: the read names the athlete's today: {body}"
    );
}

#[tokio::test]
async fn the_read_groups_a_moved_session_and_counts_each_key_a_send_would_change() {
    let fx = Fixture::new().await;
    fx.lay_season(&live_week()).await;
    // Unasked, the agent moves Tuesday's tempo to Thursday and rewrites
    // Wednesday.
    fx.agent_proposes(&[
        session(&on(2), "Hills 45"),
        session(&on(3), "Tempo 40"),
        session(&on(4), "Long 90"),
    ])
    .await;
    fx.connect_intervals().await;

    let body = fx.plan().await;
    assert!(
        body["live"]["weeks"].is_array() && body["draft"]["weeks"].is_array(),
        "bug: the live plan and the overlay read as plan cards: {body}"
    );
    assert_eq!(
        dates(&body["proposed_dates"]),
        [on(1), on(2), on(3)],
        "bug: the dates the overlay changes are proposed, and the untouched Friday is not: {body}"
    );
    assert_eq!(
        body["proposals"],
        json!([[on(1), on(3)], [on(2)]]),
        "bug: the moved tempo is one proposal from Tuesday to Thursday, and the rewrite its own: {body}"
    );

    let pending = &body["pending_send"];
    assert_eq!(
        pending["send_available"], true,
        "bug: a connected calendar Dravr writes offers « Envoyer »: {body}"
    );
    assert!(pending["reason"].is_null(), "{body}");
    assert_eq!(pending["destination"], INTERVALS_ICU, "{body}");
    let entries = pending["entries"].as_array().unwrap();
    let sessions: Vec<&Value> = entries
        .iter()
        .filter(|entry| entry["kind"] == "session")
        .collect();
    assert_eq!(
        sessions.len(),
        3,
        "bug: each live session the calendar lacks is one pending entry, never the overlay's: {body}"
    );
    assert!(
        entries.iter().all(|entry| entry["change"] == "create"
            && entry["before"].is_null()
            && entry["after"].is_object()
            && entry["destination"] == INTERVALS_ICU),
        "bug: an entry the ledger never wrote is a create with no before, its after and destination: {body}"
    );
    let pending_dates: Vec<&str> = sessions
        .iter()
        .map(|entry| entry["date"].as_str().unwrap())
        .collect();
    assert_eq!(
        pending_dates,
        [on(1), on(2), on(4)],
        "bug: what a send would change is the live plan's, the proposals left out until accepted: {body}"
    );
}

#[tokio::test]
async fn the_history_lists_the_athletes_own_plan_changes_only() {
    let fx = Fixture::new().await;
    let repos = fx.repos();
    let date = monday() + Elapsed::days(1);
    let own = CoachDraftTarget::own(fx.athlete.tenant, fx.athlete.id);
    record_plan_change(
        repos,
        &own,
        PlanChange {
            source: PlanChangeSource::Agent,
            kind: PlanChangeKind::AgentDraft,
            dates: &[date],
        },
    )
    .await;
    let coach_email = format!("coach-{}@own-plan.test", Uuid::new_v4());
    let (coach_id, _, _) =
        create_test_user_with_plan(&fx.res.agent.database, &coach_email, "professional")
            .await
            .unwrap();
    let coach = CoachDraftTarget::new(fx.athlete.tenant, fx.athlete.id, coach_id);
    record_plan_change(
        repos,
        &coach,
        PlanChange {
            source: PlanChangeSource::Coach,
            kind: PlanChangeKind::DayEdit,
            dates: &[date],
        },
    )
    .await;

    let (status, body) = fx
        .request(AxumTestRequest::get("/api/me/plan/history"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["events"],
        json!([{
            "kind": "agent_draft",
            "source": "agent",
            "dates": [date.to_string()],
            "created_at": body["events"][0]["created_at"],
        }]),
        "bug: the athlete reads their own plan's changes, never a coach's workspace: {body}"
    );

    let (status, body) = fx
        .request(AxumTestRequest::get(
            "/api/me/plan/history?before=yesterday",
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["details"]["reason"], "invalid_before", "{body}");
}

#[tokio::test]
async fn the_families_and_a_preview_answer_on_the_athletes_own_plan() {
    let fx = Fixture::new().await;
    let (status, body) = fx
        .request(AxumTestRequest::get("/api/me/plan/families?locale=en"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let family = body["families"]
        .as_array()
        .unwrap()
        .iter()
        .find(|family| family["slug"] == "vo2max_short")
        .unwrap_or_else(|| panic!("the catalogue's families are offered: {body}"));
    assert_eq!(family["default_preset"], "30_15", "{body}");

    let (status, preview) = fx
        .request(
            AxumTestRequest::post("/api/me/plan/session-preview?locale=en").json(&json!({
                "date": on(2),
                "family": "vo2max_short",
                "sport": "ride",
                "params": { "reps": 20 },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["family"], "vo2max_short", "{preview}");
    assert_eq!(preview["preset"], "30_15", "{preview}");
    assert!(
        preview["findings"]
            .as_array()
            .is_some_and(|findings| findings
                .iter()
                .any(|finding| finding["code"] == "param_out_of_range")),
        "bug: a value outside the range previews with its finding, never refused (D3): {preview}"
    );
    assert!(
        fx.plan().await["live"].is_null(),
        "bug: a preview wrote nothing"
    );

    let (status, refused) = fx
        .request(
            AxumTestRequest::post("/api/me/plan/session-preview").json(&json!({
                "date": "next tuesday",
                "family": "vo2max_short",
                "sport": "ride",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["details"]["reason"], "invalid_date", "{refused}");
}
