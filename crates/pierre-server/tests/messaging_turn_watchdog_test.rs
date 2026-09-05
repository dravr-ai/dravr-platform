// ABOUTME: Guards that a turn past its wall-clock ceiling is closed with the notice and never handed off
// ABOUTME: Its own binary because the ceiling is process-wide configuration the drain suite must not inherit
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The interruption that is NOT resumed.
//!
//! `run_bounded` ends a turn two ways: the shutdown drain, which is a healthy
//! turn on an instance that is going away and is therefore recorded and
//! re-run elsewhere (registre#126), and the watchdog, which is a turn that has
//! found something with no bound at all. Re-running a hung turn is not a fix,
//! so the watchdog keeps the closing notice of registre#109 and writes no
//! resumable row. This binary pins that split.
//!
//! `MESSAGING_TURN_WATCHDOG_SECS` is read from the process environment on
//! every turn, so the one-second ceiling set here would end every other
//! suite's turns too; that is why this test has a binary to itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod watchdog {
    use std::env;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
    use pierre_database::backends::MessagingRepository;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::messaging_ingress::resume::sweep_resumable_turns;
    use tokio::time::sleep;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::drained_turn::{
        compute_whatsapp_sig, create_active_user, link_channel, outbound_bodies,
        setup_whatsapp_config, wait_for_turns_to_finish, whatsapp_text_payload, HangingProvider,
    };
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// A hung turn is closed with the notice, and the resume sweep has
    /// nothing to claim afterwards.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_past_its_ceiling_is_apologised_for_not_handed_off() {
        env::set_var("MESSAGING_TURN_WATCHDOG_SECS", "1");

        let resources = create_test_server_resources_with_chat_provider(Arc::new(HangingProvider))
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) = create_active_user(&resources, "watchdog@example.com").await;
        let wa_secret = "wa_turn_watchdog_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;
        let sender_id = "15550009003";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload = whatsapp_text_payload(
            sender_id,
            "wamid.watchdog_001",
            "combien de watts sur ma dernière sortie?",
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

        // The provider never answers; the one-second ceiling is what ends the
        // turn. Nothing external interrupts it.
        assert!(
            wait_for_turns_to_finish(&resources, Duration::from_secs(30)).await,
            "the watchdog must end the hung turn on its own"
        );

        let expected = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
        let mut bodies = outbound_bodies(db, tenant_id, sender_id).await;
        for _ in 0..50 {
            if bodies.iter().any(|b| b == &expected) {
                break;
            }
            sleep(Duration::from_millis(100)).await;
            bodies = outbound_bodies(db, tenant_id, sender_id).await;
        }
        assert!(
            bodies.iter().any(|b| b == &expected),
            "a hung turn must tell the athlete its answer is not coming \
             (expected {expected:?}), got outbound bodies: {bodies:?}"
        );

        // No hand-off: a hung turn is not a healthy turn to run again.
        let adapters = OfflineSendAdapters::default();
        let claimed = sweep_resumable_turns(&resources, &adapters).await;
        assert_eq!(
            claimed, 0,
            "a watchdog interruption must not leave a resumable row behind"
        );
    }
}
