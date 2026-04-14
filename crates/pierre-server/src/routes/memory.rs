// ABOUTME: User-facing memory routes — list and forget what the coach remembers (GDPR)
// ABOUTME: Thin router; all business logic lives in services::memory_facts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-facing harness memory routes.
//!
//! Two endpoints power the user "what the coach remembers about me" panel:
//!
//! - `GET /api/memory/facts` — list the authenticated user's stored facts
//!   with optional `coach_id` and `kind` filters
//! - `DELETE /api/memory/facts/:fact_id` — GDPR-grade Forget for a single
//!   fact owned by the user
//!
//! The handlers themselves live in [`crate::services::memory_facts`] so
//! this route module stays narrowly scoped to URL → handler wiring.

use std::sync::Arc;

use axum::{
    routing::{delete, get},
    Router,
};

use crate::mcp::resources::ServerResources;
use crate::services::memory_facts::{forget_fact_handler, get_facts_handler};

/// User-facing harness memory routes.
pub struct MemoryRoutes;

impl MemoryRoutes {
    /// Mount the `/api/memory/facts` endpoints onto a fresh router.
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/api/memory/facts", get(get_facts_handler))
            .route("/api/memory/facts/{fact_id}", delete(forget_fact_handler))
            .with_state(resources)
    }
}
