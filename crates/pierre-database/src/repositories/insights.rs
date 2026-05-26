// ABOUTME: Repository trait definitions for the persisted insights domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use serde_json::Value;
use uuid::Uuid;

/// AI-generated insights storage repository
#[async_trait]
pub trait InsightRepository: Send + Sync {
    /// Store an AI-generated insight
    async fn store(&self, user_id: Uuid, insight_data: Value) -> AppResult<String>;
    /// Get insights for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<Value>>;
}
