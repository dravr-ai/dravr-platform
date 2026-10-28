// ABOUTME: Repository trait, shared statements and body for a coach's review of the agent's group replies
// ABOUTME: The coach's page of replies, a flag, a correction posted into the room, and the weekly count (carnet#847)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach review of group replies (carnet#847).
//!
//! A reply the agent sent into a group is a `group_transcript_entries` row
//! with speaker `coach`, attributed to the member it answered. The group's
//! human coach (`coaching_groups.coach_user_id`) reads those replies, flags
//! one and posts a correction; `coach_reply_reviews` records each flag and
//! correction, and a correction also joins the room as a `correction` entry
//! linked to the reply it corrects.
//!
//! **Who reads what.** Every read names the coach: the groups are those whose
//! `coach_user_id` is the coach, in whichever tenant owns them — every account
//! is its own tenant, so the group of an athlete who invited a coach lives in
//! the athlete's tenant, never the coach's (carnet#1210) — and the replies are
//! those to a live member who shares with the coach (`coach_sharing_consent`),
//! the rule the roster and the room apply to a group's coach (ADR-002). The group's
//! peer switch governs what members see of each other, never what the coach
//! sees, so it plays no part. A reply outside that set is not found, whoever
//! asks for it by id.
//!
//! **Tenants.** `coach_reply_reviews.tenant_id` is the group's tenant: a flag
//! or a correction is written under the tenant of the group its reply was sent
//! into, and every read of it joins that group on its own tenant
//! (`r.tenant_id = g.tenant_id`), so a review row is never read outside the
//! group it belongs to. The transcript is read by group like every room read,
//! since membership is cross-tenant. The caller's own tenant scopes nothing
//! here: it is not the group's.
//!
//! uuid columns are `uuid` on Postgres and hyphenated `TEXT` on `SQLite`; they
//! bind and read through the shell's codec (`$ids`). `$n` placeholders
//! throughout: sqlx accepts them on `SQLite` as well as Postgres.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    CoachReply, CoachReplyCorrection, CoachReplyFilter, CoachReplyFlag, CoachReplyReview,
    NewReplyCorrection, NewReplyFlag, ReplyReviewStamp,
};
use uuid::Uuid;

use super::coaching_groups::person_name_column;

/// A coach's review of the agent's replies in the groups they coach.
#[async_trait]
pub trait CoachReplyReviewRepository: Send + Sync {
    /// The agent's replies in the groups `coach_user_id` coaches, whatever
    /// tenant owns them, newest first, narrowed and paged by `filter`. Only
    /// replies to a live member who shares with the coach are listed.
    async fn list_coach_replies(
        &self,
        coach_user_id: Uuid,
        filter: &CoachReplyFilter,
    ) -> AppResult<Vec<CoachReply>>;

    /// One reply the coach may review, by its transcript entry id; `None` for
    /// an entry that is not an agent reply in a group they coach, or one to a
    /// member who does not share with them.
    async fn get_coach_reply(
        &self,
        coach_user_id: Uuid,
        entry_id: Uuid,
    ) -> AppResult<Option<CoachReply>>;

    /// Record a flag on a reply and return it.
    async fn record_flag(&self, flag: &NewReplyFlag<'_>) -> AppResult<CoachReplyFlag>;

    /// Post a correction: its room entry, linked to the reply it corrects,
    /// and its review row, in one transaction.
    async fn record_correction(
        &self,
        correction: &NewReplyCorrection<'_>,
    ) -> AppResult<CoachReplyCorrection>;

    /// Every flag and correction the coach made, in a group they coach and
    /// under that group's tenant, on a reply sent between `oldest` and
    /// `newest` (inclusive), oldest first — the reviews a page of replies
    /// spanning that window can carry.
    async fn list_reviews_for_window(
        &self,
        coach_user_id: Uuid,
        oldest: DateTime<Utc>,
        newest: DateTime<Utc>,
    ) -> AppResult<Vec<CoachReplyReview>>;

    /// When each flag and correction the coach made since `since` was made,
    /// each read under its group's tenant, oldest first — what the weekly
    /// count buckets.
    async fn list_review_stamps(
        &self,
        coach_user_id: Uuid,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<ReplyReviewStamp>>;
}

/// The columns every reply read decodes, the athlete's name joined in.
macro_rules! coach_reply_columns {
    () => {
        concat!(
            r"e.id, e.group_id, e.tenant_id, e.author_user_id, e.content,
                     e.source_conversation_id, e.source_message_id, e.created_at,
                     e.first_party_only, g.name AS group_name,
                     g.tenant_id AS group_tenant_id, ",
            person_name_column!(),
            " AS athlete_name"
        )
    };
}

/// The replies coach `$1` may review: the agent's replies (`e`) in the active
/// groups (`g`) they coach, whatever tenant owns the group — the coach
/// relationship is the access key, as for
/// `CoachingGroupRepository::list_groups_coached_by` — to a live member who
/// shares with them, never a reply to the coach themself, who may also be a
/// member of a group they run.
macro_rules! coach_reply_scope {
    () => {
        r"g.coach_user_id = $1
                AND g.is_active = TRUE
                AND e.speaker = 'coach'
                AND e.author_user_id <> $1
                AND EXISTS (
                    SELECT 1 FROM coaching_group_members gm
                    WHERE gm.group_id = e.group_id
                      AND gm.user_id = e.author_user_id
                      AND gm.left_at IS NULL
                      AND gm.coach_sharing_consent = TRUE
                )"
    };
}

/// When the cursor entry `$4` was written, read only from a group coach `$1`
/// runs, so a cursor naming any other entry matches nothing.
macro_rules! coach_reply_cursor_created_at {
    () => {
        r"(
                    SELECT c.created_at FROM group_transcript_entries c
                    JOIN coaching_groups cg ON cg.id = c.group_id
                    WHERE c.id = $4 AND cg.coach_user_id = $1
                  )"
    };
}

/// One page of the coach's replies, newest first. `$2` narrows to one group
/// and `$3` to one athlete (NULL for any); `$4` is the id of the oldest reply
/// the coach already holds (NULL for the newest page), and the page holds the
/// replies ordered strictly before it on the `(created_at, id)` order it is
/// sorted by.
pub(crate) const LIST_COACH_REPLIES_SQL: &str = concat!(
    "SELECT ",
    coach_reply_columns!(),
    r"
              FROM group_transcript_entries e
              JOIN coaching_groups g ON g.id = e.group_id
              LEFT JOIN users u ON u.id = e.author_user_id
              WHERE ",
    coach_reply_scope!(),
    r"
                AND ($2 IS NULL OR e.group_id = $2)
                AND ($3 IS NULL OR e.author_user_id = $3)
                AND (
                  $4 IS NULL
                  OR e.created_at < ",
    coach_reply_cursor_created_at!(),
    r"
                  OR (
                    e.created_at = ",
    coach_reply_cursor_created_at!(),
    r"
                    AND e.id < $4
                  )
                )
              ORDER BY e.created_at DESC, e.id DESC
              LIMIT $5"
);

/// One reply the coach may review, by id (`$2`).
pub(crate) const GET_COACH_REPLY_SQL: &str = concat!(
    "SELECT ",
    coach_reply_columns!(),
    r"
              FROM group_transcript_entries e
              JOIN coaching_groups g ON g.id = e.group_id
              LEFT JOIN users u ON u.id = e.author_user_id
              WHERE ",
    coach_reply_scope!(),
    r"
                AND e.id = $2"
);

/// One flag.
pub(crate) const INSERT_FLAG_SQL: &str = r"INSERT INTO coach_reply_reviews
              (id, tenant_id, group_id, coach_user_id, athlete_user_id, reply_entry_id, kind,
               reason, note, correction_entry_id, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, 'flag', $7, $8, NULL, $9)";

/// A correction's room entry: attributed to the member the corrected reply
/// answered, written under the group's tenant, from no conversation.
pub(crate) const INSERT_CORRECTION_ENTRY_SQL: &str = r"INSERT INTO group_transcript_entries
              (id, group_id, tenant_id, author_user_id, speaker, content,
               source_conversation_id, source_message_id, created_at, first_party_only,
               corrects_entry_id)
              VALUES ($1, $2, $3, $4, 'correction', $5, NULL, NULL, $6, $7, $8)";

/// A correction's review row.
pub(crate) const INSERT_CORRECTION_REVIEW_SQL: &str = r"INSERT INTO coach_reply_reviews
              (id, tenant_id, group_id, coach_user_id, athlete_user_id, reply_entry_id, kind,
               reason, note, correction_entry_id, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, 'correction', NULL, NULL, $7, $8)";

/// Coach `$1`'s reviews on replies sent inside `[$2, $3]` in a group they
/// coach, with each correction's words read from the room entry it posted.
/// Each review row is read under its group's own tenant (`r.tenant_id =
/// g.tenant_id`), never the caller's.
pub(crate) const LIST_REVIEWS_FOR_WINDOW_SQL: &str = r"SELECT r.id, r.reply_entry_id, r.kind, r.reason, r.note,
              r.correction_entry_id, r.created_at, ce.content AS correction_content
              FROM coach_reply_reviews r
              JOIN coaching_groups g ON g.id = r.group_id AND g.tenant_id = r.tenant_id
              JOIN group_transcript_entries e ON e.id = r.reply_entry_id
              LEFT JOIN group_transcript_entries ce ON ce.id = r.correction_entry_id
              WHERE r.coach_user_id = $1
                AND g.coach_user_id = $1
                AND e.created_at >= $2
                AND e.created_at <= $3
              ORDER BY r.created_at ASC, r.id ASC";

/// When each of coach `$1`'s reviews since `$2` was made, each review row read
/// under its group's own tenant (`r.tenant_id = g.tenant_id`). Every review
/// the coach made counts, in a group they still coach or not: the count is of
/// their own work.
pub(crate) const LIST_REVIEW_STAMPS_SQL: &str = r"SELECT r.kind, r.created_at FROM coach_reply_reviews r
              JOIN coaching_groups g ON g.id = r.group_id AND g.tenant_id = r.tenant_id
              WHERE r.coach_user_id = $1 AND r.created_at >= $2
              ORDER BY r.created_at ASC";

/// Emit the whole [`CoachReplyReviewRepository`] implementation for one
/// backend type. `$row` is the driver's row type and `$ids` the backend's
/// codec in [`super::uuid_columns`]. The body names its consts and helpers
/// unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_coach_reply_review_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Decode a reply row.
            fn coach_reply_row(r: &$row) -> AppResult<CoachReply> {
                let conversation_id: Option<String> = column(r, "source_conversation_id")?;
                let message_id: Option<String> = column(r, "source_message_id")?;
                Ok(CoachReply {
                    entry_id: $ids::read(r, "id")?,
                    group_id: $ids::read(r, "group_id")?,
                    group_name: column(r, "group_name")?,
                    group_tenant_id: {
                        let tenant: String = column(r, "group_tenant_id")?;
                        TenantId::parse_str(&tenant).map_err(|e| {
                            AppError::database(format!("coaching_groups tenant is not a uuid: {e}"))
                        })?
                    },
                    conversation_tenant_id: column(r, "tenant_id")?,
                    athlete_user_id: $ids::read(r, "author_user_id")?,
                    athlete_name: column(r, "athlete_name")?,
                    content: column(r, "content")?,
                    // Only a turn row names a chat message: a row from no
                    // conversation carries no message id of Dravr's own.
                    message_id: message_id.filter(|_| conversation_id.is_some()),
                    conversation_id,
                    created_at: instant(r, "created_at")?,
                    transport_policy: TransportPolicy::from_first_party_only(column(
                        r,
                        "first_party_only",
                    )?),
                })
            }

            /// Decode a review row read with its correction's words.
            fn coach_review_row(r: &$row) -> AppResult<CoachReplyReview> {
                let kind: String = column(r, "kind")?;
                let id = $ids::read(r, "id")?;
                let reply_entry_id = $ids::read(r, "reply_entry_id")?;
                let created_at = instant(r, "created_at")?;
                match ReplyReviewKind::parse(&kind) {
                    Some(ReplyReviewKind::Flag) => {
                        let reason: Option<String> = column(r, "reason")?;
                        let reason = reason
                            .as_deref()
                            .and_then(ReplyFlagReason::parse)
                            .ok_or_else(|| {
                                AppError::database(format!(
                                    "coach_reply_reviews {id}: a flag without a known reason"
                                ))
                            })?;
                        Ok(CoachReplyReview::Flag(CoachReplyFlag {
                            id,
                            reply_entry_id,
                            reason,
                            note: column(r, "note")?,
                            created_at,
                        }))
                    }
                    Some(ReplyReviewKind::Correction) => {
                        let correction_entry_id =
                            $ids::read_opt(r, "correction_entry_id")?.ok_or_else(|| {
                                AppError::database(format!(
                                    "coach_reply_reviews {id}: a correction without its room entry"
                                ))
                            })?;
                        let content: Option<String> = column(r, "correction_content")?;
                        let content = content.ok_or_else(|| {
                            AppError::database(format!(
                                "coach_reply_reviews {id}: the correction's room entry is gone"
                            ))
                        })?;
                        Ok(CoachReplyReview::Correction(CoachReplyCorrection {
                            id,
                            reply_entry_id,
                            correction_entry_id,
                            content,
                            created_at,
                        }))
                    }
                    None => Err(AppError::database(format!(
                        "coach_reply_reviews column `kind` holds unknown value `{kind}`"
                    ))),
                }
            }
        }

        #[async_trait::async_trait]
        impl CoachReplyReviewRepository for $ty {
            async fn list_coach_replies(
                &self,
                coach_user_id: Uuid,
                filter: &CoachReplyFilter,
            ) -> AppResult<Vec<CoachReply>> {
                let rows = sqlx::query(LIST_COACH_REPLIES_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind($ids::bind_opt(filter.group_id))
                    .bind($ids::bind_opt(filter.athlete_user_id))
                    .bind($ids::bind_opt(filter.before))
                    .bind(filter.limit.max(1))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list coach replies: {e}")))?;
                rows.iter().map(Self::coach_reply_row).collect()
            }

            async fn get_coach_reply(
                &self,
                coach_user_id: Uuid,
                entry_id: Uuid,
            ) -> AppResult<Option<CoachReply>> {
                let row = sqlx::query(GET_COACH_REPLY_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind($ids::bind(entry_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read a coach reply: {e}")))?;
                row.map(|row| Self::coach_reply_row(&row)).transpose()
            }

            async fn record_flag(&self, flag: &NewReplyFlag<'_>) -> AppResult<CoachReplyFlag> {
                let id = Uuid::new_v4();
                let created_at = Utc::now();
                sqlx::query(INSERT_FLAG_SQL)
                    .bind($ids::bind(id))
                    .bind(flag.tenant_id.to_string())
                    .bind($ids::bind(flag.group_id))
                    .bind($ids::bind(flag.coach_user_id))
                    .bind($ids::bind(flag.athlete_user_id))
                    .bind($ids::bind(flag.reply_entry_id))
                    .bind(flag.reason.as_str())
                    .bind(flag.note)
                    .bind(created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to record a reply flag: {e}")))?;
                Ok(CoachReplyFlag {
                    id,
                    reply_entry_id: flag.reply_entry_id,
                    reason: flag.reason,
                    note: flag.note.map(str::to_owned),
                    created_at,
                })
            }

            async fn record_correction(
                &self,
                correction: &NewReplyCorrection<'_>,
            ) -> AppResult<CoachReplyCorrection> {
                let id = Uuid::new_v4();
                let correction_entry_id = Uuid::new_v4();
                let created_at = Utc::now();
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin a reply correction: {e}"))
                })?;
                sqlx::query(INSERT_CORRECTION_ENTRY_SQL)
                    .bind($ids::bind(correction_entry_id))
                    .bind($ids::bind(correction.group_id))
                    .bind(correction.tenant_id.to_string())
                    .bind($ids::bind(correction.athlete_user_id))
                    .bind(correction.content)
                    .bind(created_at)
                    .bind(correction.transport_policy.is_first_party_only())
                    .bind($ids::bind(correction.reply_entry_id))
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to post a correction to the room: {e}"))
                    })?;
                sqlx::query(INSERT_CORRECTION_REVIEW_SQL)
                    .bind($ids::bind(id))
                    .bind(correction.tenant_id.to_string())
                    .bind($ids::bind(correction.group_id))
                    .bind($ids::bind(correction.coach_user_id))
                    .bind($ids::bind(correction.athlete_user_id))
                    .bind($ids::bind(correction.reply_entry_id))
                    .bind($ids::bind(correction_entry_id))
                    .bind(created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a reply correction: {e}"))
                    })?;
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit a reply correction: {e}"))
                })?;
                Ok(CoachReplyCorrection {
                    id,
                    reply_entry_id: correction.reply_entry_id,
                    correction_entry_id,
                    content: correction.content.to_owned(),
                    created_at,
                })
            }

            async fn list_reviews_for_window(
                &self,
                coach_user_id: Uuid,
                oldest: DateTime<Utc>,
                newest: DateTime<Utc>,
            ) -> AppResult<Vec<CoachReplyReview>> {
                let rows = sqlx::query(LIST_REVIEWS_FOR_WINDOW_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind(oldest)
                    .bind(newest)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list reply reviews: {e}"))
                    })?;
                rows.iter().map(Self::coach_review_row).collect()
            }

            async fn list_review_stamps(
                &self,
                coach_user_id: Uuid,
                since: DateTime<Utc>,
            ) -> AppResult<Vec<ReplyReviewStamp>> {
                let rows = sqlx::query(LIST_REVIEW_STAMPS_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind(since)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count reply reviews: {e}"))
                    })?;
                rows.iter()
                    .map(|r| {
                        let kind: String = column(r, "kind")?;
                        Ok(ReplyReviewStamp {
                            kind: ReplyReviewKind::parse(&kind).ok_or_else(|| {
                                AppError::database(format!(
                                    "coach_reply_reviews column `kind` holds unknown value `{kind}`"
                                ))
                            })?,
                            created_at: instant(r, "created_at")?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_coach_reply_review_repository;
