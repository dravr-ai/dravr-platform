// ABOUTME: Every athlete on a coach's coaching-platform roster reads as one link state against the coach's groups
// ABOUTME: A stand-in platform pins the states, the picker's email rule, the no-access refusal, and the link a join proposes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A coach's roster lists athletes; the coach's groups list members. The link
//! states say where each roster athlete stands: held by a link of the
//! coach's, bound by email to one coached member and proposable to them, or
//! why not. `coachhub`, a stand-in coaching platform registered beside the
//! built-in providers, serves a roster each test writes:
//!
//! - every state, each read from the rows a deployment holds;
//! - members are found among the coach's groups only: an athlete whose
//!   roster email is a Dravr account outside them reads as not in the
//!   coach's groups, exactly like an email no one holds;
//! - the picker of one group suggests that group's member the roster email
//!   binds, and leaves out an athlete a link holds in another group;
//! - an athlete the platform gives the coach no access to is never proposed;
//! - a member who joins by `/group join` is proposed the link the picker
//!   would suggest for them, with every consent left to them, and nothing
//!   else: the join stands whatever the roster says or fails to say, and a
//!   cached roster that binds no one is read once more, live;
//! - the request opens the group's info over the member's own copy of the
//!   group's chat, or their connections when they hold no such room in the
//!   tenant they read the request in.

use crate::common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_commands::group_membership::GroupJoinHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupInviteKind, GroupMember, GroupRespondMode,
    GroupRole,
};
use pierre_core::models::{
    Activity, Athlete, ConnectionType, DelegatedConnection, DelegationStatus, RosterAccess,
    RosterAthlete, Stats, TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::models::Notification;
use pierre_notifications::TenantId as NoticeTenant;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::delegation::{CoachRoster, DelegatedReads};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};
use pierre_services::coach_platform::coach_platform;
use pierre_services::delegated_connections::{
    propose, roster_for_group, DelegationServices, Proposal, RosterEntry,
};
use pierre_services::delegated_link_states::{roster_link_states, LinkState, RosterLinkState};
use pierre_test_support::delegation::create_group_agent;
use tokio::time::sleep;
use uuid::Uuid;

/// The stand-in coaching platform, named nowhere in the platform's code.
const COACHHUB: &str = "coachhub";
const COACH_EMAIL: &str = "coach@link-states.test";
/// How long a notice's spawned dispatch may take to reach the feed.
const NOTICE_DEADLINE: StdDuration = StdDuration::from_secs(10);

/// The roster the stand-in serves, how many times it was read, and whether
/// the platform is down.
#[derive(Clone, Default)]
struct Roster {
    athletes: Arc<Mutex<Vec<RosterAthlete>>>,
    reads: Arc<AtomicUsize>,
    down: Arc<AtomicBool>,
}

impl Roster {
    fn set(&self, athletes: Vec<RosterAthlete>) {
        *self.athletes.lock().unwrap() = athletes;
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
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

/// The stand-in provider: it answers the roster the test wrote and nothing
/// else.
struct CoachHub {
    config: ProviderConfig,
    credentials: Mutex<Option<OAuth2Credentials>>,
    roster: Roster,
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
        if self.roster.down.load(Ordering::SeqCst) {
            return Err(AppError::internal("CoachHub is down"));
        }
        Ok(CoachRoster {
            account_email: Some(COACH_EMAIL.to_owned()),
            athletes: self.roster.athletes.lock().unwrap().clone(),
        })
    }
}

struct Factory {
    roster: Roster,
}

impl Factory {
    fn build(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        Box::new(CoachHub {
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
        &[COACHHUB]
    }

    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for Factory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        if athlete_id.is_empty() || !athlete_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(AppError::invalid_input("not a CoachHub athlete id"));
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
        let mut registry = ProviderRegistry::new();
        registry.set_default_config(COACHHUB, Descriptor.to_config());
        registry.register_descriptor(COACHHUB, Box::new(Descriptor));
        registry.register_factory(
            COACHHUB,
            Box::new(Factory {
                roster: roster.clone(),
            }),
        );
        let mut context = (*base).clone();
        context.fitness.provider_registry = Arc::new(registry);
        let res = Arc::new(context);

        let owner = person(&res, "owner@link-states.test", true).await;
        let coach = person(&res, COACH_EMAIL, true).await;
        let agent_id = create_group_agent(&res.common.repos, owner.id, owner.tenant)
            .await
            .unwrap();
        connect_coachhub(&res, coach, coach.tenant).await;
        Self {
            res,
            roster,
            owner,
            coach,
            agent_id,
        }
    }

    fn services(&self) -> DelegationServices<'_> {
        DelegationServices {
            repos: &self.res.common.repos,
            registry: &self.res.fitness.provider_registry,
            cache: &self.res.common.cache,
            notifications: None,
        }
    }

    /// An active group `coach` coaches, made `age_days` ago.
    async fn group(&self, name: &str, coach: Person, age_days: i64) -> CoachingGroup {
        let repos = &self.res.common.repos;
        let made = Utc::now() - Duration::days(age_days);
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
                    created_at: made,
                    updated_at: made,
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

    /// A link `coach` proposes in `group` for roster athlete `athlete` as
    /// `member`, storing `email` as the athlete's roster email.
    async fn proposed(
        &self,
        coach: Person,
        group: &CoachingGroup,
        member: Person,
        athlete: &str,
        email: Option<&str>,
    ) -> DelegatedConnection {
        self.res
            .common
            .repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                COACHHUB.to_owned(),
                group.id,
                coach.id,
                coach.tenant,
                member.id,
                RosterAthlete::new(athlete, None, email.map(str::to_owned)),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet")
    }

    /// The same, confirmed by the member.
    async fn confirmed(
        &self,
        coach: Person,
        group: &CoachingGroup,
        member: Person,
        athlete: &str,
        email: Option<&str>,
    ) -> DelegatedConnection {
        let link = self.proposed(coach, group, member, athlete, email).await;
        self.res
            .common
            .repos
            .delegated_connections
            .confirm(link.id, member.id, member.tenant, Utc::now())
            .await
            .unwrap()
            .expect("the proposal names the member")
    }

    /// Every roster athlete's state, by athlete id, as the coach reads it.
    async fn states(&self) -> Vec<RosterLinkState> {
        let platform = coach_platform(&self.res.fitness.provider_registry, COACHHUB).unwrap();
        roster_link_states(
            self.services(),
            platform.as_ref(),
            self.coach.id,
            self.coach.tenant,
        )
        .await
        .unwrap()
    }

    /// `who` joins `group` by redeeming a fresh member invite with
    /// `/group join`, as every surface does, typed in `thread` when there is
    /// one (a dispatch with none files no room); returns the reply they read.
    async fn join_by_code(
        &self,
        group: &CoachingGroup,
        who: Person,
        thread: Option<&str>,
    ) -> String {
        let invite = self
            .res
            .common
            .group_service
            .create_invite(
                group.id,
                self.owner.id,
                self.owner.tenant,
                None,
                None,
                GroupInviteKind::Member,
            )
            .await
            .unwrap();
        let ctx = PlatformCommandContext {
            user_id: who.id,
            tenant_id: who.tenant,
            channel_type: "web".to_owned(),
            args: vec![invite.code.clone()],
            raw_text: format!("/group join {}", invite.code),
            ctx: Arc::<ServerContext>::clone(&self.res),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: false,
            conversation_id: thread.map(str::to_owned),
            conversation_tenant_id: who.tenant,
            sender_id: None,
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&self.res),
        };
        let reply = GroupJoinHandler.execute(&ctx).await.unwrap().text;
        assert!(
            self.res
                .common
                .repos
                .groups
                .get_member(&group.id.to_string(), who.id)
                .await
                .unwrap()
                .is_some(),
            "the join stands: {reply}"
        );
        reply
    }

    /// The link request `who` reads in their feed, once its dispatch has
    /// landed.
    async fn link_request(&self, who: Person) -> Notification {
        let service = self
            .res
            .common
            .notification_service
            .as_ref()
            .expect("the test server wires notifications");
        let started = Instant::now();
        loop {
            let (rows, _total, _unread) = service
                .list_notifications(
                    who.id,
                    NoticeTenant(who.tenant.as_uuid()),
                    20,
                    0,
                    Some("coach"),
                    false,
                )
                .await
                .unwrap();
            if let Some(row) = rows
                .into_iter()
                .find(|row| row.notification_type == "delegation_proposed")
            {
                return row;
            }
            assert!(
                started.elapsed() < NOTICE_DEADLINE,
                "no link request reached the member"
            );
            sleep(StdDuration::from_millis(100)).await;
        }
    }

    /// The live links `member` holds in `group`.
    async fn links_of(&self, group: &CoachingGroup, member: Person) -> Vec<DelegatedConnection> {
        self.res
            .common
            .repos
            .delegated_connections
            .list_live_for_member_in_group(group.id, member.id)
            .await
            .unwrap()
    }

    /// `group`'s picker, read live.
    async fn picker(&self, group: &CoachingGroup) -> Vec<RosterEntry> {
        let platform = coach_platform(&self.res.fitness.provider_registry, COACHHUB).unwrap();
        roster_for_group(
            self.services(),
            platform.as_ref(),
            group,
            self.coach.id,
            self.coach.tenant,
            true,
        )
        .await
        .unwrap()
    }
}

/// A user whose email is `email`, verified or not.
async fn person(res: &Arc<ServerContext>, email: &str, verified: bool) -> Person {
    let (id, _, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    if verified {
        res.common
            .repos
            .email_verification
            .mark_verified(id)
            .await
            .unwrap();
    }
    Person { id, tenant }
}

/// `who`'s own `coachhub` grant and connection, stored in `tenant`.
async fn connect_coachhub(res: &Arc<ServerContext>, who: Person, tenant: TenantId) {
    let now = Utc::now();
    let mut token = UserOAuthToken::new(
        who.id,
        tenant.to_string(),
        COACHHUB.to_owned(),
        format!("coachhub-token-{}", who.id.as_simple()),
        None,
        Some(now + Duration::hours(6)),
        None,
    );
    token.provider_user_id = Some(format!("ch{}", who.id.as_simple()));
    let repos = &res.common.repos;
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(who.id, tenant, COACHHUB, &ConnectionType::OAuth, None)
        .await
        .unwrap();
}

fn athlete(id: &str, email: Option<&str>) -> RosterAthlete {
    RosterAthlete::new(id, None, email.map(str::to_owned))
}

/// The state the roster gives athlete `id`.
fn state_of<'a>(states: &'a [RosterLinkState], id: &str) -> &'a LinkState {
    &states
        .iter()
        .find(|s| s.athlete.id == id)
        .unwrap_or_else(|| panic!("{id} is on the roster"))
        .state
}

/// The member a state names, with the group a link to them is made in.
fn member_of(state: &LinkState) -> (Uuid, Uuid) {
    let member = state
        .member()
        .unwrap_or_else(|| panic!("{state:?} names a member"));
    (member.user_id, member.group.id)
}

#[tokio::test]
async fn every_roster_athlete_reads_as_one_link_state() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach, 10).await;

    let linked = person(&w.res, "linked@link-states.test", true).await;
    let legacy = person(&w.res, "legacy@link-states.test", true).await;
    let asked = person(&w.res, "asked@link-states.test", true).await;
    let ready = person(&w.res, "ready@link-states.test", true).await;
    let unverified = person(&w.res, "unverified@link-states.test", false).await;
    let own = person(&w.res, "own@link-states.test", true).await;
    let withheld = person(&w.res, "withheld@link-states.test", true).await;
    let blocked = person(&w.res, "blocked@link-states.test", true).await;
    let elsewhere = person(&w.res, "elsewhere@link-states.test", true).await;
    let left = person(&w.res, "left@link-states.test", true).await;
    let outsider = person(&w.res, "outsider@link-states.test", true).await;
    for member in [
        linked, legacy, asked, ready, unverified, own, elsewhere, left, blocked,
    ] {
        w.join(&squad, member, true).await;
    }
    w.join(&squad, withheld, false).await;
    // The coach is a member of their own group too, and never one of the
    // athletes a roster entry binds.
    w.join(&squad, w.coach, true).await;

    // Linked, and linked by a row stored before links kept the roster email.
    let linked_link = w
        .confirmed(
            w.coach,
            &squad,
            linked,
            "a1",
            Some("linked@link-states.test"),
        )
        .await;
    let legacy_link = w.confirmed(w.coach, &squad, legacy, "a2", None).await;
    let asked_link = w
        .proposed(w.coach, &squad, asked, "a3", Some("asked@link-states.test"))
        .await;
    // Their own connection, kept under another tenant than the one they
    // coach in; the member who stopped sharing holds one too.
    connect_coachhub(&w.res, own, outsider.tenant).await;
    connect_coachhub(&w.res, withheld, withheld.tenant).await;
    // Another coach's confirmed link already reads this member.
    let other_coach = person(&w.res, "other-coach@link-states.test", true).await;
    let other_group = w.group("Other", other_coach, 5).await;
    w.join(&other_group, elsewhere, true).await;
    w.confirmed(
        other_coach,
        &other_group,
        elsewhere,
        "a11",
        Some("elsewhere@link-states.test"),
    )
    .await;
    // A link whose member then left the group speaks for no one coached.
    w.confirmed(w.coach, &squad, left, "a13", Some("left@link-states.test"))
        .await;
    assert!(w
        .res
        .common
        .repos
        .groups
        .remove_member(&squad.id.to_string(), left.id)
        .await
        .unwrap());

    w.roster.set(vec![
        athlete("a1", Some("Linked@Link-States.test")),
        athlete("a2", Some("legacy@link-states.test")),
        athlete("a3", Some("asked@link-states.test")),
        athlete("a4", Some(" READY@link-states.test")).with_access(Some(RosterAccess::Coach)),
        athlete("a5", Some("unverified@link-states.test")),
        athlete("a6", Some("own@link-states.test")),
        athlete("a7", Some("withheld@link-states.test")),
        athlete("a8", None),
        athlete("a9", Some("outsider@link-states.test")),
        athlete("a10", Some("blocked@link-states.test")).with_access(Some(RosterAccess::NoAccess)),
        athlete("a11", Some("elsewhere@link-states.test")),
        athlete("a12", Some("asked@link-states.test")),
        athlete("a13", Some("left@link-states.test")),
        athlete("a14", Some(COACH_EMAIL)),
    ]);

    let states = w.states().await;
    assert_eq!(states.len(), 14, "every roster athlete has a state");
    assert!(
        matches!(state_of(&states, "a1"), LinkState::Linked { link } if link.id == linked_link.id)
    );
    assert!(matches!(
        state_of(&states, "a2"),
        LinkState::LinkedNotReading { link, reason: "athlete_email_missing" }
            if link.id == legacy_link.id
    ));
    assert!(
        matches!(state_of(&states, "a3"), LinkState::Proposed { link } if link.id == asked_link.id)
    );
    assert!(matches!(
        state_of(&states, "a4"),
        LinkState::Proposable { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a4")), (ready.id, squad.id));
    assert!(matches!(
        state_of(&states, "a5"),
        LinkState::MemberEmailUnverified { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a5")).0, unverified.id);
    assert!(matches!(
        state_of(&states, "a6"),
        LinkState::MemberOwnConnection { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a6")).0, own.id);
    assert!(
        matches!(state_of(&states, "a7"), LinkState::MemberNotSharing { .. }),
        "a member who stopped sharing is reported as that alone, own connection or not"
    );
    assert_eq!(member_of(state_of(&states, "a7")).0, withheld.id);
    assert!(matches!(
        state_of(&states, "a8"),
        LinkState::RosterEmailMissing
    ));
    assert!(
        matches!(state_of(&states, "a9"), LinkState::NotInYourGroups),
        "a Dravr account outside the coach's groups reads like an email no one holds"
    );
    assert!(matches!(
        state_of(&states, "a10"),
        LinkState::NoAccess { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a10")).0, blocked.id);
    assert!(matches!(
        state_of(&states, "a11"),
        LinkState::LinkedThroughAnotherCoach { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a11")).0, elsewhere.id);
    assert!(matches!(
        state_of(&states, "a12"),
        LinkState::MemberHoldsAnotherLink { .. }
    ));
    assert_eq!(member_of(state_of(&states, "a12")).0, asked.id);
    assert!(
        matches!(state_of(&states, "a13"), LinkState::NotInYourGroups),
        "the link of a member who left reads as nobody's"
    );
    assert!(
        matches!(state_of(&states, "a14"), LinkState::NotInYourGroups),
        "the coach never binds as their own athlete"
    );

    // Each state carries the coach's access as the roster reported it.
    let access = |id: &str| {
        states
            .iter()
            .find(|s| s.athlete.id == id)
            .and_then(|s| s.athlete.access)
    };
    assert_eq!(access("a4"), Some(RosterAccess::Coach));
    assert_eq!(access("a10"), Some(RosterAccess::NoAccess));
    assert_eq!(access("a1"), None);

    // A second read is the cached roster, and reads the same access back.
    assert_eq!(w.roster.reads(), 1);
    let cached = w.states().await;
    assert_eq!(w.roster.reads(), 1, "served from the cache");
    assert!(matches!(
        state_of(&cached, "a10"),
        LinkState::NoAccess { .. }
    ));

    // The picker suggests the member a proposal would bind, and never one
    // who already holds a link of the coach's: proposing them is refused.
    let picker = w.picker(&squad).await;
    let suggested = |id: &str| {
        picker
            .iter()
            .find(|e| e.athlete.id == id)
            .unwrap_or_else(|| panic!("{id} is in the picker"))
            .suggested_member_user_id
    };
    assert_eq!(suggested("a4"), Some(ready.id));
    assert_eq!(
        suggested("a12"),
        None,
        "the member a3's proposal holds is no unlinked member"
    );
    let refused = propose(
        w.services(),
        &squad,
        w.coach.id,
        w.coach.tenant,
        Proposal {
            provider: COACHHUB,
            provider_athlete_id: "a12",
            member_user_id: asked.id,
        },
    )
    .await
    .expect_err("a member holds one live link per group");
    let details = refused.details.as_deref().expect("the refusal says why");
    assert_eq!(details["reason"], "already_proposed", "{details}");
}

#[tokio::test]
async fn a_group_s_picker_suggests_its_member_by_email_and_leaves_out_other_groups_links() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach, 30).await;
    let track = w.group("Track", w.coach, 2).await;

    let both = person(&w.res, "both@link-states.test", true).await;
    let track_only = person(&w.res, "track@link-states.test", true).await;
    let linked_in_track = person(&w.res, "linked-track@link-states.test", true).await;
    let blocked = person(&w.res, "blocked@link-states.test", true).await;
    let shares_in_track = person(&w.res, "shares-track@link-states.test", true).await;
    let unverified = person(&w.res, "unverified@link-states.test", false).await;
    for member in [both, blocked, unverified] {
        w.join(&squad, member, true).await;
    }
    w.join(&squad, shares_in_track, false).await;
    for member in [both, track_only, linked_in_track, shares_in_track] {
        w.join(&track, member, true).await;
    }
    let track_link = w
        .proposed(
            w.coach,
            &track,
            linked_in_track,
            "b3",
            Some("linked-track@link-states.test"),
        )
        .await;

    w.roster.set(vec![
        // The roster names the athlete as someone else: only the email binds.
        RosterAthlete::new(
            "b1",
            Some("Not The Member's Name".to_owned()),
            Some("both@link-states.test".to_owned()),
        ),
        athlete("b2", Some("track@link-states.test")),
        athlete("b3", Some("linked-track@link-states.test")),
        athlete("b4", Some("blocked@link-states.test")).with_access(Some(RosterAccess::NoAccess)),
        athlete("b5", Some("shares-track@link-states.test")),
        athlete("b6", Some("unverified@link-states.test")),
        athlete("b7", Some("outsider@link-states.test")),
    ]);

    // Coach-wide, a member in two groups is linked in the oldest one they
    // share in.
    let states = w.states().await;
    assert_eq!(member_of(state_of(&states, "b1")), (both.id, squad.id));
    assert_eq!(
        member_of(state_of(&states, "b2")),
        (track_only.id, track.id)
    );
    assert_eq!(
        member_of(state_of(&states, "b5")),
        (shares_in_track.id, track.id),
        "a group the member shares in comes before an older one they withheld"
    );

    let squad_picker = w.picker(&squad).await;
    let ids: Vec<&str> = squad_picker.iter().map(|e| e.athlete.id.as_str()).collect();
    assert_eq!(
        ids,
        ["b1", "b2", "b4", "b5", "b6", "b7"],
        "the athlete Track's link holds is left out of Squad's picker"
    );
    let suggested = |picker: &[RosterEntry], id: &str| {
        picker
            .iter()
            .find(|e| e.athlete.id == id)
            .unwrap()
            .suggested_member_user_id
    };
    assert_eq!(suggested(&squad_picker, "b1"), Some(both.id));
    assert_eq!(
        suggested(&squad_picker, "b2"),
        None,
        "Track's member is no Squad member"
    );
    assert_eq!(
        suggested(&squad_picker, "b4"),
        None,
        "no access, no proposal"
    );
    assert_eq!(
        suggested(&squad_picker, "b5"),
        Some(shares_in_track.id),
        "the picker of a group the member is in suggests them there"
    );
    assert_eq!(
        suggested(&squad_picker, "b6"),
        None,
        "an unverified email binds no one"
    );
    assert_eq!(suggested(&squad_picker, "b7"), None);
    assert!(squad_picker.iter().all(|e| e.link.is_none()));

    let track_picker = w.picker(&track).await;
    let held = track_picker.iter().find(|e| e.athlete.id == "b3").unwrap();
    assert_eq!(held.link.as_ref().map(|l| l.id), Some(track_link.id));
    assert_eq!(held.suggested_member_user_id, None);
    assert_eq!(suggested(&track_picker, "b1"), Some(both.id));
    assert_eq!(suggested(&track_picker, "b2"), Some(track_only.id));
}

#[tokio::test]
async fn an_athlete_the_platform_gives_no_access_to_is_never_proposed() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach, 3).await;
    let blocked = person(&w.res, "blocked@link-states.test", true).await;
    let follower = person(&w.res, "follower@link-states.test", true).await;
    for member in [blocked, follower] {
        w.join(&squad, member, true).await;
    }
    w.roster.set(vec![
        athlete("c1", Some("blocked@link-states.test")).with_access(Some(RosterAccess::NoAccess)),
        athlete("c2", Some("follower@link-states.test")).with_access(Some(RosterAccess::Follower)),
    ]);

    let refused = propose(
        w.services(),
        &squad,
        w.coach.id,
        w.coach.tenant,
        Proposal {
            provider: COACHHUB,
            provider_athlete_id: "c1",
            member_user_id: blocked.id,
        },
    )
    .await
    .expect_err("an athlete the coach cannot read is never proposed");
    let details = refused.details.as_deref().expect("the refusal says why");
    assert_eq!(details["reason"], "athlete_no_access", "{details}");
    assert_eq!(details["provider"], COACHHUB, "{details}");
    let live = w
        .res
        .common
        .repos
        .delegated_connections
        .list_live_for_coach(w.coach.id, w.coach.tenant, COACHHUB)
        .await
        .unwrap();
    assert!(live.is_empty(), "nothing was stored: {live:?}");

    // A follower's workouts read, so they are proposed like a coached athlete.
    let link = propose(
        w.services(),
        &squad,
        w.coach.id,
        w.coach.tenant,
        Proposal {
            provider: COACHHUB,
            provider_athlete_id: "c2",
            member_user_id: follower.id,
        },
    )
    .await
    .unwrap();
    assert_eq!(link.member_user_id, follower.id);
    assert_eq!(link.provider_athlete_id, "c2");
}

#[tokio::test]
async fn a_member_who_joins_under_their_roster_email_is_proposed_the_link() {
    let w = World::new().await;
    // A group's coach is in the group's tenant and reads their roster there,
    // as the group's picker does.
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    let squad = w.group("Squad", w.coach, 3).await;
    let joiner = person(&w.res, "joiner@link-states.test", true).await;
    w.roster.set(vec![
        athlete("d1", Some("someone-else@link-states.test")),
        // Emails bind in their normalized form.
        athlete("d2", Some(" Joiner@Link-States.test ")),
    ]);

    let reply = w.join_by_code(&squad, joiner, None).await;
    assert!(
        !reply.contains("d2") && !reply.to_lowercase().contains("joiner@"),
        "nothing from the roster reaches the reply: {reply}"
    );

    let links = w.links_of(&squad, joiner).await;
    assert_eq!(links.len(), 1, "one proposal: {links:?}");
    let link = &links[0];
    assert_eq!(link.status, DelegationStatus::Proposed);
    assert_eq!(link.provider, COACHHUB);
    assert_eq!(link.provider_athlete_id, "d2");
    assert_eq!(link.coach_user_id, w.coach.id);
    assert_eq!(link.coach_tenant_id, w.owner.tenant);
    assert_eq!(link.member_tenant_id, None, "the member confirms it");
    let consents = w
        .res
        .common
        .repos
        .groups
        .get_member(&squad.id.to_string(), joiner.id)
        .await
        .unwrap()
        .expect("the joiner's membership")
        .coach_consents;
    assert!(
        !consents.calendar_write && !consents.wellness_read,
        "every coach consent waits on the member: {consents:?}"
    );

    // Joined from no thread, so no room was filed for them: the request opens
    // their connections, whose card names the group.
    let request = w.link_request(joiner).await;
    let data = request.data.expect("the request carries its route");
    assert_eq!(data["screen"], "connections", "{data}");
    assert_eq!(data["provider"], COACHHUB, "{data}");
}

#[tokio::test]
async fn a_join_typed_in_a_thread_asks_the_member_over_their_group_room() {
    let w = World::new().await;
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    let squad = w.group("Squad", w.coach, 3).await;
    let joiner = person(&w.res, "room@link-states.test", true).await;
    w.roster
        .set(vec![athlete("g1", Some("room@link-states.test"))]);
    let repos = &w.res.common.repos;
    let member = joiner.id.to_string();
    let thread = repos
        .chat
        .create_conversation(&member, joiner.tenant, "Chat", "test-model", None, None)
        .await
        .unwrap();

    w.join_by_code(&squad, joiner, Some(&thread.id)).await;

    let room = repos
        .chat
        .find_group_conversation(&member, joiner.tenant, &squad.id.to_string())
        .await
        .unwrap()
        .expect("the join filed the member's own copy of the group's chat");
    assert_ne!(room.id, thread.id, "the room is filed beside the thread");
    // The request opens the group's info over that room, where the link
    // waits with its consents.
    let request = w.link_request(joiner).await;
    let data = request.data.expect("the request carries its route");
    assert_eq!(data["screen"], "group_info", "{data}");
    assert_eq!(data["id"], room.id.as_str(), "{data}");
    assert_eq!(w.links_of(&squad, joiner).await.len(), 1);
}

#[tokio::test]
async fn a_link_request_names_no_room_outside_the_tenant_the_member_reads_it_in() {
    let w = World::new().await;
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    let squad = w.group("Squad", w.coach, 3).await;
    let joiner = person(&w.res, "two-tenants@link-states.test", true).await;
    w.roster
        .set(vec![athlete("h1", Some("two-tenants@link-states.test"))]);
    let repos = &w.res.common.repos;
    // The member also belongs to the group's tenant, joined after their own,
    // which stays the one they sign in to and read their feed in.
    repos
        .users
        .update_tenant_id(joiner.id, w.owner.tenant)
        .await
        .unwrap();
    let tenants: Vec<TenantId> = repos
        .tenants
        .list_for_user(joiner.id)
        .await
        .unwrap()
        .iter()
        .map(|tenant| tenant.id)
        .collect();
    assert_eq!(tenants, [joiner.tenant, w.owner.tenant]);
    // Their only copy of the group's chat is filed under the group's tenant.
    let member = joiner.id.to_string();
    let squad_id = squad.id.to_string();
    repos
        .chat
        .create_conversation(
            &member,
            w.owner.tenant,
            "Squad",
            "test-model",
            Some(&squad.agent_id),
            Some(&squad_id),
        )
        .await
        .unwrap();
    assert!(repos
        .chat
        .find_group_conversation(&member, w.owner.tenant, &squad_id)
        .await
        .unwrap()
        .is_some());

    w.join_by_code(&squad, joiner, None).await;

    // A session in their own tenant cannot open that room, so the request
    // opens their connections instead.
    let request = w.link_request(joiner).await;
    let data = request.data.expect("the request carries its route");
    assert_eq!(data["screen"], "connections", "{data}");
    assert_eq!(w.links_of(&squad, joiner).await.len(), 1);
}

#[tokio::test]
async fn a_join_proposes_nothing_the_roster_does_not_bind() {
    let w = World::new().await;
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    let squad = w.group("Squad", w.coach, 3).await;
    let unverified = person(&w.res, "unverified@link-states.test", false).await;
    let blocked = person(&w.res, "blocked@link-states.test", true).await;
    let stranger = person(&w.res, "stranger@link-states.test", true).await;
    w.roster.set(vec![
        athlete("e1", Some("unverified@link-states.test")),
        athlete("e2", Some("blocked@link-states.test")).with_access(Some(RosterAccess::NoAccess)),
        athlete("e3", None),
    ]);

    for (who, why) in [
        (unverified, "an unverified email binds no one"),
        (blocked, "the platform gives the coach no access to them"),
        (stranger, "no roster athlete carries their email"),
    ] {
        w.join_by_code(&squad, who, None).await;
        assert!(w.links_of(&squad, who).await.is_empty(), "{why}");
    }
    let live = w
        .res
        .common
        .repos
        .delegated_connections
        .list_live_for_coach(w.coach.id, w.owner.tenant, COACHHUB)
        .await
        .unwrap();
    assert!(live.is_empty(), "nothing was proposed: {live:?}");
}

#[tokio::test]
async fn a_join_reads_the_roster_in_the_group_s_tenant_and_stands_when_it_cannot() {
    let w = World::new().await;
    let squad = w.group("Squad", w.coach, 3).await;
    w.roster.set(vec![
        athlete("f1", Some("first@link-states.test")),
        athlete("f2", Some("second@link-states.test")),
        athlete("f3", Some("third@link-states.test")),
    ]);

    // The coach's only credential is in their own tenant, not the group's:
    // the group's picker could not read it either, so nothing is read.
    let first = person(&w.res, "first@link-states.test", true).await;
    w.join_by_code(&squad, first, None).await;
    let got = w.links_of(&squad, first).await;
    assert!(got.is_empty(), "expected empty, got {got:?}");
    assert_eq!(w.roster.reads(), 0, "no roster is read for the group");

    // The platform is down: the member is still let in, and nothing is
    // proposed or cached.
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    w.roster.set_down(true);
    let second = person(&w.res, "second@link-states.test", true).await;
    w.join_by_code(&squad, second, None).await;
    let got = w.links_of(&squad, second).await;
    assert!(got.is_empty(), "expected empty, got {got:?}");
    assert_eq!(w.roster.reads(), 1, "the roster was asked for");

    // Back up, the next join reads it afresh and is proposed.
    w.roster.set_down(false);
    let third = person(&w.res, "third@link-states.test", true).await;
    w.join_by_code(&squad, third, None).await;
    let links = w.links_of(&squad, third).await;
    assert_eq!(links.len(), 1, "{links:?}");
    assert_eq!(links[0].provider_athlete_id, "f3");
    assert_eq!(w.roster.reads(), 2);
}

#[tokio::test]
async fn a_join_reads_a_cached_roster_live_once_more_when_it_binds_no_one() {
    let w = World::new().await;
    connect_coachhub(&w.res, w.coach, w.owner.tenant).await;
    let squad = w.group("Squad", w.coach, 3).await;
    w.roster
        .set(vec![athlete("i1", Some("early@link-states.test"))]);

    // Nothing is cached yet: the roster is read live, and a live read that
    // binds no one is not read again.
    let stranger = person(&w.res, "stranger@link-states.test", true).await;
    w.join_by_code(&squad, stranger, None).await;
    let got = w.links_of(&squad, stranger).await;
    assert!(got.is_empty(), "expected empty, got {got:?}");
    assert_eq!(w.roster.reads(), 1);

    // The cached roster binds the next member: nothing is read.
    let early = person(&w.res, "early@link-states.test", true).await;
    w.join_by_code(&squad, early, None).await;
    let links = w.links_of(&squad, early).await;
    assert_eq!(links.len(), 1, "{links:?}");
    assert_eq!(links[0].provider_athlete_id, "i1");
    assert_eq!(w.roster.reads(), 1, "the cached roster served the join");

    // The coach adds an athlete on the platform after that read. The cached
    // roster binds no one for them, so it is read once more, live.
    w.roster.set(vec![
        athlete("i1", Some("early@link-states.test")),
        athlete("i2", Some("late@link-states.test")),
    ]);
    let late = person(&w.res, "late@link-states.test", true).await;
    w.join_by_code(&squad, late, None).await;
    let links = w.links_of(&squad, late).await;
    assert_eq!(links.len(), 1, "{links:?}");
    assert_eq!(links[0].provider_athlete_id, "i2");
    assert_eq!(
        w.roster.reads(),
        2,
        "one live read found the athlete just added"
    );
}
