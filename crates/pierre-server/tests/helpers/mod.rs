// ABOUTME: Shared test helpers and utilities for integration tests
// ABOUTME: Exports synthetic data generation and common test utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod axum_test;
pub mod chat_scenario;
pub mod coach_fixtures;
pub mod messaging_eval;
#[cfg(feature = "client-messaging")]
pub mod messaging_webhooks;
pub mod sciotte_mock;
pub mod synthetic_data;
