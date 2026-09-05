// ABOUTME: The Cloud Tasks path end to end — a webhook enqueues a task, the delivery runs the turn inside the request
// ABOUTME: Drives the real webhook and turn-run routes against a stub queue and a test signer that plays Google
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn as a request Cloud Run can see (registre#126).
//!
//! On GCP a messaging webhook records the turn and enqueues its id; Cloud
//! Tasks delivers `POST /internal/turns/{id}/run` and the turn runs inside
//! that request. This suite is that path with the queue and Google stood in
//! for: the webhook must enqueue exactly one task and run nothing locally;
//! the delivery must be refused without a token minted for this service; a
//! valid delivery must answer the athlete exactly once and a repeat must
//! answer nobody; a younger turn must wait for its older sibling and then be
//! told to come back; and an instance draining mid-turn must hand the row
//! back so the next instance's delivery answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod delivered {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::Router;
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
    use pierre_core::llm::LlmProvider;
    use pierre_core::models::TenantId;
    use pierre_database::backends::MessagingRepository;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::adapter_factory::ChannelAdapterFactory;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::turn_runner::TurnRunner;
    use serde_json::Value;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, Instant};

    use crate::common::{
        create_sibling_server_resources_with_chat_provider_and_runner,
        create_test_server_resources_with_chat_provider_and_runner,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::cloud_tasks_stub::{
        cloud_tasks_turn_runner, QueueStub, ReceivedTask, SERVICE_ACCOUNT, TARGET,
    };
    use crate::helpers::drained_turn::{
        compute_whatsapp_sig, create_active_user, link_channel, outbound_bodies,
        setup_whatsapp_config, wait_for_a_tracked_turn, whatsapp_text_payload, HangingProvider,
        ParkedProvider,
    };
    use crate::helpers::google_token::{GoogleClaims, TestSigner};
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// The coaching the athlete was owed. Distinctive enough that no other
    /// outbound row (a coach proposal, an intake question) can match it.
    const ANSWER: &str = "Ton NP sur la dernière course: 245 W, soit 3,4 W/kg — solide.";
    const QUESTION: &str = "peux-tu sortir le NP de ma dernière course?";
    const WA_SECRET: &str = "wa_turn_delivery_secret";

    /// How long a delivery waits for an older turn of the same conversation
    /// before asking Cloud Tasks to come back. Short, so the test is quick;
    /// production waits four minutes.
    const CLAIM_WAIT: Duration = Duration::from_secs(1);

    /// One instance on the Cloud Tasks runner, with the queue and Google
    /// stood in for, and one linked `WhatsApp` athlete.
    struct Instance {
        resources: Arc<ServerContext>,
        router: Router,
        queue: Arc<QueueStub>,
        signer: Arc<TestSigner>,
        runner: Arc<TurnRunner>,
        tenant_id: TenantId,
        sender_id: String,
    }

    impl Instance {
        async fn boot(provider: Arc<dyn LlmProvider>, sender_id: &str) -> Self {
            let signer = Arc::new(TestSigner::generate());
            let certs = signer.serve_certs().await;
            let queue = QueueStub::accepting();
            let runner = cloud_tasks_turn_runner(&queue.serve().await, &certs, CLAIM_WAIT);
            let resources = create_test_server_resources_with_chat_provider_and_runner(
                provider,
                Arc::clone(&runner),
            )
            .await
            .unwrap();
            let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
            let (user_id, tenant_id) =
                create_active_user(&resources, &format!("{sender_id}@delivery.example.com")).await;
            setup_whatsapp_config(db, tenant_id, WA_SECRET).await;
            link_channel(db, tenant_id, user_id, sender_id).await;
            let router = Self::router_over(&resources);
            Self {
                resources,
                router,
                queue,
                signer,
                runner,
                tenant_id,
                sender_id: sender_id.to_owned(),
            }
        }

        /// The next instance: a fresh process over the same rows, on the same
        /// runner (same stub queue, same signer), with its own tracker and
        /// its own provider.
        async fn sibling(&self, provider: Arc<dyn LlmProvider>) -> Self {
            let resources = create_sibling_server_resources_with_chat_provider_and_runner(
                &self.resources,
                provider,
                Some(Arc::clone(&self.runner)),
            )
            .await
            .unwrap();
            let router = Self::router_over(&resources);
            Self {
                resources,
                router,
                queue: Arc::clone(&self.queue),
                signer: Arc::clone(&self.signer),
                runner: Arc::clone(&self.runner),
                tenant_id: self.tenant_id,
                sender_id: self.sender_id.clone(),
            }
        }

        fn router_over(resources: &Arc<ServerContext>) -> Router {
            let adapters: Arc<dyn ChannelAdapterFactory> = Arc::new(OfflineSendAdapters::default());
            MessagingRoutes::routes_with_adapters(Arc::clone(resources), adapters)
        }

        /// Post one athlete message through the real webhook route and return
        /// the task the queue received for it.
        async fn ask(&self, msg_id: &str, text: &str) -> ReceivedTask {
            let before = self.queue.received().len();
            let payload = whatsapp_text_payload(&self.sender_id, msg_id, text);
            let body_bytes = serde_json::to_vec(&payload).unwrap();
            let sig = compute_whatsapp_sig(WA_SECRET, &body_bytes);
            let status = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
                .header("content-type", "application/json")
                .header("x-hub-signature-256", &sig)
                .json(&payload)
                .send(self.router.clone())
                .await
                .status_code();
            assert_eq!(status, StatusCode::OK, "webhooks always ack");
            let received = self.queue.received();
            assert_eq!(
                received.len(),
                before + 1,
                "one message, one task: {:?}",
                received.iter().map(ReceivedTask::name).collect::<Vec<_>>()
            );
            received.last().unwrap().clone()
        }

        /// A token Cloud Tasks would carry: minted for this service's audience
        /// on behalf of the turn runner's service account.
        fn token(&self) -> String {
            self.signer
                .mint(&GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT))
        }

        /// Deliver `task` the way Cloud Tasks would, with `token` as the bearer.
        async fn deliver(&self, task: &ReceivedTask, token: Option<&str>) -> (StatusCode, Value) {
            let mut request =
                AxumTestRequest::post(&format!("/internal/turns/{}/run", task.turn_id()))
                    .header("content-type", "application/json")
                    .json(&task.delivery_body());
            if let Some(token) = token {
                request = request.header("authorization", &format!("Bearer {token}"));
            }
            let response = request.send(self.router.clone()).await;
            let status = response.status_code();
            let body: Value = if status == StatusCode::UNAUTHORIZED {
                Value::Null
            } else {
                response.json()
            };
            (status, body)
        }

        /// The same delivery, running in the background so the test can drain
        /// the instance underneath it.
        fn deliver_in_background(&self, task: &ReceivedTask) -> JoinHandle<(StatusCode, Value)> {
            let router = self.router.clone();
            let token = self.token();
            let task = task.clone();
            tokio::spawn(async move {
                let response =
                    AxumTestRequest::post(&format!("/internal/turns/{}/run", task.turn_id()))
                        .header("content-type", "application/json")
                        .header("authorization", &format!("Bearer {token}"))
                        .json(&task.delivery_body())
                        .send(router)
                        .await;
                let status = response.status_code();
                let body: Value = response.json();
                (status, body)
            })
        }

        /// SIGTERM, as the tracker sees it: a short grace, then the signal.
        async fn drain(&self) {
            let report = self
                .resources
                .common
                .turns
                .drain(Duration::from_millis(200), Duration::from_secs(30))
                .await;
            assert_eq!(report.signalled, 1, "the parked turn is signalled");
            assert_eq!(
                report.abandoned, 0,
                "the signalled turn finishes its hand-off inside its window: {report:?}"
            );
        }

        async fn replies(&self) -> Vec<String> {
            let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
            outbound_bodies(db, self.tenant_id, &self.sender_id).await
        }

        async fn answers(&self) -> usize {
            self.replies().await.iter().filter(|b| *b == ANSWER).count()
        }

        fn interrupted_notice(&self) -> String {
            self.resources
                .mcp
                .messaging_strings_registry
                .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE)
        }
    }

    /// A provider that answers straight away.
    fn released(answer: &str) -> Arc<ParkedProvider> {
        let provider = Arc::new(ParkedProvider::answering(answer));
        provider.release();
        provider
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_webhook_enqueues_one_task_and_runs_nothing_locally() {
        let instance = Instance::boot(released(ANSWER), "15550008001").await;

        let task = instance.ask("wamid.enqueue_001", QUESTION).await;

        assert!(
            task.name().ends_with("-e0"),
            "the first enqueue carries sequence 0: {}",
            task.name()
        );
        assert!(
            task.url().starts_with(&format!("{TARGET}/internal/turns/"))
                && task.url().ends_with("/run"),
            "the task targets the turn-run route on this service: {}",
            task.url()
        );
        assert_eq!(
            task.body["task"]["httpRequest"]["oidcToken"]["audience"], TARGET,
            "the token audience is the bare service URL the verifier checks"
        );
        assert_eq!(
            task.delivery_body()["tenant_id"],
            instance.tenant_id.to_string(),
            "the delivery carries the session tenant the claim runs under"
        );

        // Nothing runs on this instance: the turn waits for its delivery.
        sleep(Duration::from_millis(300)).await;
        assert!(
            instance.resources.common.turns.is_empty(),
            "the webhook must not spawn the turn when a queue delivers it"
        );
        assert!(instance.replies().await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_delivery_without_a_token_for_this_service_runs_nothing() {
        let instance = Instance::boot(released(ANSWER), "15550008002").await;
        let task = instance
            .ask("wamid.auth_001", "combien de watts hier?")
            .await;

        let (status, _) = instance.deliver(&task, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "no bearer token");

        let other_audience = instance.signer.mint(&GoogleClaims::cloud_tasks(
            "https://someone-else.run.app",
            SERVICE_ACCOUNT,
        ));
        let (status, _) = instance.deliver(&task, Some(&other_audience)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a token for another service"
        );

        let intruder = instance.signer.mint(&GoogleClaims::cloud_tasks(
            TARGET,
            "intruder@dravr-dev.iam.gserviceaccount.com",
        ));
        let (status, _) = instance.deliver(&task, Some(&intruder)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a token for another identity"
        );

        assert!(instance.replies().await.is_empty(), "nothing ran");

        // The row is untouched: the real delivery still answers.
        let (status, body) = instance.deliver(&task, Some(&instance.token())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "finished");
        assert_eq!(instance.answers().await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_delivery_answers_the_athlete_once_and_a_repeat_answers_nobody() {
        let instance = Instance::boot(released(ANSWER), "15550008003").await;
        let task = instance.ask("wamid.deliver_001", QUESTION).await;

        let (status, body) = instance.deliver(&task, Some(&instance.token())).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the turn ran inside the request: {body}"
        );
        assert_eq!(body["status"], "finished");
        assert_eq!(
            instance.answers().await,
            1,
            "the athlete gets their answer exactly once, got {:?}",
            instance.replies().await
        );
        assert!(
            instance.resources.common.turns.is_empty(),
            "the request awaited the turn to its end"
        );

        // Cloud Tasks delivers at least once; the second delivery finds the
        // row gone and answers nobody.
        let (status, body) = instance.deliver(&task, Some(&instance.token())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "finished");
        assert_eq!(instance.answers().await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_younger_turn_waits_for_its_older_sibling_then_asks_for_a_retry() {
        let instance = Instance::boot(released(ANSWER), "15550008004").await;
        let first = instance.ask("wamid.order_001", "ma dernière sortie?").await;
        let second = instance.ask("wamid.order_002", "et celle d'avant?").await;

        // Cloud Tasks gives no ordering guarantee: the second task lands
        // first. The route waits its claim wait for the older turn, then
        // asks for a retry rather than answering out of order.
        let started = Instant::now();
        let (status, body) = instance.deliver(&second, Some(&instance.token())).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["status"], "blocked");
        assert!(
            started.elapsed() >= CLAIM_WAIT,
            "the wait is spent inside the request, keeping the instance busy"
        );
        assert!(
            instance.replies().await.is_empty(),
            "nothing answered out of order"
        );

        let (status, body) = instance.deliver(&first, Some(&instance.token())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = instance.deliver(&second, Some(&instance.token())).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the retry runs once its predecessor is gone: {body}"
        );
        assert_eq!(
            instance.answers().await,
            2,
            "both questions answered, in order"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_drain_mid_delivery_hands_the_row_back_and_the_next_instance_answers() {
        let provider = Arc::new(ParkedProvider::answering(ANSWER));
        let instance =
            Instance::boot(Arc::clone(&provider) as Arc<dyn LlmProvider>, "15550008005").await;
        let task = instance.ask("wamid.drain_001", QUESTION).await;

        // The delivery runs the turn, which parks on the provider.
        let delivering = instance.deliver_in_background(&task);
        assert!(
            wait_for_a_tracked_turn(&instance.resources).await,
            "the delivered turn is countable like any other"
        );

        // SIGTERM on this instance: the turn cannot finish, the row is handed
        // back, and the request tells Cloud Tasks to deliver again.
        instance.drain().await;
        let (status, body) = delivering.await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["status"], "drained");
        let notice = instance.interrupted_notice();
        assert!(
            !instance.replies().await.iter().any(|b| b == &notice),
            "a drained delivery with an attempt left does not apologise"
        );

        // The provider the turn was parked on died with its instance; the
        // next instance's provider answers, and Cloud Tasks redelivers there.
        provider.release();
        let next = instance
            .sibling(Arc::clone(&provider) as Arc<dyn LlmProvider>)
            .await;
        let (status, body) = next.deliver(&task, Some(&next.token())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "finished");
        assert_eq!(
            next.answers().await,
            1,
            "the athlete gets their answer exactly once, from the next instance"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_drained_twice_is_apologised_for_and_its_task_closes() {
        let instance = Instance::boot(Arc::new(HangingProvider), "15550008006").await;
        let task = instance.ask("wamid.drain_002", "et mon NP?").await;

        // First drain: the row goes back, Cloud Tasks is asked to retry.
        let delivering = instance.deliver_in_background(&task);
        assert!(wait_for_a_tracked_turn(&instance.resources).await);
        instance.drain().await;
        let (status, body) = delivering.await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["status"], "drained");

        // The retry lands on the next instance, which drains too. No attempt
        // is left: the athlete is told, and the task closes so Cloud Tasks
        // stops delivering a turn that will never answer.
        let next = instance.sibling(Arc::new(HangingProvider)).await;
        let delivering = next.deliver_in_background(&task);
        assert!(wait_for_a_tracked_turn(&next.resources).await);
        next.drain().await;
        let (status, body) = delivering.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "finished");

        let notice = next.interrupted_notice();
        let mut replies = next.replies().await;
        for _ in 0..50 {
            if replies.iter().any(|b| b == &notice) {
                break;
            }
            sleep(Duration::from_millis(100)).await;
            replies = next.replies().await;
        }
        assert!(
            replies.iter().any(|b| b == &notice),
            "a turn drained twice tells the athlete its answer is not coming, got {replies:?}"
        );

        // A late redelivery finds nothing left to run.
        let (status, body) = next.deliver(&task, Some(&next.token())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "finished", "the row is gone");
        assert_eq!(
            next.replies()
                .await
                .iter()
                .filter(|b| *b == &notice)
                .count(),
            1,
            "told once"
        );
    }
}
