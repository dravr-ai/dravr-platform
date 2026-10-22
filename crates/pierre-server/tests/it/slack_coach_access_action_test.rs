// ABOUTME: A super-admin decides a coach-access request from its Slack notice: Grant/Decline taps on the ops route
// ABOUTME: Pins the room per event, the super-admin gate through the channel link, decided_by and the conflict answer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1092.
//!
//! `coach_access.requested` posts Grant and Decline under its notice. A tap
//! is honoured only in the room that event is routed to, and only from a
//! Slack identity whose channel link names an active super-admin: the room's
//! membership approves sign-ups, but a grant gives `manages_roster`, which
//! only a super-admin decides. The decision goes through the console's own
//! implementation, so `decided_by` names the admin and
//! `coach_access.granted` / `.declined` fire as they do from the console.

#[cfg(feature = "client-messaging")]
mod slack_coach_access_action {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use chrono::Utc;
    use dravr_contremaitre::config::NOTIFY_ROUTING_YAML;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
    use pierre_core::models::{
        CoachAccessRequest, CoachAccessStatus, Tenant, TenantId, User, UserStatus,
    };
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
    use crate::helpers::notify_capture::{capture_notify, named, only, CapturedEvents};

    const SIGNING_SECRET: &str = "slack_coach_access_secret_1092";
    const ACTIONS_PATH: &str = "/api/ops/slack/actions";

    /// The room the pinned routing sends `coach_access.requested` to.
    ///
    /// The route reads the process-wide routing table the contremaitre sync
    /// fills in production, empty until then; this loads the pinned copy.
    fn coach_access_room() -> String {
        NOTIFY_ROUTING_PROVIDER
            .reload(NOTIFY_ROUTING_YAML)
            .expect("the pinned routing rules parse");
        NOTIFY_ROUTING_PROVIDER
            .event_channel("coach_access.requested")
            .expect("the pinned routing names the coach-access room")
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

    /// A pending request from a coach without coach access: its id and the
    /// requester's.
    async fn pending_request(resources: &ServerContext) -> (Uuid, Uuid) {
        let (requester, _) = account(
            resources,
            "asks-to-coach@example.com",
            UserRole::User,
            UserStatus::Active,
        )
        .await;
        let request = CoachAccessRequest::pending(requester, None, Utc::now());
        assert!(resources
            .common
            .repos
            .coach_access_requests
            .open(&request)
            .await
            .unwrap());
        (request.id, requester)
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
        assert_eq!(response.status_code(), StatusCode::OK);
        response.json()
    }

    fn grant(request_id: Uuid) -> String {
        format!("grant_coach_access:{request_id}")
    }

    fn decline(request_id: Uuid) -> String {
        format!("decline_coach_access:{request_id}")
    }

    async fn stored(resources: &ServerContext, request_id: Uuid) -> CoachAccessRequest {
        resources
            .common
            .repos
            .coach_access_requests
            .get(request_id)
            .await
            .unwrap()
            .expect("the request")
    }

    async fn has_coach_access(resources: &ServerContext, user_id: Uuid) -> bool {
        resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("the requester")
            .manages_roster
    }

    /// The refusal text the route answered with.
    fn refusal(answer: &Value) -> &str {
        answer["text"]
            .as_str()
            .unwrap_or_else(|| panic!("a refusal answers with text: {answer}"))
    }

    /// A refused tap leaves the request waiting and the requester without
    /// coach access, and announces no decision.
    async fn assert_still_pending(
        resources: &ServerContext,
        events: &CapturedEvents,
        request_id: Uuid,
        requester: Uuid,
    ) {
        let request = stored(resources, request_id).await;
        assert_eq!(request.status, CoachAccessStatus::Pending);
        assert_eq!(request.decided_by, None);
        assert!(!has_coach_access(resources, requester).await);
        assert!(named(events, "coach_access.granted").is_empty());
        assert!(named(events, "coach_access.declined").is_empty());
    }

    #[tokio::test]
    #[serial]
    async fn a_linked_super_admin_grants_from_the_coach_access_room() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        let admin = linked(
            &resources,
            "U0SUPERGRANT",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        let answer = tap(&resources, "U0SUPERGRANT", &room, &grant(request_id)).await;

        assert_eq!(answer["status"], "ok", "{answer}");
        let request = stored(&resources, request_id).await;
        assert_eq!(request.status, CoachAccessStatus::Granted);
        assert_eq!(
            request.decided_by,
            Some(admin),
            "the tapping admin decided it"
        );
        assert!(has_coach_access(&resources, requester).await);
        let granted = only(&events, "coach_access.granted");
        assert_eq!(granted.field("request_id"), request_id.to_string());
        assert_eq!(granted.field("user_id"), requester.to_string());
    }

    #[tokio::test]
    #[serial]
    async fn a_linked_super_admin_declines_from_the_coach_access_room() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        let admin = linked(
            &resources,
            "U0SUPERDECLINE",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        let answer = tap(&resources, "U0SUPERDECLINE", &room, &decline(request_id)).await;

        assert_eq!(answer["status"], "ok", "{answer}");
        let request = stored(&resources, request_id).await;
        assert_eq!(request.status, CoachAccessStatus::Declined);
        assert_eq!(request.decided_by, Some(admin));
        assert!(!has_coach_access(&resources, requester).await);
        let declined = only(&events, "coach_access.declined");
        assert_eq!(declined.field("request_id"), request_id.to_string());
        assert!(named(&events, "coach_access.granted").is_empty());
    }

    /// The room approves sign-ups without a Dravr role; coach access it does
    /// not. An operator in the room who is an admin, or no admin at all, is
    /// refused, and the refusal says so.
    #[tokio::test]
    #[serial]
    async fn a_linked_operator_who_is_not_a_super_admin_is_refused() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;

        for (slack_user, role) in [
            ("U0ROOMADMIN", UserRole::Admin),
            ("U0ROOMUSER", UserRole::User),
        ] {
            linked(&resources, slack_user, role, UserStatus::Active).await;

            let (events, _guard) = capture_notify();
            for action_id in [grant(request_id), decline(request_id)] {
                let answer = tap(&resources, slack_user, &room, &action_id).await;
                assert!(
                    refusal(&answer).contains("decided only by an active super-admin"),
                    "{role:?} {action_id}: {answer}"
                );
            }
            assert_still_pending(&resources, &events, request_id, requester).await;
        }
    }

    #[tokio::test]
    #[serial]
    async fn a_slack_identity_linked_to_no_account_is_refused() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        // A super-admin exists, but never linked this Slack identity.
        account(
            &resources,
            "unlinked-super@example.com",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        let answer = tap(&resources, "U0NOLINK", &room, &grant(request_id)).await;

        assert!(
            refusal(&answer).contains("not linked to a Dravr account"),
            "{answer}"
        );
        assert_still_pending(&resources, &events, request_id, requester).await;
    }

    #[tokio::test]
    #[serial]
    async fn a_suspended_super_admin_is_refused() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        linked(
            &resources,
            "U0SUSPENDEDSUPER",
            UserRole::SuperAdmin,
            UserStatus::Suspended,
        )
        .await;

        let (events, _guard) = capture_notify();
        let answer = tap(&resources, "U0SUSPENDEDSUPER", &room, &grant(request_id)).await;

        assert!(
            refusal(&answer).contains("Account has been suspended"),
            "{answer}"
        );
        assert_still_pending(&resources, &events, request_id, requester).await;
    }

    /// The buttons are posted to the coach-access room alone; a tap anywhere
    /// else is refused before the clicker is even looked up.
    #[tokio::test]
    #[serial]
    async fn a_super_admin_grant_outside_the_coach_access_room_is_refused() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        linked(
            &resources,
            "U0SUPERELSEWHERE",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        for elsewhere in ["C0ELSEWHERE", "D0SUPERDM"] {
            assert_ne!(elsewhere, room);
            let answer = tap(
                &resources,
                "U0SUPERELSEWHERE",
                elsewhere,
                &grant(request_id),
            )
            .await;
            assert!(refusal(&answer).contains("ops channel"), "{answer}");
        }
        assert_still_pending(&resources, &events, request_id, requester).await;
    }

    /// A card whose request is gone (its requester deleted, say) answers that
    /// nothing is left to decide, and decides nothing.
    #[tokio::test]
    #[serial]
    async fn a_tap_on_a_request_that_no_longer_exists_decides_nothing() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        linked(
            &resources,
            "U0SUPERGONE",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        let gone = Uuid::new_v4();
        let answer = tap(&resources, "U0SUPERGONE", &room, &grant(gone)).await;

        assert!(refusal(&answer).contains("not found"), "{answer}");
        assert!(refusal(&answer).contains(&gone.to_string()), "{answer}");
        // The request that does exist is untouched by a tap naming another.
        assert_still_pending(&resources, &events, request_id, requester).await;
    }

    /// Two super-admins tapping one card: the first decides, the second is
    /// told it was already decided, and the first decision stands.
    #[tokio::test]
    #[serial]
    async fn a_request_already_decided_answers_the_conflict_and_stays_decided() {
        let resources = signing_server().await;
        let room = coach_access_room();
        let (request_id, requester) = pending_request(&resources).await;
        let first = linked(
            &resources,
            "U0SUPERFIRST",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;
        linked(
            &resources,
            "U0SUPERSECOND",
            UserRole::SuperAdmin,
            UserStatus::Active,
        )
        .await;

        let (events, _guard) = capture_notify();
        let granted = tap(&resources, "U0SUPERFIRST", &room, &grant(request_id)).await;
        assert_eq!(granted["status"], "ok", "{granted}");

        let late = tap(&resources, "U0SUPERSECOND", &room, &decline(request_id)).await;
        assert!(refusal(&late).contains("already decided"), "{late}");

        let request = stored(&resources, request_id).await;
        assert_eq!(request.status, CoachAccessStatus::Granted);
        assert_eq!(request.decided_by, Some(first));
        assert!(has_coach_access(&resources, requester).await);
        assert_eq!(named(&events, "coach_access.granted").len(), 1);
        assert!(named(&events, "coach_access.declined").is_empty());
    }
}
