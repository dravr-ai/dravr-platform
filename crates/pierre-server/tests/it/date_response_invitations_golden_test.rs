// ABOUTME: Response date goldens for the admin's invitations: coach-seat invitations and pre-approvals that invite
// ABOUTME: Pins the minute in UTC each message states, the expiry instant beside it, and the stored expiry the lists answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/admin/coach-seat-invitations` and
//! `/admin/pre-approved-emails` (carnet#1620, gate 3).
//!
//! An invitation expires a fixed time after it is issued, and its answer says
//! when twice: `expires_at` as an instant with an offset, and a sentence that
//! ends `until 2026-11-24 19:55 UTC`, written by hand at the minute. Both are
//! stamped from the wall clock, so the instant is compared by shape and the
//! sentence with the instant it stands beside: the same minute, in UTC.
//!
//! The lists read the stored row, which is moved to a chosen instant and
//! compared as a literal.
//!
//! The admin routes take their mailer and the frontend's address from the
//! context they are mounted with. The offline server has neither, so the
//! context is built from its parts as the server builds it, with a mailer
//! that records and delivers nothing, and an address under `.test`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use pierre_core::admin::models::CreateAdminTokenRequest;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_services::invitation::IssuedInvitation;
use pierre_services::user_approval::UserApprovalNotifier;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::date_golden::{
    clock_shape, dates_in, dates_in_stamped, get, instant, lies_days_after, on_the_clock, restamp,
    seed_account, send, table, with_json, Instant,
};

/// How long an invitation works, as the admin is told: a literal here, so a
/// lifetime counted another way is seen.
const INVITATION_DAYS: i64 = 7;
const EXPIRE_INVITATION: &str = "UPDATE user_invitations SET expires_at = $1 WHERE email = $2";
const MOVE_INVITATION: &str = "UPDATE user_invitations SET created_at = $1 WHERE email = $2";

/// A mailer that takes every invitation and sends nothing.
struct TakesEveryInvitation;

#[async_trait]
impl UserApprovalNotifier for TakesEveryInvitation {
    async fn notify_user_approved(&self, _: Uuid, _: &str, _: Option<&str>) {}

    async fn notify_user_invited(&self, _: &IssuedInvitation) -> bool {
        true
    }
}

struct Console {
    resources: Arc<ServerContext>,
    app: Router,
    token: String,
}

impl Console {
    async fn new() -> Self {
        let resources = common::create_test_server_resources().await.unwrap();
        let mut context = AdminApiContext::new(AdminApiContextInit {
            database: resources.agent.database.clone(),
            repos: resources.common.repos.clone(),
            jwt_secret: resources.auth.admin_jwt_secret.to_string(),
            auth_manager: resources.auth.auth_manager.clone(),
            jwks_manager: resources.auth.jwks_manager.clone(),
            admin_api_key_monthly_limit: resources
                .common
                .config
                .rate_limiting
                .admin_provisioned_api_key_monthly_limit,
            harness_config_registry: resources.fitness.harness_config_registry.clone(),
            guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
            prompt_registry: resources.mcp.prompt_registry.clone(),
            tool_description_registry: resources.mcp.tool_description_registry.clone(),
            evidence_registry: resources.mcp.evidence_registry.clone(),
            messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
            cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
            persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
            training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
            contremaitre_config: resources.mcp.contremaitre_config.clone(),
            app_behavior: resources.common.config.app_behavior.clone(),
        });
        context.approval_notifier = Some(Arc::new(TakesEveryInvitation));
        context.frontend_url = Some("https://app.example.test".to_owned());

        let admin = seed_account(
            &resources,
            "golden-invites-admin",
            UserRole::Admin,
            instant("2026-08-01T09:30:00"),
            instant("2026-08-01T09:30:00"),
        )
        .await;
        let token = resources
            .common
            .repos
            .admin
            .create_token(
                &CreateAdminTokenRequest {
                    service_name: format!("console:{}", Uuid::new_v4()),
                    service_description: None,
                    permissions: None,
                    expires_in_days: Some(30),
                    is_super_admin: true,
                    tenant_id: None,
                    operator_user_id: Some(admin.user_id),
                },
                &resources.auth.admin_jwt_secret,
                &resources.auth.jwks_manager,
            )
            .await
            .unwrap()
            .jwt_token
            .expose_secret()
            .to_owned();
        Self {
            resources,
            app: AdminRoutes::routes(context),
            token,
        }
    }

    async fn post(&self, uri: &str, body: &Value) -> Value {
        let (status, answered) =
            send(self.app.clone(), with_json("POST", uri, &self.token, body)).await;
        assert!(status.is_success(), "{uri}: {status} {answered}");
        answered
    }

    /// [`Self::post`] of an invitation, with its lifetime pinned: the
    /// expiry at `/data/{field}` lies seven days after a reading of the clock
    /// taken before the request and seven days after one taken after it.
    async fn invite(&self, uri: &str, body: &Value, field: &str) -> Value {
        let before = Instant::now();
        let answered = self.post(uri, body).await;
        let after = Instant::now();
        let expires_at = answered["data"][field].as_str().unwrap();
        assert!(
            lies_days_after(expires_at, before, after, INVITATION_DAYS),
            "an invitation works for {INVITATION_DAYS} days: {answered}"
        );
        answered
    }

    async fn list(&self, uri: &str) -> Value {
        let (status, answered) = send(self.app.clone(), get(uri, &self.token)).await;
        assert!(status.is_success(), "{uri}: {status} {answered}");
        answered
    }

    async fn expire(&self, email: &str, at: &str) {
        restamp(
            &self.resources.agent.database,
            EXPIRE_INVITATION,
            instant(at),
            email,
            None,
        )
        .await;
    }
}

/// The minute a sentence states for `expires_at`: the instant's own date and
/// time of day, cut at the minute, with `UTC` after it.
fn minute_of(expires_at: &str) -> String {
    assert_eq!(
        clock_shape(expires_at),
        "dddd-dd-ddTdd:dd:dd.f+dd:dd",
        "{expires_at}"
    );
    assert!(expires_at.ends_with("+00:00"), "{expires_at}");
    format!("{} UTC", expires_at[..16].replace('T', " "))
}

#[tokio::test]
async fn a_coach_seat_invitation_says_until_which_minute_in_utc_it_works() {
    on_the_clock(|| async {
        let console = Console::new().await;

        let mailed = console
            .invite(
                "/admin/coach-seat-invitations",
                &json!({ "email": "mailed-coach@example.com" }),
                "expires_at",
            )
            .await;
        let expires_at = mailed["data"]["expires_at"].as_str().unwrap();
        assert_eq!(
            mailed["message"],
            format!(
                "Coach-seat invitation sent to mailed-coach@example.com: its link works once, \
             until {}",
                minute_of(expires_at)
            ),
            "{mailed}"
        );

        let linked = console
            .invite(
                "/admin/coach-seat-invitations",
                &json!({ "email": "linked-coach@example.com", "return_link": true }),
                "expires_at",
            )
            .await;
        let expires_at = linked["data"]["expires_at"].as_str().unwrap();
        assert_eq!(
            linked["message"],
            format!(
                "Coach-seat invitation link issued, not mailed: it works once, until {}",
                minute_of(expires_at)
            ),
            "{linked}"
        );

        // The list answers the stored row, oldest first: instants in UTC with
        // `Z`, at the fraction the row holds.
        console
            .expire("mailed-coach@example.com", "2026-09-04T02:59:03.500250")
            .await;
        console
            .expire("linked-coach@example.com", "2026-09-05T23:59:59")
            .await;
        let database = &console.resources.agent.database;
        for (email, at) in [
            ("mailed-coach@example.com", "2026-08-28T02:59:03.250125"),
            ("linked-coach@example.com", "2026-08-29T23:59:59"),
        ] {
            restamp(database, MOVE_INVITATION, instant(at), email, None).await;
        }
        let listed = console.list("/admin/coach-seat-invitations").await;
        assert_eq!(
            dates_in(&listed["data"]),
            table(&[
                ("/invitations/0/created_at", "2026-08-28T02:59:03.250125Z"),
                ("/invitations/0/expires_at", "2026-09-04T02:59:03.500250Z"),
                ("/invitations/1/created_at", "2026-08-29T23:59:59Z"),
                ("/invitations/1/expires_at", "2026-09-05T23:59:59Z"),
            ]),
            "{listed}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_pre_approval_that_invites_says_until_which_minute_in_utc_the_link_works() {
    on_the_clock(|| async {
        let console = Console::new().await;

        let mailed = console
            .invite(
                "/admin/pre-approved-emails",
                &json!({ "email": "mailed-athlete@example.com", "send_invite": true }),
                "invitation_expires_at",
            )
            .await;
        let expires_at = mailed["data"]["invitation_expires_at"].as_str().unwrap();
        let message = mailed["message"].as_str().unwrap();
        assert_eq!(
            message.split_once(" — invitation").map(|(_, said)| said),
            Some(
                format!(
                    " sent: its link works once, until {}",
                    minute_of(expires_at)
                )
                .as_str()
            ),
            "{mailed}"
        );

        let linked = console
            .invite(
                "/admin/pre-approved-emails",
                &json!({
                    "email": "linked-athlete@example.com",
                    "send_invite": true,
                    "return_link": true,
                }),
                "invitation_expires_at",
            )
            .await;
        let expires_at = linked["data"]["invitation_expires_at"].as_str().unwrap();
        let message = linked["message"].as_str().unwrap();
        assert_eq!(
            message.split_once(" — invitation").map(|(_, said)| said),
            Some(
                format!(
                    " link issued, not mailed: it works once, until {}",
                    minute_of(expires_at)
                )
                .as_str()
            ),
            "{linked}"
        );

        // This list names only an invitation that still works, so the row is
        // moved far ahead.
        console
            .expire("mailed-athlete@example.com", "2099-12-31T23:59:03.500250")
            .await;
        let listed = console.list("/admin/pre-approved-emails").await;
        let row = listed["data"]["emails"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["email"] == "mailed-athlete@example.com")
            .unwrap_or_else(|| panic!("the address is listed: {listed}"));
        assert_eq!(
            dates_in_stamped(row, &["/created_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.fZ"),
                ("/invitation_expires_at", "2099-12-31T23:59:03.500250Z"),
            ]),
            "{listed}"
        );
    })
    .await;
}
