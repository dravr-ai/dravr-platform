// ABOUTME: Shared test helpers and utilities for integration tests
// ABOUTME: Exports synthetic data generation and common test utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod agent_fixtures;
pub mod axum_test;
pub mod chat_scenario;
/// A local stand-in for the Cloud Tasks API that records every task it is handed.
pub mod cloud_tasks_stub;
#[cfg(feature = "client-messaging")]
pub mod command_e2e;
/// Providers that hang, a `WhatsApp` athlete and the outbound ledger, for the
/// suites that interrupt a live turn (drain, watchdog).
#[cfg(feature = "client-messaging")]
pub mod drained_turn;
/// The first-party sign-in: authorization code + PKCE through the hosted login page.
#[cfg(feature = "protocol-rest")]
pub mod first_party_sign_in;
/// Google's OpenID Connect token endpoint and signing keys, for the hosted Google sign-in.
pub mod google_oidc;
/// A Google-shaped signing identity: openssl key + certificate, tokens minted with it.
pub mod google_token;
/// Identity Toolkit and the metadata token endpoint, for the account-deletion suites.
pub mod identity_toolkit_stub;
pub mod messaging_eval;
#[cfg(feature = "client-messaging")]
pub mod messaging_webhooks;
pub mod notify_capture;
/// A real adapter with only its outbound sends captured, for tests that
/// drive the webhook route and must not depend on reaching a channel's API.
#[cfg(feature = "client-messaging")]
pub mod offline_channel;
/// A model that records every request it is sent, for tests that assert on
/// what a turn put on the wire.
pub mod recording_llm;
pub mod sciotte_mock;
pub mod synthetic_data;
