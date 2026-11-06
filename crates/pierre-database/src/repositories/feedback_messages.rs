// ABOUTME: Repository trait for the in-app feedback thread — an athlete and the Dravr team writing to each other (carnet#1235)
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's feedback thread with the Dravr team.
//!
//! One thread per athlete and tenant. The athlete writes to it from the app;
//! the team replies from the feedback room in Slack (carnet#1392). No LLM
//! reads or answers it. A team reply stays unread until the athlete opens the
//! thread, which is what the web app's unread dot counts.
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

/// A screenshot the athlete attached to a message: the digest naming its
/// bytes in the blob store, and the media type it is served as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackImage {
    /// Lowercase hex SHA-256 of the bytes.
    pub sha256: String,
    /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
    pub media_type: String,
}

/// Who wrote a row of the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackAuthor {
    /// The athlete whose thread it is.
    Athlete,
    /// The Dravr team, replying from Slack.
    Team,
}

impl FeedbackAuthor {
    /// The stored spelling, which the `author` column's check constraint names.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Team => "team",
        }
    }

    /// Read a stored `author` back.
    ///
    /// # Errors
    /// Returns a database error for a value the check constraint would refuse,
    /// which only a hand-edited row can hold.
    pub fn from_stored(stored: &str) -> AppResult<Self> {
        match stored {
            "athlete" => Ok(Self::Athlete),
            "team" => Ok(Self::Team),
            other => Err(AppError::database(format!(
                "feedback_messages.author {other:?} is neither athlete nor team"
            ))),
        }
    }
}

/// Who is writing a new row: the athlete, or the team through the Slack user
/// who sent the reply. The replier is kept for the team's record and never
/// shown to the athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackWriter<'req> {
    /// The athlete, in their own thread.
    Athlete,
    /// The team, replying from Slack.
    Team {
        /// The Slack user who sent the reply.
        slack_user_id: &'req str,
    },
}

impl<'req> FeedbackWriter<'req> {
    /// The author the row is stored under.
    #[must_use]
    pub const fn author(self) -> FeedbackAuthor {
        match self {
            Self::Athlete => FeedbackAuthor::Athlete,
            Self::Team { .. } => FeedbackAuthor::Team,
        }
    }

    /// The Slack user behind a team reply.
    #[must_use]
    pub const fn replier_slack_user_id(self) -> Option<&'req str> {
        match self {
            Self::Athlete => None,
            Self::Team { slack_user_id } => Some(slack_user_id),
        }
    }
}

/// A message to store in an athlete's feedback thread, borrowed from the
/// request that carried it.
#[derive(Debug)]
pub struct NewFeedbackMessage<'req> {
    /// Tenant the thread belongs to.
    pub tenant_id: TenantId,
    /// The athlete whose thread it is.
    pub user_id: Uuid,
    /// Who is writing.
    pub writer: FeedbackWriter<'req>,
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
    /// The screenshot attached to it, already kept in the blob store.
    pub image: Option<&'req FeedbackImage>,
    /// When it was sent.
    pub at: DateTime<Utc>,
}

/// One stored message of a feedback thread, as the athlete sees it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackMessage {
    /// Row id.
    pub id: String,
    /// Who wrote it.
    pub author: FeedbackAuthor,
    /// What was written.
    pub body: String,
    /// The agent reply it was about, if any.
    pub about: Option<FeedbackAbout>,
    /// The screenshot attached to it, if any.
    pub image: Option<FeedbackImage>,
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

    /// The message `id` of `user_id`'s thread in `tenant_id`, if it is there.
    async fn get_feedback_message(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: &str,
    ) -> AppResult<Option<FeedbackMessage>>;

    /// How many team replies in `user_id`'s thread the athlete has not opened.
    async fn count_unread_team_replies(&self, tenant_id: TenantId, user_id: Uuid)
        -> AppResult<i64>;

    /// Mark every unread team reply in `user_id`'s thread read at `at`,
    /// returning how many there were.
    async fn mark_team_replies_read(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        at: DateTime<Utc>,
    ) -> AppResult<u64>;
}

pub(crate) const INSERT_FEEDBACK_MESSAGE_SQL: &str = r"
            INSERT INTO feedback_messages (
                id, tenant_id, user_id, author, replier_slack_user_id, body, screen,
                app_version, locale, about_conversation_id, about_message_id,
                image_sha256, image_media_type, created_at_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ";

/// Newest first so `LIMIT` keeps the latest window; the caller reverses it.
pub(crate) const LIST_FEEDBACK_MESSAGES_SQL: &str = r"
            SELECT id, author, body, about_conversation_id, about_message_id,
                   image_sha256, image_media_type, created_at_ms
            FROM feedback_messages
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY created_at_ms DESC, id DESC
            LIMIT $3
            ";

pub(crate) const GET_FEEDBACK_MESSAGE_SQL: &str = r"
            SELECT id, author, body, about_conversation_id, about_message_id,
                   image_sha256, image_media_type, created_at_ms
            FROM feedback_messages
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3
            ";

pub(crate) const COUNT_UNREAD_TEAM_REPLIES_SQL: &str = r"
            SELECT COUNT(*) AS unread
            FROM feedback_messages
            WHERE tenant_id = $1 AND user_id = $2 AND author = 'team' AND read_at_ms IS NULL
            ";

pub(crate) const MARK_TEAM_REPLIES_READ_SQL: &str = r"
            UPDATE feedback_messages
            SET read_at_ms = $3
            WHERE tenant_id = $1 AND user_id = $2 AND author = 'team' AND read_at_ms IS NULL
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

/// The screenshot a row names: both columns set, or neither.
pub(crate) fn image_from_columns(
    sha256: Option<String>,
    media_type: Option<String>,
) -> Option<FeedbackImage> {
    match (sha256, media_type) {
        (Some(sha256), Some(media_type)) => Some(FeedbackImage { sha256, media_type }),
        _ => None,
    }
}

/// Emit the whole [`FeedbackMessageRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres. `$row` is the backend's row type, which the one row
/// reader takes.
macro_rules! impl_feedback_message_repository {
    ($ty:ty, $ids:ident, $row:ty) => {
        /// One `feedback_messages` row as the thread shows it.
        fn feedback_message_from_row(row: &$row) -> AppResult<FeedbackMessage> {
            let column = |e: sqlx::Error| {
                AppError::database(format!("Invalid feedback_messages column: {e}"))
            };
            let author: String = row.try_get("author").map_err(column)?;
            Ok(FeedbackMessage {
                id: row.try_get("id").map_err(column)?,
                author: FeedbackAuthor::from_stored(&author)?,
                body: row.try_get("body").map_err(column)?,
                about: about_from_columns(
                    row.try_get("about_conversation_id").map_err(column)?,
                    row.try_get("about_message_id").map_err(column)?,
                ),
                image: image_from_columns(
                    row.try_get("image_sha256").map_err(column)?,
                    row.try_get("image_media_type").map_err(column)?,
                ),
                created_at: instant_from_millis(row.try_get("created_at_ms").map_err(column)?)?,
            })
        }

        #[async_trait::async_trait]
        impl FeedbackMessageRepository for $ty {
            async fn insert_feedback_message(
                &self,
                message: &NewFeedbackMessage<'_>,
            ) -> AppResult<FeedbackMessage> {
                // Time-ordered, and ordered within a millisecond in this
                // process, so the thread's `id` tiebreak keeps two messages
                // written in the same millisecond in the order they were sent.
                let id = Uuid::now_v7().to_string();
                let author = message.writer.author();
                sqlx::query(INSERT_FEEDBACK_MESSAGE_SQL)
                    .bind(&id)
                    .bind($ids::bind(message.tenant_id.as_uuid()))
                    .bind($ids::bind(message.user_id))
                    .bind(author.as_str())
                    .bind(message.writer.replier_slack_user_id())
                    .bind(message.body)
                    .bind(message.screen)
                    .bind(message.app_version)
                    .bind(message.locale)
                    .bind(message.about.map(|a| a.conversation_id.as_str()))
                    .bind(message.about.map(|a| a.message_id.as_str()))
                    .bind(message.image.map(|i| i.sha256.as_str()))
                    .bind(message.image.map(|i| i.media_type.as_str()))
                    .bind(message.at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store a feedback message: {e}"))
                    })?;
                Ok(FeedbackMessage {
                    id,
                    author,
                    body: message.body.to_owned(),
                    about: message.about.cloned(),
                    image: message.image.cloned(),
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
                let mut messages: Vec<FeedbackMessage> = rows
                    .iter()
                    .map(feedback_message_from_row)
                    .collect::<AppResult<_>>()?;
                messages.reverse();
                Ok(messages)
            }

            async fn get_feedback_message(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: &str,
            ) -> AppResult<Option<FeedbackMessage>> {
                let row = sqlx::query(GET_FEEDBACK_MESSAGE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a feedback message: {e}"))
                    })?;
                row.as_ref().map(feedback_message_from_row).transpose()
            }

            async fn count_unread_team_replies(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<i64> {
                let row = sqlx::query(COUNT_UNREAD_TEAM_REPLIES_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count unread team replies: {e}"))
                    })?;
                row.try_get("unread").map_err(|e| {
                    AppError::database(format!("Invalid unread team reply count: {e}"))
                })
            }

            async fn mark_team_replies_read(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                at: DateTime<Utc>,
            ) -> AppResult<u64> {
                let done = sqlx::query(MARK_TEAM_REPLIES_READ_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark team replies read: {e}"))
                    })?;
                Ok(done.rows_affected())
            }
        }
    };
}
pub(crate) use impl_feedback_message_repository;

#[cfg(test)]
mod tests {
    use super::{FeedbackAuthor, FeedbackWriter};

    #[test]
    fn an_author_reads_back_as_it_was_stored() {
        for author in [FeedbackAuthor::Athlete, FeedbackAuthor::Team] {
            assert_eq!(
                FeedbackAuthor::from_stored(author.as_str()).ok(),
                Some(author),
                "bug: {author:?} must round-trip through its stored spelling"
            );
        }
        assert!(
            FeedbackAuthor::from_stored("coach").is_err(),
            "bug: an author the check constraint refuses must not read as anyone"
        );
    }

    #[test]
    fn only_a_team_reply_names_its_replier() {
        assert_eq!(
            FeedbackWriter::Athlete.author(),
            FeedbackAuthor::Athlete,
            "bug: the athlete's row must be stored as theirs"
        );
        assert_eq!(
            FeedbackWriter::Athlete.replier_slack_user_id(),
            None,
            "bug: an athlete's row names no Slack replier"
        );

        let team = FeedbackWriter::Team {
            slack_user_id: "U0REPLIER",
        };
        assert_eq!(
            team.author(),
            FeedbackAuthor::Team,
            "bug: a reply from Slack must be stored as the team's"
        );
        assert_eq!(
            team.replier_slack_user_id(),
            Some("U0REPLIER"),
            "bug: a team reply must keep the Slack user who sent it"
        );
    }
}
