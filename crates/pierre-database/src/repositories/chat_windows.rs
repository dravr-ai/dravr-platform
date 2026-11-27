// ABOUTME: A thread read in windows from the newest back, and the topics its dividers open (carnet#840)
// ABOUTME: The statements and the reads behind them, emitted per backend inside impl_chat_repository!, like chat_subject

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::chat::{message_columns, participant_scope};

/// One window of a conversation for a participant, newest first: the
/// newest `$5` rows ordered strictly before the `$4` message on
/// `(created_at, id)`, or the newest `$5` overall when `$4` is NULL. The
/// caller asks for one row more than it serves, to learn whether older rows
/// remain, and reverses the window.
pub(crate) const GET_MESSAGE_PAGE_SQL: &str = concat!(
    "SELECT ",
    message_columns!(),
    " FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND ",
    participant_scope!(),
    " AND ($4 IS NULL \
       OR m.created_at < (SELECT b.created_at FROM chat_messages b \
                          WHERE b.id = $4 AND b.conversation_id = $1) \
       OR (m.created_at = (SELECT b.created_at FROM chat_messages b \
                           WHERE b.id = $4 AND b.conversation_id = $1) \
           AND m.id < $4)) \
     ORDER BY m.created_at DESC, m.id DESC \
     LIMIT $5"
);

/// Whether participant `$2` reads row `m` as its custodian may: a row written
/// with its record, or, written before records existed, only by the
/// conversation's owner, who held it (carnet#1511 calls 11 and 17): what it
/// drew on cannot be known, so no one else is shown it.
const LEGACY_CUSTODIAN: &str = "(m.audience_recorded = TRUE OR c.user_id = $2)";

/// [`GET_MESSAGE_PAGE_SQL`] for a reader who may read only some of the reads
/// the thread's rows carry: the `readable` keys bind from `$7` on, then the
/// reader when `author` ([`ReadableReads::author`]), and `$6` is the divider
/// stamp. A row is left out when a row at or before it in the same topic
/// carries any other read ([`withheld_member_taint`]), so the window counts
/// only the rows served and the `before` cursor still names a row the reader
/// holds.
///
/// [`withheld_member_taint`]: super::chat_member_rows::withheld_member_taint
/// [`ReadableReads::author`]: super::chat_member_rows::ReadableReads::author
pub(crate) fn message_page_withholding_sql(readable: usize, author: bool) -> String {
    format!(
        "SELECT {} FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         JOIN conversation_participants p ON p.conversation_id = c.id \
         WHERE m.conversation_id = $1 AND {} \
         AND ($4 IS NULL \
           OR m.created_at < (SELECT b.created_at FROM chat_messages b \
                              WHERE b.id = $4 AND b.conversation_id = $1) \
           OR (m.created_at = (SELECT b.created_at FROM chat_messages b \
                               WHERE b.id = $4 AND b.conversation_id = $1) \
               AND m.id < $4)) \
         AND NOT {} AND {LEGACY_CUSTODIAN} \
         ORDER BY m.created_at DESC, m.id DESC \
         LIMIT $5",
        message_columns!(),
        participant_scope!(),
        super::chat_member_rows::withheld_member_taint(
            6,
            7,
            readable,
            author.then_some(7 + readable)
        ),
    )
}

/// Every row of a conversation a participant reads, oldest first, for a
/// reader who may read the `readable` keys, bound from `$5` on, then the
/// reader when `author`, `$4` the divider stamp: the rule
/// [`message_page_withholding_sql`] serves a window under, over the whole
/// transcript (carnet#1422).
pub(crate) fn readable_messages_sql(readable: usize, author: bool) -> String {
    format!(
        "SELECT {} FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         JOIN conversation_participants p ON p.conversation_id = c.id \
         WHERE m.conversation_id = $1 AND {} AND NOT {} AND {LEGACY_CUSTODIAN} \
         ORDER BY m.created_at ASC, m.id ASC",
        message_columns!(),
        participant_scope!(),
        super::chat_member_rows::withheld_member_taint(
            4,
            5,
            readable,
            author.then_some(5 + readable)
        ),
    )
}

/// Whether the row `$4` of a conversation restates a read its participant
/// may not read, the `readable` keys bound from `$6` on, then the reader when
/// `author`, `$5` the divider stamp: the rule [`readable_messages_sql`]
/// leaves a row out under, asked of one row.
pub(crate) fn message_withheld_sql(readable: usize, author: bool) -> String {
    format!(
        "SELECT m.id FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         JOIN conversation_participants p ON p.conversation_id = c.id \
         WHERE m.conversation_id = $1 AND m.id = $4 AND {} \
           AND ({} OR NOT {LEGACY_CUSTODIAN})",
        participant_scope!(),
        super::chat_member_rows::withheld_member_taint(
            5,
            6,
            readable,
            author.then_some(6 + readable)
        ),
    )
}

/// The divider rows of a conversation for a participant, newest first, each
/// with the athlete's first words after it and before the next divider. `$4`
/// is the divider stamp, `$5` the command stamp — a slash command is account
/// state, never a topic's title — and `$6` the most dividers read.
pub(crate) const LIST_DIVIDER_TOPICS_SQL: &str = concat!(
    "SELECT m.id, m.created_at, \
       (SELECT u.content FROM chat_messages u \
        WHERE u.conversation_id = m.conversation_id AND u.role = 'user' \
          AND (u.finish_reason IS NULL OR u.finish_reason <> $5) \
          AND (u.created_at > m.created_at OR (u.created_at = m.created_at AND u.id > m.id)) \
          AND NOT EXISTS (SELECT 1 FROM chat_messages n \
            WHERE n.conversation_id = m.conversation_id AND n.finish_reason = $4 \
              AND (n.created_at > m.created_at OR (n.created_at = m.created_at AND n.id > m.id)) \
              AND (n.created_at < u.created_at OR (n.created_at = u.created_at AND n.id < u.id))) \
        ORDER BY u.created_at ASC, u.id ASC LIMIT 1) AS first_words \
     FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND m.finish_reason = $4 AND ",
    participant_scope!(),
    " ORDER BY m.created_at DESC, m.id DESC \
     LIMIT $6"
);

/// The thread's opening, for a participant: when its first row was written,
/// and the athlete's first words before the first divider. No row when the
/// thread opens on a divider or holds nothing. `$4` is the divider stamp and
/// `$5` the command stamp.
pub(crate) const THREAD_OPENING_SQL: &str = concat!(
    "SELECT m.created_at, \
       (SELECT u.content FROM chat_messages u \
        WHERE u.conversation_id = m.conversation_id AND u.role = 'user' \
          AND (u.finish_reason IS NULL OR u.finish_reason <> $5) \
          AND NOT EXISTS (SELECT 1 FROM chat_messages n \
            WHERE n.conversation_id = m.conversation_id AND n.finish_reason = $4 \
              AND (n.created_at < u.created_at OR (n.created_at = u.created_at AND n.id < u.id))) \
        ORDER BY u.created_at ASC, u.id ASC LIMIT 1) AS first_words \
     FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND (m.finish_reason IS NULL OR m.finish_reason <> $4) AND ",
    participant_scope!(),
    " AND NOT EXISTS (SELECT 1 FROM chat_messages d \
         WHERE d.conversation_id = m.conversation_id AND d.finish_reason = $4 \
           AND (d.created_at < m.created_at OR (d.created_at = m.created_at AND d.id < m.id))) \
     ORDER BY m.created_at ASC, m.id ASC \
     LIMIT 1"
);

/// The newest `user` row of a conversation for a participant that is not a
/// slash command. `$4` is the command stamp.
pub(crate) const LATEST_USER_MESSAGE_SQL: &str = concat!(
    "SELECT m.id FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND m.role = 'user' \
       AND (m.finish_reason IS NULL OR m.finish_reason <> $4) AND ",
    participant_scope!(),
    " ORDER BY m.created_at DESC, m.id DESC \
     LIMIT 1"
);

/// The newest divider row of a conversation in a tenant. `$3` is the divider
/// stamp, bound rather than spelled so the one constant names it.
pub(crate) const LATEST_DIVIDER_SQL: &str = r"
    SELECT m.id FROM chat_messages m
    JOIN chat_conversations c ON c.id = m.conversation_id
    WHERE m.conversation_id = $1 AND c.tenant_id = $2 AND m.finish_reason = $3
    ORDER BY m.created_at DESC, m.id DESC
    LIMIT 1";

/// Emit the window and topic reads for one backend type: the bodies behind
/// [`ChatRepository::get_message_page`], [`ChatRepository::get_readable_messages`],
/// [`ChatRepository::message_withheld`], [`ChatRepository::list_topics`],
/// [`ChatRepository::get_recent_messages`] and
/// [`ChatRepository::latest_user_message_id`].
/// Invoked inside `impl_chat_repository!`, whose row parsers and helpers it
/// names unqualified.
///
/// [`ChatRepository::get_message_page`]: super::chat::ChatRepository::get_message_page
/// [`ChatRepository::get_readable_messages`]: super::chat::ChatRepository::get_readable_messages
/// [`ChatRepository::message_withheld`]: super::chat::ChatRepository::message_withheld
/// [`ChatRepository::list_topics`]: super::chat::ChatRepository::list_topics
/// [`ChatRepository::get_recent_messages`]: super::chat::ChatRepository::get_recent_messages
/// [`ChatRepository::latest_user_message_id`]: super::chat::ChatRepository::latest_user_message_id
macro_rules! impl_chat_windows {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// One window of a conversation for a participant, oldest first,
            /// with whether older rows remain.
            async fn message_page(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                before: Option<&str>,
                limit: i64,
                readable: Option<ReadableReads<'_>>,
            ) -> AppResult<MessagePage> {
                let withholding = readable.map(|readable| {
                    $crate::repositories::chat_windows::message_page_withholding_sql(
                        readable.keys.len(),
                        readable.author.is_some(),
                    )
                });
                let mut query = sqlx::query(
                    withholding
                        .as_deref()
                        .unwrap_or($crate::repositories::chat_windows::GET_MESSAGE_PAGE_SQL),
                )
                .bind(conversation_id)
                .bind($ids::bind_text(user_id)?)
                .bind(tenant_id.to_string())
                .bind(before)
                .bind(limit.saturating_add(1));
                if let Some(readable) = readable {
                    query = query.bind(DIVIDER_FINISH_REASON);
                    for key in readable.keys {
                        query = query.bind(key);
                    }
                    if let Some(author) = readable.author {
                        query = query.bind($ids::bind(author));
                    }
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to get a message page: {e}"))
                })?;
                let mut messages = rows
                    .iter()
                    .map(message_from_row)
                    .collect::<AppResult<Vec<_>>>()?;
                // One row past the window says older rows remain.
                let has_more = i64::try_from(messages.len()).is_ok_and(|n| n > limit);
                messages.truncate(usize::try_from(limit).unwrap_or(0));
                // Newest first from the statement; oldest first for the caller.
                messages.reverse();
                Ok(MessagePage { messages, has_more })
            }

            /// Every row of a conversation a participant reads, oldest
            /// first, none restating a read outside `readable`.
            async fn readable_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                readable: Option<ReadableReads<'_>>,
            ) -> AppResult<Vec<MessageRecord>> {
                let withholding = readable.map(|readable| {
                    $crate::repositories::chat_windows::readable_messages_sql(
                        readable.keys.len(),
                        readable.author.is_some(),
                    )
                });
                let mut query = sqlx::query(withholding.as_deref().unwrap_or(GET_MESSAGES_SQL))
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string());
                if let Some(readable) = readable {
                    query = query.bind(DIVIDER_FINISH_REASON);
                    for key in readable.keys {
                        query = query.bind(key);
                    }
                    if let Some(author) = readable.author {
                        query = query.bind($ids::bind(author));
                    }
                }
                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get messages: {e}")))?;
                rows.iter().map(message_from_row).collect()
            }

            /// Whether the row `message_id` restates a read outside `readable`.
            async fn message_withheld(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                message_id: &str,
                readable: ReadableReads<'_>,
            ) -> AppResult<bool> {
                let sql = $crate::repositories::chat_windows::message_withheld_sql(
                    readable.keys.len(),
                    readable.author.is_some(),
                );
                let mut query = sqlx::query(&sql)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(message_id)
                    .bind(DIVIDER_FINISH_REASON);
                for key in readable.keys {
                    query = query.bind(key);
                }
                if let Some(author) = readable.author {
                    query = query.bind($ids::bind(author));
                }
                let row = query.fetch_optional(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to check a withheld message: {e}"))
                })?;
                Ok(row.is_some())
            }

            /// The newest non-command `user` row's id, for a participant.
            async fn latest_user_message(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query($crate::repositories::chat_windows::LATEST_USER_MESSAGE_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(COMMAND_FINISH_REASON)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the latest user message: {e}"))
                    })?;
                row.map(|row| row.try_get("id").map_err(|e| chat_column_error("id", &e)))
                    .transpose()
            }

            /// The newest `limit` readable rows of a conversation for a
            /// participant, oldest first.
            async fn recent_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
                readable: TransportPolicy,
            ) -> AppResult<Vec<MessageRecord>> {
                let rows = sqlx::query(GET_RECENT_MESSAGES_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .bind(readable.is_first_party_only())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get recent messages: {e}"))
                    })?;
                // Newest first from the statement; chronological for the caller.
                let mut messages = rows
                    .iter()
                    .map(message_from_row)
                    .collect::<AppResult<Vec<_>>>()?;
                messages.reverse();
                Ok(messages)
            }

            /// A conversation's topics for a participant, newest first.
            async fn thread_topics(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<ThreadTopic>> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("Failed to list topics: {e}"));
                let user = $ids::bind_text(user_id)?;
                let tenant = tenant_id.to_string();
                let rows = sqlx::query($crate::repositories::chat_windows::LIST_DIVIDER_TOPICS_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(DIVIDER_FINISH_REASON)
                    .bind(COMMAND_FINISH_REASON)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(failed)?;
                let mut topics = rows
                    .iter()
                    .map(|row| {
                        Ok(ThreadTopic {
                            divider_id: Some(
                                row.try_get("id").map_err(|e| chat_column_error("id", &e))?,
                            ),
                            started_at: stamp_column(row, "created_at")?,
                            first_words: row
                                .try_get("first_words")
                                .map_err(|e| chat_column_error("first_words", &e))?,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                // The opening is the oldest topic, so it belongs only on a
                // list that reached the thread's first divider.
                if i64::try_from(topics.len()).is_ok_and(|n| n < limit) {
                    let opening =
                        sqlx::query($crate::repositories::chat_windows::THREAD_OPENING_SQL)
                            .bind(conversation_id)
                            .bind(&user)
                            .bind(&tenant)
                            .bind(DIVIDER_FINISH_REASON)
                            .bind(COMMAND_FINISH_REASON)
                            .fetch_optional(self.pool())
                            .await
                            .map_err(failed)?;
                    if let Some(row) = opening {
                        topics.push(ThreadTopic {
                            divider_id: None,
                            started_at: stamp_column(&row, "created_at")?,
                            first_words: row
                                .try_get("first_words")
                                .map_err(|e| chat_column_error("first_words", &e))?,
                        });
                    }
                }
                Ok(topics)
            }
        }
    };
}
pub(crate) use impl_chat_windows;
