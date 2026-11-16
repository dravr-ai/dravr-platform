// ABOUTME: One table of what every admin route that acts on an account answers, by caller and by target
// ABOUTME: Pins the rank rule (an admin account takes a super caller) route by route, with its order among the other refusals

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Seven admin routes act on an account: suspend, approve, pre-approve its
//! address, disconnect a provider, delete, reset its onboarding, reset its
//! password. Each refuses a service token without the super-admin flag
//! when the account is an admin (403, one sentence), and each is otherwise
//! answered as before (carnet#1589).
//!
//! The table drives every route with a scoped `ManageUsers` token, a
//! super-admin token and, where the console mounts the route, an admin's
//! own session, against an admin account and a plain one, and reads the
//! store after each answer. The tests below it pin where the rule stands
//! among the other refusals of the same routes: before a token's tenant,
//! before the last active admin, before "already approved", before a
//! missing connection and before a disconnect chokepoint that is not wired.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_role_rules_test::reset_tokens_issued_for;
use crate::admin_token_routes_test::{bearer, mint, mint_bound};
use crate::admin_user_removal_test::{chokepoint, connect_strava, resources, RevokeStub};
use crate::common::generate_test_token;
use crate::helpers::axum_test::AxumTestRequest;

use std::sync::Arc;

use axum::Router;
use chrono::Utc;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, GeneratedAdminToken};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_services::user_removal::held_providers;
use serde_json::{json, Value};
use uuid::Uuid;

/// What a caller without the rank is answered on an admin account.
const RANK_REFUSAL: &str = "Acting on an admin account requires a super-admin token";

/// How the refusal of a token acting outside its tenant starts.
const TENANT_REFUSAL: &str = "Token is scoped to tenant";

/// What stands where an answer carried a password reset token.
const RESET_TOKEN_NOT_QUOTED: &str = "[a reset token, not quoted]";

/// A route of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Suspend,
    Approve,
    Allow,
    Disconnect,
    Delete,
    ResetOnboarding,
    ResetPassword,
}

impl Op {
    /// Every route that acts on an account, on the admin-token mount.
    const ALL: [Self; 7] = [
        Self::Suspend,
        Self::Approve,
        Self::Allow,
        Self::Disconnect,
        Self::Delete,
        Self::ResetOnboarding,
        Self::ResetPassword,
    ];

    /// The routes the console mounts too, behind an admin's session.
    const ON_THE_CONSOLE: [Self; 4] = [
        Self::Suspend,
        Self::Approve,
        Self::Allow,
        Self::ResetPassword,
    ];

    /// The status an account is in for the route to have something to do.
    const fn starts_as(self) -> UserStatus {
        match self {
            Self::Approve => UserStatus::Suspended,
            Self::Allow => UserStatus::Pending,
            Self::Suspend
            | Self::Disconnect
            | Self::Delete
            | Self::ResetOnboarding
            | Self::ResetPassword => UserStatus::Active,
        }
    }

    /// The request for `target` under `prefix` (`""` on the token mount,
    /// `"/api"` on the console).
    fn request(self, prefix: &str, target: &User) -> AxumTestRequest {
        let id = target.id;
        let reason = json!({ "reason": "carnet#1589 table" });
        match self {
            Self::Suspend => {
                AxumTestRequest::post(&format!("{prefix}/admin/suspend-user/{id}")).json(&reason)
            }
            Self::Approve => {
                AxumTestRequest::post(&format!("{prefix}/admin/approve-user/{id}")).json(&reason)
            }
            Self::Allow => AxumTestRequest::post(&format!("{prefix}/admin/pre-approved-emails"))
                .json(&json!({ "email": target.email })),
            Self::Disconnect => {
                AxumTestRequest::delete(&format!("{prefix}/admin/users/{id}/providers/strava"))
            }
            Self::Delete => {
                AxumTestRequest::delete(&format!("{prefix}/admin/users/{id}")).json(&reason)
            }
            Self::ResetOnboarding => {
                AxumTestRequest::post(&format!("{prefix}/admin/users/{id}/onboarding/reset"))
                    .json(&reason)
            }
            Self::ResetPassword => {
                AxumTestRequest::post(&format!("{prefix}/admin/users/{id}/reset-password"))
            }
        }
    }
}

/// The server under test, its Strava revocation stub, and who asks.
struct Desk {
    resources: Arc<ServerContext>,
    stub: RevokeStub,
    /// A `ManageUsers` token bound to no tenant, without the super-admin flag.
    scoped: GeneratedAdminToken,
    /// A super-admin token.
    root: GeneratedAdminToken,
    /// An active admin, who keeps the platform from ever being down to its
    /// last one, and the bearer header of its console session.
    standing: User,
    standing_session: String,
}

impl Desk {
    async fn open() -> Self {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        let scoped = mint(
            &resources,
            "support-desk",
            vec![AdminPermission::ManageUsers],
            false,
        )
        .await
        .unwrap();
        let root = mint(
            &resources,
            "platform-ops",
            AdminPermissions::super_admin().to_vec(),
            true,
        )
        .await
        .unwrap();
        let (standing, _) = seed(&resources, UserRole::Admin, UserStatus::Active).await;
        let standing_session = format!(
            "Bearer {}",
            generate_test_token(&resources, &standing).await
        );
        Self {
            resources,
            stub,
            scoped,
            root,
            standing,
            standing_session,
        }
    }

    /// The admin context the composition root builds, the disconnect
    /// chokepoint wired when `wired`.
    fn context(&self, wired: bool) -> AdminApiContext {
        let resources = &self.resources;
        let mut context = AdminApiContext::new(AdminApiContextInit {
            database: resources.agent.database.clone(),
            repos: resources.common.repos.clone(),
            jwt_secret: resources.auth.admin_jwt_secret.to_string(),
            auth_manager: resources.auth.auth_manager.clone(),
            jwks_manager: resources.auth.jwks_manager.clone(),
            admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
            harness_config_registry: resources.fitness.harness_config_registry.clone(),
            guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
            prompt_registry: resources.mcp.prompt_registry.clone(),
            tool_description_registry: resources.mcp.tool_description_registry.clone(),
            evidence_registry: resources.mcp.evidence_registry.clone(),
            messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
            cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
            persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
            training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
            contremaitre_config: None,
            app_behavior: resources.common.config.app_behavior.clone(),
        });
        if wired {
            context.provider_disconnector = Some(chokepoint(resources, &self.stub.url));
        }
        context
    }

    /// The admin-token mount `pierre-cli` reaches.
    fn token_mount(&self) -> Router {
        AdminRoutes::routes(self.context(true))
    }

    /// The console mount: the same handlers behind an admin's session.
    fn console_mount(&self) -> Router {
        AdminRoutes::cookie_admin_routes::<ServerContext>(self.context(true), &self.resources)
    }

    /// Send `op` for `target` on the token mount as `caller`.
    async fn ask(&self, op: Op, caller: &GeneratedAdminToken, target: &User) -> (u16, Value) {
        let request = op
            .request("", target)
            .header("authorization", &bearer(caller));
        answer_of(request, self.token_mount()).await
    }

    /// Send `op` for `target` on the console as the standing admin.
    async fn ask_on_the_console(&self, op: Op, target: &User) -> (u16, Value) {
        let request = op
            .request("/api", target)
            .header("authorization", &self.standing_session);
        answer_of(request, self.console_mount()).await
    }

    /// An account for `op` to act on, with `role`, holding a Strava grant.
    async fn target(&self, op: Op, role: UserRole) -> User {
        let (user, tenant) = seed(&self.resources, role, op.starts_as()).await;
        connect_strava(&self.resources.common.repos, user.id, tenant, None).await;
        user
    }

    /// Whether what `op` writes for `target` is in the store.
    async fn landed(&self, op: Op, target: &User) -> bool {
        let repos = &self.resources.common.repos;
        let stored = repos.users.get_global(target.id).await.unwrap();
        match op {
            Op::Suspend => stored.unwrap().user_status == UserStatus::Suspended,
            Op::Approve | Op::Allow => stored.unwrap().user_status == UserStatus::Active,
            // The reset disconnects every provider it does not keep for a group.
            Op::Disconnect | Op::ResetOnboarding => {
                held_providers(repos, target.id).await.unwrap().is_empty()
            }
            Op::Delete => stored.is_none(),
            Op::ResetPassword => reset_tokens_issued_for(&self.resources, target.id).await > 0,
        }
    }
}

/// An account with `role` in `status`, owning a tenant.
async fn seed(
    resources: &Arc<ServerContext>,
    role: UserRole,
    status: UserStatus,
) -> (User, TenantId) {
    let email = format!("{}@example.com", Uuid::new_v4());
    let mut user = User::new(email.clone(), "hash".to_owned(), Some(email));
    user.role = role;
    user.user_status = status;
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "table tenant".to_owned(),
            slug: format!("table-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user, tenant_id)
}

/// The status and the answer of `request` on `router`, fit to quote in a
/// failed assertion: the reset route answers with the reset token itself,
/// which is replaced before the answer is returned.
async fn answer_of(request: AxumTestRequest, router: Router) -> (u16, Value) {
    let response = request.send(router).await;
    let status = response.status();
    let text = response.text();
    let mut answer: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if let Some(token) = answer.pointer_mut("/data/reset_token") {
        *token = json!(RESET_TOKEN_NOT_QUOTED);
    }
    (status, answer)
}

/// The sentence a refusal answers with.
fn sentence(answer: &Value) -> &str {
    answer["message"].as_str().unwrap_or_default()
}

/// The table: every route, a caller with the rank and one without, an admin
/// account and a plain one. Without the rank an admin account answers 403
/// in the rank rule's words and nothing is written; every other cell is
/// answered 200 and its write is in the store.
#[tokio::test]
async fn every_route_refuses_an_admin_account_to_a_caller_without_the_rank() {
    let desk = Desk::open().await;

    for op in Op::ALL {
        for (caller, has_the_rank) in [(&desk.scoped, false), (&desk.root, true)] {
            for role in [UserRole::Admin, UserRole::User] {
                let target = desk.target(op, role).await;
                let cell = format!("{op:?}, {}, {role:?} account", caller.service_name);

                let (status, answer) = desk.ask(op, caller, &target).await;

                if role == UserRole::Admin && !has_the_rank {
                    assert_eq!(status, 403, "{cell}: {answer}");
                    assert_eq!(sentence(&answer), RANK_REFUSAL, "{cell}: {answer}");
                    assert!(
                        !desk.landed(op, &target).await,
                        "{cell}: the refusal wrote nothing"
                    );
                } else {
                    assert_eq!(status, 200, "{cell}: {answer}");
                    assert_eq!(answer["success"], json!(true), "{cell}: {answer}");
                    assert!(desk.landed(op, &target).await, "{cell}: the write landed");
                }
            }
        }
    }

    // An admin's console session holds the rank on every route it mounts.
    for op in Op::ON_THE_CONSOLE {
        for role in [UserRole::Admin, UserRole::User] {
            let target = desk.target(op, role).await;
            let cell = format!("{op:?}, console session, {role:?} account");
            let (status, answer) = desk.ask_on_the_console(op, &target).await;
            assert_eq!(status, 200, "{cell}: {answer}");
            assert!(desk.landed(op, &target).await, "{cell}: the write landed");
        }
    }
}

/// A token bound to one tenant, without the flag, on an admin held outside
/// that tenant is refused twice over. The rank rule answers first, on every
/// route that checks both; a plain account outside the tenant is answered
/// by the tenant rule. The password reset reads the account inside the
/// token's tenant before anything else, so outside it answers 404, and
/// inside it the rank rule answers.
#[tokio::test]
async fn the_rank_rule_answers_before_a_tokens_tenant() {
    let desk = Desk::open().await;
    let (_, own_tenant) = seed(&desk.resources, UserRole::User, UserStatus::Active).await;
    let bound = mint_bound(
        &desk.resources,
        "club-desk",
        vec![AdminPermission::ManageUsers],
        own_tenant,
    )
    .await
    .unwrap();

    for op in [
        Op::Suspend,
        Op::Approve,
        Op::Disconnect,
        Op::Delete,
        Op::ResetOnboarding,
    ] {
        let admin = desk.target(op, UserRole::Admin).await;
        let (status, answer) = desk.ask(op, &bound, &admin).await;
        assert_eq!(status, 403, "{op:?}, an admin outside: {answer}");
        assert_eq!(sentence(&answer), RANK_REFUSAL, "{op:?}: {answer}");
        assert!(!desk.landed(op, &admin).await, "{op:?}: nothing written");

        let plain = desk.target(op, UserRole::User).await;
        let (status, answer) = desk.ask(op, &bound, &plain).await;
        assert_eq!(status, 403, "{op:?}, a user outside: {answer}");
        assert!(
            sentence(&answer).starts_with(TENANT_REFUSAL),
            "{op:?}: {answer}"
        );
        assert!(!desk.landed(op, &plain).await, "{op:?}: nothing written");
    }

    let outside = desk.target(Op::ResetPassword, UserRole::Admin).await;
    let (status, answer) = desk.ask(Op::ResetPassword, &bound, &outside).await;
    assert_eq!(status, 404, "an admin outside the tenant: {answer}");

    let (inside, inside_tenant) = seed(&desk.resources, UserRole::Admin, UserStatus::Active).await;
    let bound_inside = mint_bound(
        &desk.resources,
        "admin-club-desk",
        vec![AdminPermission::ManageUsers],
        inside_tenant,
    )
    .await
    .unwrap();
    let (status, answer) = desk.ask(Op::ResetPassword, &bound_inside, &inside).await;
    assert_eq!(status, 403, "an admin inside the tenant: {answer}");
    assert_eq!(sentence(&answer), RANK_REFUSAL, "{answer}");
    assert!(!desk.landed(Op::ResetPassword, &inside).await);
}

/// The rank rule answers before what the write itself would refuse: an
/// admin that is already active is not "already approved" to a caller
/// without the rank, and one with no Strava connection is not "no
/// connection". The same requests from a super-admin token get those
/// answers. An allow reads the rank only for a pending account: the
/// address of an active admin changes nothing and is answered as such.
#[tokio::test]
async fn the_rank_rule_answers_before_what_the_write_would_refuse() {
    let desk = Desk::open().await;
    let (admin, _) = seed(&desk.resources, UserRole::Admin, UserStatus::Active).await;

    let (status, answer) = desk.ask(Op::Approve, &desk.scoped, &admin).await;
    assert_eq!(status, 403, "approve, already active: {answer}");
    assert_eq!(sentence(&answer), RANK_REFUSAL, "{answer}");
    let (status, answer) = desk.ask(Op::Approve, &desk.root, &admin).await;
    assert_eq!(
        status, 400,
        "approve, already active, with the rank: {answer}"
    );
    assert_eq!(sentence(&answer), "User is already approved", "{answer}");

    let (status, answer) = desk.ask(Op::Disconnect, &desk.scoped, &admin).await;
    assert_eq!(status, 403, "disconnect, no connection: {answer}");
    assert_eq!(sentence(&answer), RANK_REFUSAL, "{answer}");
    let (status, answer) = desk.ask(Op::Disconnect, &desk.root, &admin).await;
    assert_eq!(
        status, 404,
        "disconnect, no connection, with the rank: {answer}"
    );

    let (status, answer) = desk.ask(Op::Allow, &desk.scoped, &admin).await;
    assert_eq!(status, 200, "allow, an active admin's address: {answer}");
    assert_eq!(
        answer["data"]["outcome"],
        json!("already_active"),
        "{answer}"
    );

    // The console's own admin is refused its own suspension, as before: the
    // rank it holds is not what answers.
    let (status, answer) = desk.ask_on_the_console(Op::Suspend, &desk.standing).await;
    assert_eq!(status, 400, "an admin suspending itself: {answer}");
    assert_eq!(sentence(&answer), "Cannot suspend yourself", "{answer}");
}

/// The platform's last active admin is suspended and deleted by no one, and
/// a caller without the rank is not told so: the rank rule answers it
/// first. With the rank, the last-admin refusal answers.
#[tokio::test]
async fn the_rank_rule_answers_before_the_last_active_admin() {
    let desk = Desk::open().await;
    let only = &desk.standing;

    for op in [Op::Suspend, Op::Delete] {
        let (status, answer) = desk.ask(op, &desk.scoped, only).await;
        assert_eq!(status, 403, "{op:?}, the last admin: {answer}");
        assert_eq!(sentence(&answer), RANK_REFUSAL, "{op:?}: {answer}");

        let (status, answer) = desk.ask(op, &desk.root, only).await;
        assert_eq!(
            status, 400,
            "{op:?}, the last admin, with the rank: {answer}"
        );
        assert!(
            sentence(&answer).contains("last active admin"),
            "{op:?}: {answer}"
        );
        let stored = desk.resources.common.repos.users.get_global(only.id).await;
        assert!(
            stored.unwrap().is_some_and(|user| user.is_active_admin()),
            "{op:?}: the last admin stands"
        );
    }
}

/// On a server whose disconnect chokepoint is not wired, a disconnect is
/// answered 503 to a caller with the rank, and a caller without it is still
/// answered by the rank rule on an admin account.
#[tokio::test]
async fn the_rank_rule_answers_before_a_chokepoint_that_is_not_wired() {
    let desk = Desk::open().await;
    let admin = desk.target(Op::Disconnect, UserRole::Admin).await;
    let unwired = || AdminRoutes::routes(desk.context(false));

    let request = Op::Disconnect
        .request("", &admin)
        .header("authorization", &bearer(&desk.scoped));
    let (status, answer) = answer_of(request, unwired()).await;
    assert_eq!(status, 403, "without the rank: {answer}");
    assert_eq!(sentence(&answer), RANK_REFUSAL, "{answer}");

    let request = Op::Disconnect
        .request("", &admin)
        .header("authorization", &bearer(&desk.root));
    let (status, answer) = answer_of(request, unwired()).await;
    assert_eq!(status, 503, "with the rank: {answer}");
    assert!(!desk.landed(Op::Disconnect, &admin).await);
}
