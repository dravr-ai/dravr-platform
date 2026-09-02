// ABOUTME: User-facing persona cards route — « Style de coaching » from the live contract registry
// ABOUTME: Thin router; all rendering logic lives in pierre_services::personas
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-facing persona cards route.
//!
//! One endpoint powers the « Style de coaching » settings surface:
//!
//! - `GET /api/personas` — one card per `CoachingPersona` variant with
//!   localized summary, contract-derived rules, and enforcement badge,
//!   rendered from the live persona-contract registry snapshot
//!
//! The handler lives in [`crate::services::personas`] so this route
//! module stays narrowly scoped to URL → handler wiring.

use std::sync::Arc;

use axum::{routing::get, Router};

use crate::mcp::resources::ServerContext;
use crate::services::personas::get_personas_handler;

/// User-facing persona cards routes.
pub struct PersonasRoutes;

impl PersonasRoutes {
    /// Mount the `/api/personas` endpoint onto a fresh router.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/personas", get(get_personas_handler))
            .with_state(resources)
    }
}
