// ABOUTME: Coach assignment and user preference operations (assign, unassign, hide, show)
// ABOUTME: Manages coach-to-user relationships and per-user visibility preferences
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::errors::{AppError, AppResult};
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::types::{Coach, CoachAssignment};
use super::{row_to_coach, CoachesManager};

impl CoachesManager {
    /// Get user-specific preferences for a coach from `coach_assignments`.
    ///
    /// Returns `(is_favorite, is_active, use_count, last_used_at)` tuple.
    /// If no assignment row exists, returns defaults `(false, false, 0, None)`.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, bool, u32, Option<DateTime<Utc>>)> {
        let row = sqlx::query(
            r"
            SELECT is_favorite, is_active, use_count, last_used_at
            FROM coach_assignments
            WHERE coach_id = $1 AND user_id = $2
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user preferences: {e}")))?;

        row.map_or(Ok((false, false, 0, None)), |r| {
            let is_favorite: i64 = r.get("is_favorite");
            let is_active: i64 = r.get("is_active");
            let use_count: i64 = r.get("use_count");
            let last_used_at_str: Option<String> = r.get("last_used_at");
            let last_used_at = last_used_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok((
                is_favorite == 1,
                is_active == 1,
                use_count as u32,
                last_used_at,
            ))
        })
    }

    /// Assign a coach to a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        // Use INSERT OR IGNORE to handle duplicates gracefully
        let result = sqlx::query(
            r"
            INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $4, $5, 0, 0, 0, NULL)
            ",
        )
        .bind(id.to_string())
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(assigned_by.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Unassign a coach from a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM coach_assignments
            WHERE coach_id = $1 AND user_id = $2
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// List all assignments for a coach (no tenant filtering).
    ///
    /// Used by tests where `tenant_users` table may not be set up.
    /// Production code should use `list_assignments_for_tenant` instead.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>> {
        let rows = sqlx::query(
            r"
            SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
            FROM coach_assignments ca
            LEFT JOIN users u ON ca.user_id = u.id
            WHERE ca.coach_id = $1
            ORDER BY ca.created_at DESC
            ",
        )
        .bind(coach_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;

        rows.iter()
            .map(|row| {
                let user_id: String = row.get("user_id");
                let created_at: String = row.get("created_at");
                let assigned_by: Option<String> = row.get("assigned_by");
                let user_email: Option<String> = row.get("email");

                Ok(CoachAssignment {
                    user_id,
                    user_email,
                    assigned_at: created_at,
                    assigned_by,
                })
            })
            .collect()
    }

    /// List assignments for a coach, scoped to a specific tenant.
    ///
    /// Only returns assignments where the assigned user belongs to the given tenant,
    /// preventing cross-tenant data leakage.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>> {
        let rows = sqlx::query(
            r"
            SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
            FROM coach_assignments ca
            LEFT JOIN users u ON ca.user_id = u.id
            INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
            WHERE ca.coach_id = $1
            ORDER BY ca.created_at DESC
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;

        rows.iter()
            .map(|row| {
                let user_id: String = row.get("user_id");
                let created_at: String = row.get("created_at");
                let assigned_by: Option<String> = row.get("assigned_by");
                let user_email: Option<String> = row.get("email");

                Ok(CoachAssignment {
                    user_id,
                    user_email,
                    assigned_at: created_at,
                    assigned_by,
                })
            })
            .collect()
    }

    // ============================================
    // User Coach Preferences Methods
    // ============================================

    /// Hide a coach from a user's view
    ///
    /// Only system or assigned coaches can be hidden (not personal coaches).
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        // Check if the coach is hideable (must be system or assigned, not personal)
        if !self.is_coach_hideable(coach_id, user_id).await? {
            return Err(AppError::invalid_input(
                "Only system or assigned coaches can be hidden",
            ));
        }

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        // Use INSERT OR REPLACE to update existing preference
        sqlx::query(
            r"
            INSERT INTO user_coach_preferences (id, user_id, coach_id, is_hidden, created_at)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT(user_id, coach_id) DO UPDATE SET is_hidden = 1
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(coach_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;

        Ok(true)
    }

    /// Show a previously hidden coach
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM user_coach_preferences
            WHERE coach_id = $1 AND user_id = $2 AND is_hidden = 1
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// List hidden coaches for a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(
            r"
            SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations
            FROM coaches c
            INNER JOIN user_coach_preferences ucp ON c.id = ucp.coach_id
            WHERE ucp.user_id = $1 AND ucp.is_hidden = 1 AND c.tenant_id = $2
            ORDER BY c.title
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;

        rows.iter().map(row_to_coach).collect()
    }

    /// Check if a coach can be hidden by a user
    ///
    /// A coach is hideable if it's a system coach or assigned to the user,
    /// but NOT if it's a personal coach created by the user.
    async fn is_coach_hideable(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        // Check if it's a system coach
        // System coaches are visible across all tenants, so no tenant_id restriction here
        let is_system = sqlx::query(
            r"
            SELECT 1 FROM coaches
            WHERE id = $1 AND is_system = 1
            ",
        )
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check system coach: {e}")))?
        .is_some();

        if is_system {
            return Ok(true);
        }

        // Check if it's assigned to the user
        let is_assigned = sqlx::query(
            r"
            SELECT 1 FROM coach_assignments
            WHERE coach_id = $1 AND user_id = $2
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
        .is_some();

        Ok(is_assigned)
    }
}
