// ABOUTME: A human coach's thread bound to one athlete (carnet#1070): the row it opens, the statements, and the attempt they run in
// ABOUTME: Emitted per backend inside impl_chat_repository!, like chat_rows; one thread per coach per athlete, settled by a unique index

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::{ConversationRecord, ConversationSubject, TenantId};

use super::chat::conversation_columns;

/// The thread [`ChatRepository::open_subject_conversation`] opens: a group's
/// human coach's own thread about one athlete they coach.
///
/// [`ChatRepository::open_subject_conversation`]: super::chat::ChatRepository::open_subject_conversation
#[derive(Debug, Clone, Copy)]
pub struct NewSubjectConversation<'a> {
    /// The coach, written as the `owner` participant too.
    pub user_id: &'a str,
    /// The coach's tenant, which owns the row.
    pub tenant_id: TenantId,
    /// Title a created thread starts with; a reopened one keeps its own.
    pub title: &'a str,
    /// Model a created thread runs on.
    pub model: &'a str,
    /// The agent of the group the coach opened the thread from.
    pub agent_id: &'a str,
    /// The athlete and the group that lets the coach act for them.
    pub subject: ConversationSubject,
}

/// What [`ChatRepository::open_subject_conversation`] did.
///
/// [`ChatRepository::open_subject_conversation`]: super::chat::ChatRepository::open_subject_conversation
#[derive(Debug, Clone)]
pub struct OpenedSubjectThread {
    /// The thread, as it stands now: active, and bound to the group and agent
    /// it was opened with.
    pub conversation: ConversationRecord,
    /// Whether this call created it rather than reopening the one the coach
    /// already had about this athlete.
    pub created: bool,
}

/// A coach's thread about one athlete, in a tenant. `$1` is the tenant, `$2`
/// the coach, `$3` the athlete.
pub(crate) const SUBJECT_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     WHERE c.tenant_id = $1 AND c.user_id = $2 AND c.subject_user_id = $3"
);

/// Reopen a coach's thread about one athlete: active again, and bound to the
/// group the coach opened it from and that group's agent. `$1` is the group,
/// `$2` the agent, `$3` the tenant, `$4` the coach, `$5` the athlete.
pub(crate) const REOPEN_SUBJECT_CONVERSATION_SQL: &str = r"
    UPDATE chat_conversations
    SET archived_at = NULL, subject_group_id = $1, agent_id = $2
    WHERE tenant_id = $3 AND user_id = $4 AND subject_user_id = $5";

/// Insert a coach's thread about one athlete. It is bound to no room and is
/// not the coach's personal thread, which is what keeps the row inside the
/// invariant `PostgreSQL` checks and `SQLite` does not.
pub(crate) const CREATE_SUBJECT_CONVERSATION_SQL: &str = r"
    INSERT INTO chat_conversations
        (id, user_id, tenant_id, title, model, agent_id, subject_user_id, subject_group_id,
         total_tokens, created_at, updated_at)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, $9, $9)";

/// Emit `open_subject_once` on one backend's repository type, with its uuid
/// codec and the statement its transaction opens with.
///
/// Invoked from inside [`impl_chat_repository!`](super::chat_backend::impl_chat_repository),
/// which names its helpers and types unqualified, so the invoking shell must
/// `use` every one of them.
macro_rules! impl_chat_subject_open {
    ($ty:ty, $ids:ident, $begin:literal) => {
        impl $ty {
            /// One attempt at
            /// [`ChatRepository::open_subject_conversation`], in one
            /// transaction: reopen the coach's thread about the athlete, or
            /// create it with its owner row. `None` when a racing open
            /// created it between this attempt's reopen and its insert — the
            /// unique index refused the second row — so the caller tries once
            /// more and reopens the winner.
            async fn open_subject_once(
                &self,
                new: &NewSubjectConversation<'_>,
            ) -> AppResult<Option<OpenedSubjectThread>> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to open the athlete's thread: {e}"))
                };
                let tenant = new.tenant_id.to_string();
                let user = $ids::bind_text(new.user_id)?;
                let athlete = $ids::bind(new.subject.athlete_user_id);
                let group = $ids::bind(new.subject.group_id);
                let mut tx = self.pool().begin_with($begin).await.map_err(failed)?;

                let reopened = sqlx::query(REOPEN_SUBJECT_CONVERSATION_SQL)
                    .bind(&group)
                    .bind(new.agent_id)
                    .bind(&tenant)
                    .bind(&user)
                    .bind(&athlete)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?
                    .rows_affected()
                    > 0;
                if !reopened {
                    let id = Uuid::new_v4().to_string();
                    let now = Utc::now();
                    let inserted = sqlx::query(CREATE_SUBJECT_CONVERSATION_SQL)
                        .bind(&id)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(new.title)
                        .bind(new.model)
                        .bind(new.agent_id)
                        .bind(&athlete)
                        .bind(&group)
                        .bind(now)
                        .execute(&mut *tx)
                        .await;
                    match inserted {
                        Ok(_) => {}
                        // Dropping the transaction rolls it back.
                        Err(e)
                            if e.as_database_error()
                                .is_some_and(|db| db.is_unique_violation()) =>
                        {
                            return Ok(None);
                        }
                        Err(e) => return Err(failed(e)),
                    }
                    sqlx::query(CREATE_OWNER_PARTICIPANT_SQL)
                        .bind(&id)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(ParticipantRole::Owner.as_str())
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                }

                let row = sqlx::query(SUBJECT_CONVERSATION_SQL)
                    .bind(&tenant)
                    .bind(&user)
                    .bind(&athlete)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(failed)?;
                let conversation = conversation_from_row(&row)?;
                tx.commit().await.map_err(failed)?;
                Ok(Some(OpenedSubjectThread {
                    conversation,
                    created: !reopened,
                }))
            }
        }
    };
}
pub(crate) use impl_chat_subject_open;
