// ABOUTME: get_provider_roster and propose_athlete_link — a coach's coaching-platform roster and link proposals from chat
// ABOUTME: Pins the states and counts, that no roster name, email or id reaches the model, the refusals and the transport gate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(
    feature = "tools-groups",
    feature = "provider-intervals-icu",
    feature = "provider-sciotte",
    feature = "client-notifications"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A human coach reads their coaching-platform roster from their own
//! conversation (`get_provider_roster`) and asks a member of a group they
//! coach to confirm a link to their roster entry (`propose_athlete_link`).
//!
//! The registry is the built-in one, so each platform keeps its own terms:
//! TrainingPeaks keeps its data first-party, Intervals.icu serves any
//! transport. The Intervals.icu provider is a stand-in (a test double, per the
//! repo's mock rule: the real one's roster read is pinned by
//! `delegated_intervals_icu_read_test`) answering the roster each test writes
//! and counting its reads. TrainingPeaks reads its roster on the scraper
//! service, which no test here reaches: its roster is served from the cache,
//! as a coach account's is, so only the transport gate decides whether it is
//! read at all.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_cache::CacheResource;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::{
    INTERVALS_ICU, SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    Activity, Athlete, CoachAccessStatus, ConnectionType, DelegatedConnection, DelegationStatus,
    ProviderAccountRole, RosterAccess, RosterAthlete, Stats, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::TenantId as NoticeTenantId;
use pierre_providers::ai_scope::{serve_over, tracking, Provenance};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::delegation::{CoachRoster, DelegatedReads};
use pierre_providers::registry::ProviderRegistry;
use pierre_services::delegated_connections::roster_cache_key;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::athlete_link::ProposeAthleteLinkTool;
use pierre_tool_runtime::implementations::provider_roster::GetProviderRosterTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

const COACH_EMAIL: &str = "coach@roster-tool.test";
/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// How long a fire-and-forget notice gets to land in the feed.
const NOTICE_DEADLINE: StdDuration = StdDuration::from_secs(10);

/// The Intervals.icu roster the stand-in serves, the account it says the key
/// belongs to, and how many times it was read.
#[derive(Clone)]
struct Roster {
    athletes: Arc<Mutex<Vec<RosterAthlete>>>,
    account_email: Arc<Mutex<String>>,
    reads: Arc<AtomicUsize>,
}

impl Default for Roster {
    fn default() -> Self {
        Self {
            athletes: Arc::default(),
            account_email: Arc::new(Mutex::new(COACH_EMAIL.to_owned())),
            reads: Arc::default(),
        }
    }
}

impl Roster {
    fn set(&self, athletes: Vec<RosterAthlete>) {
        *self.athletes.lock().unwrap() = athletes;
    }

    /// The account the stand-in says the key belongs to.
    fn belongs_to(&self, email: &str) {
        email.clone_into(&mut self.account_email.lock().unwrap());
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// The stand-in Intervals.icu provider: it answers the roster the test wrote
/// and nothing else.
struct StandIn {
    config: ProviderConfig,
    credentials: Mutex<Option<OAuth2Credentials>>,
    roster: Roster,
}

#[async_trait]
impl FitnessProvider for StandIn {
    fn name(&self) -> &'static str {
        INTERVALS_ICU
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
        Err(AppError::internal("not read by this test"))
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
        if self.credentials.lock().unwrap().is_none() {
            return Err(AppError::internal("roster read without a credential"));
        }
        self.roster.reads.fetch_add(1, Ordering::SeqCst);
        Ok(CoachRoster {
            account_email: Some(self.roster.account_email.lock().unwrap().clone()),
            athletes: self.roster.athletes.lock().unwrap().clone(),
        })
    }
}

struct Factory {
    roster: Roster,
}

impl Factory {
    fn build(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        Box::new(StandIn {
            config,
            credentials: Mutex::new(None),
            roster: self.roster.clone(),
        })
    }
}

impl ProviderFactory for Factory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(self.build(config))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[INTERVALS_ICU]
    }

    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for Factory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        if athlete_id.is_empty() || !athlete_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(AppError::invalid_input("not an Intervals.icu athlete id"));
        }
        Ok(())
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        self.check_athlete_id(athlete_id)?;
        Ok(self.build(config))
    }
}

/// One user, in their own tenant.
#[derive(Clone, Copy)]
struct Person {
    id: Uuid,
    tenant: TenantId,
}

struct World {
    res: Arc<ServerContext>,
    roster: Roster,
    owner: Person,
    coach: Person,
    agent_id: String,
}

impl World {
    async fn new() -> Self {
        let roster = Roster::default();
        let base = create_test_server_resources().await.unwrap();
        // The built-in providers, each with its own terms; only the
        // Intervals.icu API is stood in.
        let mut registry = ProviderRegistry::new();
        registry.register_factory(
            INTERVALS_ICU,
            Box::new(Factory {
                roster: roster.clone(),
            }),
        );
        let mut context = (*base).clone();
        context.fitness.provider_registry = Arc::new(registry);
        let res = Arc::new(context);

        let owner = person(&res, "owner@roster-tool.test", "Olive Owner", true).await;
        let coach = person(&res, COACH_EMAIL, "Casey Coach", true).await;
        res.common
            .repos
            .users
            .set_manages_roster(coach.id, true)
            .await
            .unwrap();
        let agent_id = create_group_agent(&res.common.repos, owner.id, owner.tenant)
            .await
            .unwrap();
        Self {
            res,
            roster,
            owner,
            coach,
            agent_id,
        }
    }

    /// An active group `coach` coaches and `owner` owns.
    async fn group(&self, name: &str, coach: Person) -> CoachingGroup {
        let repos = &self.res.common.repos;
        let now = Utc::now();
        let group = repos
            .groups
            .create_group(
                self.owner.tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: self.owner.tenant.to_string(),
                    name: name.to_owned(),
                    description: None,
                    agent_id: self.agent_id.clone(),
                    owner_id: self.owner.id,
                    coach_user_id: None,
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 20,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        assert!(repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), self.owner.tenant)
            .await
            .unwrap());
        repos
            .groups
            .get_group(&group.id.to_string(), self.owner.tenant)
            .await
            .unwrap()
            .expect("the group was just made")
    }

    /// `who` joins `group`, sharing with its coach or not.
    async fn join(&self, group: &CoachingGroup, who: Person, shares: bool) {
        let now = Utc::now();
        self.res
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: who.id,
                tenant_id: who.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: shares,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// `coach`'s own Intervals.icu API key.
    async fn connect_intervals(&self, coach: Person) {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: coach.id,
                tenant_id: coach.tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: "coach-key".to_owned(),
                refresh_token: None,
                token_type: API_KEY_TOKEN_TYPE.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some(COACH_ATHLETE.to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                coach.id,
                coach.tenant,
                INTERVALS_ICU,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }

    /// `coach`'s own TrainingPeaks coach-account session, with `athletes`
    /// cached as the roster it last read: a coach account's roster is served
    /// from the cache, so no scraper is reached.
    async fn connect_trainingpeaks(&self, coach: Person, athletes: Vec<RosterAthlete>) {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: coach.id,
                tenant_id: coach.tenant.to_string(),
                provider: SCIOTTE_TRAININGPEAKS.to_owned(),
                access_token: json!({ "session_id": "coach-session", "cookies": [] }).to_string(),
                refresh_token: None,
                token_type: TOKEN_TYPE_SESSION.to_owned(),
                expires_at: Some(now + Duration::hours(6)),
                scope: None,
                provider_user_id: None,
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                coach.id,
                coach.tenant,
                SCIOTTE_TRAININGPEAKS,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
        assert!(repos
            .provider_connections
            .set_account_role(
                coach.id,
                coach.tenant,
                SCIOTTE_TRAININGPEAKS,
                ProviderAccountRole::Coach,
            )
            .await
            .unwrap());
        repos
            .users
            .record_provider_terms(
                coach.id,
                SCIOTTE_TRAININGPEAKS,
                provider_terms_version(SCIOTTE_TRAININGPEAKS).expect("TrainingPeaks has a notice"),
            )
            .await
            .unwrap();
        self.res
            .common
            .cache
            .set(
                &roster_cache_key(coach.id, coach.tenant, SCIOTTE_TRAININGPEAKS),
                &athletes,
                CacheResource::ProviderRoster.recommended_ttl(),
            )
            .await
            .unwrap();
    }

    /// A confirmed Intervals.icu link of `coach`'s in `group` for roster
    /// athlete `athlete` as `member`, whose roster email is `email`.
    async fn linked(
        &self,
        coach: Person,
        group: &CoachingGroup,
        member: Person,
        athlete: &str,
        email: &str,
    ) -> DelegatedConnection {
        let repos = &self.res.common.repos;
        let link = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                INTERVALS_ICU.to_owned(),
                group.id,
                coach.id,
                coach.tenant,
                member.id,
                RosterAthlete::new(athlete, None, Some(email.to_owned())),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet");
        repos
            .delegated_connections
            .confirm(link.id, member.id, member.tenant, Utc::now())
            .await
            .unwrap()
            .expect("the proposal names the member")
    }

    /// Call `tool` as `who` with `args`: its payload, and whether it refused.
    async fn call<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        who: Person,
        args: Value,
    ) -> (Value, bool) {
        let runtime: Arc<dyn ToolRuntime> = Arc::clone(&self.res) as Arc<dyn ToolRuntime>;
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = tool.execute(&runtime, &ctx, args).await;
        (
            response
                .structured_content
                .expect("tool result carries structured content"),
            response.is_error,
        )
    }

    async fn roster(&self, who: Person, args: Value) -> (Value, bool) {
        self.call(&GetProviderRosterTool, who, args).await
    }

    async fn link(&self, who: Person, args: Value) -> (Value, bool) {
        self.call(&ProposeAthleteLinkTool, who, args).await
    }

    /// Every live link `member` holds.
    async fn live_links(&self, member: Person) -> Vec<DelegatedConnection> {
        self.res
            .common
            .repos
            .delegated_connections
            .list_live_for_member(member.id)
            .await
            .unwrap()
    }

    /// The `delegation_proposed` notices `member` reads, waiting until
    /// `count` of them have landed.
    async fn proposal_notices(&self, member: Person, count: usize) -> usize {
        let service = ToolRuntime::notification_service(self.res.as_ref())
            .expect("the test server wires notifications");
        let started = Instant::now();
        loop {
            let (rows, _, _) = service
                .list_notifications(
                    member.id,
                    NoticeTenantId(member.tenant.as_uuid()),
                    50,
                    0,
                    None,
                    false,
                )
                .await
                .unwrap();
            let landed = rows
                .iter()
                .filter(|n| n.notification_type == "delegation_proposed")
                .count();
            if landed >= count {
                return landed;
            }
            assert!(
                started.elapsed() < NOTICE_DEADLINE,
                "{landed} of {count} proposal notices landed"
            );
            sleep(StdDuration::from_millis(100)).await;
        }
    }
}

/// A user whose email is `email` and whose display name is `name`.
async fn person(res: &Arc<ServerContext>, email: &str, name: &str, verified: bool) -> Person {
    let (id, _, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    if verified {
        repos.email_verification.mark_verified(id).await.unwrap();
    }
    Person { id, tenant }
}

fn athlete(
    id: &str,
    name: &str,
    email: Option<&str>,
    access: Option<RosterAccess>,
) -> RosterAthlete {
    RosterAthlete::new(id, Some(name.to_owned()), email.map(str::to_owned)).with_access(access)
}

/// The platform `provider` in a roster payload.
fn platform<'a>(payload: &'a Value, provider: &str) -> &'a Value {
    payload["platforms"]
        .as_array()
        .expect("platforms array")
        .iter()
        .find(|p| p["provider"] == provider)
        .unwrap_or_else(|| panic!("{provider} missing: {payload:#}"))
}

/// Each member's (name, state) on a platform.
fn member_states(platform: &Value) -> Vec<(String, String)> {
    platform["members"]
        .as_array()
        .expect("members array")
        .iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap().to_owned(),
                m["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_roster_names_members_by_their_dravr_name_and_counts_everyone_else() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let alex = person(&w.res, "alex@roster-tool.test", "Alex Athlete", true).await;
    let sam = person(&w.res, "sam@roster-tool.test", "Sam Swimmer", true).await;
    let robin = person(&w.res, "robin@roster-tool.test", "Robin Rower", false).await;
    let quinn = person(&w.res, "quinn@roster-tool.test", "Quinn Quiet", true).await;
    let blake = person(&w.res, "blake@roster-tool.test", "Blake Blocked", true).await;
    // A Dravr account outside the coach's groups reads like an email no one
    // holds.
    person(&w.res, "outsider@roster-tool.test", "Oscar Outside", true).await;
    for member in [alex, sam, robin, blake] {
        w.join(&squad, member, true).await;
    }
    w.join(&squad, quinn, false).await;
    w.connect_intervals(w.coach).await;
    w.linked(w.coach, &squad, alex, "i201", "alex@roster-tool.test")
        .await;

    let coach_access = Some(RosterAccess::Coach);
    // The coach's own entry is not served: Intervals.icu's roster mapping
    // keeps it as the account's email and lists only the athletes.
    w.roster.set(vec![
        // The roster names the linked member as someone else: only their
        // Dravr name is shown.
        athlete(
            "i201",
            "Alexandra Roster",
            Some("alex@roster-tool.test"),
            coach_access,
        ),
        athlete(
            "i202",
            "Samuel Roster",
            Some("sam@roster-tool.test"),
            coach_access,
        ),
        athlete(
            "i203",
            "Robin Roster",
            Some("robin@roster-tool.test"),
            coach_access,
        ),
        athlete(
            "i204",
            "Quinn Roster",
            Some("quinn@roster-tool.test"),
            coach_access,
        ),
        athlete(
            "i205",
            "Blake Roster",
            Some("blake@roster-tool.test"),
            Some(RosterAccess::NoAccess),
        ),
        athlete(
            "i206",
            "Oscar Roster",
            Some("outsider@roster-tool.test"),
            Some(RosterAccess::Follower),
        ),
        athlete(
            "i207",
            "Stranger Roster",
            Some("stranger@elsewhere.test"),
            coach_access,
        ),
        athlete("i208", "Nomail Roster", None, None),
    ]);

    let (payload, refused) = w.roster(w.coach, json!({})).await;
    assert!(!refused, "{payload:#}");
    assert_eq!(payload["platforms"].as_array().unwrap().len(), 1);
    let intervals = platform(&payload, "intervals_icu");
    assert_eq!(intervals["roster_size"], 8);
    assert_eq!(
        member_states(intervals),
        [
            ("Alex Athlete".to_owned(), "linked".to_owned()),
            ("Blake Blocked".to_owned(), "no_access".to_owned()),
            ("Quinn Quiet".to_owned(), "member_not_sharing".to_owned()),
            (
                "Robin Rower".to_owned(),
                "member_email_unverified".to_owned()
            ),
            ("Sam Swimmer".to_owned(), "proposable".to_owned()),
        ],
        "{payload:#}"
    );
    assert!(intervals["members"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["group"] == "Squad"));
    assert_eq!(intervals["members"][0]["access"], "coach");
    assert_eq!(intervals["members"][1]["access"], "no_access");
    assert_eq!(
        intervals["others"],
        json!([
            { "state": "roster_email_missing", "access": "unknown", "athletes": 1 },
            { "state": "not_in_your_groups", "access": "coach", "athletes": 1 },
            { "state": "not_in_your_groups", "access": "follower", "athletes": 1 },
        ]),
        "{payload:#}"
    );
    assert_eq!(payload["coach_access"], true);
    assert_eq!(payload["groups_coached"], 1);
    // The coach is attached by a coach invite, with no membership of their
    // own: they cannot issue the group's invite.
    let steps = payload["next_steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1, "{payload:#}");
    assert!(steps[0].as_str().unwrap().contains("ask a group's owner"));

    // Nothing the platform lists about anyone reaches the model: no roster
    // name, no email, no athlete id.
    let text = payload.to_string();
    for leak in [
        "Roster", "@", "i201", "i202", "i203", "i204", "i205", "i206", "i207", "i208",
    ] {
        assert!(!text.contains(leak), "{leak} leaked: {payload:#}");
    }
    assert_eq!(w.roster.reads(), 1);
}

#[tokio::test]
async fn a_coach_with_no_group_reads_their_roster_and_how_coach_access_is_granted() {
    let w = World::new().await;
    let newcomer = person(&w.res, "newcomer@roster-tool.test", "Nico Newcomer", true).await;
    w.roster.belongs_to("newcomer@roster-tool.test");
    w.connect_intervals(newcomer).await;
    w.roster.set(vec![
        athlete(
            "i301",
            "One Roster",
            Some("one@elsewhere.test"),
            Some(RosterAccess::Coach),
        ),
        athlete(
            "i302",
            "Two Roster",
            Some("two@elsewhere.test"),
            Some(RosterAccess::Coach),
        ),
    ]);

    let (payload, refused) = w.roster(newcomer, json!({})).await;
    assert!(!refused, "{payload:#}");
    let intervals = platform(&payload, "intervals_icu");
    assert_eq!(
        intervals["members"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    assert_eq!(
        intervals["others"],
        json!([{ "state": "not_in_your_groups", "access": "coach", "athletes": 2 }])
    );
    assert_eq!(payload["coach_access"], false);
    // Athletes whose standing the platform did not report are no evidence,
    // so no request was filed for this coach.
    assert_eq!(payload["coach_access_request_pending"], false);
    assert_eq!(payload["groups_coached"], 0);
    let steps: Vec<&str> = payload["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(steps[0].contains("coach access"), "{steps:?}");
    assert!(
        steps[1].contains("You coach no Dravr group yet"),
        "{steps:?}"
    );
}

#[tokio::test]
async fn a_coach_whose_roster_files_their_request_reads_that_it_waits() {
    let w = World::new().await;
    let newcomer = person(&w.res, "waiting@roster-tool.test", "Wren Waiting", true).await;
    w.roster.belongs_to("waiting@roster-tool.test");
    w.connect_intervals(newcomer).await;
    // An athlete the key coaches and the platform lists as active: the live
    // read files the coach's request for coach access.
    w.roster.set(vec![athlete(
        "i401",
        "Active Roster",
        Some("active@elsewhere.test"),
        Some(RosterAccess::Coach),
    )
    .with_active(Some(true))]);

    let (payload, refused) = w.roster(newcomer, json!({})).await;
    assert!(!refused, "{payload:#}");
    assert_eq!(payload["coach_access"], false);
    assert_eq!(payload["coach_access_request_pending"], true, "{payload:#}");
    let steps = payload["next_steps"].as_array().unwrap();
    assert!(
        steps[0]
            .as_str()
            .unwrap()
            .contains("waiting for the Dravr team"),
        "{payload:#}"
    );
    let repos = &w.res.common.repos;
    let request = repos
        .coach_access_requests
        .latest_for_user(newcomer.id)
        .await
        .unwrap()
        .expect("the live read filed the request");
    assert_eq!(request.status, CoachAccessStatus::Pending);
    let user = repos.users.get_global(newcomer.id).await.unwrap().unwrap();
    assert!(!user.manages_roster, "the request grants nothing");
}

#[tokio::test]
async fn a_roster_that_cannot_be_read_is_refused_with_its_reason() {
    let w = World::new().await;

    let (payload, refused) = w.roster(w.coach, json!({})).await;
    assert!(refused);
    assert_eq!(
        payload["reason"], "coach_platform_not_connected",
        "{payload:#}"
    );

    let (payload, refused) = w.roster(w.coach, json!({ "provider": "strava" })).await;
    assert!(refused);
    assert_eq!(payload["reason"], "unsupported_provider", "{payload:#}");

    // A key whose account is not the coach's own lists no roster for them.
    w.connect_intervals(w.coach).await;
    w.roster.belongs_to("someone-else@roster-tool.test");
    let (payload, refused) = w
        .roster(w.coach, json!({ "provider": "intervals_icu" }))
        .await;
    assert!(refused);
    assert_eq!(
        payload["reason"], "coach_platform_email_mismatch",
        "{payload:#}"
    );
    assert_eq!(payload["provider"], "intervals_icu");
}

#[tokio::test]
async fn a_group_s_room_is_refused_before_any_roster_is_read() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    w.connect_intervals(w.coach).await;
    let sam = person(&w.res, "sam@roster-tool.test", "Sam Swimmer", true).await;
    w.join(&squad, sam, true).await;
    w.roster.set(vec![athlete(
        "i202",
        "Samuel Roster",
        Some("sam@roster-tool.test"),
        Some(RosterAccess::Coach),
    )]);

    let chat = &w.res.common.repos.chat;
    let room = chat
        .create_conversation(
            &w.coach.id.to_string(),
            w.coach.tenant,
            "squad",
            "gemini-2.0-flash",
            None,
            Some(&squad.id.to_string()),
        )
        .await
        .unwrap();
    let (payload, refused) = CONVERSATION_ID
        .scope(Some(room.id.clone()), w.roster(w.coach, json!({})))
        .await;
    assert!(refused);
    assert_eq!(payload["reason"], "group_room", "{payload:#}");
    let (payload, refused) = CONVERSATION_ID
        .scope(Some(room.id), w.link(w.coach, json!({ "athlete": "Sam" })))
        .await;
    assert!(refused);
    assert_eq!(payload["reason"], "group_room", "{payload:#}");

    // A conversation the coach's tenant cannot resolve is a shared room's.
    let shared = chat
        .create_conversation(
            &w.owner.id.to_string(),
            w.owner.tenant,
            "room",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap();
    let (payload, refused) = CONVERSATION_ID
        .scope(Some(shared.id), w.roster(w.coach, json!({})))
        .await;
    assert!(refused);
    assert_eq!(payload["reason"], "group_room", "{payload:#}");
    assert_eq!(w.roster.reads(), 0, "no roster is read for a room");
    assert_eq!(w.live_links(sam).await, []);

    // The coach's own conversation reads it.
    let own = chat
        .create_conversation(
            &w.coach.id.to_string(),
            w.coach.tenant,
            "mine",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap();
    let (payload, refused) = CONVERSATION_ID
        .scope(Some(own.id), w.roster(w.coach, json!({})))
        .await;
    assert!(!refused, "{payload:#}");
    assert_eq!(w.roster.reads(), 1);
}

#[tokio::test]
async fn trainingpeaks_is_refused_over_mcp_before_its_roster_is_read_while_intervals_is_served() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let sam = person(&w.res, "sam@roster-tool.test", "Sam Swimmer", true).await;
    let tara = person(&w.res, "tara@roster-tool.test", "Tara Trainer", true).await;
    w.join(&squad, sam, true).await;
    w.join(&squad, tara, true).await;
    w.connect_intervals(w.coach).await;
    w.roster.set(vec![athlete(
        "i202",
        "Samuel Roster",
        Some("sam@roster-tool.test"),
        Some(RosterAccess::Coach),
    )]);
    w.connect_trainingpeaks(
        w.coach,
        vec![RosterAthlete::new(
            "900001",
            Some("Tara Roster".to_owned()),
            Some("tara@roster-tool.test".to_owned()),
        )],
    )
    .await;

    let (payload, refused) = serve_over(Transport::McpHttp, w.roster(w.coach, json!({}))).await;
    assert!(!refused, "{payload:#}");
    let trainingpeaks = platform(&payload, "trainingpeaks");
    assert_eq!(
        trainingpeaks["refusal"]["reason"], "unavailable_over_transport",
        "{payload:#}"
    );
    assert!(
        trainingpeaks.get("roster_size").is_none(),
        "the cached TrainingPeaks roster is not read over MCP: {payload:#}"
    );
    assert_eq!(
        trainingpeaks["members"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    let intervals = platform(&payload, "intervals_icu");
    assert!(intervals.get("refusal").is_none(), "{payload:#}");
    assert_eq!(
        member_states(intervals),
        [("Sam Swimmer".to_owned(), "proposable".to_owned())]
    );
    assert_eq!(w.roster.reads(), 1);

    // Named alone over MCP, TrainingPeaks' refusal is the answer.
    let (payload, refused) = serve_over(
        Transport::McpHttp,
        w.roster(w.coach, json!({ "provider": "trainingpeaks" })),
    )
    .await;
    assert!(refused);
    assert_eq!(
        payload["reason"], "unavailable_over_transport",
        "{payload:#}"
    );
    assert_eq!(payload["provider"], "trainingpeaks");

    // In Dravr's own app the same cached roster is read, and the turn is
    // stamped as serving first-party-only data.
    let turn = Provenance::new();
    let (payload, refused) = tracking(
        turn.clone(),
        serve_over(
            Transport::WebApp,
            w.roster(w.coach, json!({ "provider": "trainingpeaks" })),
        ),
    )
    .await;
    assert!(!refused, "{payload:#}");
    assert_eq!(
        member_states(platform(&payload, "trainingpeaks")),
        [("Tara Trainer".to_owned(), "proposable".to_owned())]
    );
    assert_eq!(turn.policy(), TransportPolicy::FirstPartyOnly);
}

#[tokio::test]
async fn a_member_is_proposed_a_link_and_notified() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let sam = person(&w.res, "sam@roster-tool.test", "Sam Swimmer", true).await;
    w.join(&squad, sam, true).await;
    w.connect_intervals(w.coach).await;
    w.roster.set(vec![athlete(
        "i202",
        "Samuel Roster",
        Some("sam@roster-tool.test"),
        Some(RosterAccess::Coach),
    )]);

    let (payload, refused) = w.link(w.coach, json!({ "athlete": "sam" })).await;
    assert!(!refused, "{payload:#}");
    assert_eq!(
        payload,
        json!({
            "member": "Sam Swimmer",
            "group": "Squad",
            "provider": "intervals_icu",
            "next_step": "Sam Swimmer is asked to confirm the link in the Dravr app; nothing is \
                          read until they do.",
        })
    );
    let links = w.live_links(sam).await;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].status, DelegationStatus::Proposed);
    assert_eq!(links[0].provider, INTERVALS_ICU);
    assert_eq!(links[0].provider_athlete_id, "i202");
    assert_eq!(links[0].group_id, squad.id);
    assert_eq!(w.proposal_notices(sam, 1).await, 1);

    // Asked again, the proposal already holds them.
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Sam" })).await;
    assert!(refused);
    assert_eq!(payload["reason"], "already_proposed", "{payload:#}");
    assert_eq!(w.live_links(sam).await.len(), 1);
}

#[tokio::test]
async fn a_proposal_is_refused_when_no_one_member_or_roster_entry_binds() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let alex = person(&w.res, "alex@roster-tool.test", "Alex Athlete", true).await;
    let alma = person(&w.res, "alma@roster-tool.test", "Alma Athlete", true).await;
    let blake = person(&w.res, "blake@roster-tool.test", "Blake Blocked", true).await;
    let nora = person(&w.res, "nora@roster-tool.test", "Nora Nowhere", true).await;
    let quinn = person(&w.res, "quinn@roster-tool.test", "Quinn Quiet", true).await;
    for member in [alex, alma, blake, nora] {
        w.join(&squad, member, true).await;
    }
    w.join(&squad, quinn, false).await;
    w.connect_intervals(w.coach).await;
    w.linked(w.coach, &squad, alex, "i201", "alex@roster-tool.test")
        .await;
    w.roster.set(vec![
        athlete(
            "i201",
            "Alex Roster",
            Some("alex@roster-tool.test"),
            Some(RosterAccess::Coach),
        ),
        athlete(
            "i205",
            "Blake Roster",
            Some("blake@roster-tool.test"),
            Some(RosterAccess::NoAccess),
        ),
        athlete(
            "i209",
            "Quinn Roster",
            Some("quinn@roster-tool.test"),
            Some(RosterAccess::Coach),
        ),
    ]);

    let mut refusals = Vec::new();
    let mut refused_as = |payload: Value, refused: bool, reason: &str| {
        assert!(refused, "{payload:#}");
        assert_eq!(payload["reason"], reason, "{payload:#}");
        refusals.push(payload);
    };
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Athlete" })).await;
    refused_as(payload, refused, "member_ambiguous");
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Zed" })).await;
    refused_as(payload, refused, "member_not_found");
    // The coach is attached to the group, never one of its members.
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Casey" })).await;
    refused_as(payload, refused, "member_not_found");

    // A member who stopped sharing is refused before any roster is read.
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Quinn" })).await;
    refused_as(payload, refused, "member_not_sharing");
    assert_eq!(w.roster.reads(), 0);

    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Nora" })).await;
    refused_as(payload, refused, "no_roster_match");
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Alex" })).await;
    refused_as(payload, refused, "athlete_already_linked");
    let (payload, refused) = w.link(w.coach, json!({ "athlete": "Blake" })).await;
    refused_as(payload, refused, "athlete_no_access");
    let (payload, refused) = w.link(w.coach, json!({})).await;
    refused_as(payload, refused, "athlete_required");

    // No refusal names anything the roster lists.
    for payload in &refusals {
        for leak in ["Roster", "@", "i201", "i205", "i209"] {
            assert!(!payload.to_string().contains(leak), "{leak}: {payload:#}");
        }
    }
    assert!(w.live_links(blake).await.is_empty(), "nothing was stored");
    assert_eq!(w.live_links(nora).await, []);
    assert_eq!(w.live_links(alex).await.len(), 1);
}

#[tokio::test]
async fn over_mcp_a_coach_on_both_platforms_proposes_through_intervals_only() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let alex = person(&w.res, "alex@roster-tool.test", "Alex Athlete", true).await;
    w.join(&squad, alex, true).await;
    w.connect_intervals(w.coach).await;
    w.roster.set(vec![athlete(
        "i201",
        "Alex Roster",
        Some("alex@roster-tool.test"),
        Some(RosterAccess::Coach),
    )]);
    // TrainingPeaks lists Alex too, under the same email.
    w.connect_trainingpeaks(
        w.coach,
        vec![RosterAthlete::new(
            "900001",
            None,
            Some("alex@roster-tool.test".to_owned()),
        )],
    )
    .await;

    // Named over MCP, TrainingPeaks is refused: no roster read, no row.
    let (payload, refused) = serve_over(
        Transport::McpHttp,
        w.link(
            w.coach,
            json!({ "athlete": "Alex", "provider": "trainingpeaks" }),
        ),
    )
    .await;
    assert!(refused);
    assert_eq!(
        payload["reason"], "unavailable_over_transport",
        "{payload:#}"
    );
    assert_eq!(w.roster.reads(), 0);
    assert_eq!(w.live_links(alex).await, []);

    // In the app both rosters list Alex: the coach is asked which.
    let (payload, refused) = serve_over(
        Transport::WebApp,
        w.link(w.coach, json!({ "athlete": "Alex" })),
    )
    .await;
    assert!(refused);
    assert_eq!(payload["reason"], "provider_ambiguous", "{payload:#}");
    assert_eq!(w.live_links(alex).await, []);

    // Unnamed over MCP, only Intervals.icu is considered.
    let (payload, refused) = serve_over(
        Transport::McpHttp,
        w.link(w.coach, json!({ "athlete": "Alex" })),
    )
    .await;
    assert!(!refused, "{payload:#}");
    assert_eq!(payload["provider"], "intervals_icu");
    let links = w.live_links(alex).await;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider, INTERVALS_ICU);
    assert_eq!(links[0].provider_athlete_id, "i201");
    // One notice, the Intervals.icu proposal's: the refused calls sent none.
    assert_eq!(w.proposal_notices(alex, 1).await, 1);
}

#[tokio::test]
async fn an_in_app_trainingpeaks_proposal_stamps_the_turn_first_party_only() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach).await;
    let tara = person(&w.res, "tara@roster-tool.test", "Tara Trainer", true).await;
    w.join(&squad, tara, true).await;
    w.connect_trainingpeaks(
        w.coach,
        vec![RosterAthlete::new(
            "900002",
            None,
            Some("tara@roster-tool.test".to_owned()),
        )],
    )
    .await;

    let turn = Provenance::new();
    let (payload, refused) = tracking(
        turn.clone(),
        serve_over(
            Transport::WebApp,
            w.link(w.coach, json!({ "athlete": "Tara" })),
        ),
    )
    .await;
    assert!(!refused, "{payload:#}");
    assert_eq!(payload["provider"], "trainingpeaks");
    let links = w.live_links(tara).await;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider, SCIOTTE_TRAININGPEAKS);
    assert_eq!(turn.policy(), TransportPolicy::FirstPartyOnly);
}
