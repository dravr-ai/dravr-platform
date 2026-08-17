// ABOUTME: ServerActivityRefresher — turns a sweep refresh request into provider fetches
// ABOUTME: Thin seam holding the ToolRuntime handle the tool-runtime refresh machinery needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binary-side implementation of [`ActivityRefresher`].
//!
//! The sweep in `pierre_services::commitment_sweep` can read the activity
//! cache but cannot reach a provider; the authenticate-and-fetch machinery
//! lives in `pierre-tool-runtime` behind [`ToolRuntime`]. This seam owns the
//! composition-root runtime handle and forwards the sweep's request to
//! `commitment_refresh::spawn_commitment_activity_refresh`, which fetches the
//! commitment window from every provider the athlete has connected and writes
//! it through to the cache.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use pierre_services::commitment_sweep::ActivityRefresher;
use pierre_tool_runtime::commitment_refresh::spawn_commitment_activity_refresh;
use pierre_tool_runtime::ToolRuntime;
use uuid::Uuid;

/// [`ActivityRefresher`] backed by the shared tool runtime.
pub struct ServerActivityRefresher {
    /// Composition-root runtime the provider fetches authenticate through.
    /// `Arc` because every spawned refresh owns a handle for the life of its
    /// detached background task.
    runtime: Arc<dyn ToolRuntime>,
}

impl ServerActivityRefresher {
    /// Build the refresher over the composition-root runtime handle.
    #[must_use]
    pub fn new(runtime: Arc<dyn ToolRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl ActivityRefresher for ServerActivityRefresher {
    async fn request_refresh(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        window_start: DateTime<Utc>,
    ) -> bool {
        spawn_commitment_activity_refresh(
            Arc::clone(&self.runtime),
            user_id,
            *tenant_id,
            window_start.timestamp(),
        )
    }
}
