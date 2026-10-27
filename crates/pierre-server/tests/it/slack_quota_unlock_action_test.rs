// ABOUTME: A super-admin raises a capped athlete to Enterprise from the quota.limit_reached Slack card
// ABOUTME: Pins the super-admin gate through the channel link, set_by on the tier override, and that a refusal writes no tier

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1192.
//!
//! `quota.limit_reached` posts Raise to Enterprise under its notice. A tap is
//! honoured only in the room that event is routed to, and only from a Slack
//! identity whose channel link names an active super-admin: Enterprise lifts
//! every tier cap, and a tier change is a super-admin's call on every other
//! surface. The raise goes through the admin tier override with that admin
//! as `set_by`.

#[cfg(feature = "client-messaging")]
mod slack_quota_unlock_action {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use chrono::Utc;
    use dravr_contremaitre::config::NOTIFY_ROUTING_YAML;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use serial_test::serial;
    use sha2::Sha256;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources_with_config, test_server_config};
    use crate::helpers::axum_test::AxumTestRequest;

    const SIGNING_SECRET: &str = "slack_quota_unlock_secret_1192";
    const ACTIONS_PATH: &str = "/api/ops/slack/actions";
    const QUOTA_ROOM: &str = "C0QUOTAROOM";

    /// Route `quota.limit_reached` to [`QUOTA_ROOM`] for one test, and put the
    /// pinned table back when it ends.
    ///
    /// These tests pin the clicker's gate, not which room the shipped routing
    /// names, so they load a table of their own into the process-wide
    /// provider the route reads.
    struct QuotaRoom;

    impl QuotaRoom {
        fn routed() -> Self {
            NOTIFY_ROUTING_PROVIDER
                .reload(&format!(
                    "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                     quota.limit_reached:\n    channel: \"{QUOTA_ROOM}\"\n"
                ))
                .expect("the quota routing table parses");
            Self
        }
    }

    impl Drop for QuotaRoom {
        fn drop(&mut self) {
            NOTIFY_ROUTING_PROVIDER
                .reload(NOTIFY_ROUTING_YAML)
                .expect("the pinned routing rules parse");
        }
    }

    /// A server whose `SLACK_SIGNING_SECRET` is [`SIGNING_SECRET`].
    async fn signing_server() -> Arc<ServerContext> {
        let mut config = test_server_config();
        config.webhooks.slack_signing_secret = Some(SecretString::from(SIGNING_SECRET));
        create_test_server_resources_with_config(config)
            .await
            .unwrap()
    }

    /// An account with `role` and `status`, owning a tenant.
    async fn account(
        resources: &ServerContext,
        email: &str,
        role: UserRole,
        status: UserStatus,
    ) -> (Uuid, TenantId) {
        let mut user = User::new(email.to_owned(), "not-a-login-path".to_owned(), None);
        user.is_admin = role.is_admin_or_higher();
        user.role = role;
        user.user_status = status;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        resources
            .common
            .repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("Tenant for {email}"),
                slug: format!("tenant-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        (user_id, tenant_id)
    }

    /// Link `slack_user` to the account — the row the messaging link flow
    /// writes, and the only proof the clicker owns it.
    async fn link_slack(
        resources: &ServerContext,
        (user_id, tenant_id): (Uuid, TenantId),
        slack_user: &str,
    ) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_user,
            display_name: Some("Decider"),
        })
        .await
        .unwrap();
    }

    /// A linked account with `role` and `status`, tapping as `slack_user`.
    async fn linked(
        resources: &ServerContext,
        slack_user: &str,
        role: UserRole,
        status: UserStatus,
    ) -> Uuid {
        let owner = account(
            resources,
            &format!("{}@example.com", slack_user.to_lowercase()),
            role,
            status,
        )
        .await;
        link_slack(resources, owner, slack_user).await;
        owner.0
    }

    /// A Starter athlete who hit a cap.
    async fn capped_athlete(resources: &ServerContext) -> Uuid {
        account(
            resources,
            "capped-athlete@example.com",
            UserRole::User,
            UserStatus::Active,
        )
        .await
        .0
    }

    /// `slack_user` tapping `action_id` on a card in `channel_id`, signed as
    /// Slack signs it; the route's answer.
    async fn tap(
        resources: &Arc<ServerContext>,
        slack_user: &str,
        channel_id: &str,
        action_id: &str,
    ) -> Value {
        let payload = json!({
            "type": "block_actions",
            "user": { "id": slack_user, "username": "decider" },
            "channel": { "id": channel_id, "name": "" },
            "message": { "ts": "1727440000.000100", "blocks": [] },
            "actions": [{ "action_id": action_id }],
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
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "the route answers every signed tap with 200, a refusal included"
        );
        response.json()
    }

    fn raise(user_id: Uuid) -> String {
        format!("raise_tier_enterprise:{user_id}")
    }

    /// The refusal text the route answered with.
    fn refusal(answer: &Value) -> &str {
        answer["text"]
            .as_str()
            .unwrap_or_else(|| panic!("a refusal answers with text: {answer}"))
    }

    /// A refused tap leaves the athlete on their tier with no override row.
    async fn assert_not_raised(resources: &ServerContext, athlete: Uuid) {
        let user = resources
            .common
            .repos
            .users
            .get_global(athlete)
            .await
            .unwrap()
            .expect("the athlete");
        assert_eq!(user.tier, UserTier::Starter, "the tier is unchanged");
        assert!(
            resources
                .common
                .repos
                .user_tier_overrides
                .get(athlete)
                .await
                .unwrap()
                .is_none(),
            "no tier override was written"
        );
    }

    #[tokio::test]
    #[serial]
    async fn a_linked_super_admin_raises_the_athlete_from_the_quota_room() {
        let resources = signing_server().await;
        let _room = QuotaRoom::routed();
        let athlete = capped_athlete(&resources).await;
        let admin = linked(
            &resources,
            "U0SUPERRAISE",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let answer = tap(&resources, "U0SUPERRAISE", QUOTA_ROOM, &raise(athlete)).await;

        assert_eq!(answer["status"], "ok", "{answer}");
        let user = resources
            .common
            .repos
            .users
            .get_global(athlete)
            .await
            .unwrap()
            .expect("the athlete");
        assert_eq!(user.tier, UserTier::Enterprise, "the athlete was raised");
        let marker = resources
            .common
            .repos
            .user_tier_overrides
            .get(athlete)
            .await
            .unwrap()
            .expect("the raise records an admin override");
        assert_eq!(marker.set_by, Some(admin), "the tapping admin is on record");
    }

    /// The quota room is where operators read the card, not who may raise a
    /// tier: an admin or a plain account tapping from it is refused.
    #[tokio::test]
    #[serial]
    async fn a_linked_operator_who_is_not_a_super_admin_is_refused_and_writes_no_tier() {
        let resources = signing_server().await;
        let _room = QuotaRoom::routed();
        let athlete = capped_athlete(&resources).await;

        for (slack_user, role) in [
            ("U0QUOTAADMIN", UserRole::Admin),
            ("U0QUOTAUSER", UserRole::User),
        ] {
            linked(&resources, slack_user, role, UserStatus::Active).await;

            let answer = tap(&resources, slack_user, QUOTA_ROOM, &raise(athlete)).await;

            assert!(
                refusal(&answer).contains("raised only by an active super-admin"),
                "{role:?}: {answer}"
            );
            assert_not_raised(&resources, athlete).await;
        }
    }

    #[tokio::test]
    #[serial]
    async fn a_slack_identity_linked_to_no_account_is_refused_and_writes_no_tier() {
        let resources = signing_server().await;
        let _room = QuotaRoom::routed();
        let athlete = capped_athlete(&resources).await;

        let answer = tap(&resources, "U0QUOTANOLINK", QUOTA_ROOM, &raise(athlete)).await;

        assert!(
            refusal(&answer).contains("not linked to a Dravr account"),
            "{answer}"
        );
        assert_not_raised(&resources, athlete).await;
    }

    #[tokio::test]
    #[serial]
    async fn a_suspended_super_admin_is_refused_and_writes_no_tier() {
        let resources = signing_server().await;
        let _room = QuotaRoom::routed();
        let athlete = capped_athlete(&resources).await;
        linked(
            &resources,
            "U0QUOTASUSPENDED",
            UserRole::SuperAdmin,
            UserStatus::Suspended,
        )
        .await;

        let answer = tap(&resources, "U0QUOTASUSPENDED", QUOTA_ROOM, &raise(athlete)).await;

        assert!(
            refusal(&answer).contains("Account has been suspended"),
            "a super-admin role is not enough without an active account: {answer}"
        );
        assert_not_raised(&resources, athlete).await;
    }

    #[tokio::test]
    #[serial]
    async fn a_super_admin_raise_outside_the_quota_room_is_refused() {
        let resources = signing_server().await;
        let _room = QuotaRoom::routed();
        let athlete = capped_athlete(&resources).await;
        linked(
            &resources,
            "U0SUPERQUOTAELSEWHERE",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let answer = tap(
            &resources,
            "U0SUPERQUOTAELSEWHERE",
            "C0DEFAULT",
            &raise(athlete),
        )
        .await;

        assert!(refusal(&answer).contains("ops channel"), "{answer}");
        assert_not_raised(&resources, athlete).await;
    }
}
