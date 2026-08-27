// ABOUTME: Guards that a detached messaging turn is tracked, bounded, and closed instead of lost
// ABOUTME: Drives the 2026-08-26 incident end-to-end — instance drains mid-turn, athlete is told

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What happens to a turn nobody is waiting for.
//!
//! A messaging webhook answers HTTP 200 as soon as it has stored the inbound
//! message, and the LLM turn keeps running behind it. Cloud Run counts
//! in-flight *requests*, so the instance reads as idle from the athlete's
//! first second and any rollout or scaledown is free to take the turn with it.
//! On 2026-08-26 one did: a group chart ask reached the tool loop, produced
//! 1542 characters, opened a second session for the final answer, and the
//! instance drained mid-retry. Nothing tracked the turn, nothing awaited it,
//! and nothing told the athlete — the "génération de la réponse…" placeholder
//! is still open (registre#109).
//!
//! Three things had to become true, and each is asserted here:
//!
//! 1. the turn is **countable** — spawned through `InFlightTurns`, so
//!    shutdown can see it;
//! 2. shutdown **spends its grace window on it** rather than sleeping through
//!    it, and signals whatever is left instead of dying quietly;
//! 3. a turn that cannot finish **says so**, so the athlete stops waiting.
//!
//! The e2e test at the bottom is the incident itself: a hung provider, a
//! drain, and an assertion that the athlete's channel receives the localized
//! interruption notice. On the code before the fix it hangs forever and the
//! athlete gets nothing at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_TURN_INTERRUPTED};
use pierre_core::errors::AppError;
use pierre_mcp_server::services::turn_lifecycle::InFlightTurns;
use tokio::time::sleep;

// ════════════════════════════════════════════════════════════════
// The tracker: shutdown can see the turns, and spends its window on them
// ════════════════════════════════════════════════════════════════

/// A turn that finishes inside the grace window runs to its own end and the
/// athlete never learns the instance was going away.
///
/// The flag is the assertion that matters: `drain` returning is not evidence
/// the turn finished, only that the tracker emptied. This proves the turn
/// reached its last line.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_finishes_inside_the_grace_is_awaited_not_cut() {
    let turns = InFlightTurns::new();
    let finished = Arc::new(AtomicBool::new(false));

    let flag = Arc::clone(&finished);
    turns.spawn(async move {
        sleep(Duration::from_millis(50)).await;
        flag.store(true, Ordering::SeqCst);
    });

    // The turn is countable while it runs — this is what the process had no
    // way of knowing before, and what makes the drain below possible.
    assert_eq!(turns.len(), 1, "a spawned turn must be visible to shutdown");
    assert!(!turns.is_empty());

    let report = turns
        .drain(Duration::from_secs(5), Duration::from_secs(1))
        .await;

    assert!(
        finished.load(Ordering::SeqCst),
        "the drain must await the turn, not merely outlive the tracker"
    );
    assert_eq!(report.in_flight_at_signal, 1);
    assert_eq!(
        report.signalled, 0,
        "it finished on its own; nothing to signal"
    );
    assert_eq!(report.abandoned, 0);
    assert!(report.is_clean());
    assert!(turns.is_empty());
}

/// A turn still running when the grace expires is told, and gets its own
/// window to close its placeholder.
///
/// This is the shape of the real fix: the answer is lost either way, but the
/// athlete learns it is lost instead of watching a placeholder forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_outlives_the_grace_is_signalled_and_closes() {
    let turns = InFlightTurns::new();
    let closed = Arc::new(AtomicBool::new(false));

    let drain = turns.drain_token();
    let flag = Arc::clone(&closed);
    turns.spawn(async move {
        // Stands in for a turn parked on an LLM call that will not return
        // before the process dies.
        tokio::select! {
            () = sleep(Duration::from_mins(1)) => {}
            () = drain.cancelled() => {
                // The placeholder close: one channel API edit.
                sleep(Duration::from_millis(20)).await;
                flag.store(true, Ordering::SeqCst);
            }
        }
    });

    let report = turns
        .drain(Duration::from_millis(100), Duration::from_secs(5))
        .await;

    assert!(
        closed.load(Ordering::SeqCst),
        "the drain signal must reach the turn in time for it to close"
    );
    assert_eq!(report.in_flight_at_signal, 1);
    assert_eq!(report.signalled, 1, "the turn outlived the grace");
    assert_eq!(
        report.abandoned, 0,
        "it closed inside the signal window, so nothing was abandoned"
    );
    assert!(report.is_clean());
}

/// A turn that ignores the signal is reported as abandoned, not as a clean
/// shutdown.
///
/// Every abandoned turn is an athlete holding an open placeholder, so a drain
/// that reported success here would be the same silence the fix removes —
/// only now with a log line claiming otherwise.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_ignores_the_signal_is_reported_abandoned() {
    let turns = InFlightTurns::new();

    turns.spawn(async {
        sleep(Duration::from_mins(1)).await;
    });

    let report = turns
        .drain(Duration::from_millis(50), Duration::from_millis(50))
        .await;

    assert_eq!(report.in_flight_at_signal, 1);
    assert_eq!(report.signalled, 1);
    assert_eq!(report.abandoned, 1);
    assert!(
        !report.is_clean(),
        "an abandoned turn must not be reported as a clean drain"
    );
}

/// An instance with nothing in flight pays nothing to shut down.
///
/// The drain replaced an unconditional 3s sleep; if it blocked for the full
/// grace window on an idle instance it would have made every rollout slower
/// while fixing nothing.
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_instance_drains_immediately() {
    let turns = InFlightTurns::new();

    let report = turns
        .drain(Duration::from_secs(30), Duration::from_secs(30))
        .await;

    assert_eq!(report.in_flight_at_signal, 0);
    assert!(report.is_clean());
    assert!(
        report.elapsed < Duration::from_secs(1),
        "an idle drain must return at once, not sit out the grace window \
         (took {:?})",
        report.elapsed
    );
}

/// Turns from different conversations drain together, not one after another.
///
/// The grace window is a fixed budget shared by every turn on the instance;
/// draining serially would mean the second athlete's turn is abandoned
/// whenever the first one is slow.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_turns_share_one_grace_window() {
    let turns = InFlightTurns::new();

    for _ in 0..4 {
        turns.spawn(async {
            sleep(Duration::from_millis(200)).await;
        });
    }
    assert_eq!(turns.len(), 4);

    let report = turns
        .drain(Duration::from_secs(5), Duration::from_secs(1))
        .await;

    assert!(report.is_clean());
    assert_eq!(report.in_flight_at_signal, 4);
    assert!(
        report.elapsed < Duration::from_millis(600),
        "four 200ms turns must drain concurrently, not serially (took {:?})",
        report.elapsed
    );
}

// ════════════════════════════════════════════════════════════════
// The two endings a detached turn otherwise has no source for
// ════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod bounded_turn {
    use super::{sleep, AppError, Duration};
    use pierre_mcp_server::services::messaging_ingress::turn_guard::{
        run_bounded, TurnInterruption, TurnOutcome,
    };
    use tokio_util::sync::CancellationToken;

    /// A turn that outlives its wall-clock ceiling is reported as a watchdog
    /// interruption, not left running.
    ///
    /// An unbounded turn does not just fail to answer — it holds its
    /// conversation's dispatch lock, so the athlete's next question queues
    /// behind an answer that is never coming.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_past_its_budget_reports_the_watchdog() {
        let never = async {
            sleep(Duration::from_mins(1)).await;
            TurnOutcome::Failed(AppError::internal("unreachable"))
        };

        let outcome =
            run_bounded(never, Duration::from_millis(50), &CancellationToken::new()).await;

        match outcome {
            TurnOutcome::Interrupted(cause) => assert_eq!(cause, TurnInterruption::Watchdog),
            _ => panic!("a turn past its budget must report an interruption"),
        }
    }

    /// A turn running when the process starts going away is reported as a
    /// drain interruption — the 2026-08-26 incident's shape.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_caught_by_shutdown_reports_the_drain() {
        let drain = CancellationToken::new();
        let signal = drain.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            signal.cancel();
        });

        let never = async {
            sleep(Duration::from_mins(1)).await;
            TurnOutcome::Failed(AppError::internal("unreachable"))
        };

        let outcome = run_bounded(never, Duration::from_mins(1), &drain).await;

        match outcome {
            TurnOutcome::Interrupted(cause) => assert_eq!(cause, TurnInterruption::Drain),
            _ => panic!("a turn caught by shutdown must report an interruption"),
        }
    }

    /// A finished turn wins over a deadline that came due in the same poll.
    ///
    /// `run_bounded`'s select is biased for this: left to chance, a turn that
    /// actually produced an answer would sometimes be reported to the athlete
    /// as interrupted, which is worse than the bug being fixed — the answer
    /// exists and gets thrown away.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_turn_beats_an_expired_budget() {
        let drain = CancellationToken::new();
        drain.cancel();

        let done = async { TurnOutcome::Failed(AppError::internal("the turn's own outcome")) };

        // Both endings are already due: zero budget, cancelled drain.
        let outcome = run_bounded(done, Duration::ZERO, &drain).await;

        match outcome {
            TurnOutcome::Failed(e) => assert!(e.to_string().contains("the turn's own outcome")),
            _ => panic!("a turn that finished must report its own outcome"),
        }
    }

    /// The two causes stay distinguishable in the log.
    ///
    /// The athlete gets one sentence either way, but a hung turn is a bug and
    /// a drained one is a deploy; collapsing them is how the first hides
    /// inside the second.
    #[test]
    fn each_interruption_names_its_cause() {
        assert_eq!(TurnInterruption::Watchdog.as_str(), "turn_watchdog");
        assert_eq!(TurnInterruption::Drain.as_str(), "shutdown_drain");
        assert_ne!(
            TurnInterruption::Watchdog.as_str(),
            TurnInterruption::Drain.as_str()
        );
    }
}

// ════════════════════════════════════════════════════════════════
// What the athlete reads
// ════════════════════════════════════════════════════════════════

/// The closing notice exists, and differs, in all five compiled-in locales.
///
/// A key that resolves to the same string everywhere is a key that fell back
/// to its default for four of them, which reads as English text arriving in a
/// French conversation.
#[test]
fn the_interruption_notice_speaks_all_five_locales() {
    let reg = MessagingStringsRegistry::new();
    let mut seen: Vec<String> = Vec::new();

    for locale in ["fr", "en", "es", "de", "pt"] {
        let notice = reg.get(KEY_TURN_INTERRUPTED, locale);
        assert!(
            !notice.trim().is_empty(),
            "no interruption notice for {locale}"
        );
        assert!(
            !seen.contains(&notice),
            "{locale} fell back to another locale's notice: {notice:?}"
        );
        seen.push(notice);
    }
    assert_eq!(seen.len(), 5);
}

// ════════════════════════════════════════════════════════════════
// The incident, end to end
// ════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod drained_mid_turn {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
    };
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use sha2::Sha256;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// A provider that never answers, standing in for the ACP session the
    /// 2026-08-26 turn was parked on when its instance was drained.
    struct HangingProvider;

    #[async_trait]
    impl LlmProvider for HangingProvider {
        fn name(&self) -> &'static str {
            "hanging_mock"
        }
        fn display_name(&self) -> &'static str {
            "Hanging Mock LLM (turn-lifecycle e2e)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            // Far longer than the test's drain budget: the turn must be ended
            // by the drain, never by this returning.
            sleep(Duration::from_mins(10)).await;
            Err(AppError::internal("hanging provider must never answer"))
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            sleep(Duration::from_mins(10)).await;
            let chunk = StreamChunk {
                delta: String::new(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// An Active user with a tenant and a synthetic provider, so the turn
    /// clears the status and onboarding gates and reaches the pipeline.
    async fn create_active_user(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("DrainPin123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Drain User".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());

        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Drain Tenant {email}"),
            slug: format!("drain-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();

        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                user_id,
                tenant_id,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();

        (user_id, tenant_id)
    }

    async fn setup_whatsapp_config(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        secret: &str,
    ) {
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_drain_test_token"),
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: Some("wa_drain_test_business_id"),
            phone_number: Some("15550000003"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    }

    async fn link_channel(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        sender_id: &str,
    ) {
        let link_id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id_str,
            channel_type: "whatsapp",
            channel_user_id: sender_id,
            display_name: Some("Drain Linked User"),
        })
        .await
        .unwrap();
    }

    fn whatsapp_text_payload(sender_id: &str, msg_id: &str, text: &str) -> serde_json::Value {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "wa_drain_test_business_id",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15550000003",
                            "phone_number_id": "15550000003"
                        },
                        "messages": [{
                            "from": sender_id,
                            "id": msg_id,
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": { "body": text }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        })
    }

    /// Outbound bodies stored for the sender's session. A reply whose live
    /// send fails in tests is still persisted alongside its retry-queue
    /// entry, so the notice is observable here either way.
    async fn outbound_bodies(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        sender_id: &str,
    ) -> Vec<String> {
        let Ok(Some(session)) = db
            .get_session_by_channel_identity(tenant_id, "whatsapp", sender_id, None)
            .await
        else {
            return Vec::new();
        };
        let Some(session_id) = session["id"].as_str() else {
            return Vec::new();
        };
        db.get_session_messages(session_id, tenant_id, 100, 0)
            .await
            .map(|rows| {
                rows.into_iter()
                    .filter(|r| r["direction"].as_str() == Some("outbound"))
                    .filter_map(|r| r["content_body"].as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The 2026-08-26 incident, reproduced and closed.
    ///
    /// An athlete asks a question, the turn parks on a provider that never
    /// answers, and the instance shuts down underneath it. Before the fix
    /// this is exactly where the athlete's answer disappeared: the turn was
    /// spawned detached, nothing counted it, nothing signalled it, and the
    /// status placeholder stayed open forever.
    ///
    /// Three assertions, in the order the fix has to hold:
    ///
    /// 1. the running turn is visible to shutdown at all;
    /// 2. the drain reaches it and it closes inside the signal window;
    /// 3. the athlete's channel receives the localized interruption notice.
    ///
    /// The third is the one the athlete experiences, and it is why the first
    /// two exist.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_drained_mid_flight_tells_the_athlete_instead_of_vanishing() {
        let resources = create_test_server_resources_with_chat_provider(Arc::new(HangingProvider))
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) =
            create_active_user(&resources, "drained_mid_turn@example.com").await;

        let wa_secret = "wa_turn_lifecycle_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;
        let sender_id = "15550009002";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload = whatsapp_text_payload(
            sender_id,
            "wamid.drain_001",
            "peux-tu sortir le NP de ma dernière course?",
        );
        let body_bytes = serde_json::to_vec(&payload).unwrap();
        let sig = compute_whatsapp_sig(wa_secret, &body_bytes);
        let router = MessagingRoutes::routes(Arc::clone(&resources));

        let status = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &sig)
            .json(&payload)
            .send(router)
            .await
            .status_code();
        assert_eq!(status, StatusCode::OK, "webhooks always ack");

        // 1. The turn the webhook started is countable. Before the fix this
        //    was a bare `tokio::spawn` whose handle was dropped, so the
        //    process had no way to know a turn existed at all.
        let mut tracked = false;
        for _ in 0..100 {
            if !resources.common.turns.is_empty() {
                tracked = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            tracked,
            "the turn the webhook spawned must be visible to shutdown"
        );

        // 2. SIGTERM. The turn is parked on a provider that will not answer
        //    for another ten minutes, so the grace window expires and the
        //    drain signal is what ends it.
        let report = resources
            .common
            .turns
            .drain(Duration::from_millis(200), Duration::from_secs(30))
            .await;

        assert_eq!(
            report.signalled, 1,
            "the hung turn must outlive the grace and be signalled"
        );
        assert_eq!(
            report.abandoned, 0,
            "the signalled turn must finish closing inside its window, \
             got report: {report:?}"
        );

        // 3. What the athlete reads. The turn produced no answer and never
        //    could, but it says so — the placeholder that used to stay open
        //    forever now carries this.
        let expected = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
        let bodies = outbound_bodies(db, tenant_id, sender_id).await;
        assert!(
            bodies.iter().any(|b| b == &expected),
            "a drained turn must tell the athlete its answer is not coming \
             (expected {expected:?}), got outbound bodies: {bodies:?}"
        );
    }
}
