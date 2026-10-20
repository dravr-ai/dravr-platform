// ABOUTME: An Intervals.icu key whose roster coaches an active athlete files its owner's coach-access request (K15)
// ABOUTME: Pins when a live read or the link probe files one, that it never grants, and that nothing is asked twice

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Any Intervals.icu account can give another one edit access, so a key's
//! roster is evidence its owner coaches, never proof: it files the owner's
//! coach-access request for a super-admin to grant, and grants nothing
//! itself (ADR-018). These cases drive the two reads that file it — a live
//! roster read through the coach-platform path, and the probe the link route
//! spawns off the request — against a loopback Intervals.icu stand-in (a test
//! double, per the repo's mock rule) that serves the coach's own entry beside
//! the roster each case writes, and counts its roster reads. A request filed
//! this way names no group, so the last cases pin that it takes the group the
//! coach then makes or asks from, which is the one a grant attaches them to.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off
// the cfg empties the crate (dropping any inner `#![allow(missing_docs)]`),
// so without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::cmp::Reverse;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{
    CoachAccessRequest, CoachAccessStatus, ConnectionType, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_email::templates::CoachAccessOrigin;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::GroupRoutes;
use pierre_services::admin_ops::set_user_manages_roster;
use pierre_services::analytics::resolve_user_email;
use pierre_services::coach_access::{decline, grant, request_access, RequestOutcome};
use pierre_services::coach_access_evidence::probe_intervals_icu_roster;
use pierre_services::coach_platform::coach_platform;
use pierre_services::delegated_connections::{coach_roster, DelegationServices};
use pierre_test_support::delegation::{create_coachless_group, create_group_agent};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The one key the stand-in accepts.
const COACH_KEY: &str = "coach-key";
/// How long the probe the link route spawns gets to file its request.
const PROBE_DEADLINE: StdDuration = StdDuration::from_secs(10);

/// What the stand-in answers, and how many roster reads it served.
#[derive(Clone, Default)]
struct Api {
    /// The email of the key's own account, in its own roster entry.
    account_email: Arc<Mutex<String>>,
    /// The roster entries listed beside the account's own.
    roster: Arc<Mutex<Vec<Value>>>,
    roster_reads: Arc<AtomicUsize>,
}

impl Api {
    fn serve(&self, account_email: &str, roster: Vec<Value>) {
        account_email.clone_into(&mut self.account_email.lock().unwrap());
        *self.roster.lock().unwrap() = roster;
    }

    fn roster_reads(&self) -> usize {
        self.roster_reads.load(Ordering::SeqCst)
    }
}

/// Whether `headers` carry the coach's key as Intervals.icu's Basic pair.
fn carries_coach_key(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|pair| String::from_utf8(pair).ok())
        .is_some_and(|pair| pair == format!("API_KEY:{COACH_KEY}"))
}

async fn athletes_route(State(api): State<Api>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !carries_coach_key(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    api.roster_reads.fetch_add(1, Ordering::SeqCst);
    let email = api.account_email.lock().unwrap().clone();
    let mut roster = vec![json!({
        "id": COACH_ATHLETE, "name": "Casey", "email": email,
        "icu_permission": "WRITE", "status": "ACTIVE"
    })];
    roster.extend(api.roster.lock().unwrap().iter().cloned());
    (StatusCode::OK, Json(Value::Array(roster)))
}

async fn athlete_route(
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> (StatusCode, Json<Value>) {
    if !carries_coach_key(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    (
        StatusCode::OK,
        Json(json!({ "id": athlete, "name": "Casey" })),
    )
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route("/api/v1/athletes", get(athletes_route))
        .route("/api/v1/athlete/{athlete}", get(athlete_route))
        .with_state(api);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A roster entry the key's account reaches with `permission` and `status`
/// as the wire carries them (`Value::Null` sends an explicit `null`, a
/// missing key is left out).
fn entry(id: &str, permission: Option<Value>, status: Option<Value>) -> Value {
    let mut entry =
        json!({ "id": id, "name": "Roster Athlete", "email": format!("{id}@elsewhere.test") });
    if let Some(permission) = permission {
        entry["icu_permission"] = permission;
    }
    if let Some(status) = status {
        entry["status"] = status;
    }
    entry
}

/// An athlete the key coaches and the platform lists as active.
fn coached(id: &str) -> Value {
    entry(id, Some(json!("WRITE")), Some(json!("ACTIVE")))
}

struct Person {
    id: Uuid,
    tenant: TenantId,
    email: String,
}

struct World {
    res: Arc<ServerContext>,
    registry: Arc<ProviderRegistry>,
    api: Api,
}

impl World {
    async fn new() -> Self {
        let api = Api::default();
        let base = spawn_api(api.clone()).await;
        let mut config = default_config();
        config.api_base_url = base;
        let mut registry = ProviderRegistry::new();
        registry.set_default_config(INTERVALS_ICU, config);
        Self {
            res: create_test_server_resources().await.unwrap(),
            registry: Arc::new(registry),
            api,
        }
    }

    /// A user whose email is `email`, verified when `verified`.
    async fn person(&self, email: &str, verified: bool) -> Person {
        let (id, _, tenant) =
            create_test_user_with_plan(&self.res.agent.database, email, "professional")
                .await
                .unwrap();
        if verified {
            self.res
                .common
                .repos
                .email_verification
                .mark_verified(id)
                .await
                .unwrap();
        }
        Person {
            id,
            tenant,
            email: email.to_owned(),
        }
    }

    /// `who`'s own Intervals.icu key, stored as the link route stores it.
    async fn connect(&self, who: &Person) {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: who.id,
                tenant_id: who.tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: COACH_KEY.to_owned(),
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
                who.id,
                who.tenant,
                INTERVALS_ICU,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }

    /// One live read of `who`'s roster, through the path every coach-platform
    /// read takes.
    async fn live_read(&self, who: &Person) -> bool {
        self.read(who, true).await
    }

    /// A read of `who`'s roster that the ten-minute cache serves when it
    /// holds one.
    async fn cached_read(&self, who: &Person) -> bool {
        self.read(who, false).await
    }

    async fn read(&self, who: &Person, refresh: bool) -> bool {
        let platform = coach_platform(&self.registry, INTERVALS_ICU).expect("a coaching platform");
        let services = DelegationServices {
            repos: &self.res.common.repos,
            registry: &self.registry,
            cache: &self.res.common.cache,
            notifications: None,
        };
        coach_roster(services, platform.as_ref(), who.id, who.tenant, refresh)
            .await
            .is_ok()
    }

    /// Every request `who` ever had, newest first.
    async fn requests(&self, who: &Person) -> Vec<CoachAccessRequest> {
        let repos = &self.res.common.repos;
        let mut all = Vec::new();
        for status in [
            CoachAccessStatus::Pending,
            CoachAccessStatus::Granted,
            CoachAccessStatus::Declined,
        ] {
            all.extend(
                repos
                    .coach_access_requests
                    .list_by_status(status)
                    .await
                    .unwrap()
                    .into_iter()
                    .filter(|r| r.user_id == who.id),
            );
        }
        all.sort_by_key(|r| Reverse(r.created_at));
        all
    }

    async fn manages_roster(&self, who: &Person) -> bool {
        self.res
            .common
            .repos
            .users
            .get_global(who.id)
            .await
            .unwrap()
            .expect("user")
            .manages_roster
    }

    /// The auth routes, reading Intervals.icu through the stand-in, and
    /// `who`'s bearer header.
    async fn link_routes(&self, who: &Person) -> (Router, String) {
        let user = self
            .res
            .common
            .repos
            .users
            .get_global(who.id)
            .await
            .unwrap()
            .expect("user");
        let token = self
            .res
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &user,
                &self.res.auth.jwks_manager,
                Some(who.tenant.to_string()),
            )
            .unwrap();
        let mut context = self.res.auth_routes_context();
        context.provider_registry = Arc::clone(&self.registry);
        (AuthRoutes::routes(context), format!("Bearer {token}"))
    }

    /// `who` links the Intervals.icu key `api_key` through the link route.
    async fn link(&self, who: &Person, api_key: &str) -> StatusCode {
        let (router, auth) = self.link_routes(who).await;
        AxumTestRequest::post("/api/providers/intervals_icu/link-credentials")
            .header("authorization", &auth)
            .json(&json!({ "athlete_id": COACH_ATHLETE, "api_key": api_key }))
            .send(router)
            .await
            .status_code()
    }

    /// `who` makes a group through the route the onboarding group step uses,
    /// asking to coach it when `coach_is_me`; the group's id and coach.
    async fn make_group(&self, who: &Person, coach_is_me: bool) -> (Uuid, Value) {
        let repos = &self.res.common.repos;
        let user = repos.users.get_global(who.id).await.unwrap().expect("user");
        let auth = format!("Bearer {}", generate_test_token(&self.res, &user).await);
        let agent_id = create_group_agent(repos, who.id, who.tenant).await.unwrap();
        let response = AxumTestRequest::post("/api/groups")
            .header("authorization", &auth)
            .json(&json!({ "name": "Les Rouleurs", "agent_id": agent_id, "coach_is_me": coach_is_me }))
            .send(GroupRoutes::routes(Arc::clone(&self.res)))
            .await;
        assert_eq!(response.status_code(), StatusCode::CREATED);
        let group: Value = response.json();
        let id = Uuid::parse_str(group["id"].as_str().expect("group id")).unwrap();
        (id, group["coach_user_id"].clone())
    }

    /// The coach `group_id` has now, read back from the store.
    async fn group_coach(&self, who: &Person, group_id: Uuid) -> Option<Uuid> {
        self.res
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), who.tenant)
            .await
            .unwrap()
            .expect("group")
            .coach_user_id
    }
}

#[tokio::test]
async fn a_bound_key_coaching_an_active_athlete_files_one_pending_request_and_grants_nothing() {
    let w = World::new().await;
    let coach = w.person("bound-coach@coach-access.test", true).await;
    w.connect(&coach).await;
    w.api.serve(
        &coach.email,
        vec![
            coached("i201"),
            entry("i202", Some(json!("READ")), Some(json!("ACTIVE"))),
        ],
    );

    let (events, _guard) = capture_notify();
    assert!(w.live_read(&coach).await, "a bound key's roster reads");
    let filed = w.requests(&coach).await;
    assert_eq!(filed.len(), 1, "{filed:?}");
    assert_eq!(filed[0].status, CoachAccessStatus::Pending);
    assert_eq!(filed[0].group_id, None, "the request names no group");
    assert!(
        !w.manages_roster(&coach).await,
        "the request alone grants nothing"
    );
    let pinged = named(&events, "coach_access.requested");
    assert_eq!(pinged.len(), 1, "the request announces itself: {pinged:?}");
    assert_eq!(pinged[0].field("request_id"), filed[0].id.to_string());
    // The notice says it was filed from the roster, never asked, on how many
    // coached athletes, and whose it is: a live read emails nobody, so the
    // notice is all a super-admin sees of it.
    assert_eq!(pinged[0].field("origin"), "intervals_icu_roster");
    assert_eq!(pinged[0].field("coached_active_athletes"), "1");
    assert!(
        !pinged[0].fields.contains_key("group_id"),
        "a request naming no group renders none: {pinged:?}"
    );
    // The event is also a log line: the requester's name never rides it,
    // though they have one; the enricher names them by email in Slack alone.
    assert!(
        !pinged[0].fields.contains_key("display_name"),
        "no display name on the log line: {pinged:?}"
    );
    assert_eq!(
        resolve_user_email(&coach.id.to_string()).as_deref(),
        Some(coach.email.as_str()),
        "the requester never signed in on this pod, yet the notice names them"
    );

    // Another live read finds the request waiting and files nothing more.
    assert!(w.live_read(&coach).await);
    let again = w.requests(&coach).await;
    assert_eq!(again.len(), 1, "{again:?}");
    assert_eq!(again[0].id, filed[0].id);
    assert_eq!(named(&events, "coach_access.requested").len(), 1);
    assert_eq!(w.api.roster_reads(), 2, "both reads were live");
}

#[tokio::test]
async fn a_roster_coaching_no_active_athlete_files_nothing() {
    let w = World::new().await;
    let coach = w.person("no-evidence@coach-access.test", true).await;
    w.connect(&coach).await;
    for roster in [
        // Followed, not coached.
        vec![entry("i201", Some(json!("READ")), Some(json!("ACTIVE")))],
        // Coached, but no longer active.
        vec![entry("i201", Some(json!("WRITE")), Some(json!("ARCHIVED")))],
        vec![entry("i201", Some(json!("WRITE")), Some(json!("DORMANT")))],
        // Coached, standing not reported.
        vec![entry("i201", Some(json!("WRITE")), None)],
        vec![entry("i201", Some(json!("WRITE")), Some(Value::Null))],
        vec![entry("i201", Some(json!("WRITE")), Some(json!("PAUSED")))],
        // Active, access not reported or unknown.
        vec![entry("i201", None, Some(json!("ACTIVE")))],
        vec![entry("i201", Some(Value::Null), Some(json!("ACTIVE")))],
        vec![entry("i201", Some(json!("OWNER")), Some(json!("ACTIVE")))],
        vec![entry("i201", Some(json!("NONE")), Some(json!("ACTIVE")))],
        // Nobody but the account itself.
        Vec::new(),
    ] {
        let listed = roster.clone();
        w.api.serve(&coach.email, roster);
        assert!(w.live_read(&coach).await);
        assert!(
            w.requests(&coach).await.is_empty(),
            "nothing is filed for {listed:?}"
        );
    }
    assert_eq!(w.api.roster_reads(), 11);
}

#[tokio::test]
async fn a_key_whose_account_is_not_the_users_own_files_nothing() {
    let w = World::new().await;
    // The key's account lists another email than the user's.
    let mismatched = w.person("mismatched@coach-access.test", true).await;
    w.connect(&mismatched).await;
    w.api
        .serve("someone-else@coach-access.test", vec![coached("i201")]);
    assert!(
        !w.live_read(&mismatched).await,
        "an unbound key reads no roster"
    );
    assert_eq!(w.requests(&mismatched).await, []);

    // The same email, never verified on Dravr, proves no one.
    let unverified = w.person("unverified@coach-access.test", false).await;
    w.connect(&unverified).await;
    w.api.serve(&unverified.email, vec![coached("i201")]);
    assert!(!w.live_read(&unverified).await);
    assert_eq!(w.requests(&unverified).await, []);

    // The account lists no email at all.
    let missing = w.person("missing@coach-access.test", true).await;
    w.connect(&missing).await;
    w.api.serve("", vec![coached("i201")]);
    assert!(!w.live_read(&missing).await);
    assert_eq!(w.requests(&missing).await, []);
    assert_eq!(w.api.roster_reads(), 3, "each roster was read");
}

#[tokio::test]
async fn a_user_who_holds_coach_access_files_nothing() {
    let w = World::new().await;
    let repos = &w.res.common.repos;
    let earned = w.person("earned@coach-access.test", true).await;
    repos
        .users
        .set_manages_roster(earned.id, true)
        .await
        .unwrap();
    let admin = w.person("admin@coach-access.test", true).await;
    repos.users.set_admin_status(admin.id, true).await.unwrap();
    for who in [&earned, &admin] {
        w.connect(who).await;
        w.api.serve(&who.email, vec![coached("i201")]);
        assert!(w.live_read(who).await);
        assert!(w.requests(who).await.is_empty(), "{}", who.email);
    }
}

#[tokio::test]
async fn a_decided_request_is_never_asked_again() {
    let w = World::new().await;
    let repos = &w.res.common.repos;

    // Declined: the coach may ask again themselves; a roster read does not.
    let declined = w.person("declined@coach-access.test", true).await;
    let RequestOutcome::Opened(asked) =
        request_access(repos, declined.id, None, CoachAccessOrigin::Asked)
            .await
            .unwrap()
    else {
        panic!("a first request opens");
    };
    decline(repos, asked.id, None).await.unwrap();

    // Granted, then revoked by an operator: the revoke stands.
    let revoked = w.person("revoked@coach-access.test", true).await;
    let RequestOutcome::Opened(asked_too) =
        request_access(repos, revoked.id, None, CoachAccessOrigin::Asked)
            .await
            .unwrap()
    else {
        panic!("a first request opens");
    };
    grant(repos, asked_too.id, None).await.unwrap();
    set_user_manages_roster(repos, revoked.id, false, None)
        .await
        .unwrap();

    for (who, status) in [
        (&declined, CoachAccessStatus::Declined),
        (&revoked, CoachAccessStatus::Granted),
    ] {
        w.connect(who).await;
        w.api.serve(&who.email, vec![coached("i201")]);
        assert!(w.live_read(who).await);
        let requests = w.requests(who).await;
        assert_eq!(requests.len(), 1, "{}: {requests:?}", who.email);
        assert_eq!(requests[0].status, status);
        assert!(!w.manages_roster(who).await, "{}", who.email);
    }
}

#[tokio::test]
async fn linking_a_coaching_key_files_the_request_off_the_request() {
    let w = World::new().await;
    let coach = w.person("linked-coach@coach-access.test", true).await;
    w.api.serve(&coach.email, vec![coached("i201")]);

    assert_eq!(w.link(&coach, COACH_KEY).await, StatusCode::OK);

    let started = Instant::now();
    let filed = loop {
        let requests = w.requests(&coach).await;
        if !requests.is_empty() {
            break requests;
        }
        assert!(
            started.elapsed() < PROBE_DEADLINE,
            "the link probe filed no request"
        );
        sleep(StdDuration::from_millis(50)).await;
    };
    assert_eq!(filed.len(), 1, "{filed:?}");
    assert_eq!(filed[0].status, CoachAccessStatus::Pending);
    assert_eq!(filed[0].group_id, None);
    assert!(!w.manages_roster(&coach).await, "a link grants nothing");
    assert_eq!(w.api.roster_reads(), 1, "one live roster read");
}

#[tokio::test]
async fn a_rejected_key_is_still_400_and_reads_no_roster() {
    let w = World::new().await;
    let coach = w.person("rejected@coach-access.test", true).await;
    w.api.serve(&coach.email, vec![coached("i201")]);

    assert_eq!(
        w.link(&coach, "wrong-key").await,
        StatusCode::BAD_REQUEST,
        "a rejected key is a bad field, never a dead session"
    );
    assert!(w
        .res
        .common
        .repos
        .oauth_tokens
        .get_token(coach.id, coach.tenant, INTERVALS_ICU)
        .await
        .unwrap()
        .is_none());
    assert_eq!(w.api.roster_reads(), 0, "no probe runs for a failed link");
    assert_eq!(w.requests(&coach).await, []);
}

#[tokio::test]
async fn a_roster_the_cache_serves_files_nothing() {
    let w = World::new().await;
    let coach = w.person("cached@coach-access.test", true).await;
    w.connect(&coach).await;
    // The first live read lists nobody the key coaches and fills the cache.
    w.api.serve(&coach.email, Vec::new());
    assert!(w.live_read(&coach).await);
    assert_eq!(w.requests(&coach).await, []);

    // The account now coaches an active athlete, but a read the cache serves
    // never reaches Intervals.icu, so it is no evidence.
    w.api.serve(&coach.email, vec![coached("i201")]);
    assert!(w.cached_read(&coach).await);
    assert_eq!(w.api.roster_reads(), 1, "the cache served the second read");
    assert_eq!(w.requests(&coach).await, []);

    // The next live read files it.
    assert!(w.live_read(&coach).await);
    assert_eq!(w.requests(&coach).await.len(), 1);
}

#[tokio::test]
async fn the_link_probe_reads_nothing_for_a_user_who_could_not_get_a_request() {
    let w = World::new().await;
    let repos = &w.res.common.repos;
    let earned = w.person("probe-earned@coach-access.test", true).await;
    repos
        .users
        .set_manages_roster(earned.id, true)
        .await
        .unwrap();
    let asked = w.person("probe-asked@coach-access.test", true).await;
    let RequestOutcome::Opened(request) =
        request_access(repos, asked.id, None, CoachAccessOrigin::Asked)
            .await
            .unwrap()
    else {
        panic!("a first request opens");
    };
    decline(repos, request.id, None).await.unwrap();

    for who in [&earned, &asked] {
        w.connect(who).await;
        w.api.serve(&who.email, vec![coached("i201")]);
        let filed = probe_intervals_icu_roster(repos, &w.registry, who.id, who.tenant)
            .await
            .unwrap();
        assert_eq!(filed, None, "{}", who.email);
    }
    assert_eq!(w.api.roster_reads(), 0, "neither probe read a roster");
    assert_eq!(w.requests(&asked).await.len(), 1);
    assert_eq!(w.requests(&earned).await, []);
}

#[tokio::test]
async fn the_group_a_coach_then_makes_to_coach_is_the_one_a_grant_attaches() {
    let w = World::new().await;
    let repos = &w.res.common.repos;
    // Onboarding asks a coach to connect their platform before it asks for
    // their group: the key files the request first, naming no group.
    let coach = w.person("onboarding@coach-access.test", true).await;
    w.connect(&coach).await;
    w.api.serve(&coach.email, vec![coached("i201")]);
    assert!(w.live_read(&coach).await);
    let filed = w.requests(&coach).await;
    assert_eq!(filed.len(), 1);
    assert_eq!(filed[0].group_id, None);

    // A group made without asking to coach it is not the one to attach.
    let (bystander, _) = w.make_group(&coach, false).await;
    assert_eq!(w.requests(&coach).await[0].group_id, None);

    // The group step makes the group the coach asks to coach; it comes back
    // coachless, and the request already sent names it from now on.
    let (onboarding, coach_user) = w.make_group(&coach, true).await;
    assert_eq!(coach_user, Value::Null, "asking to coach grants nothing");
    let after = w.requests(&coach).await;
    assert_eq!(after.len(), 1, "no second request: {after:?}");
    assert_eq!(after[0].id, filed[0].id);
    assert_eq!(after[0].status, CoachAccessStatus::Pending);
    assert_eq!(after[0].group_id, Some(onboarding));
    assert_eq!(after[0].group_tenant_id, Some(coach.tenant.to_string()));

    // A later ask from another group of theirs keeps the group already named.
    let agent = create_group_agent(repos, coach.id, coach.tenant)
        .await
        .unwrap();
    let other = create_coachless_group(repos, coach.id, coach.tenant, &agent, "Other")
        .await
        .unwrap();
    let RequestOutcome::AlreadyPending(kept) = request_access(
        repos,
        coach.id,
        Some((other, coach.tenant)),
        CoachAccessOrigin::Asked,
    )
    .await
    .unwrap() else {
        panic!("one request waits already");
    };
    assert_eq!(kept.group_id, Some(onboarding));

    let outcome = grant(repos, filed[0].id, None).await.unwrap();
    assert_eq!(outcome.attached_group, Some(onboarding));
    assert_eq!(w.group_coach(&coach, onboarding).await, Some(coach.id));
    assert_eq!(w.group_coach(&coach, bystander).await, None);
    assert_eq!(w.group_coach(&coach, other).await, None);
}

#[tokio::test]
async fn asking_from_a_group_names_it_on_a_request_filed_without_one() {
    let w = World::new().await;
    let repos = &w.res.common.repos;
    let coach = w.person("asks-later@coach-access.test", true).await;
    w.connect(&coach).await;
    w.api.serve(&coach.email, vec![coached("i201")]);
    let (events, _guard) = capture_notify();
    assert!(w.live_read(&coach).await);
    let filed = w.requests(&coach).await;
    assert_eq!(filed[0].group_id, None);

    let agent = create_group_agent(repos, coach.id, coach.tenant)
        .await
        .unwrap();
    let group = create_coachless_group(repos, coach.id, coach.tenant, &agent, "Tuesday Rides")
        .await
        .unwrap();
    let RequestOutcome::AlreadyPending(waiting) = request_access(
        repos,
        coach.id,
        Some((group, coach.tenant)),
        CoachAccessOrigin::Asked,
    )
    .await
    .unwrap() else {
        panic!("the filed request waits already");
    };
    assert_eq!(waiting.id, filed[0].id);
    assert_eq!(waiting.group_id, Some(group));
    assert_eq!(w.requests(&coach).await[0].group_id, Some(group));
    assert_eq!(
        named(&events, "coach_access.requested").len(),
        1,
        "naming the group announces no second request"
    );

    let outcome = grant(repos, waiting.id, None).await.unwrap();
    assert_eq!(outcome.attached_group, Some(group));
    assert_eq!(w.group_coach(&coach, group).await, Some(coach.id));
}
