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
//! 3. a turn that cannot finish here is **handed to the next instance** and
//!    answered there (registre#126) — and only a turn drained twice tells the
//!    athlete its answer is not coming.
//!
//! The e2e tests at the bottom are the incident itself: a hung provider, a
//! drain, and an assertion on what the athlete's channel receives. On the
//! code before registre#109 the turn hung forever and the athlete got
//! nothing; on the code before registre#126 they got an apology.

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
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
    use pierre_core::llm::LlmProvider;
    use pierre_core::models::messaging::MessageContent;
    use pierre_core::models::TenantId;
    use pierre_database::backends::MessagingRepository;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::messaging_ingress::resume::sweep_resumable_turns;
    use tokio::time::sleep;

    use crate::common::{
        create_sibling_server_resources_with_chat_provider,
        create_test_server_resources_with_chat_provider,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::drained_turn::{
        compute_whatsapp_sig, create_active_user, link_channel, outbound_bodies,
        setup_whatsapp_config, wait_for_a_tracked_turn, wait_for_turns_to_finish,
        whatsapp_text_payload, HangingProvider, ParkedProvider,
    };
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// The coaching the athlete was owed. Distinctive enough that no other
    /// outbound row (a coach proposal, an intake question) can match it.
    const ANSWER: &str = "Ton NP sur la dernière course: 245 W, soit 3,4 W/kg — solide.";

    /// Seed one `WhatsApp` athlete and post one question through the real
    /// webhook route. Returns the tenant the session lives under.
    async fn ask_via_webhook(
        resources: &Arc<ServerContext>,
        email: &str,
        sender_id: &str,
        msg_id: &str,
    ) -> TenantId {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (user_id, tenant_id) = create_active_user(resources, email).await;
        let wa_secret = "wa_turn_lifecycle_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload = whatsapp_text_payload(
            sender_id,
            msg_id,
            "peux-tu sortir le NP de ma dernière course?",
        );
        let body_bytes = serde_json::to_vec(&payload).unwrap();
        let sig = compute_whatsapp_sig(wa_secret, &body_bytes);
        let router = MessagingRoutes::routes(Arc::clone(resources));

        let status = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &sig)
            .json(&payload)
            .send(router)
            .await
            .status_code();
        assert_eq!(status, StatusCode::OK, "webhooks always ack");
        tenant_id
    }

    fn text_of(content: &MessageContent) -> Option<&str> {
        match content {
            MessageContent::Text { body } | MessageContent::RichText { body } => Some(body),
            MessageContent::Media { .. }
            | MessageContent::Location { .. }
            | MessageContent::Card { .. } => None,
        }
    }

    /// SIGTERM, as the tracker sees it: a short grace, then the signal.
    async fn drain(resources: &ServerContext) {
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
            "the signalled turn must finish its hand-off inside its window, \
             got report: {report:?}"
        );
    }

    /// The 2026-08-26 incident, reproduced and closed the second time.
    ///
    /// An athlete asks a question, the turn parks on a provider that never
    /// answers, and the instance shuts down underneath it. registre#109
    /// made the drain visible and told the athlete the answer was not
    /// coming. This is what replaces that apology: the drained turn is
    /// recorded, the next instance's sweep claims it, and the athlete gets
    /// the coaching they asked for — once.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_drained_mid_flight_is_handed_off_and_answered_by_the_next_instance() {
        let provider = Arc::new(ParkedProvider::answering(ANSWER));
        let llm: Arc<dyn LlmProvider> = Arc::clone(&provider) as Arc<dyn LlmProvider>;
        let resources = create_test_server_resources_with_chat_provider(llm)
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let sender_id = "15550009002";
        let tenant_id = ask_via_webhook(
            &resources,
            "drained_mid_turn@example.com",
            sender_id,
            "wamid.drain_001",
        )
        .await;

        // 1. The turn the webhook started is countable.
        assert!(
            wait_for_a_tracked_turn(&resources).await,
            "the turn the webhook spawned must be visible to shutdown"
        );

        // 2. SIGTERM. The turn is parked on a provider that will not answer,
        //    so the grace expires and the drain signal ends it — as a
        //    hand-off, not a closing notice.
        drain(&resources).await;

        let notice = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
        let after_drain = outbound_bodies(db, tenant_id, sender_id).await;
        assert!(
            !after_drain.iter().any(|b| b == &notice),
            "a drained turn with an attempt left must not apologise, got {after_drain:?}"
        );
        assert!(
            !after_drain.iter().any(|b| b == ANSWER),
            "nothing was answered on the drained instance"
        );

        // 3. The next instance boots — a fresh process over the same rows,
        //    with its own tracker and an unfired drain token — and sweeps:
        //    the row is claimed and the same dispatch path runs again, this
        //    time to an answer. The provider the first run was parked on died
        //    with its instance; from here the provider answers.
        let calls_before_resume = provider.calls();
        provider.release();
        let next_instance = create_sibling_server_resources_with_chat_provider(
            &resources,
            Arc::clone(&provider) as Arc<dyn LlmProvider>,
        )
        .await
        .unwrap();
        let adapters = OfflineSendAdapters::default();
        let claimed = sweep_resumable_turns(&next_instance, &adapters).await;
        assert_eq!(claimed, 1, "the drained turn is on file for the sweep");
        assert!(
            wait_for_turns_to_finish(&next_instance, Duration::from_mins(1)).await,
            "the resumed turn must run to its end"
        );

        let bodies = outbound_bodies(db, tenant_id, sender_id).await;
        assert_eq!(
            bodies.iter().filter(|b| *b == ANSWER).count(),
            1,
            "the athlete gets their answer exactly once, got {bodies:?}"
        );
        assert!(
            !bodies.iter().any(|b| b == &notice),
            "no apology anywhere in a turn that was answered, got {bodies:?}"
        );
        let sent: Vec<String> = adapters
            .sends()
            .iter()
            .filter_map(|m| text_of(&m.content).map(str::to_owned))
            .collect();
        assert!(
            sent.iter().any(|b| b == ANSWER),
            "the answer must go through the channel adapter, sent: {sent:?}"
        );
        // How many calls the pipeline makes per turn is the pipeline's
        // business; what matters is that the resumed run reached the provider
        // at all, on top of whatever the drained run managed before it died.
        assert!(
            provider.calls() > calls_before_resume,
            "the resumed run must call the provider, got {} calls before and {} after",
            calls_before_resume,
            provider.calls()
        );

        // 4. Idempotent: the row is gone the moment the turn is answered, so
        //    a sibling sweeping later finds nothing and answers nobody twice.
        let again = sweep_resumable_turns(&next_instance, &OfflineSendAdapters::default()).await;
        assert_eq!(again, 0, "an answered turn is never resumed again");
    }

    /// Two scaledowns in a row: the hand-off is used up and the athlete is
    /// told, through the same channel, that the answer is not coming.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_drained_twice_is_apologised_for_and_finished() {
        let resources = create_test_server_resources_with_chat_provider(Arc::new(HangingProvider))
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let sender_id = "15550009004";
        let tenant_id = ask_via_webhook(
            &resources,
            "drained_twice@example.com",
            sender_id,
            "wamid.drain_002",
        )
        .await;
        assert!(wait_for_a_tracked_turn(&resources).await);

        // First drain: handed off, nothing said.
        drain(&resources).await;
        let notice = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
        assert!(
            !outbound_bodies(db, tenant_id, sender_id)
                .await
                .iter()
                .any(|b| b == &notice),
            "the first drain is a hand-off, not an apology"
        );

        // The next instance resumes it, on a provider that still never
        // answers.
        let next_instance = create_sibling_server_resources_with_chat_provider(
            &resources,
            Arc::new(HangingProvider),
        )
        .await
        .unwrap();
        let adapters = OfflineSendAdapters::default();
        assert_eq!(sweep_resumable_turns(&next_instance, &adapters).await, 1);
        assert!(
            wait_for_a_tracked_turn(&next_instance).await,
            "the resumed turn is countable like any other"
        );

        // Second drain, of the second instance: no attempt left, so the
        // placeholder the athlete is still looking at carries the notice,
        // and the row is finished.
        drain(&next_instance).await;
        let mut bodies = outbound_bodies(db, tenant_id, sender_id).await;
        for _ in 0..50 {
            if bodies.iter().any(|b| b == &notice) {
                break;
            }
            sleep(Duration::from_millis(100)).await;
            bodies = outbound_bodies(db, tenant_id, sender_id).await;
        }
        assert!(
            bodies.iter().any(|b| b == &notice),
            "a turn drained twice must tell the athlete its answer is not coming \
             (expected {notice:?}), got outbound bodies: {bodies:?}"
        );
        assert_eq!(
            sweep_resumable_turns(&next_instance, &OfflineSendAdapters::default()).await,
            0,
            "an apologised-for turn is finished, not re-run"
        );
    }
}
