// ABOUTME: Social database operations covering profiles, insights, and social features
// ABOUTME: Enables ProfileRepository and InsightRepository blanket impls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use serde_json::Value;
use uuid::Uuid;

/// Profile, insight, and social database operations
#[async_trait]
pub trait SocialDbOps: Send + Sync + Clone {
    // --- Insights & Analytics ---

    /// Store an AI-generated insight
    async fn store_insight(&self, user_id: Uuid, insight_data: Value) -> AppResult<String>;

    /// Get insights for a user
    async fn get_user_insights(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<Value>>;
}
