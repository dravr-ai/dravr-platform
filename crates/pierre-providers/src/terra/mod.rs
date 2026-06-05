// ABOUTME: Terra unified API provider scaffold (feature-gated, not production-wired)
// ABOUTME: Webhook-based ingestion into an in-memory-only cache; no webhook route is mounted
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
//! - **In-memory cache only.** Both [`TerraDataCache::new_in_memory`] and
//!   [`TerraDataCache::with_config`] back the cache with in-process `HashMap`s.
//!   Webhook data is held in process memory and is **lost on restart** — nothing
//!   is persisted to the database.
//! - **No webhook route mounted.** No HTTP route in `pierre-server` /
//!   `pierre-routes-auth` accepts Terra webhook POSTs, so the
//!   [`webhook::TerraWebhookHandler`] is never invoked by the running server. The
//!   provider therefore reads from a cache that nothing populates at runtime.
//!
//! Treat the architecture diagram below as the *intended* design, not as wired
//! production behavior.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                       Pierre MCP Server                      │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────────────────────────────────────────────────┐│
//! │  │                    TerraProvider                        ││
//! │  │  (implements FitnessProvider, reads from cache)         ││
//! │  └───────────────────────────┬─────────────────────────────┘│
//! │                              │                               │
//! │  ┌───────────────────────────▼─────────────────────────────┐│
//! │  │                    TerraDataCache                       ││
//! │  │       (In-memory storage for webhook data)              ││
//! │  └───────────────────────────┬─────────────────────────────┘│
//! │                              │                               │
//! │  ┌───────────────────────────▼─────────────────────────────┐│
//! │  │                 TerraWebhookHandler                     ││
//! │  │    (receives POST from Terra, validates signature)      ││
//! │  └───────────────────────────┬─────────────────────────────┘│
//! └──────────────────────────────┼───────────────────────────────┘
//!                                │ Webhook POST
//!                                ▼
//!                        ┌───────────────┐
//!                        │   Terra API   │
//!                        │ (150+ sources)│
//!                        └───────────────┘
//! ```
//!
//! ## Supported Data Types
//!
//! - **Activities**: Workouts, runs, rides, swims, etc.
//! - **Sleep**: Sleep sessions with stages (deep, light, REM, awake)
//! - **Body**: Weight, body fat percentage, BMI
//! - **Daily**: Daily activity summaries, steps, calories
//! - **Nutrition**: Food logs from integrations like MyFitnessPal
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
//! ## Example Usage
//!
//! ```rust,no_run
//! use pierre_providers::terra::{TerraProvider, TerraDataCache};
//! use pierre_providers::CoreFitnessProvider;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create cache and provider
//! let cache = Arc::new(TerraDataCache::new_in_memory());
//! let provider = TerraProvider::new(cache);
//!
//! // Provider reads from cache populated by webhook handler
//! let activities = provider.get_activities(Some(10), None).await?;
//! # Ok(())
//! # }
//! ```

mod api_client;
mod cache;
mod converters;
mod provider;

// Public modules for external access
pub mod constants;
pub mod models;
pub mod webhook;

pub use api_client::{TerraApiClient, TerraApiConfig};
pub use cache::TerraDataCache;
pub use converters::TerraConverters;
pub use models::{
    TerraActivity, TerraAthlete, TerraBody, TerraDaily, TerraNutrition, TerraSleep,
    TerraWebhookPayload,
};
pub use provider::{TerraDescriptor, TerraProvider, TerraProviderFactory};
pub use webhook::{
    SignatureValidation, TerraWebhookHandler, WebhookResult, WebhookSignatureValidator,
};
