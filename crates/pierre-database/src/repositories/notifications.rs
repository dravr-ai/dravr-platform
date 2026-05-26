// ABOUTME: Repository trait definitions for the OAuth + system notification persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::OAuthNotification;
use uuid::Uuid;

/// OAuth notification repository
#[async_trait]
pub trait NotificationRepository: Send + Sync {
    /// Store OAuth completion notification for MCP resource delivery
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String>;
    /// Get unread OAuth notifications for a user
    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>>;
    /// Mark OAuth notification as read
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// Mark all OAuth notifications as read for a user
    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64>;
    /// Get all OAuth notifications for a user (read and unread)
    async fn get_all(&self, user_id: Uuid, limit: Option<i64>)
        -> AppResult<Vec<OAuthNotification>>;
}
