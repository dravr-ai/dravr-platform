// ABOUTME: The sweep on the Cloud Tasks runner — a turn no delivery ever ran goes back on the queue, under a new name
// ABOUTME: Drives sweep_resumable_turns against a stub queue and asserts which rows it re-enqueues, skips and reaps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The safety net under the queue (registre#126).
//!
//! A Cloud Tasks delivery can be lost for reasons the queue's own retries do
//! not cover: the create call failed at ingress, the task was delivered to an
//! instance that died before it claimed anything, or a run took its lease and
//! then vanished. The sweep is what finds those rows. On this runner it does
//! not run them — it puts them back on the queue, under the next enqueue
//! sequence, because a task name that has already executed is unusable for a
//! day. What it must not touch is just as pinned: a row recorded a moment ago
//! whose delivery is still in flight, and a row a live run holds the lease on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod swept {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use pierre_core::models::TenantId;
    use pierre_database::repositories::{
        ResumableTurnRepository, ResumableTurnRow, TurnClaim, TurnLease,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::services::messaging_ingress::resume::{
        sweep_resumable_turns, MAX_TURN_ATTEMPTS, MAX_TURN_ENQUEUES,
    };
    use uuid::Uuid;

    use crate::common::create_test_server_resources_with_chat_provider_and_runner;
    use crate::helpers::cloud_tasks_stub::{cloud_tasks_turn_runner, QueueStub, ReceivedTask};
    use crate::helpers::drained_turn::ParkedProvider;
    use crate::helpers::google_token::TestSigner;
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// Comfortably older than the sweep's grace on never-leased rows.
    const LONG_AGO_MS: i64 = 10 * 60 * 1_000;

    /// One instance on the Cloud Tasks runner, with the queue stood in for.
    struct Swept {
        resources: Arc<ServerContext>,
        queue: Arc<QueueStub>,
        adapters: Arc<OfflineSendAdapters>,
    }

    impl Swept {
        async fn boot() -> Self {
            let signer = TestSigner::generate();
            let certs = signer.serve_certs().await;
            let queue = QueueStub::accepting();
            let runner =
                cloud_tasks_turn_runner(&queue.serve().await, &certs, Duration::from_secs(1));
            let provider = Arc::new(ParkedProvider::answering("unused: the sweep only enqueues"));
            let resources =
                create_test_server_resources_with_chat_provider_and_runner(provider, runner)
                    .await
                    .unwrap();
            Self {
                resources,
                queue,
                adapters: Arc::new(OfflineSendAdapters::default()),
            }
        }

        fn repo(&self) -> &dyn ResumableTurnRepository {
            self.resources.common.repos.resumable_turns.as_ref()
        }

        /// Record one turn `age_ms` old and return its row.
        async fn record(&self, conversation: &str, age_ms: i64) -> ResumableTurnRow {
            let tenant = TenantId::generate();
            let row = ResumableTurnRow {
                id: Uuid::new_v4().to_string(),
                tenant_id: tenant.to_string(),
                channel_tenant_id: tenant.to_string(),
                user_tenant_id: tenant.to_string(),
                session_id: Uuid::new_v4().to_string(),
                conversation: conversation.to_owned(),
                user_id: Uuid::new_v4().to_string(),
                channel: "whatsapp".to_owned(),
                sender_id: "15550009900".to_owned(),
                conversation_id: Some("chat-7".to_owned()),
                channel_message_id: format!("wamid.{}", Uuid::new_v4().simple()),
                thread_id: None,
                text_content: "peux-tu sortir le NP de ma dernière course?".to_owned(),
                is_group_chat: false,
                locale: "fr".to_owned(),
                turn_id: Uuid::new_v4().to_string(),
                placeholder_message_id: None,
                attempts: 0,
                enqueue_seq: 0,
                created_at_ms: Utc::now().timestamp_millis() - age_ms,
            };
            assert!(self.repo().record_resumable_turn(&row).await.unwrap());
            row
        }

        async fn sweep(&self) -> usize {
            sweep_resumable_turns(&self.resources, self.adapters.as_ref()).await
        }

        /// Every task the queue holds for `row`, in order.
        fn tasks_for(&self, row: &ResumableTurnRow) -> Vec<ReceivedTask> {
            self.queue
                .received()
                .into_iter()
                .filter(|t| t.turn_id() == row.id)
                .collect()
        }

        /// NOT read-only: this CLAIMS the row, so it leases it and bumps its
        /// attempts. Safe as a final assertion, never inside a loop that
        /// sweeps again afterwards — a leased row no longer matches the stale
        /// predicate and the next sweep sees nothing, for the wrong reason.
        async fn still_on_file(&self, row: &ResumableTurnRow) -> bool {
            let tenant = TenantId::parse_str(&row.tenant_id).unwrap();
            // A claim that finds nothing reports the row missing.
            let lease = TurnLease {
                leased_by: "probe",
                now_ms: Utc::now().timestamp_millis(),
                lease_until_ms: Utc::now().timestamp_millis() + 1_000,
                max_attempts: MAX_TURN_ATTEMPTS,
            };
            !matches!(
                self.repo()
                    .claim_resumable_turn(tenant, &row.id, &lease)
                    .await
                    .unwrap(),
                TurnClaim::Missing
            )
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stale_turn_goes_back_on_the_queue_and_a_fresh_one_is_left_alone() {
        let swept = Swept::boot().await;
        let stale = swept.record("conv-stale", LONG_AGO_MS).await;
        let fresh = swept.record("conv-fresh", 0).await;

        let taken = swept.sweep().await;
        assert_eq!(taken, 1, "only the stale row is swept");

        let tasks = swept.tasks_for(&stale);
        assert_eq!(tasks.len(), 1, "the stale turn is enqueued once");
        assert!(
            tasks[0].name().ends_with("-e1"),
            "a re-enqueue carries the next sequence, because the first task name is spent: {}",
            tasks[0].name()
        );
        assert_eq!(
            tasks[0].delivery_body()["tenant_id"],
            stale.tenant_id,
            "the delivery carries the tenant the claim runs under"
        );
        assert!(
            swept.tasks_for(&fresh).is_empty(),
            "a turn recorded a moment ago still has a delivery in flight"
        );

        // The sweep enqueues; it does not run. The row waits for its
        // delivery, exactly as it did at ingress.
        assert!(swept.resources.common.turns.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_pass_enqueues_the_same_turn_under_a_new_name() {
        let swept = Swept::boot().await;
        let stale = swept.record("conv-repeat", LONG_AGO_MS).await;

        assert_eq!(swept.sweep().await, 1);
        assert_eq!(swept.sweep().await, 1);

        let names: Vec<String> = swept
            .tasks_for(&stale)
            .iter()
            .map(|t| t.name().to_owned())
            .collect();
        assert_eq!(names.len(), 2, "one task per pass: {names:?}");
        assert!(names[0].ends_with("-e1"), "{names:?}");
        assert!(names[1].ends_with("-e2"), "{names:?}");
        assert_ne!(
            names[0], names[1],
            "a name Cloud Tasks has already executed would be refused for a day"
        );
        assert!(
            swept.still_on_file(&stale).await,
            "the row is the delivery's to claim; the sweep never removes it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_a_live_run_holds_is_not_enqueued_again() {
        let swept = Swept::boot().await;
        let running = swept.record("conv-running", LONG_AGO_MS).await;
        let tenant = TenantId::parse_str(&running.tenant_id).unwrap();
        let now = Utc::now().timestamp_millis();
        let claimed = swept
            .repo()
            .claim_resumable_turn(
                tenant,
                &running.id,
                &TurnLease {
                    leased_by: "instance-a",
                    now_ms: now,
                    lease_until_ms: now + 90_000,
                    max_attempts: MAX_TURN_ATTEMPTS,
                },
            )
            .await
            .unwrap();
        assert!(matches!(claimed, TurnClaim::Claimed(_)));

        assert_eq!(swept.sweep().await, 0, "a leased turn is somebody's");
        assert!(swept.tasks_for(&running).is_empty());

        // The run dies without a word: its lease ends, and the next pass puts
        // the turn back on the queue.
        swept
            .repo()
            .release_resumable_turn(tenant, &running.id, now - 1_000)
            .await
            .unwrap();
        assert_eq!(swept.sweep().await, 1);
        assert_eq!(swept.tasks_for(&running).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_past_its_attempt_cap_is_dropped_rather_than_enqueued() {
        let swept = Swept::boot().await;
        let doomed = swept.record("conv-doomed", LONG_AGO_MS).await;
        let tenant = TenantId::parse_str(&doomed.tenant_id).unwrap();
        let now = Utc::now().timestamp_millis();

        // Spend every attempt, then let the last run die: nothing will claim
        // this row again, and the athlete was never told.
        for _ in 0..=MAX_TURN_ATTEMPTS {
            let _ = swept
                .repo()
                .claim_resumable_turn(
                    tenant,
                    &doomed.id,
                    &TurnLease {
                        leased_by: "instance-a",
                        now_ms: now,
                        lease_until_ms: now + 90_000,
                        max_attempts: MAX_TURN_ATTEMPTS,
                    },
                )
                .await
                .unwrap();
            swept
                .repo()
                .release_resumable_turn(tenant, &doomed.id, now - 1_000)
                .await
                .unwrap();
        }

        assert_eq!(swept.sweep().await, 0, "nothing left to enqueue");
        assert!(
            swept.tasks_for(&doomed).is_empty(),
            "a turn nobody can claim is not put back on the queue forever"
        );
        assert!(
            !swept.still_on_file(&doomed).await,
            "the reap drops it, so the queue and the table both stop carrying it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_the_queue_can_never_get_claimed_is_dropped_rather_than_minted_forever() {
        // The row a delivery never reaches: a refused token, an unmounted
        // route, a 5xx on the way in. `attempts` is bumped by the CLAIM, so
        // none of those touch it — the row comes back to every sweep looking
        // exactly as it did, matching the stale predicate forever. Without a
        // ceiling this mints a fresh Cloud Tasks task every minute for the
        // life of the table and never reaches either terminal state.
        let swept = Swept::boot().await;
        let doomed = swept.record("conv-unclaimable", LONG_AGO_MS).await;

        // Every pass up to the ceiling puts it back, exactly as before.
        for pass in 1..=MAX_TURN_ENQUEUES {
            assert_eq!(swept.sweep().await, 1, "pass {pass} still re-queues");
            assert_eq!(
                swept.tasks_for(&doomed).len(),
                usize::try_from(pass).unwrap(),
                "one task per pass up to the ceiling"
            );
            // Deliberately no `still_on_file` here: it claims, and a claimed
            // row stops matching the stale predicate, so the probe would end
            // the loop it is meant to observe. That the next pass re-queues at
            // all is the proof the row survived.
        }

        // The pass that crosses it drops the row instead of minting again.
        swept.sweep().await;
        assert_eq!(
            swept.tasks_for(&doomed).len(),
            usize::try_from(MAX_TURN_ENQUEUES).unwrap(),
            "no task is minted once the ceiling is crossed"
        );
        assert!(
            !swept.still_on_file(&doomed).await,
            "the row is dropped, so it stops matching the stale predicate"
        );

        // And it stays dropped: the loop is closed, not merely paused.
        assert_eq!(swept.sweep().await, 0, "nothing left to sweep");
        assert_eq!(
            swept.tasks_for(&doomed).len(),
            usize::try_from(MAX_TURN_ENQUEUES).unwrap(),
            "still no further tasks"
        );
    }
}
