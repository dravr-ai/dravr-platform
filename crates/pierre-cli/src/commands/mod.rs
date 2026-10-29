// ABOUTME: Re-exports command modules for pierre-cli
// ABOUTME: Provides access to token, user, API key and tool management commands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub(crate) mod api_key;
pub(crate) mod auth;
pub(crate) mod canary;
pub(crate) mod config;
pub(crate) mod deletion;
pub(crate) mod drift;
pub(crate) mod fuel_ingest;
#[cfg(feature = "tools-verification")]
pub(crate) mod harness;
pub(crate) mod key;
pub(crate) mod provider_data;
pub(crate) mod seed;
#[cfg(feature = "e2e-seeded-sessions")]
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod signup_code;
pub(crate) mod strava_pool;
pub(crate) mod strava_webhook;
pub(crate) mod tenant;
pub(crate) mod token;
pub(crate) mod tool;
pub(crate) mod user;
pub(crate) mod user_admin;
pub(crate) mod zones;
