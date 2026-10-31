// ABOUTME: Terra unified API provider scaffold (feature-gated, not production-wired)
// ABOUTME: Webhook payloads parse into records the platform stores; reads come from the activity cache; no route is mounted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Terra Provider Module
//!
//! Terra is a unified API platform that provides access to 150+ wearables and health data sources
//! through a single integration. Unlike direct provider integrations (Strava, Garmin), Terra uses
//! a push-based webhook model where data is automatically sent to your endpoint when users sync
//! their devices.
//!
//! ## Production Status — NOT production-wired
//!
//! This module is **feature-gated scaffolding** behind the `provider-terra` cargo
//! feature, which is **not** part of the `server-full` / default production build —
//! it is only enabled by the `all-providers` / `production-providers` aggregate
//! features. It is intentionally incomplete and must not be presented as a live
//! integration:
//!
//! - **Stored in Dravr's own stores.** The webhook handler parses a payload
//!   into [`webhook::TerraRecords`]; the platform writes the activities
//!   through the activity cache and the wellness records through the
//!   wellness store, for the athlete the connection belongs to. The provider
//!   reads activities back from the activity cache (it is `PUSH_ONLY`).
//! - **No webhook route mounted.** No HTTP route in `pierre-server` /
//!   `pierre-routes-auth` accepts Terra webhook POSTs, so the
//!   [`webhook::TerraWebhookHandler`] is never invoked by the running server,
//!   and nothing reaches the stores from Terra at runtime.
//!
//! LIMITATION(registre#886): `TerraWebhookHandler` is never mounted — no route accepts Terra webhook POSTs.
//!
//! LIMITATION(registre#1061): `TerraProvider` has no tier gate, so a Starter athlete could connect Terra-routed devices billed per account.
//!
//! Treat the architecture diagram below as the *intended* design, not as wired
//! production behavior.
//!
//! ## Architecture
//!
//! ```text
//!   Terra API ──POST──▶ TerraWebhookHandler ──TerraRecords──▶ platform sink
//!                       (signature, parse)                    │
//!                                    activity cache ◀─────────┤
//!                                    wellness store ◀─────────┘
//!   TerraProvider ──reads──▶ activity cache rows under `terra`
//! ```
//!
//! ## Supported Data Types
//!
//! - **Activities**: Workouts, runs, rides, swims, etc.
//! - **Sleep**: Sleep sessions with stages (deep, light, REM, awake)
//! - **Body**: Weight, body fat percentage, BMI
//! - **Daily**: Daily activity summaries, steps, calories
//! - **Nutrition**: parsed, never stored (Dravr has no nutrition store)
//!
//! ## Webhook Events
//!
//! Terra pushes the following event types:
//! - `activity` - Completed workout data
//! - `sleep` - Sleep session data
//! - `body` - Body measurements
//! - `daily` - Daily activity metrics
//! - `nutrition` - Nutrition/food log data
//! - `auth` - Authentication status changes
//!
mod api_client;
mod converters;
mod provider;

// Public modules for external access
pub mod constants;
pub mod models;
pub mod webhook;

pub use api_client::{TerraApiClient, TerraApiConfig};
pub use converters::TerraConverters;
pub use models::{
    TerraActivity, TerraAthlete, TerraBody, TerraDaily, TerraNutrition, TerraSleep,
    TerraWebhookPayload,
};
pub use provider::{TerraDescriptor, TerraProvider, TerraProviderFactory};
pub use webhook::{
    SignatureValidation, TerraRecords, TerraWebhookHandler, WebhookResult,
    WebhookSignatureValidator,
};
