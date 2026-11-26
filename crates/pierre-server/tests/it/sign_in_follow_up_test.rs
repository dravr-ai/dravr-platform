// ABOUTME: The follow-up a refused provider sign-in queues, delivered the way Cloud Tasks delivers it ten minutes later
// ABOUTME: Drives the real queue call and internal route: superseded, still not connected, and recovered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A refusal is an event; what ops acts on is an athlete still not connected
//! (carnet#1837). Each refusal records itself as the athlete's last one and
//! queues a follow-up; the delivery acts only for the latest refusal, and
//! says whether any backend serving the provider now holds a live connection.
//! This suite seeds the record and the connection rows directly, queues the
//! follow-up through the auth routes' own queue against a stub Cloud Tasks,
//! and delivers it through the messaging router with Google stood in for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(all(
    feature = "client-messaging",
    feature = "protocol-rest",
    feature = "provider-sciotte"
))]
mod delivered {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::Router;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_cache::{CacheKey, CacheResource};
    use pierre_core::constants::oauth_providers::{self, TOKEN_TYPE_SESSION};
    use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_routes_auth::sign_in_follow_up::{FollowUp, FOLLOW_UP_PATH};
    use pierre_services::channel_adapters::ChannelAdapterFactory;
    use pierre_test_support::log_capture::{self, CapturedEvent};
    use serde_json::{json, Value};
    use tracing::Level;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources_with_chat_provider_and_runner, create_test_user_with_plan,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::cloud_tasks_stub::{
        cloud_tasks_turn_runner, QueueStub, QUEUE, SERVICE_ACCOUNT, TARGET,
    };
    use crate::helpers::drained_turn::HangingProvider;
    use crate::helpers::google_token::{GoogleClaims, TestSigner};
    use crate::helpers::offline_channel::OfflineSendAdapters;
    use crate::helpers::sciotte_mock::seed_signed_in_session;

    /// When the refusal the follow-up is for happened.
    const REFUSED_AT_MS: i64 = 1_764_000_000_000;

    /// The target both follow-up lines are pinned to, which the ops alert
    /// filters on.
    const TARGET_OF_LINES: &str = "pierre_routes_auth::sign_in_follow_up";

    /// How long the seeded record and window live.
    const WINDOW: Duration = Duration::from_mins(30);

    /// One instance on the Cloud Tasks runner and one athlete.
    struct Instance {
        resources: Arc<ServerContext>,
        router: Router,
        queue: Arc<QueueStub>,
        signer: Arc<TestSigner>,
        user_id: Uuid,
        tenant: TenantId,
    }

    impl Instance {
        async fn boot(label: &str) -> Self {
            let signer = Arc::new(TestSigner::generate());
            let jwks = signer.serve_jwks().await;
            let queue = QueueStub::accepting();
            let runner =
                cloud_tasks_turn_runner(&queue.serve().await, &jwks.url, Duration::from_secs(1));
            let resources = create_test_server_resources_with_chat_provider_and_runner(
                Arc::new(HangingProvider),
                runner,
            )
            .await
            .unwrap();
            let email = format!("{label}-{}@follow-up.example.com", Uuid::new_v4());
            let (user_id, _user, tenant) =
                create_test_user_with_plan(&resources.agent.database, &email, "starter")
                    .await
                    .unwrap();
            let adapters: Arc<dyn ChannelAdapterFactory> = Arc::new(OfflineSendAdapters::default());
            let router = MessagingRoutes::routes_with_adapters(Arc::clone(&resources), adapters);
            Self {
                resources,
                router,
                queue,
                signer,
                user_id,
                tenant,
            }
        }

        /// The follow-up queued for the refusal at `refused_at_ms`.
        fn follow_up(&self, refused_at_ms: i64) -> FollowUp {
            FollowUp {
                tenant_id: self.tenant.as_uuid(),
                user_id: self.user_id,
                backend: oauth_providers::SCIOTTE_GARMIN.to_owned(),
                refused_at_ms,
            }
        }

        /// The athlete's Garmin entry of `resource`, keyed under the
        /// user-facing provider as the refusal writes it.
        fn key(&self, resource: CacheResource) -> CacheKey {
            CacheKey::new(self.tenant, self.user_id, "garmin".to_owned(), resource)
        }

        /// Record the athlete's last refusal as a refusal at `refused_at_ms`
        /// leaves it, and its window's count.
        async fn record_refusal(&self, refused_at_ms: i64, refusals: u64) {
            let cache = &self.resources.common.cache;
            let last = json!({
                "refused_at_ms": refused_at_ms,
                "reason": "Invalid password",
                "method_used": "email",
                "account": "j***@e***.com",
                "refusals": refusals,
            });
            cache
                .set(&self.key(CacheResource::SignInLastRefusal), &last, WINDOW)
                .await
                .unwrap();
            for _ in 0..refusals {
                cache
                    .count_in_window(&self.key(CacheResource::SignInRefusals), WINDOW)
                    .await
                    .unwrap();
            }
        }

        /// Whether the record and the window are both gone.
        async fn window_cleared(&self) -> bool {
            let cache = &self.resources.common.cache;
            let last: Option<Value> = cache
                .get(&self.key(CacheResource::SignInLastRefusal))
                .await
                .unwrap();
            let count: Option<u64> = cache
                .get(&self.key(CacheResource::SignInRefusals))
                .await
                .unwrap();
            last.is_none() && count.is_none()
        }

        /// Deliver `body` to the follow-up route as Cloud Tasks would, with a
        /// token minted for this service when `signed`.
        async fn deliver_body(&self, body: &Value, signed: bool) -> (StatusCode, Value) {
            let mut request = AxumTestRequest::post(FOLLOW_UP_PATH)
                .header("content-type", "application/json")
                .json(body);
            if signed {
                let token = self
                    .signer
                    .mint(&GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT));
                request = request.header("authorization", &format!("Bearer {token}"));
            }
            let response = request.send(self.router.clone()).await;
            let status = response.status_code();
            let answer = if status == StatusCode::OK {
                response.json()
            } else {
                Value::Null
            };
            (status, answer)
        }

        async fn deliver(&self, follow_up: &FollowUp, signed: bool) -> (StatusCode, Value) {
            self.deliver_body(&serde_json::to_value(follow_up).unwrap(), signed)
                .await
        }

        /// The username-and-password Garmin session and its connection, as
        /// a sign-in that went through writes them.
        async fn connect_session(&self) {
            seed_signed_in_session(
                &self.resources,
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE_GARMIN,
                "garmin-web-session",
            )
            .await;
            self.register_session_connection().await;
        }

        async fn register_session_connection(&self) {
            self.resources
                .common
                .repos
                .provider_connections
                .register_connection(
                    self.user_id,
                    self.tenant,
                    oauth_providers::SCIOTTE_GARMIN,
                    &ConnectionType::Manual,
                    None,
                )
                .await
                .unwrap();
        }
    }

    /// The follow-up lines among `events`.
    fn follow_up_lines(events: Vec<CapturedEvent>) -> Vec<CapturedEvent> {
        events
            .into_iter()
            .filter(|event| event.target == TARGET_OF_LINES)
            .collect()
    }

    #[tokio::test]
    async fn a_refusal_queues_its_follow_up_on_the_turns_queue_for_ten_minutes_later() {
        let instance = Instance::boot("queued").await;
        instance.record_refusal(REFUSED_AT_MS, 1).await;
        let auth_queue = instance
            .resources
            .auth_routes_context()
            .sign_in_follow_ups
            .expect("the composition root wires the follow-up queue");
        let follow_up = instance.follow_up(REFUSED_AT_MS);

        auth_queue.queue(follow_up.clone()).await.unwrap();

        let received = instance.queue.received();
        assert_eq!(received.len(), 1, "one task per refusal");
        let task = &received[0];
        assert_eq!(
            task.queue_path, QUEUE,
            "the turns queue, no queue of its own"
        );
        assert_eq!(task.url(), format!("{TARGET}{FOLLOW_UP_PATH}"));
        assert_eq!(
            task.name(),
            format!("{QUEUE}/tasks/{}", follow_up.task_id()),
            "named for its refusal, so a later refusal queues its own"
        );
        let due = Timestamp::from_epoch_millis(REFUSED_AT_MS + 600_000).unwrap();
        assert_eq!(
            task.body["task"]["scheduleTime"],
            Value::String(due.to_rfc3339()),
            "due ten minutes after the refusal"
        );
        assert_eq!(
            task.body["task"]["httpRequest"]["oidcToken"]["serviceAccountEmail"],
            SERVICE_ACCOUNT
        );

        // What Cloud Tasks would post is a follow-up the route acts on.
        let (status, answer) = instance.deliver_body(&task.delivery_body(), true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["status"], "still_not_connected");
    }

    #[tokio::test]
    async fn a_delivery_without_a_token_minted_for_this_service_is_refused() {
        let instance = Instance::boot("unsigned").await;
        instance.record_refusal(REFUSED_AT_MS, 1).await;
        let (status, _) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), false)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            !instance.window_cleared().await,
            "bug: an unauthenticated delivery acted on the record"
        );
    }

    #[tokio::test]
    async fn the_follow_up_of_an_earlier_refusal_is_superseded_and_logs_nothing() {
        let instance = Instance::boot("superseded").await;
        instance.record_refusal(REFUSED_AT_MS + 60_000, 2).await;
        let (log, _guard) = log_capture::capture_events();

        let (status, answer) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), true)
            .await;

        assert_eq!(status, StatusCode::OK, "Cloud Tasks must stop retrying it");
        assert_eq!(answer["status"], "superseded");
        assert!(
            follow_up_lines(log.events()).is_empty(),
            "only the latest refusal's follow-up speaks"
        );
        assert!(
            !instance.window_cleared().await,
            "bug: a superseded follow-up cleared what the latest refusal's follow-up reads"
        );

        // A follow-up whose record is gone (lapsed, or a redelivery after the
        // first acted) is superseded too, never a retry.
        let stranger = Instance::boot("no-record").await;
        let (status, answer) = stranger
            .deliver(&stranger.follow_up(REFUSED_AT_MS), true)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["status"], "superseded");
    }

    #[tokio::test]
    async fn an_athlete_still_not_connected_is_a_warn_ops_is_alerted_on() {
        let instance = Instance::boot("stuck").await;
        instance.record_refusal(REFUSED_AT_MS, 3).await;
        let (log, _guard) = log_capture::capture_events();

        let (status, answer) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), true)
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["status"], "still_not_connected");
        let lines = follow_up_lines(log.events());
        assert_eq!(lines.len(), 1, "one line per follow-up: {lines:#?}");
        let line = &lines[0];
        assert_eq!(line.level, Level::WARN);
        assert_eq!(
            line.message(),
            Some("Provider sign-in still not connected after refusals")
        );
        assert_eq!(line.field("provider"), Some("garmin"));
        assert_eq!(line.field("backend"), Some("sciotte_garmin"));
        assert_eq!(line.field("refusals"), Some("3"));
        assert_eq!(line.field("reason"), Some("Invalid password"));
        assert_eq!(line.field("method_used"), Some("email"));
        assert_eq!(line.field("account"), Some("j***@e***.com"));
        let tenant = instance.tenant.to_string();
        assert_eq!(line.field("tenant_id"), Some(tenant.as_str()));
        assert!(
            line.field("routes_checked")
                .is_some_and(|routes| routes.contains("sciotte_garmin")),
            "the sign-in route is among those checked: {line:#?}"
        );
        assert!(
            instance.window_cleared().await,
            "the next refusal starts a new window"
        );
    }

    #[tokio::test]
    async fn a_dead_session_row_left_flagged_is_not_a_connection() {
        let instance = Instance::boot("flagged").await;
        instance.connect_session().await;
        let repos = &instance.resources.common.repos;
        repos
            .provider_connections
            .mark_needs_reauth(
                instance.user_id,
                instance.tenant,
                oauth_providers::SCIOTTE_GARMIN,
                Some("session_expired"),
                Timestamp::now(),
            )
            .await
            .unwrap();
        let flagged = repos
            .provider_connections
            .get_for_user(instance.user_id, Some(instance.tenant))
            .await
            .unwrap()
            .into_iter()
            .any(|c| {
                c.provider == oauth_providers::SCIOTTE_GARMIN
                    && c.status == ConnectionStatus::NeedsReauth
            });
        assert!(flagged, "the fixture flags the dead session");
        instance.record_refusal(REFUSED_AT_MS, 1).await;

        let (status, answer) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), true)
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            answer["status"], "still_not_connected",
            "bug: the refused re-sign-in's dead row read as a recovery"
        );
    }

    #[tokio::test]
    async fn a_session_row_past_its_expiry_is_not_a_connection() {
        let instance = Instance::boot("expired").await;
        let past = Timestamp::now() - Elapsed::hours(1);
        let mut token = UserOAuthToken::new(
            instance.user_id,
            instance.tenant.to_string(),
            oauth_providers::SCIOTTE_GARMIN.to_owned(),
            json!({ "session_id": "garmin-web-session", "cookies": [] }).to_string(),
            None,
            Some(past),
            None,
        );
        TOKEN_TYPE_SESSION.clone_into(&mut token.token_type);
        instance
            .resources
            .common
            .repos
            .oauth_tokens
            .upsert_token(&token)
            .await
            .unwrap();
        instance.register_session_connection().await;
        instance.record_refusal(REFUSED_AT_MS, 1).await;

        let (status, answer) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), true)
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            answer["status"], "still_not_connected",
            "bug: an expired session nobody flagged yet read as a recovery"
        );
    }

    #[tokio::test]
    async fn an_athlete_who_connected_after_the_refusals_is_a_quiet_recovery() {
        let instance = Instance::boot("recovered").await;
        instance.record_refusal(REFUSED_AT_MS, 2).await;
        instance.connect_session().await;
        let (log, _guard) = log_capture::capture_events();

        let (status, answer) = instance
            .deliver(&instance.follow_up(REFUSED_AT_MS), true)
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["status"], "recovered");
        let lines = follow_up_lines(log.events());
        assert_eq!(lines.len(), 1, "one line per follow-up: {lines:#?}");
        let line = &lines[0];
        assert_eq!(line.level, Level::INFO, "a recovery posts nothing");
        assert_eq!(
            line.message(),
            Some("Provider sign-in recovered after refusals")
        );
        assert_eq!(line.field("route"), Some("sciotte_garmin"));
        assert_eq!(line.field("refusals"), Some("2"));
        assert!(instance.window_cleared().await);
    }
}
