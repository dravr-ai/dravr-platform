// ABOUTME: A group invite link opened signed out is held on the server until the account can join (carnet#1648)
// ABOUTME: Drives the real register, hold, waiting-screen and POST /api/chat/groups/join routes over one database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What a held group invite does, and what it never does:
//!
//! 1. the code a `/groups/join/<code>` link carried reaches the server with
//!    the sign-up, the call after a sign-in, or the waiting screen, and is
//!    held against the account; a dead code never refuses the sign-up;
//! 2. once the account is active, `POST /api/chat/groups/join` joins the
//!    invite's group the way `/group join` does — as an athlete for a member
//!    invite, as the group's human coach for a coach invite the account may
//!    redeem — files the caller's room with the join's reply in it, and drops
//!    the hold;
//! 3. a member invite's expiry no longer applies once held, but a withdrawn
//!    invite, a used-up invite, a full group, an expired coach invite and a
//!    lapsed hold are refused, and every decided join drops the hold;
//! 4. a hold spends no use of the invite; the join spends one;
//! 5. one hold per account: a newer link replaces the older one.

use crate::common;
use crate::helpers;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::dates::{Elapsed, Timestamp};
use helpers::axum_test::AxumTestRequest;
use helpers::impersonation::{assert_refusal_audited, assert_support_refused, impersonation_of};
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_COACH_JOINED_NOTICE, KEY_GROUP_COACH_SHARING_PENDING, KEY_GROUP_JOINED,
    KEY_GROUP_JOINED_AS_COACH, KEY_GROUP_JOIN_FULL, KEY_GROUP_JOIN_INVALID_CODE,
};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::groups::{GroupInvite, GroupInviteKind};
use pierre_core::models::{
    default_locale, CreateGroupRequest, GroupInviteHold, TenantId, User, UserStatus,
};
use pierre_core::usage_log;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_groups::strategies::tier::EnterpriseTierStrategy;
use pierre_groups::SIGNUP_INVITE_MAX_DAYS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::GroupRoutes;
use pierre_services::delegated_connections::person_name;
use pierre_services::signup_policy::{set_signup_policy, SignupPolicyUpdate};
use pierre_test_support::log_capture::{capture_target, drain};
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";
/// The uniform refusal of a code no one could hold (the signup-code wording).
const INVALID: &str = "This code isn't valid";
const GROUP_NAME: &str = "Held Invite Crew";
const ROOM_MODEL: &str = "gemini-2.0-flash";
const EXPIRE_INVITE: &str =
    "UPDATE group_invites SET expires_at = $1 WHERE id = $2 AND tenant_id = $3";
/// A tenant membership row, filed the way the invitation flows file it.
const INSERT_TENANT_MEMBER: &str =
    "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) \
     VALUES ($1, $2, $3, 'member', $4, $5)";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The auth routes behind a fresh client address per call, so the
/// per-address sign-up and code windows never decide a test.
pub(crate) fn auth_routes(resources: &ServerContext) -> Router {
    static NEXT: AtomicU16 = AtomicU16::new(1);
    let [hi, lo] = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    AuthRoutes::routes(resources.auth_routes_context()).layer(MockConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(198, 19, hi, lo)),
        40_000,
    )))
}

pub(crate) fn body_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// `POST /api/auth/register`, carrying the code a group link carried.
async fn register(resources: &ServerContext, email: &str, link_code: Option<&str>) -> (u16, Value) {
    let mut body = json!({ "email": email, "password": PASSWORD });
    if let Some(code) = link_code {
        body["group_invite_code"] = json!(code);
    }
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&body)
        .send(auth_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

pub(crate) async fn bearer(resources: &Arc<ServerContext>, user: &User) -> String {
    format!("Bearer {}", generate_test_token(resources, user).await)
}

/// `PUT /api/user/group-invite` — the call right after a sign-in that
/// started from a group link.
pub(crate) async fn hold_link(
    resources: &Arc<ServerContext>,
    user: &User,
    code: &str,
) -> (u16, Value) {
    let response = AxumTestRequest::put("/api/user/group-invite")
        .header("authorization", &bearer(resources, user).await)
        .json(&json!({ "code": code }))
        .send(auth_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

/// `POST /api/user/signup-code/redeem` — the waiting screen's code field.
async fn redeem(resources: &Arc<ServerContext>, user: &User, code: &str) -> (u16, Value) {
    let response = AxumTestRequest::post("/api/user/signup-code/redeem")
        .header("authorization", &bearer(resources, user).await)
        .json(&json!({ "code": code }))
        .send(auth_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

/// `POST /api/chat/groups/join` — the app redeeming the held invite.
pub(crate) async fn join_held(resources: &Arc<ServerContext>, user: &User) -> (u16, Value) {
    let response = AxumTestRequest::post("/api/chat/groups/join")
        .header("authorization", &bearer(resources, user).await)
        .send(ChatRoutes::routes(Arc::clone(resources)))
        .await;
    (response.status(), body_of(&response.body_text()))
}

async fn account(resources: &ServerContext, email: &str) -> User {
    resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account exists")
}

/// An active account with its own tenant, as an already-approved athlete.
pub(crate) async fn athlete(resources: &Arc<ServerContext>, label: &str) -> (User, TenantId) {
    let (_, user, tenant) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("{label}-{}@held-invite.test", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    (user, tenant)
}

async fn tenant_of(resources: &ServerContext, user_id: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("the account owns a tenant")
        .id
}

/// Prove the account's address as the confirmation link does.
async fn confirm_email(resources: &ServerContext, user_id: Uuid) -> UserStatus {
    resources
        .common
        .repos
        .email_verification
        .mark_verified(user_id)
        .await
        .unwrap();
    resources
        .auth_routes_context()
        .account_service()
        .apply_approval_after_verification(user_id, true)
        .await
        .unwrap()
}

/// Sign-up open to verified addresses: a proven address activates the
/// account (the posture a group link's athlete signs up under).
async fn open_to_verified_addresses(resources: &ServerContext) {
    set_signup_policy(
        &resources.agent.database,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(true),
            ..SignupPolicyUpdate::default()
        },
    )
    .await
    .unwrap();
}

/// A roster-managing Professional owner of one group named [`GROUP_NAME`].
pub(crate) struct Coach {
    pub(crate) id: Uuid,
    pub(crate) tenant: TenantId,
    pub(crate) group_id: Uuid,
    pub(crate) auth: String,
}

impl Coach {
    pub(crate) async fn new(resources: &Arc<ServerContext>, max_members: i32) -> Self {
        let (id, owner, _) = create_test_user_with_plan(
            &resources.agent.database,
            &format!("coach-{}@held-invite.test", Uuid::new_v4()),
            "professional",
        )
        .await
        .unwrap();
        let repos = &resources.common.repos;
        repos.users.set_manages_roster(id, true).await.unwrap();
        let tenant = tenant_of(resources, id).await;
        let agent: CreateAgentRequest = serde_json::from_value(
            json!({"title":"Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
        )
        .unwrap();
        let agent_id = repos
            .agents
            .create(id, tenant, &agent)
            .await
            .unwrap()
            .id
            .to_string();
        let group = resources
            .group_service()
            .create_group(
                &CreateGroupRequest {
                    name: GROUP_NAME.to_owned(),
                    description: None,
                    agent_id,
                    max_members: Some(max_members),
                    coach_user_id: None,
                },
                id,
                tenant,
                &EnterpriseTierStrategy,
            )
            .await
            .unwrap();
        Self {
            id,
            tenant,
            group_id: group.id,
            auth: bearer(resources, &owner).await,
        }
    }

    /// A plain member invite, live for two weeks.
    pub(crate) async fn invite(
        &self,
        resources: &ServerContext,
        max_uses: Option<i32>,
    ) -> GroupInvite {
        self.invite_of(resources, GroupInviteKind::Member, max_uses)
            .await
    }

    /// A plain invite of `kind`, live for two weeks.
    pub(crate) async fn invite_of(
        &self,
        resources: &ServerContext,
        kind: GroupInviteKind,
        max_uses: Option<i32>,
    ) -> GroupInvite {
        resources
            .group_service()
            .create_invite(
                self.group_id,
                self.id,
                self.tenant,
                Some(14),
                max_uses,
                kind,
            )
            .await
            .unwrap()
    }

    /// An active account with coach access that belongs to the owner's
    /// tenant only, as an invited staff coach does: the account a coach
    /// invite is meant for.
    pub(crate) async fn staff_coach(&self, resources: &ServerContext) -> User {
        let mut user = User::new(
            format!("staff-coach-{}@held-invite.test", Uuid::new_v4()),
            "hash_not_verified_in_tests".to_owned(),
            Some("Staff Coach".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Timestamp::now());
        user.manages_roster = true;
        let repos = &resources.common.repos;
        repos.users.create(&user).await.unwrap();
        // Membership rows are written by the invitation flows in production;
        // the repository exposes no direct writer, so the fixture files the
        // row the way those flows do.
        let now = Timestamp::now();
        match resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                sqlx::query(INSERT_TENANT_MEMBER)
                    .bind(Uuid::new_v4().to_string())
                    .bind(self.tenant.to_string())
                    .bind(user.id.to_string())
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .execute(sqlite.pool())
                    .await
                    .unwrap();
            }
            // `tenant_users` keys are `uuid` columns and the timestamps are
            // `timestamptz` on PostgreSQL, so the binds carry native types.
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(postgres) => {
                sqlx::query(INSERT_TENANT_MEMBER)
                    .bind(Uuid::new_v4())
                    .bind(self.tenant.as_uuid())
                    .bind(user.id)
                    .bind(now)
                    .bind(now)
                    .execute(postgres.pool())
                    .await
                    .unwrap();
            }
        }
        repos
            .users
            .update_tenant_id(user.id, self.tenant)
            .await
            .unwrap();
        user
    }

    pub(crate) async fn coach_user_id(&self, resources: &ServerContext) -> Option<Uuid> {
        resources
            .common
            .repos
            .groups
            .get_group(&self.group_id.to_string(), self.tenant)
            .await
            .unwrap()
            .expect("the group exists")
            .coach_user_id
    }

    /// Seat the owner as the group's human coach and file their copy of the
    /// group's chat, where a join notice lands. Returns that conversation.
    async fn seat_as_coach_with_chat(&self, resources: &ServerContext) -> String {
        let repos = &resources.common.repos;
        assert!(
            repos
                .groups
                .set_group_coach_user(&self.group_id.to_string(), Some(self.id), self.tenant)
                .await
                .unwrap(),
            "fixture: the owner is seated as the group's coach"
        );
        repos
            .chat
            .create_conversation(
                &self.id.to_string(),
                self.tenant,
                GROUP_NAME,
                ROOM_MODEL,
                None,
                Some(&self.group_id.to_string()),
            )
            .await
            .unwrap()
            .id
    }

    async fn is_member(&self, resources: &ServerContext, user_id: Uuid) -> bool {
        resources
            .common
            .repos
            .groups
            .get_member(&self.group_id.to_string(), user_id)
            .await
            .unwrap()
            .is_some()
    }

    /// How many assistant rows in the coach's conversation `chat` read `text`.
    pub(crate) async fn rows_reading(
        &self,
        resources: &ServerContext,
        chat: &str,
        text: &str,
    ) -> usize {
        resources
            .common
            .repos
            .chat
            .get_messages(chat, &self.id.to_string(), self.tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.role == "assistant" && m.content == text)
            .count()
    }

    async fn member_count(&self, resources: &ServerContext) -> i64 {
        resources
            .common
            .repos
            .groups
            .count_members(&self.group_id.to_string())
            .await
            .unwrap()
    }
}

pub(crate) async fn held(resources: &ServerContext, user_id: Uuid) -> Option<GroupInviteHold> {
    resources
        .common
        .repos
        .signup_codes
        .held_group_invite(user_id)
        .await
        .unwrap()
}

async fn use_count(resources: &ServerContext, invite_id: Uuid) -> i32 {
    resources
        .common
        .repos
        .groups
        .get_invite_by_id(&invite_id.to_string())
        .await
        .unwrap()
        .expect("the invite exists")
        .use_count
}

/// Move an invite of `tenant`'s group to expire at `at`, as the passage of
/// time would.
pub(crate) async fn expire_invite(
    resources: &ServerContext,
    tenant: TenantId,
    invite_id: Uuid,
    at: Timestamp,
) {
    let changed = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(EXPIRE_INVITE)
            .bind(at)
            .bind(invite_id.to_string())
            .bind(tenant.to_string())
            .execute(sqlite.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::query(EXPIRE_INVITE)
            .bind(at)
            .bind(invite_id)
            // `group_invites.tenant_id` stays `text` on PostgreSQL.
            .bind(tenant.to_string())
            .execute(postgres.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(changed, 1, "the invite's expiry moved");
}

/// The text a test user reads: every account here keeps the default locale.
pub(crate) fn rendered(resources: &ServerContext, key: &str, args: &[&str]) -> String {
    resources
        .mcp
        .messaging_strings_registry
        .render(key, &default_locale(), args)
}

// ---------------------------------------------------------------------------
// 1. Taking the hold
// ---------------------------------------------------------------------------

/// The production case (carnet#1648): the link's code rides the sign-up, the
/// account confirms its address in another tab, and the app joins the group
/// once the account is active, filing the member's room with the join's
/// reply in it.
#[tokio::test]
async fn a_link_code_at_sign_up_joins_the_group_once_the_address_is_proven() {
    let resources = create_test_server_resources().await.unwrap();
    open_to_verified_addresses(&resources).await;
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;

    let (status, body) = register(&resources, "jacob@held-invite.test", Some(&invite.code)).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["user_status"], "pending", "{body}");
    let user = account(&resources, "jacob@held-invite.test").await;
    let hold = held(&resources, user.id)
        .await
        .expect("the sign-up held the link's invite");
    assert_eq!(
        hold.invite_id, invite.id,
        "the hold names the link's invite"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "a Pending account joins nothing yet"
    );

    assert_eq!(
        confirm_email(&resources, user.id).await,
        UserStatus::Active,
        "fixture: a proven address activates the account"
    );
    let user = account(&resources, "jacob@held-invite.test").await;
    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "joined", "{body}");
    assert_eq!(
        body["group_id"],
        coach.group_id.to_string(),
        "the answer names the invite's group: {body}"
    );
    assert_eq!(
        body["group_name"], GROUP_NAME,
        "the answer names the group: {body}"
    );
    assert_eq!(body["kind"], "member", "joined as an athlete: {body}");
    assert!(
        coach.is_member(&resources, user.id).await,
        "the active account joined the group"
    );

    let tenant = tenant_of(&resources, user.id).await;
    let room = resources
        .common
        .repos
        .chat
        .find_group_conversation(&user.id.to_string(), tenant, &coach.group_id.to_string())
        .await
        .unwrap()
        .expect("the join filed the member's room");
    assert_eq!(
        body["conversation_id"], room.id,
        "the answer names the caller's room for the group"
    );
    let expected = format!(
        "{}\n\n{}",
        rendered(&resources, KEY_GROUP_JOINED, &[GROUP_NAME]),
        rendered(&resources, KEY_GROUP_COACH_SHARING_PENDING, &[])
    );
    assert!(
        expected.contains(GROUP_NAME),
        "the expected reply names the group"
    );
    let replies = resources
        .common
        .repos
        .chat
        .get_messages(&room.id, &user.id.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.role == "assistant" && m.content == expected)
        .count();
    assert_eq!(replies, 1, "the join's reply waits in the room once");

    assert!(
        held(&resources, user.id).await.is_none(),
        "the hold did its work"
    );
    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "outcome": "none" }),
        "nothing is held any more"
    );
}

#[tokio::test]
async fn a_dead_link_code_never_refuses_the_sign_up_and_holds_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    assert!(
        resources
            .common
            .repos
            .groups
            .deactivate_invite(&coach.group_id.to_string(), &invite.id.to_string())
            .await
            .unwrap(),
        "fixture: the invite is withdrawn"
    );

    let (status, body) = register(&resources, "late@held-invite.test", Some(&invite.code)).await;
    assert_eq!(
        status, 201,
        "a withdrawn link still lets the athlete sign up: {body}"
    );
    assert_eq!(body["user_status"], "pending", "{body}");
    let user = account(&resources, "late@held-invite.test").await;
    assert!(
        held(&resources, user.id).await.is_none(),
        "a dead code is not held"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "a dead code joins no group"
    );
}

/// The call right after a sign-in that started from a group link: a Pending
/// account holds the invite, a dead code is refused like any unusable code
/// and leaves the hold in place, and the account joins once active.
#[tokio::test]
async fn a_pending_account_holds_a_link_after_signing_in_and_joins_once_active() {
    let resources = create_test_server_resources().await.unwrap();
    open_to_verified_addresses(&resources).await;
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    register(&resources, "signed-in@held-invite.test", None).await;
    let user = account(&resources, "signed-in@held-invite.test").await;
    assert_eq!(
        user.user_status,
        UserStatus::Pending,
        "fixture: the new account waits as Pending"
    );

    let (status, body) = hold_link(&resources, &user, &invite.code).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "held": true, "kind": "member" }),
        "the hold names a member's invite"
    );
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        invite.id,
        "the hold names the link's invite"
    );

    let (status, body) = hold_link(&resources, &user, "NOPE2345").await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains(INVALID), "{body}");
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        invite.id,
        "a refused code leaves the earlier hold alone"
    );

    assert_eq!(
        confirm_email(&resources, user.id).await,
        UserStatus::Active,
        "fixture: a proven address activates the account"
    );
    let user = account(&resources, "signed-in@held-invite.test").await;
    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "joined", "{body}");
    assert!(
        coach.is_member(&resources, user.id).await,
        "the account joined once active"
    );
}

/// The waiting screen's code field holds a plain member invite rather than
/// redeeming it as a signup code: it approves nothing and spends no use.
#[tokio::test]
async fn the_waiting_screen_holds_a_plain_member_invite_without_redeeming_it() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, Some(5)).await;
    register(&resources, "waiting@held-invite.test", None).await;
    let user = account(&resources, "waiting@held-invite.test").await;

    let (status, body) = redeem(&resources, &user, &invite.code).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "user_status": "pending", "group_joined": null, "group_invite_held": true }),
        "the waiting screen holds the code and redeems nothing"
    );
    assert_eq!(
        account(&resources, "waiting@held-invite.test")
            .await
            .user_status,
        UserStatus::Pending,
        "a member invite approves no one"
    );
    assert!(
        resources
            .common
            .repos
            .signup_codes
            .redemption_for(user.id)
            .await
            .unwrap()
            .is_none(),
        "no signup-code redemption is recorded"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        0,
        "no use is claimed"
    );
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        invite.id,
        "the waiting screen held the plain invite"
    );
}

#[tokio::test]
async fn the_waiting_screen_still_redeems_an_approving_coach_code() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let response = AxumTestRequest::post(&format!("/api/groups/{}/invites", coach.group_id))
        .header("authorization", &coach.auth)
        .json(&json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14 }))
        .send(GroupRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(
        response.status(),
        201,
        "fixture: the coach mints an approving code"
    );
    let code = body_of(&response.body_text())["code"]
        .as_str()
        .unwrap()
        .to_owned();
    register(&resources, "coached@held-invite.test", None).await;
    let user = account(&resources, "coached@held-invite.test").await;
    resources
        .common
        .repos
        .email_verification
        .mark_verified(user.id)
        .await
        .unwrap();

    let (status, body) = redeem(&resources, &user, &code).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["user_status"], &body["group_joined"]),
        (&json!("active"), &json!(true)),
        "an approving code approves and joins as before: {body}"
    );
}

// ---------------------------------------------------------------------------
// 2. Redeeming the hold
// ---------------------------------------------------------------------------

/// An athlete who opened the link in time is not turned away because the
/// invite expired while they confirmed their address.
#[tokio::test]
async fn a_held_invite_that_expired_since_still_joins() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "slow-confirm").await;
    let (status, body) = hold_link(&resources, &user, &invite.code).await;
    assert_eq!(status, 200, "held while live: {body}");

    expire_invite(
        &resources,
        coach.tenant,
        invite.id,
        Timestamp::now() - Elapsed::minutes(1),
    )
    .await;
    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "joined", "{body}");
    assert!(
        coach.is_member(&resources, user.id).await,
        "an invite that expired after the hold still joins"
    );
}

#[tokio::test]
async fn a_held_invite_its_coach_withdrew_is_refused_and_the_hold_dropped() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "withdrawn").await;
    assert_eq!(
        hold_link(&resources, &user, &invite.code).await.0,
        200,
        "fixture: the live invite is held"
    );
    assert!(
        resources
            .common
            .repos
            .groups
            .deactivate_invite(&coach.group_id.to_string(), &invite.id.to_string())
            .await
            .unwrap(),
        "fixture: the coach withdraws the invite"
    );

    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "refused",
            "reason": "invalid",
            "message": rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[]),
        }),
        "a withdrawn invite is refused as an unusable code"
    );
    assert!(
        held(&resources, user.id).await.is_none(),
        "a decided join drops the hold"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "a withdrawn invite joins no one"
    );
}

#[tokio::test]
async fn a_hold_older_than_its_window_lapses_and_is_dropped() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "lapsed").await;
    // A hold waits as long as a coach's signup code stays redeemable; this
    // one is a day older than that.
    resources
        .common
        .repos
        .signup_codes
        .hold_group_invite(
            user.id,
            invite.id,
            Timestamp::now() - Elapsed::days(SIGNUP_INVITE_MAX_DAYS + 1),
        )
        .await
        .unwrap();

    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "refused",
            "reason": "invalid",
            "message": rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[]),
        }),
        "a lapsed hold is refused as an unusable code"
    );
    assert!(
        held(&resources, user.id).await.is_none(),
        "a lapsed hold is dropped"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "a lapsed hold joins no one"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        0,
        "a lapsed hold spends no use"
    );
}

/// A member holding the invite again is told so: the hold is dropped and the
/// coach is not told a second time.
#[tokio::test]
async fn a_member_holding_the_invite_again_is_already_a_member_and_the_coach_hears_once() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let coach_chat = coach.seat_as_coach_with_chat(&resources).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "twice").await;
    let notice = rendered(
        &resources,
        KEY_GROUP_COACH_JOINED_NOTICE,
        &[&person_name(&user), GROUP_NAME],
    );

    assert_eq!(
        hold_link(&resources, &user, &invite.code).await.0,
        200,
        "fixture: the live invite is held"
    );
    let (_, body) = join_held(&resources, &user).await;
    assert_eq!(body["outcome"], "joined", "{body}");
    assert_eq!(
        coach.rows_reading(&resources, &coach_chat, &notice).await,
        1,
        "the first join tells the coach"
    );
    let members = coach.member_count(&resources).await;

    assert_eq!(
        hold_link(&resources, &user, &invite.code).await.0,
        200,
        "the invite is held a second time"
    );
    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "already_member",
            "group_id": coach.group_id.to_string(),
            "group_name": GROUP_NAME,
            "kind": "member",
        }),
        "a member is told they are already in the group"
    );
    assert!(
        held(&resources, user.id).await.is_none(),
        "the hold is dropped"
    );
    assert_eq!(
        coach.rows_reading(&resources, &coach_chat, &notice).await,
        1,
        "no second notice reaches the coach"
    );
    assert_eq!(
        coach.member_count(&resources).await,
        members,
        "the second ask adds no member"
    );
}

/// A coach invite travels as a `/groups/join/<code>` link too. Held, it
/// attaches an account with coach access acting in the group's tenant as the
/// group's human coach, filing their room with `/group join`'s reply; any
/// other account is refused as an unusable code and attaches nothing.
#[tokio::test]
async fn a_held_coach_invite_attaches_an_eligible_coach_and_refuses_anyone_else() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;

    // An athlete with no coach access, acting in their own tenant: the
    // coach link is refused at hold time, not as a dead code.
    let (outsider, _) = athlete(&resources, "not-a-coach").await;
    let (status, body) = hold_link(&resources, &outsider, &invite.code).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        body["details"]["reason"], "coach_invite_not_for_account",
        "the refusal says the link is not for this account: {body}"
    );
    assert!(
        held(&resources, outsider.id).await.is_none(),
        "a refused coach link holds nothing"
    );

    // An account that lost its eligibility between hold and join is still
    // refused at join time as an unusable code, and the hold is dropped.
    resources
        .common
        .repos
        .signup_codes
        .hold_group_invite(outsider.id, invite.id, Timestamp::now())
        .await
        .unwrap();
    let (status, body) = join_held(&resources, &outsider).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "refused",
            "reason": "invalid",
            "message": rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[]),
        }),
        "an ineligible account is refused as an unusable code"
    );
    assert!(
        held(&resources, outsider.id).await.is_none(),
        "a refused coach invite drops the hold"
    );
    assert_eq!(
        coach.coach_user_id(&resources).await,
        None,
        "an ineligible account attaches as no one's coach"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        0,
        "a refused coach invite spends no use"
    );

    let staff = coach.staff_coach(&resources).await;
    assert_eq!(
        hold_link(&resources, &staff, &invite.code).await.0,
        200,
        "the coach invite is held for the eligible coach"
    );
    let (status, body) = join_held(&resources, &staff).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "joined", "{body}");
    assert_eq!(body["kind"], "coach", "{body}");
    assert_eq!(
        body["group_id"],
        coach.group_id.to_string(),
        "the answer names the invite's group: {body}"
    );
    assert_eq!(
        body["group_name"], GROUP_NAME,
        "the answer names the group: {body}"
    );
    assert_eq!(
        coach.coach_user_id(&resources).await,
        Some(staff.id),
        "the held coach invite seats its coach"
    );
    assert!(
        !coach.is_member(&resources, staff.id).await,
        "a coach is attached, never added as an athlete"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        1,
        "seating the coach spends one use"
    );
    assert!(
        held(&resources, staff.id).await.is_none(),
        "the attach drops the hold"
    );

    let room = resources
        .common
        .repos
        .chat
        .find_group_conversation(
            &staff.id.to_string(),
            coach.tenant,
            &coach.group_id.to_string(),
        )
        .await
        .unwrap()
        .expect("the attach filed the coach's room");
    assert_eq!(
        body["conversation_id"], room.id,
        "the answer names the coach's room for the group"
    );
    let reply = rendered(&resources, KEY_GROUP_JOINED_AS_COACH, &[GROUP_NAME]);
    let replies = resources
        .common
        .repos
        .chat
        .get_messages(&room.id, &staff.id.to_string(), coach.tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.role == "assistant" && m.content == reply)
        .count();
    assert_eq!(
        replies, 1,
        "the coach reads /group join's reply in the room"
    );
}

#[tokio::test]
async fn a_coach_already_attached_holding_the_coach_invite_again_is_told_so() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;
    let staff = coach.staff_coach(&resources).await;
    assert_eq!(
        hold_link(&resources, &staff, &invite.code).await.0,
        200,
        "fixture: the coach invite is held"
    );
    assert_eq!(
        join_held(&resources, &staff).await.1["outcome"],
        "joined",
        "fixture: the first attach seats the coach"
    );

    assert_eq!(
        hold_link(&resources, &staff, &invite.code).await.0,
        200,
        "the coach invite is held a second time"
    );
    let (status, body) = join_held(&resources, &staff).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "already_member",
            "group_id": coach.group_id.to_string(),
            "group_name": GROUP_NAME,
            "kind": "coach",
        }),
        "an attached coach is told they already coach the group"
    );
    assert!(
        held(&resources, staff.id).await.is_none(),
        "the second ask drops the hold"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        1,
        "asking again spends no second use"
    );
}

/// Unlike a member invite, a coach invite is checked whole at the join,
/// expiry included: it goes to an account that already coaches and can
/// redeem it at once.
#[tokio::test]
async fn a_held_coach_invite_that_expired_since_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;
    let staff = coach.staff_coach(&resources).await;
    assert_eq!(
        hold_link(&resources, &staff, &invite.code).await.0,
        200,
        "fixture: the coach invite is held"
    );
    expire_invite(
        &resources,
        coach.tenant,
        invite.id,
        Timestamp::now() - Elapsed::minutes(1),
    )
    .await;

    let (status, body) = join_held(&resources, &staff).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "refused",
            "reason": "invalid",
            "message": rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[]),
        }),
        "an expired coach invite is refused as an unusable code"
    );
    assert_eq!(
        coach.coach_user_id(&resources).await,
        None,
        "an expired coach invite seats no one"
    );
    assert!(
        held(&resources, staff.id).await.is_none(),
        "the refusal drops the hold"
    );
}

// ---------------------------------------------------------------------------
// 3. Uses and capacity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_hold_spends_no_use_and_the_join_spends_one() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, Some(5)).await;
    let (user, _) = athlete(&resources, "counted").await;

    assert_eq!(
        hold_link(&resources, &user, &invite.code).await.0,
        200,
        "fixture: the live invite is held"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        0,
        "the hold claims no use"
    );
    let (_, body) = join_held(&resources, &user).await;
    assert_eq!(body["outcome"], "joined", "{body}");
    assert_eq!(
        use_count(&resources, invite.id).await,
        1,
        "the join claims one"
    );
}

#[tokio::test]
async fn an_invite_used_up_between_hold_and_join_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, Some(1)).await;
    let (holder, _) = athlete(&resources, "holder").await;
    let (quicker, _) = athlete(&resources, "quicker").await;
    assert_eq!(
        hold_link(&resources, &holder, &invite.code).await.0,
        200,
        "fixture: the last use is held"
    );
    resources
        .group_service()
        .join_group(&invite.code, quicker.id, coach.tenant)
        .await
        .unwrap();
    assert_eq!(
        use_count(&resources, invite.id).await,
        1,
        "the last use is spent"
    );

    let (status, body) = join_held(&resources, &holder).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "outcome": "refused",
            "reason": "invalid",
            "message": rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[]),
        }),
        "a used-up invite is refused as an unusable code"
    );
    assert!(
        held(&resources, holder.id).await.is_none(),
        "the refusal drops the hold"
    );
    assert!(
        !coach.is_member(&resources, holder.id).await,
        "a used-up invite joins no one"
    );
    assert_eq!(
        use_count(&resources, invite.id).await,
        1,
        "the refused join spends no use"
    );
}

#[tokio::test]
async fn a_group_full_at_join_time_is_refused_by_name_and_keeps_the_hold_for_a_retry() {
    let resources = create_test_server_resources().await.unwrap();
    // The owner's own membership plus one athlete fills it.
    let coach = Coach::new(&resources, 2).await;
    let invite = coach.invite(&resources, None).await;
    let (holder, _) = athlete(&resources, "too-late").await;
    let (quicker, _) = athlete(&resources, "first-in").await;
    assert_eq!(
        hold_link(&resources, &holder, &invite.code).await.0,
        200,
        "fixture: the live invite is held"
    );
    resources
        .group_service()
        .join_group(&invite.code, quicker.id, coach.tenant)
        .await
        .unwrap();
    assert_eq!(coach.member_count(&resources).await, 2, "the group is full");

    let (usage_lines, usage_guard) = capture_target(usage_log::TARGET);
    let (status, body) = join_held(&resources, &holder).await;
    drop(usage_guard);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "refused", "{body}");
    assert_eq!(body["reason"], "group_full", "{body}");

    // The refusal is logged as the group's member cap, against the athlete
    // who could not join (carnet#1630).
    let lines = drain(&usage_lines);
    assert_eq!(lines.len(), 1, "one usage line per refusal: {lines:?}");
    let line = &lines[0];
    assert_eq!(
        line.get("user_id").map(String::as_str),
        Some(holder.id.to_string().as_str()),
        "the line names the athlete refused: {line:?}"
    );
    assert_eq!(
        line.get("limit_type").map(String::as_str),
        Some("group_members"),
        "the line names the member cap: {line:?}"
    );
    assert_eq!(
        line.get("current").map(String::as_str),
        Some("2"),
        "the owner and one athlete fill it: {line:?}"
    );
    assert_eq!(
        line.get("limit").map(String::as_str),
        Some("2"),
        "the group holds two members: {line:?}"
    );
    assert_eq!(
        line.get("surface").map(String::as_str),
        Some("held_group_invite"),
        "the refusal was met redeeming a held invite: {line:?}"
    );
    let message = body["message"].as_str().unwrap();
    assert_eq!(
        message,
        rendered(&resources, KEY_GROUP_JOIN_FULL, &[GROUP_NAME]),
        "a full group is refused by name"
    );
    assert!(message.contains(GROUP_NAME), "{message}");
    let kept = held(&resources, holder.id)
        .await
        .expect("a full group keeps the hold");
    assert_eq!(kept.invite_id, invite.id, "the same invite is kept");
    assert!(
        kept.refused_full_at.is_some(),
        "the kept hold is stamped as refused by a full group"
    );
    assert!(
        !coach.is_member(&resources, holder.id).await,
        "a full group admits no one"
    );

    // The athlete's own read of the hold says so.
    let (status, read) = read_hold(&resources, &holder).await;
    assert_eq!(status, 200, "{read}");
    assert_eq!(read["held"]["state"], "group_full", "{read}");

    // A seat opens: the retry joins, and the hold is released.
    resources
        .common
        .repos
        .groups
        .remove_member(&coach.group_id.to_string(), quicker.id)
        .await
        .unwrap();
    let (status, body) = join_held(&resources, &holder).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "joined", "{body}");
    assert!(coach.is_member(&resources, holder.id).await);
    assert!(
        held(&resources, holder.id).await.is_none(),
        "a successful join releases the hold"
    );
}

/// A fresh link replaces a kept full-group hold, and the stamp goes with it.
#[tokio::test]
async fn a_new_hold_replaces_a_full_group_stamp() {
    let resources = create_test_server_resources().await.unwrap();
    // The owner's membership plus one athlete fills it.
    let full = Coach::new(&resources, 2).await;
    let open = Coach::new(&resources, 10).await;
    let full_invite = full.invite(&resources, None).await;
    let (filler, _) = athlete(&resources, "fills-the-group").await;
    resources
        .group_service()
        .join_group(&full_invite.code, filler.id, full.tenant)
        .await
        .unwrap();
    let open_invite = open.invite(&resources, None).await;
    let (holder, _) = athlete(&resources, "redirected").await;
    hold_link(&resources, &holder, &full_invite.code).await;
    let (_, body) = join_held(&resources, &holder).await;
    assert_eq!(body["reason"], "group_full", "{body}");
    assert!(held(&resources, holder.id)
        .await
        .unwrap()
        .refused_full_at
        .is_some());

    assert_eq!(
        hold_link(&resources, &holder, &open_invite.code).await.0,
        200
    );
    let replaced = held(&resources, holder.id).await.unwrap();
    assert_eq!(replaced.invite_id, open_invite.id);
    assert!(
        replaced.refused_full_at.is_none(),
        "the new hold starts with no full-group stamp"
    );
    // Holding the full group's link again starts over too.
    hold_link(&resources, &holder, &full_invite.code).await;
    let (_, body) = join_held(&resources, &holder).await;
    assert_eq!(body["reason"], "group_full", "{body}");
    hold_link(&resources, &holder, &full_invite.code).await;
    assert!(
        held(&resources, holder.id)
            .await
            .unwrap()
            .refused_full_at
            .is_none(),
        "holding the same link again clears the stamp"
    );
}

// ---------------------------------------------------------------------------
// 3b. The holder reads their hold (carnet#1650)
// ---------------------------------------------------------------------------

/// `GET /api/user/group-invite`.
async fn read_hold(resources: &Arc<ServerContext>, user: &User) -> (u16, Value) {
    let response = AxumTestRequest::get("/api/user/group-invite")
        .header("authorization", &bearer(resources, user).await)
        .send(auth_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

#[tokio::test]
async fn the_holder_reads_the_group_and_inviter_of_their_hold_and_never_the_code() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    resources
        .common
        .repos
        .users
        .update_display_name(coach.id, "Jeremy Bonneau")
        .await
        .unwrap();
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "reader").await;

    let (status, none) = read_hold(&resources, &user).await;
    assert_eq!(status, 200, "{none}");
    assert_eq!(none, json!({ "held": null }), "nothing held reads null");

    hold_link(&resources, &user, &invite.code).await;
    let (status, read) = read_hold(&resources, &user).await;
    assert_eq!(status, 200, "{read}");
    let held_at = read["held"]["held_at"]
        .as_str()
        .expect("a held_at")
        .to_owned();
    assert_eq!(
        read,
        json!({ "held": {
            "kind": "member",
            "group_name": GROUP_NAME,
            "inviter_name": "Jeremy B.",
            "inviter_initials": "JB",
            "has_coach": false,
            "coach_name": null,
            "held_at": held_at,
            "state": "pending",
        }}),
        "exactly the card's fields"
    );
    assert!(
        !read.to_string().contains(&invite.code),
        "the code never comes back"
    );

    // A lapsed hold reads as none.
    resources
        .common
        .repos
        .signup_codes
        .hold_group_invite(
            user.id,
            invite.id,
            Timestamp::now() - Elapsed::days(SIGNUP_INVITE_MAX_DAYS + 1),
        )
        .await
        .unwrap();
    let (_, lapsed) = read_hold(&resources, &user).await;
    assert_eq!(lapsed, json!({ "held": null }), "a lapsed hold reads null");

    // The read needs a session.
    let response = AxumTestRequest::get("/api/user/group-invite")
        .send(auth_routes(&resources))
        .await;
    assert_eq!(response.status(), 401);
}

// ---------------------------------------------------------------------------
// 3c. A coach link never displaces a member hold (carnet#1650)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_coach_link_does_not_replace_the_member_hold_of_an_account_that_cannot_coach() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let member_invite = coach.invite(&resources, None).await;
    let coach_invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;
    let (user, _) = athlete(&resources, "member-opens-coach-link").await;
    assert_eq!(
        hold_link(&resources, &user, &member_invite.code).await.0,
        200
    );

    let (status, body) = hold_link(&resources, &user, &coach_invite.code).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        body["details"]["reason"], "coach_invite_not_for_account",
        "{body}"
    );
    assert!(
        !body.to_string().contains("isn't valid"),
        "not worded as a dead code: {body}"
    );
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        member_invite.id,
        "the member hold stays in place"
    );

    // Through the waiting screen's field the same rule holds.
    let (status, _) = redeem(&resources, &user, &coach_invite.code).await;
    assert_ne!(status, 200, "the waiting screen does not hold it either");
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        member_invite.id,
        "still the member hold"
    );

    // An eligible coach does hold it.
    let staff = coach.staff_coach(&resources).await;
    assert_eq!(
        hold_link(&resources, &staff, &coach_invite.code).await.0,
        200
    );
    assert_eq!(
        held(&resources, staff.id).await.unwrap().invite_id,
        coach_invite.id
    );
}

/// An ineligible coach code at sign-up leaves the account created.
#[tokio::test]
async fn an_ineligible_coach_link_at_sign_up_does_not_fail_the_sign_up() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let coach_invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;
    let email = format!("fresh-{}@held-invite.test", Uuid::new_v4());
    let (status, body) = register(&resources, &email, Some(&coach_invite.code)).await;
    assert!(
        (200..300).contains(&status),
        "the sign-up goes through: {status} {body}"
    );
    let created = account(&resources, &email).await;
    assert!(
        held(&resources, created.id).await.is_none(),
        "the coach link is not held for an account that cannot coach"
    );
}

// ---------------------------------------------------------------------------
// 4. Who may redeem, and one hold per account
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pending_account_cannot_redeem_and_keeps_its_hold() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (status, _) = register(&resources, "early@held-invite.test", Some(&invite.code)).await;
    assert_eq!(status, 201, "fixture: the sign-up succeeds");
    let user = account(&resources, "early@held-invite.test").await;
    assert_eq!(
        user.user_status,
        UserStatus::Pending,
        "fixture: the new account waits as Pending"
    );

    let (status, body) = join_held(&resources, &user).await;
    // The bearer path of the auth gate answers a Pending account's token as
    // invalid credentials (401), as it does on every authenticated route; the
    // app asks only once the account is active.
    assert_eq!(
        status, 401,
        "the auth gate refuses a Pending account: {body}"
    );
    assert_eq!(body["code"], "AuthInvalid", "{body}");
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        invite.id,
        "the hold waits for the account to be active"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "a Pending account joins nothing"
    );
}

#[tokio::test]
async fn a_newer_hold_replaces_the_older_and_releasing_the_older_keeps_it() {
    let resources = create_test_server_resources().await.unwrap();
    let first_coach = Coach::new(&resources, 10).await;
    let second_coach = Coach::new(&resources, 10).await;
    let older = first_coach.invite(&resources, None).await;
    let newer = second_coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "two-links").await;
    let repos = &resources.common.repos;

    assert_eq!(
        hold_link(&resources, &user, &older.code).await.0,
        200,
        "the older link is held"
    );
    assert_eq!(
        hold_link(&resources, &user, &newer.code).await.0,
        200,
        "the newer link is held"
    );
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        newer.id,
        "one hold per account: the newer link wins"
    );

    assert!(
        !repos
            .signup_codes
            .release_group_invite_hold(user.id, older.id)
            .await
            .unwrap(),
        "releasing the older invite releases nothing"
    );
    assert_eq!(
        held(&resources, user.id).await.unwrap().invite_id,
        newer.id,
        "the newer hold survives releasing the older invite"
    );
    assert!(
        repos
            .signup_codes
            .release_group_invite_hold(user.id, newer.id)
            .await
            .unwrap(),
        "releasing the held invite drops it"
    );
    assert!(
        held(&resources, user.id).await.is_none(),
        "nothing is held after the release"
    );
}

/// Joining grants the group's coach sharing, so an admin acting as the
/// athlete joins nothing and leaves the hold for the athlete's own session
/// (carnet#1634).
#[tokio::test]
async fn a_support_session_redeems_no_held_invite() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "supported").await;
    let (status, body) = hold_link(&resources, &user, &invite.code).await;
    assert_eq!(
        status, 200,
        "fixture: the athlete's own session holds the link: {body}"
    );

    let support = impersonation_of(&resources, &bearer(&resources, &user).await).await;
    let response = AxumTestRequest::post("/api/chat/groups/join")
        .header("authorization", &support)
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_support_refused(
        response.status(),
        &response.body_text(),
        "join a held group invite",
    );
    assert_refusal_audited(&resources, user.id, "join a held group invite").await;
    assert_eq!(
        held(&resources, user.id).await.map(|hold| hold.invite_id),
        Some(invite.id),
        "the refused support session leaves the hold in place"
    );
    assert!(
        !coach.is_member(&resources, user.id).await,
        "the support session joined nothing"
    );

    let (status, body) = join_held(&resources, &user).await;
    assert_eq!(status, 200, "the athlete's own session is served: {body}");
    assert_eq!(
        body["outcome"], "joined",
        "the athlete's own session joins: {body}"
    );
}

/// The hold names a group the account will join, so a support session takes
/// none either (carnet#1634).
#[tokio::test]
async fn a_support_session_holds_no_group_invite() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let (user, _) = athlete(&resources, "supported-hold").await;

    let support = impersonation_of(&resources, &bearer(&resources, &user).await).await;
    let response = AxumTestRequest::put("/api/user/group-invite")
        .header("authorization", &support)
        .json(&json!({ "code": invite.code }))
        .send(auth_routes(&resources))
        .await;
    assert_support_refused(
        response.status(),
        &response.body_text(),
        "hold a group invite",
    );
    assert_refusal_audited(&resources, user.id, "hold a group invite").await;
    assert!(
        held(&resources, user.id).await.is_none(),
        "the refused support session holds nothing"
    );
}
