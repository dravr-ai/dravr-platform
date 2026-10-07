// ABOUTME: Any provider declaring COACH_ROSTER is linked and read for a confirmed member through the coach's credential
// ABOUTME: A stand-in platform pins provider-neutral delegation: roster, propose, confirm, the read, and refusals before any read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Delegated coach reads are provider-neutral (carnet#720). A coaching
//! platform is one because its descriptor declares
//! [`ProviderCapabilities::COACH_ROSTER`]; nothing names it. `coachhub`, a
//! stand-in registered beside the built-in providers and named nowhere in the
//! platform's code, walks the whole flow over REST and the read path:
//!
//! - the coach reads the roster their stored OAuth grant lists;
//! - the coach proposes a roster athlete as a member, and the member confirms;
//! - the member's read is served by a provider built for the member's athlete
//!   id, holding the coach's credential.
//!
//! And the refusals stay typed, decided before any provider is built:
//!
//! - a member whose link was only proposed reads nothing through it;
//! - a link the coach ended reads nothing;
//! - an athlete id the platform could not have issued is refused on propose.

mod common;
mod helpers;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::Router;
use chrono::{Duration as ChronoDuration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    Activity, ActivityBuilder, AgentCategory, Athlete, ConnectionType, CreateAgentRequest,
    RosterAthlete, SportType, Stats, TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::{
    ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    ProviderFactory,
};
use pierre_providers::delegation::{CoachRoster, DelegatedReads};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};
use pierre_providers::CoreFitnessProvider;
use pierre_routes_groups::DelegatedConnectionRoutes;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

/// The stand-in coaching platform, named nowhere in the platform's code.
const COACHHUB: &str = "coachhub";
/// The coach's stored OAuth access token on the platform.
const COACH_TOKEN: &str = "coachhub-coach-token";

const COACH_EMAIL: &str = "coach@coachhub-read.test";
const M1_EMAIL: &str = "m1@coachhub-read.test";
const M2_EMAIL: &str = "m2@coachhub-read.test";

/// The roster athletes the coach's account lists.
const M1_ATHLETE: &str = "ch201";
const M2_ATHLETE: &str = "ch202";

/// How long the warm-up read a confirm spawns gets to land.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// One read the stand-in served: the athlete it was built for (`None` for
/// the account's own), and the token and kind of the credential it held.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Read {
    athlete: Option<String>,
    token: Option<String>,
    kind: Option<CredentialKind>,
}

/// What the stand-in saw: every provider built, every read served.
#[derive(Clone, Default)]
struct Seen {
    built: Arc<AtomicUsize>,
    reads: Arc<Mutex<Vec<Read>>>,
}

impl Seen {
    fn built(&self) -> usize {
        self.built.load(Ordering::SeqCst)
    }

    fn reads(&self) -> Vec<Read> {
        self.reads.lock().unwrap().clone()
    }
}

struct Descriptor;

impl ProviderDescriptor for Descriptor {
    fn name(&self) -> &'static str {
        COACHHUB
    }

    fn display_name(&self) -> &'static str {
        "CoachHub"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::COACH_ROSTER)
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
        "http://127.0.0.1:9"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// The stand-in provider, built for one athlete or for the account's own.
struct CoachHub {
    config: ProviderConfig,
    athlete: Option<String>,
    credentials: Mutex<Option<OAuth2Credentials>>,
    seen: Seen,
}

impl CoachHub {
    /// Record a read, refusing one made without a credential.
    fn record(&self) -> AppResult<()> {
        let credentials = self.credentials.lock().unwrap().clone();
        let Some(credentials) = credentials else {
            return Err(AppError::internal("read without a credential"));
        };
        self.seen.reads.lock().unwrap().push(Read {
            athlete: self.athlete.clone(),
            token: credentials.access_token,
            kind: Some(credentials.kind),
        });
        Ok(())
    }
}

#[async_trait]
impl FitnessProvider for CoachHub {
    fn name(&self) -> &'static str {
        COACHHUB
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        *self.credentials.lock().unwrap() = Some(credentials);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        self.credentials.lock().unwrap().is_some()
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
        self.record()?;
        let athlete = self.athlete.as_deref().unwrap_or("coach");
        Ok(vec![ActivityBuilder::new(
            format!("{athlete}-run"),
            "Easy run",
            SportType::Run,
            Utc::now() - ChronoDuration::days(1),
            1_800,
            COACHHUB,
        )
        .build()])
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }

    async fn read_coach_roster(&self) -> AppResult<CoachRoster> {
        self.record()?;
        Ok(CoachRoster {
            account_email: Some(COACH_EMAIL.to_owned()),
            athletes: vec![
                RosterAthlete {
                    id: M1_ATHLETE.to_owned(),
                    name: Some("Alex Athlete".to_owned()),
                    email: Some(M1_EMAIL.to_owned()),
                },
                RosterAthlete {
                    id: M2_ATHLETE.to_owned(),
                    name: Some("Sam Swimmer".to_owned()),
                    email: Some(M2_EMAIL.to_owned()),
                },
            ],
        })
    }
}

struct Factory {
    seen: Seen,
}

impl Factory {
    fn build(&self, config: ProviderConfig, athlete: Option<&str>) -> Box<dyn FitnessProvider> {
        self.seen.built.fetch_add(1, Ordering::SeqCst);
        Box::new(CoachHub {
            config,
            athlete: athlete.map(str::to_owned),
            credentials: Mutex::new(None),
            seen: self.seen.clone(),
        })
    }
}

impl ProviderFactory for Factory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(self.build(config, None))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[COACHHUB]
    }

    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for Factory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        if athlete_id.is_empty() || !athlete_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(AppError::invalid_input(format!(
                "{athlete_id:?} is not a CoachHub athlete id"
            )));
        }
        Ok(())
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        self.check_athlete_id(athlete_id)?;
        Ok(self.build(config, Some(athlete_id)))
    }
}

/// One signed-in user.
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

struct World {
    runtime: Arc<dyn ToolRuntime>,
    router: Router,
    seen: Seen,
    group: Uuid,
    coach: Person,
    m1: Person,
    m2: Person,
}

/// The test server, its provider registry holding the stand-in beside the
/// built-in providers.
async fn server(seen: &Seen) -> Arc<ServerContext> {
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.set_default_config(COACHHUB, Descriptor.to_config());
    registry.register_descriptor(COACHHUB, Box::new(Descriptor));
    registry.register_factory(COACHHUB, Box::new(Factory { seen: seen.clone() }));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    // A verified email: what binds the coach's account, and a roster
    // athlete, to this person.
    repos.email_verification.mark_verified(id).await.unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person { id, tenant, auth }
}

fn agent_request() -> CreateAgentRequest {
    CreateAgentRequest {
        title: "Group Agent".to_owned(),
        description: None,
        system_prompt: "You are helpful".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    }
}

async fn world() -> World {
    let seen = Seen::default();
    let res = server(&seen).await;
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@coachhub-read.test", "Olive Owner").await;
    let coach = person(&res, COACH_EMAIL, "Casey Coach").await;
    let m1 = person(&res, M1_EMAIL, "Alex Athlete").await;
    let m2 = person(&res, M2_EMAIL, "Sam Swimmer").await;

    let agent_id = repos
        .agents
        .create(owner.id, owner.tenant, &agent_request())
        .await
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: "Squad".to_owned(),
                description: None,
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id;
    assert!(repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    for member in [&m1, &m2] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    // The coach's own CoachHub OAuth grant.
    let mut token = UserOAuthToken::new(
        coach.id,
        coach.tenant.to_string(),
        COACHHUB.to_owned(),
        COACH_TOKEN.to_owned(),
        None,
        Some(now + ChronoDuration::hours(6)),
        None,
    );
    token.provider_user_id = Some("ch100".to_owned());
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(
            coach.id,
            coach.tenant,
            COACHHUB,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = res.clone();
    let router = DelegatedConnectionRoutes::routes(Arc::clone(&res));
    World {
        runtime,
        router,
        seen,
        group,
        coach,
        m1,
        m2,
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
    }

    async fn get(&self, path: &str, who: &Person) -> (StatusCode, Value) {
        let resp = AxumTestRequest::get(path)
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    async fn post(&self, path: &str, who: &Person, body: &Value) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post(path)
            .header("authorization", &who.auth)
            .json(body)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    async fn delete(&self, path: &str, who: &Person) -> StatusCode {
        AxumTestRequest::delete(path)
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await
            .status_code()
    }

    async fn propose(&self, athlete: &str, member: &Person) -> (StatusCode, Value) {
        self.post(
            &self.links_path(),
            &self.coach,
            &json!({
                "provider": COACHHUB,
                "provider_athlete_id": athlete,
                "member_user_id": member.id,
            }),
        )
        .await
    }

    /// The provider `who`'s `CoachHub` reads are served by, or the refusal's
    /// words.
    async fn authenticate(&self, who: &Person) -> Result<Box<dyn CoreFitnessProvider>, String> {
        AuthService::new(Arc::clone(&self.runtime))
            .create_authenticated_provider(COACHHUB, who.id, Some(&who.tenant.to_string()))
            .await
            .map_err(|response| response.error.unwrap_or_default())
    }

    /// Wait for a read of `athlete` through the coach's token: the warm-up
    /// a confirm spawns.
    async fn wait_for_read_of(&self, athlete: &str) {
        let deadline = Instant::now() + SPAWN_DEADLINE;
        while !self
            .seen
            .reads()
            .iter()
            .any(|read| read.athlete.as_deref() == Some(athlete))
        {
            assert!(Instant::now() < deadline, "the warm-up read never happened");
            sleep(Duration::from_millis(50)).await;
        }
    }
}

/// The coach reads the roster their stored grant lists, with or without
/// naming the platform: it is the one they connected.
async fn the_coach_reads_the_roster_through_their_grant(w: &World) {
    for path in [
        format!("{}/roster?provider={COACHHUB}", w.links_path()),
        format!("{}/roster", w.links_path()),
    ] {
        let (status, body) = w.get(&path, &w.coach).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["provider"], COACHHUB, "{body}");
        let ids: Vec<&str> = body["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["provider_athlete_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [M1_ATHLETE, M2_ATHLETE], "{body}");
    }
    let roster_read = w
        .seen
        .reads()
        .first()
        .cloned()
        .expect("the roster was read");
    assert_eq!(
        roster_read,
        Read {
            athlete: None,
            token: Some(COACH_TOKEN.to_owned()),
            kind: Some(CredentialKind::OAuthBearer),
        }
    );
}

/// An athlete id the platform could not have issued is refused on propose,
/// with its reason, before any provider is built.
async fn a_malformed_athlete_id_is_refused_on_propose(w: &World) {
    let built = w.seen.built();
    for refused in ["", "ch2/../ch3"] {
        let (status, body) = w.propose(refused, &w.m1).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused:?}: {body}");
        assert_eq!(body["details"]["reason"], "invalid_athlete", "{body}");
        assert_eq!(body["details"]["provider"], COACHHUB, "{body}");
    }
    assert_eq!(
        w.seen.built(),
        built,
        "no provider was built for a refused id"
    );
}

/// The coach links the first member, who confirms; the second is proposed
/// and never confirms. Returns the confirmed link's id.
async fn the_coach_links_and_the_member_confirms(w: &World) -> String {
    let (status, proposed) = w.propose(M1_ATHLETE, &w.m1).await;
    assert_eq!(status, StatusCode::CREATED, "{proposed}");
    assert_eq!(proposed["provider"], COACHHUB, "{proposed}");
    let id = proposed["id"].as_str().unwrap().to_owned();
    let (status, confirmed) = w
        .post(
            &format!("{}/{id}/confirm", w.links_path()),
            &w.m1,
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(confirmed["status"], "confirmed", "{confirmed}");
    w.wait_for_read_of(M1_ATHLETE).await;

    let (status, pending) = w.propose(M2_ATHLETE, &w.m2).await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["status"], "proposed", "{pending}");
    id
}

/// The confirmed member's read is served by a provider built for their
/// athlete id, holding the coach's grant.
async fn the_confirmed_member_reads_through_the_coach(w: &World) {
    let provider = w
        .authenticate(&w.m1)
        .await
        .expect("served through the coach's grant");
    let activities = provider
        .get_activities_with_params(&ActivityQueryParams {
            limit: Some(5),
            offset: None,
            before: None,
            after: None,
        })
        .await
        .expect("the member's activities read");
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].id(), format!("{M1_ATHLETE}-run"));
    assert_eq!(
        w.seen.reads().last().cloned(),
        Some(Read {
            athlete: Some(M1_ATHLETE.to_owned()),
            token: Some(COACH_TOKEN.to_owned()),
            kind: Some(CredentialKind::OAuthBearer),
        })
    );
}

/// A member whose link was only proposed reads nothing through it: the read
/// is refused as their own missing connection, and nothing is built.
async fn an_unconfirmed_member_reads_nothing(w: &World) {
    let built = w.seen.built();
    let refused = w
        .authenticate(&w.m2)
        .await
        .err()
        .expect("an unconfirmed link reads nothing");
    assert!(
        refused.contains("CoachHub") || refused.contains(COACHHUB),
        "{refused}"
    );
    assert_eq!(w.seen.built(), built, "nothing was built: {refused}");
}

/// A link the coach ended reads nothing, and nothing is built.
async fn an_ended_link_reads_nothing(w: &World, link: &str) {
    let status = w
        .delete(&format!("{}/{link}", w.links_path()), &w.coach)
        .await;
    assert!(status.is_success(), "{status}");
    let built = w.seen.built();
    let refused = w
        .authenticate(&w.m1)
        .await
        .err()
        .expect("an ended link reads nothing");
    assert!(!refused.is_empty());
    assert_eq!(w.seen.built(), built, "nothing was built: {refused}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_reads_any_coach_roster_platform_through_the_coach() {
    let w = world().await;

    the_coach_reads_the_roster_through_their_grant(&w).await;
    a_malformed_athlete_id_is_refused_on_propose(&w).await;
    let link = the_coach_links_and_the_member_confirms(&w).await;
    the_confirmed_member_reads_through_the_coach(&w).await;
    an_unconfirmed_member_reads_nothing(&w).await;
    an_ended_link_reads_nothing(&w, &link).await;
}
