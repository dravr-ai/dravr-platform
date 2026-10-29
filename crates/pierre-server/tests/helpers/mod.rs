// ABOUTME: Shared test helpers and utilities for integration tests
// ABOUTME: Exports synthetic data generation and common test utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub(crate) mod agent_fixtures;
pub(crate) mod axum_test;
pub(crate) mod chat_scenario;
/// A local stand-in for the Cloud Tasks API that records every task it is handed.
pub(crate) mod cloud_tasks_stub;
#[cfg(feature = "client-messaging")]
pub(crate) mod command_e2e;
/// The dravr-contremaitre tree the build compiled against, found without cargo.
pub(crate) mod contremaitre_checkout;
/// Providers that hang, a `WhatsApp` athlete and the outbound ledger, for the
/// suites that interrupt a live turn (drain, watchdog).
#[cfg(feature = "client-messaging")]
pub(crate) mod drained_turn;
/// An approved fuel-product label an athlete can carry (carnet#155).
pub(crate) mod fuel_fixtures;
/// The first-party sign-in: authorization code + PKCE through the hosted login page.
#[cfg(feature = "protocol-rest")]
pub(crate) mod first_party_sign_in;
/// Google's OpenID Connect token endpoint and signing keys, for the hosted Google sign-in.
pub(crate) mod google_oidc;
/// A Google-shaped signing identity: openssl key + certificate, tokens minted with it.
pub(crate) mod google_token;
/// Identity Toolkit and the metadata token endpoint, for the account-deletion suites.
pub(crate) mod identity_toolkit_stub;
/// Answers the turn's language-classification call so a scripted mock keeps
/// its script for the turn.
pub(crate) mod language_classification;
pub(crate) mod messaging_eval;
#[cfg(feature = "client-messaging")]
pub(crate) mod messaging_webhooks;
pub(crate) mod notify_capture;
/// A real adapter with only its outbound sends captured, for tests that
/// drive the webhook route and must not depend on reaching a channel's API.
#[cfg(feature = "client-messaging")]
pub(crate) mod offline_channel;
/// A model that records every request it is sent, for tests that assert on
/// what a turn put on the wire.
pub(crate) mod recording_llm;
pub(crate) mod sciotte_mock;
pub(crate) mod synthetic_data;
