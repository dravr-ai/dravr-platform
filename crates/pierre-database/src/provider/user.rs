// ABOUTME: User database operations trait covering user CRUD and profile management
// ABOUTME: Enables UserRepository blanket impl with focused trait bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantId, User, UserStatus};
use pierre_core::pagination::{CursorPage, PaginationParams};
use serde_json::Value;
use uuid::Uuid;

/// User management database operations
#[async_trait]
pub trait UserDbOps: Send + Sync + Clone {
    /// Create a new user account
    async fn create_user(&self, user: &User) -> AppResult<Uuid>;

    /// Get user by ID, scoped to a specific tenant for multi-tenant isolation
    async fn get_user(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>>;

    /// Get user by ID without tenant scoping (for system-level operations)
    ///
    /// SECURITY: Use only when tenant context is not available — authentication
    /// middleware, CLI tools, login/registration, and tenant resolution flows.
    /// Prefer `get_user()` with `tenant_id` in all user-facing routes.
    async fn get_user_global(&self, user_id: Uuid) -> AppResult<Option<User>>;

    /// Get user by email address
    async fn get_user_by_email(&self, email: &str) -> AppResult<Option<User>>;

    /// Get user by email (required - fails if not found)
    async fn get_user_by_email_required(&self, email: &str) -> AppResult<User>;

    /// Get user by Firebase UID
    async fn get_user_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>>;

    /// Update user's last active timestamp
    async fn update_last_active(&self, user_id: Uuid) -> AppResult<()>;

    /// Get total number of users
    async fn get_user_count(&self) -> AppResult<i64>;

    /// Get users by status (pending, active, suspended), optionally scoped to a tenant
    async fn get_users_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>>;

    /// Get users by status with cursor-based pagination
    async fn get_users_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>>;

    /// Update user status and approval information
    ///
    /// # Arguments
    /// * `user_id` - The user to update
    /// * `new_status` - The new status to set
    /// * `approved_by` - UUID of the admin user who approved (None for service token approvals)
    async fn update_user_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<User>;

    /// Update user's `tenant_id` to link them to a tenant
    async fn update_user_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;

    /// Update user's password hash
    async fn update_user_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()>;

    /// Update user's display name
    async fn update_user_display_name(&self, user_id: Uuid, display_name: &str) -> AppResult<User>;

    /// Delete a user and all associated data
    ///
    /// This permanently removes the user from the database.
    /// Associated data (tokens, conversations, etc.) are cascade deleted.
    async fn delete_user(&self, user_id: Uuid) -> AppResult<()>;

    /// Get the first admin user by creation date
    ///
    /// Used for system seeding to associate with a valid admin user
    async fn get_first_admin_user(&self) -> AppResult<Option<User>>;

    /// Upsert user profile data
    async fn upsert_user_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()>;

    /// Get user profile data
    async fn get_user_profile(&self, user_id: Uuid) -> AppResult<Option<Value>>;

    /// Create a new goal for a user
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> AppResult<String>;

    /// Get all goals for a user
    async fn get_user_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>>;

    /// Update progress on a goal, scoped to the owning user
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()>;

    /// Get user configuration data
    async fn get_user_configuration(&self, user_id: &str) -> AppResult<Option<String>>;

    /// Save user configuration data
    async fn save_user_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()>;

    /// Check if a user has synthetic activities seeded
    async fn user_has_synthetic_activities(&self, user_id: Uuid) -> AppResult<bool>;
}
