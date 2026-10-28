// ABOUTME: Repository trait for the in-app feedback thread — what an athlete wrote to the Dravr team (carnet#1235)
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's feedback thread with the Dravr team.
//!
//! One thread per athlete and tenant, written by the athlete alone: nothing
//! answers it automatically, and no LLM reads it. The team reads each
//! message where `feedback.recorded` lands.
//!
//! Every statement is keyed by `tenant_id` and `user_id`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The agent reply a feedback message is about, when it was opened from a
/// thumbs-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackAbout {
    /// The conversation the reply belongs to.
    pub conversation_id: String,
    /// The reply itself.
    pub message_id: String,
}

/// A message to store in an athlete's feedback thread, borrowed from the
/// request that carried it.
#[derive(Debug)]
pub struct NewFeedbackMessage<'req> {
    /// Tenant the thread belongs to.
    pub tenant_id: TenantId,
    /// The athlete writing.
    pub user_id: Uuid,
    /// What they wrote, already trimmed and bounded by the caller.
    pub body: &'req str,
    /// The web route the thread was opened from.
    pub screen: Option<&'req str>,
    /// The client's build when it sent.
    pub app_version: Option<&'req str>,
    /// The client's language when it sent.
    pub locale: Option<&'req str>,
    /// The agent reply the message is about, if any.
    pub about: Option<&'req FeedbackAbout>,
    /// When it was sent.
    pub at: DateTime<Utc>,
}

/// One stored message of a feedback thread, as the athlete sees it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackMessage {
    /// Row id.
    pub id: String,
    /// What the athlete wrote.
    pub body: String,
    /// The agent reply it was about, if any.
    pub about: Option<FeedbackAbout>,
    /// When it was sent, to the millisecond.
    pub created_at: DateTime<Utc>,
}

/// The athletes' feedback threads with the Dravr team.
#[async_trait]
pub trait FeedbackMessageRepository: Send + Sync {
    /// Store one message in `message.user_id`'s thread and return it as a
    /// later read would.
    async fn insert_feedback_message(
        &self,
        message: &NewFeedbackMessage<'_>,
    ) -> AppResult<FeedbackMessage>;

    /// The newest `limit` messages of `user_id`'s thread in `tenant_id`,
    /// oldest first.
    async fn list_feedback_messages(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<FeedbackMessage>>;
}

pub(crate) const INSERT_FEEDBACK_MESSAGE_SQL: &str = r"
            INSERT INTO feedback_messages (
                id, tenant_id, user_id, body, screen, app_version, locale,
                about_conversation_id, about_message_id, created_at_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ";

/// Newest first so `LIMIT` keeps the latest window; the caller reverses it.
pub(crate) const LIST_FEEDBACK_MESSAGES_SQL: &str = r"
            SELECT id, body, about_conversation_id, about_message_id, created_at_ms
            FROM feedback_messages
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY created_at_ms DESC, id DESC
            LIMIT $3
            ";

/// A stored epoch-millisecond stamp as the instant it records.
///
/// # Errors
/// Returns a database error for a value outside chrono's range, which only a
/// hand-edited row can hold.
pub(crate) fn instant_from_millis(millis: i64) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(millis).ok_or_else(|| {
        AppError::database(format!(
            "feedback_messages.created_at_ms {millis} is out of range"
        ))
    })
}

/// The reply a row is about: both columns set, or neither.
pub(crate) fn about_from_columns(
    conversation_id: Option<String>,
    message_id: Option<String>,
) -> Option<FeedbackAbout> {
    match (conversation_id, message_id) {
        (Some(conversation_id), Some(message_id)) => Some(FeedbackAbout {
            conversation_id,
            message_id,
        }),
        _ => None,
    }
}

/// Emit the whole [`FeedbackMessageRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_feedback_message_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl FeedbackMessageRepository for $ty {
            async fn insert_feedback_message(
                &self,
                message: &NewFeedbackMessage<'_>,
            ) -> AppResult<FeedbackMessage> {
                let id = Uuid::new_v4().to_string();
                sqlx::query(INSERT_FEEDBACK_MESSAGE_SQL)
                    .bind(&id)
                    .bind($ids::bind(message.tenant_id.as_uuid()))
                    .bind($ids::bind(message.user_id))
                    .bind(message.body)
                    .bind(message.screen)
                    .bind(message.app_version)
                    .bind(message.locale)
                    .bind(message.about.map(|a| a.conversation_id.as_str()))
                    .bind(message.about.map(|a| a.message_id.as_str()))
                    .bind(message.at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store a feedback message: {e}"))
                    })?;
                Ok(FeedbackMessage {
                    id,
                    body: message.body.to_owned(),
                    about: message.about.cloned(),
                    created_at: instant_from_millis(message.at.timestamp_millis())?,
                })
            }

            async fn list_feedback_messages(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                limit: i64,
            ) -> AppResult<Vec<FeedbackMessage>> {
                let rows = sqlx::query(LIST_FEEDBACK_MESSAGES_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list feedback messages: {e}"))
                    })?;
                let column = |e: sqlx::Error| {
                    AppError::database(format!("Invalid feedback_messages column: {e}"))
                };
                let mut messages: Vec<FeedbackMessage> = rows
                    .iter()
                    .map(|row| {
                        Ok(FeedbackMessage {
                            id: row.try_get("id").map_err(column)?,
                            body: row.try_get("body").map_err(column)?,
                            about: about_from_columns(
                                row.try_get("about_conversation_id").map_err(column)?,
                                row.try_get("about_message_id").map_err(column)?,
                            ),
                            created_at: instant_from_millis(
                                row.try_get("created_at_ms").map_err(column)?,
                            )?,
                        })
                    })
                    .collect::<AppResult<_>>()?;
                messages.reverse();
                Ok(messages)
            }
        }
    };
}
pub(crate) use impl_feedback_message_repository;
