// ABOUTME: The plan-push target is resolved from the athlete's connections, not hardcoded (carnet#721)
// ABOUTME: Mock calendar providers are pushed to and reconciled through the real tools without touching the reconciler

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, ConnectionType, PlannedSession, PrescribedWorkout, Stats,
    TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_services::plan_calendar_push::resolve_calendar_target;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

const MOCK_A: &str = "mock_calendar_a";
const MOCK_B: &str = "mock_calendar_b";
const READ_ONLY: &str = "mock_read_only";

/// What a mock calendar holds: provider, event id, the session written.
static CALENDARS: Mutex<Vec<(String, String, PlannedSession)>> = Mutex::new(Vec::new());

fn events_of(provider: &str, user_id: Uuid) -> Vec<(String, PlannedSession)> {
    CALENDARS
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _, session)| {
            name == provider && session.external_id.contains(&user_id.to_string())
        })
        .map(|(_, id, session)| (id.clone(), session.clone()))
        .collect()
}

struct MockDescriptor {
    name: &'static str,
    capabilities: ProviderCapabilities,
}

impl ProviderDescriptor for MockDescriptor {
    fn name(&self) -> &'static str {
        self.name
    }
    fn display_name(&self) -> &'static str {
        self.name
    }
    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }
    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }
    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }
    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None
    }
    fn api_base_url(&self) -> &'static str {
        "http://localhost/mock"
    }
    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// A second calendar provider: the four calendar methods and nothing else.
struct MockCalendar {
    name: &'static str,
    config: ProviderConfig,
}

#[async_trait]
impl FitnessProvider for MockCalendar {
    fn name(&self) -> &'static str {
        self.name
    }
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }
    async fn is_authenticated(&self) -> bool {
        true
    }
    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }
    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }
    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(Vec::new())
    }
    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(Vec::new(), None, None, false))
    }
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }
    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        Ok(CALENDARS
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _, s)| name == self.name && s.date >= from && s.date <= to)
            .map(|(_, id, s)| CalendarEventRef {
                provider_event_id: id.clone(),
                external_id: Some(s.external_id.clone()),
                date: s.date,
                updated_at: None,
            })
            .collect())
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        let id = format!("{}-{}", self.name, Uuid::new_v4());
        CALENDARS
            .lock()
            .unwrap()
            .push((self.name.to_owned(), id.clone(), session.clone()));
        Ok(id)
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        let mut calendars = CALENDARS.lock().unwrap();
        let entry = calendars
            .iter_mut()
            .find(|(name, id, _)| name == self.name && id == provider_event_id)
            .ok_or_else(|| AppError::not_found(provider_event_id.to_owned()))?;
        entry.2 = session.clone();
        Ok(())
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        let mut calendars = CALENDARS.lock().unwrap();
        let before = calendars.len();
        calendars.retain(|(name, id, _)| !(name == self.name && provider_event_ids.contains(id)));
        Ok((before - calendars.len()) as u64)
    }
}

fn mock_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    let name = match config.name.as_str() {
        MOCK_A => MOCK_A,
        MOCK_B => MOCK_B,
        _ => READ_ONLY,
    };
    Box::new(MockCalendar { name, config })
}

struct Fixture {
    executor: Arc<UniversalToolExecutor>,
    user_id: Uuid,
    tenant: TenantId,
}

/// An athlete connected to `connected`, on a server whose registry knows two
/// calendar-writing mocks and one read-only one beside the built-ins.
async fn fixture(connected: &[&str]) -> Result<Fixture> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = create_test_server_resources().await?;
    let mut registry = ProviderRegistry::new();
    let calendar = ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::CALENDAR_WRITE);
    for (name, capabilities) in [
        (MOCK_A, calendar),
        (MOCK_B, calendar),
        (
            READ_ONLY,
            ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::PLANNED_WORKOUTS),
        ),
    ] {
        registry.register_provider_bundle(ProviderBundle::new(
            Box::new(MockDescriptor { name, capabilities }),
            mock_factory,
        ));
    }
    let mut context = (*resources).clone();
    context.fitness.provider_registry = Arc::new(registry);
    let executor = Arc::new(
        UniversalToolExecutor::new(Arc::new(context)).with_scopes(OAuthScope::self_grant()),
    );
    let resources = &executor.resources;

    let (user_id, user) = create_test_user(resources.database()).await?;
    let repos = resources.repos();
    let tenant = repos
        .tenants
        .list_for_user(user.id)
        .await?
        .first()
        .expect("user has a tenant")
        .id;
    for provider in connected {
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken::new(
                user_id,
                tenant.to_string(),
                (*provider).to_owned(),
                "access".to_owned(),
                Some("refresh".to_owned()),
                Some(Utc::now() + Duration::hours(6)),
                None,
            ))
            .await?;
        repos
            .provider_connections
            .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
            .await?;
    }
    Ok(Fixture {
        executor,
        user_id,
        tenant,
    })
}

impl Fixture {
    async fn execute(&self, tool_name: &str, params: Value) -> Result<UniversalResponse> {
        Ok(self
            .executor
            .execute_tool(UniversalRequest {
                tool_name: tool_name.to_owned(),
                parameters: params,
                user_id: self.user_id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(self.tenant.to_string()),
            })
            .await?)
    }

    async fn ok(&self, tool_name: &str, params: Value) -> Result<Value> {
        let resp = self.execute(tool_name, params).await?;
        assert!(resp.success, "{tool_name} must succeed: {:?}", resp.error);
        Ok(resp.result.expect("result payload"))
    }

    /// The refusal text of a call that must fail.
    async fn refused(&self, tool_name: &str, params: Value) -> String {
        match self.execute(tool_name, params).await {
            Ok(resp) => {
                assert!(!resp.success, "{tool_name} must be refused");
                format!("{:?}", resp.error)
            }
            Err(err) => format!("{err:?}"),
        }
    }

    async fn live(&self, provider: &str) -> Vec<PrescribedWorkout> {
        self.executor
            .resources
            .repos()
            .prescribed_workouts
            .list_live_calendar_events(self.tenant, self.user_id, provider, None)
            .await
            .expect("list live")
    }

    async fn save_plan(&self) {
        let monday = {
            let mut day = Utc::now().date_naive() + Duration::days(7);
            while day.weekday() != Weekday::Mon {
                day += Duration::days(1);
            }
            day
        };
        let iso = |offset: i64| {
            (monday + Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        };
        let day = |offset: i64, sport: &str, workout: &str| PlannedDay {
            date: iso(offset),
            sport: sport.to_owned(),
            workout: workout.to_owned(),
            duration_min: (sport != "rest").then_some(60),
            intensity: String::new(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        };
        let days = vec![
            day(0, "run", "Easy run"),
            day(1, "rest", ""),
            day(2, "bike", "Endurance ride"),
        ];
        let goal = GoalRace {
            name: "Fall race".to_owned(),
            date: iso(70),
            discipline: "gravel".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: iso(0),
            weeks: 10,
            intent: "rebuild volume".to_owned(),
            target_hours: None,
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let week_start = iso(0);
        self.executor
            .resources
            .repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.tenant.to_string(),
                user_id: &self.user_id.to_string(),
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "",
                    days: &days,
                    adjustment_reason: "",
                    phase_index: None,
                }],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .expect("save plan bundle");
    }
}

fn session() -> Value {
    json!({
        "name": "Easy spin",
        "sport": "run",
        "intensity_distribution": "polarized",
        "structure": [{ "label": "Steady", "duration_seconds": 1800, "target_zone": "Z2" }],
    })
}

fn prescription_date() -> String {
    (Utc::now().date_naive() + Duration::days(10))
        .format("%Y-%m-%d")
        .to_string()
}

#[tokio::test]
async fn a_second_calendar_provider_is_pushed_to_and_reconciled_without_touching_the_reconciler(
) -> Result<()> {
    let fx = fixture(&[MOCK_B]).await?;
    fx.save_plan().await;

    let first = fx.ok("push_training_plan", json!({})).await?;
    assert_eq!(first["provider"], MOCK_B, "{first}");
    assert_eq!(first["created"].as_u64(), Some(2), "the two training days");
    assert_eq!(events_of(MOCK_B, fx.user_id).len(), 2);
    assert_eq!(
        fx.live(MOCK_B).await.len(),
        2,
        "ledger rows are per provider"
    );
    assert!(fx.live(MOCK_A).await.is_empty());

    let again = fx.ok("push_training_plan", json!({})).await?;
    assert_eq!(again["created"].as_u64(), Some(0), "{again}");
    assert_eq!(again["unchanged"].as_u64(), Some(2), "{again}");
    assert_eq!(events_of(MOCK_B, fx.user_id).len(), 2, "nothing duplicated");

    let plan = fx.ok("get_training_plan", json!({})).await?;
    assert_eq!(plan["calendar"]["provider"], MOCK_B, "{plan}");
    assert_eq!(
        plan["calendar"]["entries"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(plan["calendar"]["stale"].as_bool(), Some(false));
    Ok(())
}

#[tokio::test]
async fn several_calendars_default_alphabetically_and_a_named_provider_overrides() -> Result<()> {
    let fx = fixture(&[MOCK_B, MOCK_A]).await?;
    fx.save_plan().await;

    let default = fx.ok("push_training_plan", json!({})).await?;
    assert_eq!(default["provider"], MOCK_A, "{default}");
    assert_eq!(fx.live(MOCK_A).await.len(), 2);
    assert!(fx.live(MOCK_B).await.is_empty());

    let named = fx
        .ok("push_training_plan", json!({ "provider": MOCK_B }))
        .await?;
    assert_eq!(named["provider"], MOCK_B, "{named}");
    assert_eq!(named["created"].as_u64(), Some(2), "{named}");
    assert_eq!(fx.live(MOCK_B).await.len(), 2);
    assert_eq!(
        fx.live(MOCK_A).await.len(),
        2,
        "the other ledger is untouched"
    );
    Ok(())
}

#[tokio::test]
async fn a_prescription_stays_on_its_provider_and_withdraws_from_it() -> Result<()> {
    let fx = fixture(&[MOCK_A, MOCK_B]).await?;

    let prescribed = fx
        .ok(
            "prescribe_workout",
            json!({ "date": prescription_date(), "session": session(), "provider": MOCK_B }),
        )
        .await?;
    assert_eq!(prescribed["provider"], MOCK_B, "{prescribed}");
    let id = prescribed["prescription_id"].as_str().unwrap().to_owned();
    assert_eq!(fx.live(MOCK_B).await.len(), 1);
    assert!(fx.live(MOCK_A).await.is_empty());

    let wrong = fx
        .refused(
            "withdraw_prescribed_workout",
            json!({ "prescription_id": id, "provider": MOCK_A }),
        )
        .await;
    assert!(
        wrong.contains(MOCK_B),
        "names the entry's provider: {wrong}"
    );
    assert_eq!(
        fx.live(MOCK_B).await.len(),
        1,
        "a refused call removes nothing"
    );

    let replaced_elsewhere = fx
        .refused(
            "prescribe_workout",
            json!({
                "date": prescription_date(), "session": session(),
                "replaces": id, "provider": MOCK_A,
            }),
        )
        .await;
    assert!(replaced_elsewhere.contains(MOCK_B), "{replaced_elsewhere}");

    let withdrawn = fx
        .ok(
            "withdraw_prescribed_workout",
            json!({ "prescription_id": id }),
        )
        .await?;
    assert_eq!(withdrawn["provider"], MOCK_B, "{withdrawn}");
    assert!(fx.live(MOCK_B).await.is_empty());
    Ok(())
}

#[tokio::test]
async fn an_athlete_with_no_calendar_capable_connection_is_told_which_capability_is_missing(
) -> Result<()> {
    let fx = fixture(&[READ_ONLY]).await?;
    fx.save_plan().await;

    for (tool, params) in [
        ("push_training_plan", json!({})),
        (
            "prescribe_workout",
            json!({ "date": prescription_date(), "session": session() }),
        ),
    ] {
        let refusal = fx.refused(tool, params).await;
        assert!(
            refusal.contains("calendar_write"),
            "{tool} must name the missing capability: {refusal}"
        );
        assert!(
            refusal.contains(MOCK_A),
            "{tool} lists who has it: {refusal}"
        );
    }
    assert!(fx.live(MOCK_A).await.is_empty() && fx.live(MOCK_B).await.is_empty());

    // Reading the plan is not a write: the calendar block is just empty.
    let plan = fx.ok("get_training_plan", json!({})).await?;
    assert_eq!(
        plan["calendar"]["entries"].as_array().map(Vec::len),
        Some(0)
    );
    Ok(())
}

#[tokio::test]
async fn a_named_provider_that_does_not_write_a_calendar_is_refused() -> Result<()> {
    let fx = fixture(&[READ_ONLY, MOCK_A]).await?;
    fx.save_plan().await;

    let refusal = fx
        .refused("push_training_plan", json!({ "provider": READ_ONLY }))
        .await;
    assert!(refusal.contains("calendar_write"), "{refusal}");
    assert!(events_of(READ_ONLY, fx.user_id).is_empty());
    Ok(())
}

#[test]
fn intervals_icu_declares_the_capability_and_stays_the_target_of_an_athlete_connected_to_it() {
    let registry = ProviderRegistry::new();
    assert!(registry
        .calendar_write_providers()
        .contains(&"intervals_icu"));
    let target = resolve_calendar_target(&registry, &["strava", "intervals_icu"], None);
    assert_eq!(target.unwrap(), "intervals_icu");
}
