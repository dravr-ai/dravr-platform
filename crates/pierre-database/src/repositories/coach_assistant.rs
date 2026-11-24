// ABOUTME: What a member answers about their human coach's AI assistant (carnet#1511): trait, statements, shared body
// ABOUTME: The one-time coach tools notice and the roster opt-in, both on the member's own coaching_group_members row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! [`CoachAssistantRepository`], written once for both backends.
//!
//! A supertrait of [`CoachingGroupRepository`]: the same repository object
//! answers it, and the registry needs no second field. `group_id` and
//! `user_id` bind through the backend's uuid codec.

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::GroupMember;
use uuid::Uuid;

#[cfg(doc)]
use super::CoachingGroupRepository;

/// What a member answers about their human coach's AI assistant (carnet#1511).
#[async_trait]
pub trait CoachAssistantRepository: Send + Sync {
    /// When the live member `user_id` of `group_id` answered the one-time
    /// notice of their coach's AI tools; `None` while they have not, or when
    /// they are not a live member. No tenant filter, as
    /// [`CoachingGroupRepository::update_coach_sharing_consent`].
    async fn assistant_notice_answered_at(
        &self,
        group_id: &str,
        user_id: Uuid,
    ) -> AppResult<Option<Timestamp>>;

    /// Record that the live member `user_id` of `group_id` answered the
    /// one-time notice of their coach's AI tools at `at`. Returns `false`
    /// when `user_id` is not a live member. No tenant filter, as
    /// [`CoachingGroupRepository::update_coach_sharing_consent`].
    async fn answer_assistant_notice(
        &self,
        group_id: &str,
        user_id: Uuid,
        at: Timestamp,
    ) -> AppResult<bool>;

    /// The live member `user_id` of `group_id` turns on (`granted`) or off
    /// their human coach's AI assistant including them in the roster tools
    /// (carnet#1511), stamped `now` when the value changes. Scoped to the
    /// member's own row, so no coach can change it. Returns the membership as
    /// it now stands, or `None` when `user_id` is not a live member. No
    /// tenant filter, as [`CoachingGroupRepository::set_coach_consents`].
    async fn set_coach_assistant_read(
        &self,
        group_id: &str,
        user_id: Uuid,
        granted: bool,
        now: Timestamp,
    ) -> AppResult<Option<GroupMember>>;
}

/// When a live member answered the notice of their coach's AI tools, NULL
/// while they have not.
pub(crate) const GET_ASSISTANT_NOTICE_ANSWERED_SQL: &str = r"SELECT assistant_notice_answered_at FROM coaching_group_members
              WHERE group_id = $1 AND user_id = $2 AND left_at IS NULL";

/// Record that a live member answered the notice of their coach's AI tools,
/// at `$1`.
pub(crate) const ANSWER_ASSISTANT_NOTICE_SQL: &str = r"UPDATE coaching_group_members SET assistant_notice_answered_at = $1
              WHERE group_id = $2 AND user_id = $3 AND left_at IS NULL";

/// A live member turns on or off `$1` their human coach's AI assistant
/// including them in the roster tools, stamped `$2` when the value changes.
pub(crate) const SET_COACH_ASSISTANT_READ_SQL: &str = r"UPDATE coaching_group_members
        SET coach_assistant_read = $1,
            coach_assistant_read_at = CASE WHEN $1 = coach_assistant_read
                                           THEN coach_assistant_read_at ELSE $2 END
      WHERE group_id = $3 AND user_id = $4 AND left_at IS NULL";

/// Emit the [`CoachAssistantRepository`] implementation for one backend
/// type. `$ids` is the backend's uuid codec from [`super::uuid_columns`]; the
/// invoking shell must `use` every statement above, [`CoachingGroupRepository`]
/// and the decode helper `column` of [`super::coaching_groups`].
macro_rules! impl_coach_assistant_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl CoachAssistantRepository for $ty {
            async fn assistant_notice_answered_at(
                &self,
                group_id: &str,
                user_id: Uuid,
            ) -> AppResult<Option<Timestamp>> {
                let row = sqlx::query(GET_ASSISTANT_NOTICE_ANSWERED_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read when a member answered the coach tools notice: {e}"
                        ))
                    })?;
                row.map_or(Ok(None), |r| column(&r, "assistant_notice_answered_at"))
            }

            async fn answer_assistant_notice(
                &self,
                group_id: &str,
                user_id: Uuid,
                at: Timestamp,
            ) -> AppResult<bool> {
                let result = sqlx::query(ANSWER_ASSISTANT_NOTICE_SQL)
                    .bind(at)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record a member's answer to the coach tools notice: {e}"
                        ))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn set_coach_assistant_read(
                &self,
                group_id: &str,
                user_id: Uuid,
                granted: bool,
                now: Timestamp,
            ) -> AppResult<Option<GroupMember>> {
                let result = sqlx::query(SET_COACH_ASSISTANT_READ_SQL)
                    .bind(granted)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to set a member's coach assistant opt-in: {e}"
                        ))
                    })?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                self.get_member(group_id, user_id).await
            }
        }
    };
}
pub(crate) use impl_coach_assistant_repository;
