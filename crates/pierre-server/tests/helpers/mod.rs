// ABOUTME: Shared test helpers and utilities for integration tests
// ABOUTME: Exports synthetic data generation and common test utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod axum_test;
pub mod chat_scenario;
/// A local stand-in for the Cloud Tasks API that records every task it is handed.
pub mod cloud_tasks_stub;
pub mod coach_fixtures;
#[cfg(feature = "client-messaging")]
pub mod command_e2e;
/// Providers that hang, a `WhatsApp` athlete and the outbound ledger, for the
/// suites that interrupt a live turn (drain, watchdog).
#[cfg(feature = "client-messaging")]
pub mod drained_turn;
/// A Google-shaped signing identity: openssl key + certificate, tokens minted with it.
pub mod google_token;
pub mod messaging_eval;
#[cfg(feature = "client-messaging")]
pub mod messaging_webhooks;
pub mod notify_capture;
/// A real adapter with only its outbound sends captured, for tests that
/// drive the webhook route and must not depend on reaching a channel's API.
#[cfg(feature = "client-messaging")]
pub mod offline_channel;
pub mod sciotte_mock;
pub mod synthetic_data;
