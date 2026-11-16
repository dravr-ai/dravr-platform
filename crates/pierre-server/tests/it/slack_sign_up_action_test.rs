// ABOUTME: The Approve and Reject buttons of a sign-up notice act on sign-ups and never on an admin account
// ABOUTME: Pins the refusal on an admin and on an account no longer pending, the sign-up approved and rejected, and the Slack row for credentials a rejection withdraws

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1574.
//!
//! The room is the whole authorization of a sign-up tap, so the tap names no
//! admin, and it authorizes sign-ups only: a card is about a sign-up, and an
//! admin is never one. A tap that names an admin account is refused on
//! either button and writes nothing. A rejection takes the suspension the
//! console and `pierre-cli` take, so an account that still holds what an
//! admin did loses it in the same write, on the trail under the Slack
//! source.
//!
//! carnet#1589: a sign-up is a pending account. A tap on a card whose
//! account was approved or suspended since changes nothing either, and one
//! that matches what the account already is decides it no second time.

#[cfg(feature = "client-messaging")]
mod slack_sign_up_action {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use chrono::Utc;
    use dravr_contremaitre::config::NOTIFY_ROUTING_YAML;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
    use pierre_core::admin::models::CreateAdminTokenRequest;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_core::permissions::impersonation::ImpersonationSession;
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::repositories::{AuditEventFilter, AuditEventRecord};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::admin_ops::ADMIN_CREDENTIALS_REVOKED;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use serial_test::serial;
    use sha2::Sha256;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources_with_config, test_server_config};
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::notify_capture::{capture_logs, named, CapturedEvents};

    const SIGNING_SECRET: &str = "slack_sign_up_secret_1574";
    const ACTIONS_PATH: &str = "/api/ops/slack/actions";
    const SIGN_UP_ROOM: &str = "C0SIGNUPROOM";

    /// Route `user.pending_approval` to [`SIGN_UP_ROOM`] for one test, and put
    /// the pinned table back when it ends: the room a tap is honoured in is
    /// the one that event's rule names.
    struct SignUpRoom;

    impl SignUpRoom {
        fn routed() -> Self {
            NOTIFY_ROUTING_PROVIDER
                .reload(&format!(
                    "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                     user.pending_approval:\n    channel: \"{SIGN_UP_ROOM}\"\n"
                ))
                .expect("the sign-up routing table parses");
            Self
        }
    }

    impl Drop for SignUpRoom {
        fn drop(&mut self) {
            NOTIFY_ROUTING_PROVIDER
                .reload(NOTIFY_ROUTING_YAML)
                .expect("the pinned routing rules parse");
        }
    }

    /// A server whose `SLACK_SIGNING_SECRET` is [`SIGNING_SECRET`], and
    /// which holds no bot token whatever the process exports: no test calls
    /// Slack, and a redraw stops at the warning [`CARD_DRAWN`] counts.
    async fn signing_server() -> Arc<ServerContext> {
        let mut config = test_server_config();
        config.webhooks.slack_signing_secret = Some(SecretString::from(SIGNING_SECRET));
        config.webhooks.slack_bot_token = None;
        create_test_server_resources_with_config(config)
            .await
            .unwrap()
    }

    /// An active account with `role`.
    async fn account(resources: &ServerContext, email: &str, role: UserRole) -> Uuid {
        account_in(resources, email, role, UserStatus::Active).await
    }

    /// An account with `role`, stored in `status`.
    async fn account_in(
        resources: &ServerContext,
        email: &str,
        role: UserRole,
        status: UserStatus,
    ) -> Uuid {
        let mut user = User::new(email.to_owned(), "not-a-login-path".to_owned(), None);
        user.role = role;
        user.user_status = status;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    /// The token a device login approved by `admin` mints: it acts as that
    /// admin, with every power. Its id.
    async fn device_login_of(resources: &ServerContext, admin: Uuid) -> String {
        let request = CreateAdminTokenRequest {
            service_name: format!("device-cli:{admin}"),
            service_description: None,
            permissions: None,
            expires_in_days: None,
            is_super_admin: true,
            tenant_id: None,
            operator_user_id: Some(admin),
        };
        resources
            .common
            .repos
            .admin
            .create_token(
                &request,
                resources.auth.admin_jwt_secret.as_ref(),
                &resources.auth.jwks_manager,
            )
            .await
            .unwrap()
            .token_id
    }

    /// Whether the token `token_id` is still live.
    async fn token_is_live(resources: &ServerContext, token_id: &str) -> bool {
        resources
            .common
            .repos
            .admin
            .list_tokens(false)
            .await
            .unwrap()
            .iter()
            .any(|token| token.id == token_id)
    }

    /// The account's status, as the next request will read it.
    async fn status_of(resources: &ServerContext, user_id: Uuid) -> UserStatus {
        resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("the account")
            .user_status
    }

    /// The rows the trail holds of admin credentials withdrawn.
    async fn revocations(resources: &ServerContext) -> Vec<AuditEventRecord> {
        let filter = AuditEventFilter {
            action_prefix: Some(ADMIN_CREDENTIALS_REVOKED.to_owned()),
            ..AuditEventFilter::default()
        };
        resources
            .common
            .repos
            .admin_audit
            .list(&filter, 50)
            .await
            .unwrap()
    }

    /// The Reject button of `user_id`'s sign-up notice, tapped in the sign-up
    /// room and signed as Slack signs it.
    async fn tap_reject(resources: &Arc<ServerContext>, user_id: Uuid) {
        tap(resources, "reject_user", user_id).await;
    }

    /// The Approve button of the same notice.
    async fn tap_approve(resources: &Arc<ServerContext>, user_id: Uuid) {
        tap(resources, "approve_user", user_id).await;
    }

    /// One button (`reject_user` or `approve_user`) of `user_id`'s sign-up
    /// notice, tapped in the sign-up room and signed as Slack signs it.
    async fn tap(resources: &Arc<ServerContext>, button: &str, user_id: Uuid) {
        let payload = json!({
            "type": "block_actions",
            "user": { "id": "U0REVIEWER", "username": "reviewer" },
            "channel": { "id": SIGN_UP_ROOM, "name": "" },
            "message": { "ts": "1727440000.000100", "blocks": [] },
            "actions": [{ "action_id": format!("{button}:{user_id}") }],
        })
        .to_string();
        let body = format!("payload={}", urlencoding::encode(&payload));

        let timestamp = Utc::now().timestamp().to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        let response = AxumTestRequest::post(ACTIONS_PATH)
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .text(&body)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let answer: Value = response.json();
        assert_eq!(
            answer["status"], "ok",
            "the tap reached the sign-up handler: {answer}"
        );
    }

    /// A sign-up card decides sign-ups, and an admin is never one. The
    /// Reject button on an admin's card suspends nothing: the admin stays
    /// active and keeps its device login and its impersonation, and no row
    /// reaches the trail. A second admin is active throughout, so the
    /// last-active-admin rule is never what answers.
    #[tokio::test]
    #[serial]
    async fn a_reject_tap_on_an_admin_is_refused_and_writes_nothing() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        let standing = account(&resources, "standing@example.com", UserRole::Admin).await;
        account(&resources, "staying@example.com", UserRole::Admin).await;
        let athlete = account(&resources, "athlete@example.com", UserRole::User).await;
        let token_id = device_login_of(&resources, standing).await;
        let session = ImpersonationSession::new(standing, athlete, None);
        let impersonation = &resources.common.repos.impersonation;
        impersonation.create_session(&session).await.unwrap();

        tap_reject(&resources, standing).await;

        assert_eq!(
            status_of(&resources, standing).await,
            UserStatus::Active,
            "the room suspends no admin"
        );
        assert!(
            token_is_live(&resources, &token_id).await,
            "the admin keeps the device login it approved"
        );
        let kept = impersonation.get_session(&session.id).await.unwrap();
        assert!(
            kept.expect("the session row").is_active,
            "and the impersonation it opened"
        );
        assert!(revocations(&resources).await.is_empty());
    }

    /// The Approve button on a suspended admin's card reinstates no one: the
    /// admin stays suspended. Another admin is the one in good standing.
    #[tokio::test]
    #[serial]
    async fn an_approve_tap_on_a_suspended_admin_is_refused_and_writes_nothing() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        account(&resources, "staying@example.com", UserRole::Admin).await;
        let suspended = account_in(
            &resources,
            "suspended@example.com",
            UserRole::Admin,
            UserStatus::Suspended,
        )
        .await;
        let pending = account_in(
            &resources,
            "pending-admin@example.com",
            UserRole::Admin,
            UserStatus::Pending,
        )
        .await;

        tap_approve(&resources, suspended).await;
        tap_approve(&resources, pending).await;

        assert_eq!(
            status_of(&resources, suspended).await,
            UserStatus::Suspended,
            "the room reinstates no admin"
        );
        assert_eq!(
            status_of(&resources, pending).await,
            UserStatus::Pending,
            "nor activates an account that already holds the role"
        );
    }

    /// The positive control: the same buttons, the same room, plain
    /// sign-ups. One pending account is approved and another rejected.
    #[tokio::test]
    #[serial]
    async fn a_pending_sign_up_is_still_approved_and_rejected() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        let welcomed = account_in(
            &resources,
            "welcomed@example.com",
            UserRole::User,
            UserStatus::Pending,
        )
        .await;
        let turned_away = account_in(
            &resources,
            "turned-away@example.com",
            UserRole::User,
            UserStatus::Pending,
        )
        .await;

        tap_approve(&resources, welcomed).await;
        tap_reject(&resources, turned_away).await;

        assert_eq!(status_of(&resources, welcomed).await, UserStatus::Active);
        assert_eq!(
            status_of(&resources, turned_away).await,
            UserStatus::Suspended
        );
        assert!(
            revocations(&resources).await.is_empty(),
            "a sign-up held nothing as an admin"
        );
    }

    /// A card decides a pending sign-up and nothing else (carnet#1589).
    /// The Reject button of a card whose account was approved since
    /// suspends no one: the account stays active and keeps what it holds,
    /// and nothing reaches the trail. The Approve button of a card whose
    /// account was suspended since reinstates no one. Neither account is an
    /// admin, so the admin refusal is not what answers.
    #[tokio::test]
    #[serial]
    async fn a_tap_on_an_account_that_is_no_longer_pending_changes_nothing() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        account(&resources, "staying@example.com", UserRole::Admin).await;
        let approved = account(&resources, "approved-since@example.com", UserRole::User).await;
        let suspended = account_in(
            &resources,
            "suspended-since@example.com",
            UserRole::User,
            UserStatus::Suspended,
        )
        .await;
        // What a suspension would withdraw, so a suspension would show.
        let token_id = device_login_of(&resources, approved).await;

        tap_reject(&resources, approved).await;
        tap_approve(&resources, suspended).await;

        assert_eq!(
            status_of(&resources, approved).await,
            UserStatus::Active,
            "a stale Reject suspends no active account"
        );
        assert!(token_is_live(&resources, &token_id).await);
        assert!(revocations(&resources).await.is_empty());
        assert_eq!(
            status_of(&resources, suspended).await,
            UserStatus::Suspended,
            "a stale Approve reinstates no suspended account"
        );
    }

    /// The line a redraw of a card logs in a test. No test server holds a
    /// bot token, so `update_card` stops at its first line, this warning:
    /// one per card a tap draws, and the only trace a draw leaves here.
    const CARD_DRAWN: &str = "SLACK_BOT_TOKEN not configured; the Slack card was not updated";

    /// The line a tap logs when it leaves the card as the deciding tap drew
    /// it.
    const CARD_LEFT: &str =
        "Sign-up tap found the account already decided that way; the card is left as it is";

    /// The line a tap logs when its write found the sign-up decided by
    /// another tap first: it draws nothing either.
    const CARD_LOST: &str =
        "Sign-up tap lost at the write: the sign-up was decided first; the card is left as it is";

    /// How many times the notify event `event` was raised about `user_id`.
    fn raised(events: &CapturedEvents, event: &str, user_id: Uuid) -> usize {
        let user_id = user_id.to_string();
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.fields.get("event").is_some_and(|name| name == event))
            .filter(|line| line.fields.get("user_id") == Some(&user_id))
            .count()
    }

    /// A tap that matches the account's standing is the same decision
    /// arriving twice (carnet#1589), one tap after the other or both at
    /// once. The sign-up is decided once: one approval is announced and
    /// provisions one tenant, a rejection none, and nothing reaches the
    /// trail. The second tap draws nothing, so the card keeps the name of
    /// whoever decided: each sign-up's card is drawn once and left once.
    ///
    /// Two Approves at once both read a pending account, so only the write
    /// tells them apart. Unguarded, both went on to provision the tenant,
    /// the second failed on the tenant's slug and drew its failure over the
    /// card: a fourth card, which is what this counts. A second tap that
    /// was told "no longer pending" drew one too.
    #[tokio::test]
    #[serial]
    async fn the_same_button_tapped_twice_decides_a_sign_up_once() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        let tenants = &resources.common.repos.tenants;
        let mut sign_ups = Vec::new();
        for name in ["welcomed", "raced", "turned-away"] {
            let email = format!("{name}@example.com");
            sign_ups
                .push(account_in(&resources, &email, UserRole::User, UserStatus::Pending).await);
        }
        let [welcomed, raced, turned_away] = sign_ups[..] else {
            panic!("three sign-ups");
        };

        let (lines, _guard) = capture_logs();
        tap_approve(&resources, welcomed).await;
        tap_approve(&resources, welcomed).await;
        tokio::join!(
            tap_approve(&resources, raced),
            tap_approve(&resources, raced)
        );
        tap_reject(&resources, turned_away).await;
        tap_reject(&resources, turned_away).await;

        for approved in [welcomed, raced] {
            assert_eq!(status_of(&resources, approved).await, UserStatus::Active);
            assert_eq!(
                raised(&lines, "user.approved", approved),
                1,
                "one approval is announced once"
            );
            assert_eq!(
                tenants.list_for_user(approved).await.unwrap().len(),
                1,
                "one approval provisions one tenant"
            );
        }
        assert_eq!(
            status_of(&resources, turned_away).await,
            UserStatus::Suspended
        );
        assert_eq!(raised(&lines, "user.suspended", turned_away), 1);
        assert!(tenants.list_for_user(turned_away).await.unwrap().is_empty());
        assert!(revocations(&resources).await.is_empty());
        assert_eq!(
            named(&lines, CARD_DRAWN).len(),
            3,
            "the tap that decides each sign-up draws its card"
        );
        // The second of two Approves at once either read the account
        // already active or lost at the write: it draws nothing both ways.
        assert_eq!(
            named(&lines, CARD_LEFT).len() + named(&lines, CARD_LOST).len(),
            3,
            "and the tap that repeats it draws nothing"
        );
    }

    /// How many sign-ups the two buttons are raced on: the order two taps
    /// reach the store in is the runtime's, so one round proves little.
    const TAP_RACE_ROUNDS: usize = 8;

    /// Two reviewers tap the two buttons of one card at once (carnet#1589).
    /// Both taps read a pending account, so the read cannot be what decides:
    /// the write is, and only one of the two finds the account still
    /// pending. The sign-up ends as that one tap made it. Checked and then
    /// written, the rejection suspended the account the approval had just
    /// made active, and the approval went on to provision its tenant: a
    /// suspended account with the tenant of an approved one.
    ///
    /// The tap that loses at the write draws nothing. It used to read the
    /// account again and draw "no longer pending" over the card, which could
    /// land after the winner's card and replace who decided: two cards a
    /// round, every round. A tap that reads the account already decided,
    /// before it writes anything, still draws that refusal, so every tap
    /// either draws or loses at the write, and over the rounds at least one
    /// loses there.
    #[tokio::test]
    #[serial]
    async fn two_taps_at_once_decide_a_sign_up_once() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        let tenants = &resources.common.repos.tenants;

        let (lines, _guard) = capture_logs();
        for round in 0..TAP_RACE_ROUNDS {
            let contested = account_in(
                &resources,
                &format!("contested-{round}@example.com"),
                UserRole::User,
                UserStatus::Pending,
            )
            .await;

            tokio::join!(
                tap_approve(&resources, contested),
                tap_reject(&resources, contested)
            );

            let provisioned = !tenants.list_for_user(contested).await.unwrap().is_empty();
            match status_of(&resources, contested).await {
                UserStatus::Active => assert!(
                    provisioned,
                    "round {round}: the approval that landed provisions the tenant"
                ),
                UserStatus::Suspended => assert!(
                    !provisioned,
                    "round {round}: a rejected sign-up was never approved, so it has no tenant"
                ),
                UserStatus::Pending => panic!("round {round}: one of the two taps decides"),
            }
        }

        let (drawn, lost) = (
            named(&lines, CARD_DRAWN).len(),
            named(&lines, CARD_LOST).len(),
        );
        assert_eq!(
            drawn + lost,
            2 * TAP_RACE_ROUNDS,
            "every tap draws its card or loses at the write: {drawn} drawn, {lost} lost"
        );
        assert!(
            drawn < 2 * TAP_RACE_ROUNDS,
            "a tap that loses at the write draws nothing, and over {TAP_RACE_ROUNDS} rounds \
             at least one does: {drawn} cards drawn"
        );
    }

    /// The line a tap logs when its approval was put back: the sign-up is
    /// pending again and the card is not replaced.
    const APPROVAL_PUT_BACK: &str = "Sign-up approval was put back: the sign-up is pending again and the card keeps its buttons";

    /// A tenant owned by `owner` holding the slug an approval gives
    /// `sign_up`'s own tenant, so that tenant cannot be provisioned. Its id.
    async fn take_the_slug_of(resources: &ServerContext, sign_up: Uuid, owner: Uuid) -> TenantId {
        let tenant_id = TenantId::generate();
        resources
            .common
            .repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: "in the way".to_owned(),
                slug: format!("user-{}", sign_up.as_simple()),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: owner,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        tenant_id
    }

    /// Remove the tenant `tenant_id` and its memberships, past the
    /// repository, which has no delete.
    async fn remove_tenant(resources: &ServerContext, tenant_id: TenantId) {
        for statement in [
            "DELETE FROM tenant_users WHERE CAST(tenant_id AS TEXT) = $1",
            "DELETE FROM tenants WHERE CAST(id AS TEXT) = $1",
        ] {
            match resources.agent.database.backend() {
                DatabaseBackend::SQLite(backend) => {
                    sqlx::query(statement)
                        .bind(tenant_id.to_string())
                        .execute(backend.pool())
                        .await
                        .unwrap();
                }
                #[cfg(feature = "postgresql")]
                DatabaseBackend::PostgreSQL(backend) => {
                    sqlx::query(statement)
                        .bind(tenant_id.to_string())
                        .execute(backend.pool())
                        .await
                        .unwrap();
                }
            }
        }
    }

    /// An Approve lands and then the account's own tenant cannot be made
    /// (carnet#1589): here its slug is taken. The tap used to leave the
    /// account active with no tenant, nobody told, and a card whose buttons
    /// were gone, so neither the room nor the console could finish. The
    /// approval is put back: the sign-up is pending again, nothing is
    /// announced and the card is not replaced. Once the tenant can be made,
    /// the next Approve on the same card finishes: one tenant, one
    /// announcement, one card.
    #[tokio::test]
    #[serial]
    async fn an_approval_that_cannot_make_the_tenant_is_put_back_and_the_next_tap_finishes() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        let tenants = &resources.common.repos.tenants;
        let squatter = account(&resources, "squatter@example.com", UserRole::User).await;
        let sign_up = account_in(
            &resources,
            "held-up@example.com",
            UserRole::User,
            UserStatus::Pending,
        )
        .await;
        let in_the_way = take_the_slug_of(&resources, sign_up, squatter).await;

        let (lines, _guard) = capture_logs();
        tap_approve(&resources, sign_up).await;

        assert_eq!(
            status_of(&resources, sign_up).await,
            UserStatus::Pending,
            "an approval that could not make the tenant leaves the sign-up pending"
        );
        assert!(tenants.list_for_user(sign_up).await.unwrap().is_empty());
        assert_eq!(
            raised(&lines, "user.approved", sign_up),
            0,
            "an approval that was put back is not announced"
        );
        assert_eq!(named(&lines, APPROVAL_PUT_BACK).len(), 1);
        assert_eq!(
            named(&lines, CARD_DRAWN).len(),
            0,
            "the card is not replaced: its buttons are what the next tap needs"
        );

        remove_tenant(&resources, in_the_way).await;
        tap_approve(&resources, sign_up).await;

        assert_eq!(status_of(&resources, sign_up).await, UserStatus::Active);
        assert_eq!(tenants.list_for_user(sign_up).await.unwrap().len(), 1);
        assert_eq!(raised(&lines, "user.approved", sign_up), 1);
        assert_eq!(
            named(&lines, CARD_DRAWN).len(),
            1,
            "the tap that finishes draws the outcome"
        );
        assert_eq!(named(&lines, APPROVAL_PUT_BACK).len(), 1);
    }

    /// The withdrawal a rejection carries is still reachable without an
    /// admin target. Before the role's writes withdrew what an admin held, a
    /// demotion left the device logins it had approved and the
    /// impersonations it had opened live in their tables, under an account
    /// that is a plain user now. Such an account waiting as a pending
    /// sign-up (a card acts on no other) is suspended by a rejection
    /// through the one suspension path, which ends both, and the trail says
    /// Slack.
    #[tokio::test]
    #[serial]
    async fn a_rejected_account_loses_what_an_older_demotion_left_it_and_the_trail_says_slack() {
        let _room = SignUpRoom::routed();
        let resources = signing_server().await;
        account(&resources, "staying@example.com", UserRole::Admin).await;
        let former = account_in(
            &resources,
            "former-admin@example.com",
            UserRole::User,
            UserStatus::Pending,
        )
        .await;
        let athlete = account(&resources, "athlete@example.com", UserRole::User).await;
        // Written as that build left them: rows naming an account that
        // holds no admin role.
        let token_id = device_login_of(&resources, former).await;
        let session = ImpersonationSession::new(former, athlete, None);
        let impersonation = &resources.common.repos.impersonation;
        impersonation.create_session(&session).await.unwrap();

        tap_reject(&resources, former).await;

        assert_eq!(status_of(&resources, former).await, UserStatus::Suspended);
        assert!(
            !token_is_live(&resources, &token_id).await,
            "the device login left under the account is deactivated"
        );
        let ended = impersonation.get_session(&session.id).await.unwrap();
        assert!(
            !ended.expect("the session row").is_active,
            "the impersonation left open under it is ended"
        );

        let trail = revocations(&resources).await;
        assert_eq!(trail.len(), 1, "{trail:?}");
        assert_eq!(trail[0].source, "slack");
        assert_eq!(
            trail[0].actor_user_id, None,
            "the room names no admin account"
        );
        assert_eq!(trail[0].metadata["cause"], "deactivated");
        assert_eq!(trail[0].metadata["device_tokens_revoked"], 1);
        assert_eq!(trail[0].metadata["impersonations_ended"], 1);
        assert_eq!(trail[0].metadata["device_token_ids"], json!([token_id]));
    }
}
