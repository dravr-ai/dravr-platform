// ABOUTME: A thread's rows about group members (carnet#1174, carnet#1422): the prompt read that keeps only the readable ones
// ABOUTME: The statements and the reads behind them, emitted per backend inside impl_chat_repository!, like chat_windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use uuid::Uuid;

use super::chat::{message_columns, participant_scope};

/// The key of link row `$alias` of `chat_message_members` or
/// `group_transcript_entry_members`: how its member was read, as
/// [`CarriedRead::key`](pierre_core::models::CarriedRead::key) writes it, so a
/// statement asks only whether a row's key is one the reader's rule admitted
/// (carnet#1511 call 19). A coach's write onto the member is stored as
/// `coach` with `written` set and keyed `written` (call 35). Never NULL.
macro_rules! carried_read_key {
    ($alias:literal) => {
        concat!(
            "(CASE WHEN ",
            $alias,
            ".written THEN 'written' ELSE ",
            $alias,
            ".via_kind END || '|' || CAST(",
            $alias,
            ".member_user_id AS TEXT) || '|' || COALESCE(CAST(",
            $alias,
            ".via_group AS TEXT), '') || '|' || COALESCE(CAST(",
            $alias,
            ".via_coach AS TEXT), '') || '|' || CASE WHEN ",
            $alias,
            ".beyond_activities THEN 'beyond_activities' ELSE ",
            $alias,
            ".data_class END || '|' || COALESCE(",
            $alias,
            ".tier, ''))"
        )
    };
}
pub(crate) use carried_read_key;

/// `expr IN` the `count` placeholders bound from `$first` on, or `FALSE`
/// when there are none: whether a key is one of the keys a reader may read.
/// The count is all that is formatted; the keys bind as values.
pub(crate) fn key_in(expr: &str, first: usize, count: usize) -> String {
    if count == 0 {
        return "FALSE".to_owned();
    }
    let placeholders = (first..first + count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{expr} IN ({placeholders})")
}

/// What of a conversation's rows a turn's prompt may replay (carnet#1174,
/// carnet#1422, carnet#1511 call 17).
///
/// A row is about every member a row at or before it in the conversation
/// carries: a reply drawn from replayed history restates it, and so does
/// every reply drawn from that one. So a row replays only when it comes
/// before the first row carrying a read the replaying turn may not read
/// (`tainted_from`, decided by the caller from [`RowRead`]s). A row written
/// before carried reads existed names one member in `about_user_id`, and
/// replays only when that member is one of `readable_about`; and no such row
/// replays at all where `recorded_only` (a room, or a thread others read). The
/// turn's own row always replays.
#[derive(Clone, Copy)]
pub struct MemberReplay<'reads> {
    /// The members a legacy row may be about and replay.
    pub readable_about: &'reads [Uuid],
    /// The first row carrying a read the turn may not read: no row at or
    /// after it replays.
    pub tainted_from: Option<&'reads str>,
    /// Whether no row but the turn's own replays: what the turn may read
    /// could not be decided.
    pub withhold_all: bool,
    /// Whether only rows written with their record replay: a room, or a
    /// thread others read, never replays a legacy row (call 17).
    pub recorded_only: bool,
    /// The turn's own row, which always replays.
    pub current: Option<&'reads str>,
}

impl MemberReplay<'_> {
    /// A replay of rows about nobody, with no taint: what a caller that
    /// reads no member's rows asks for.
    #[must_use]
    pub const fn about_nobody() -> Self {
        Self {
            readable_about: &[],
            tainted_from: None,
            withhold_all: false,
            recorded_only: false,
            current: None,
        }
    }
}

// Ids only: what a turn may replay, never a row's content.
impl fmt::Debug for MemberReplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemberReplay")
            .field("readable_about", &self.readable_about.len())
            .field("tainted_from", &self.tainted_from)
            .field("withhold_all", &self.withhold_all)
            .field("recorded_only", &self.recorded_only)
            .field("current", &self.current)
            .finish()
    }
}

/// What of a thread's rows carrying member reads one reader is served on
/// their own screen, their export, or a durable read of the thread.
#[derive(Clone, Copy, Debug)]
pub struct ReadableReads<'reads> {
    /// The keys of the carried reads the reader may read
    /// ([`CarriedRead::key`](pierre_core::models::CarriedRead::key), or a
    /// legacy row's `about` key): a row carrying any other is withheld, with
    /// every platform row after it in its topic (carnet#1422).
    pub keys: &'reads [String],
    /// The reader, when a row they wrote keeps the peer reads it carries for
    /// them: a member's withdrawal never hides from its author what the room
    /// showed them (carnet#1511 call 20). A coach read is never exempt: the
    /// coach loses it with the member's consent (call 2).
    pub author: Option<Uuid>,
}

/// One read a conversation's row carries, by key, oldest first.
///
/// The key is a carried
/// read's ([`CarriedRead::key`](pierre_core::models::CarriedRead::key)), a
/// legacy row's `about` member ([`CarriedRead::legacy_about_key`]), or the
/// row's author when another participant wrote it (`author|<id>`): their own
/// words, which a replay of the row reads (carnet#1511 R2).
///
/// [`CarriedRead::legacy_about_key`]: pierre_core::models::CarriedRead::legacy_about_key
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowRead {
    /// The row.
    pub message_id: String,
    /// The read it carries.
    pub key: String,
}

/// The key prefix of a row's author when another participant wrote it.
pub const AUTHOR_READ_PREFIX: &str = "author|";

/// The key of a legacy platform row that may carry reads no row names.
///
/// Per carnet#1511 A8, such a row was written before rows carried their reads, naming no
/// member, by the platform (never a `user` row, the caller's own words, nor
/// a topic divider), in a conversation where a legacy row names a member
/// other than the reader. Such a row named its member only when its turn
/// read exactly one, so such a row may have drawn on several, and none can be
/// told. No reader's rule admits it: what it carries is unknown.
pub const UNATTRIBUTED_READ_KEY: &str = "unattributed|";

/// [`GET_RECENT_MESSAGES_SQL`] for a turn's prompt, under a
/// [`MemberReplay`]: a legacy row about a group member only when the member
/// is one of the `readable_about` ids bound from `$10` on (carnet#1174), with
/// none only legacy rows about nobody; no legacy row at all when `$9`
/// (`recorded_only`); and no row at or after the tainted row `$6`, nor any
/// when `$8`, but the turn's own `$7` (carnet#1422). One placeholder per id,
/// as `users_by_ids_sql` binds its batch, so the ids bind through the codec
/// on both drivers: the count is all that is formatted.
///
/// [`GET_RECENT_MESSAGES_SQL`]: super::chat::GET_RECENT_MESSAGES_SQL
pub(crate) fn recent_prompt_messages_sql(readable_about: usize) -> String {
    let about = if readable_about == 0 {
        "m.about_user_id IS NULL".to_owned()
    } else {
        let placeholders = (10..10 + readable_about)
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("(m.about_user_id IS NULL OR m.about_user_id IN ({placeholders}))")
    };
    format!(
        "SELECT {} FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         JOIN conversation_participants p ON p.conversation_id = c.id \
         WHERE m.conversation_id = $1 AND (m.first_party_only = FALSE OR $5) AND {about} AND {} \
         AND (m.audience_recorded = TRUE OR NOT $9 OR m.id = $7) \
         AND (m.id = $7 OR ($6 IS NULL AND NOT $8) \
              OR (m.created_at, m.id) < (SELECT t.created_at, t.id FROM chat_messages t \
                                         WHERE t.id = $6 AND t.conversation_id = $1)) \
         ORDER BY m.created_at DESC, m.id DESC \
         LIMIT $4",
        message_columns!(),
        participant_scope!(),
    )
}

/// Every read a conversation's rows carry, by key, with the row's place:
/// its carried reads (`chat_message_members`), the member a legacy row names
/// (`about_user_id`), and its author when the row is another participant's
/// (`$2` the reader). The union the replay walks and the screen withholds by.
macro_rules! thread_row_reads {
    () => {
        concat!(
            "SELECT m.id AS rid, m.created_at AS rat, '",
            "about|",
            "' || CAST(m.about_user_id AS TEXT) AS rkey, 'about' AS rway, \
                    m.author_user_id AS rauthor \
             FROM chat_messages m \
             WHERE m.conversation_id = $1 AND m.about_user_id IS NOT NULL \
             UNION ALL \
             SELECT m.id AS rid, m.created_at AS rat, ",
            carried_read_key!("mm"),
            " AS rkey, mm.via_kind AS rway, m.author_user_id AS rauthor \
             FROM chat_message_members mm JOIN chat_messages m ON m.id = mm.message_id \
             WHERE m.conversation_id = $1"
        )
    };
}

/// Every read a conversation's rows carry since its newest divider, oldest
/// first, for a participant ([`RowRead`]): the carried reads, the member a
/// legacy row names, the author of each row another participant wrote, and
/// each legacy row whose reads no row names ([`UNATTRIBUTED_READ_KEY`]).
/// Only rows after the newest divider (`$4` its finish reason): "+" and
/// `/reset` open a topic the model starts over, so nothing before one is
/// restated after it.
pub(crate) const MEMBER_READS_SQL: &str = concat!(
    "SELECT r.rid AS message_id, r.rkey AS rkey FROM ( ",
    thread_row_reads!(),
    " UNION ALL \
     SELECT m.id AS rid, m.created_at AS rat, '",
    "author|",
    "' || CAST(m.author_user_id AS TEXT) AS rkey, 'author' AS rway, \
            m.author_user_id AS rauthor \
     FROM chat_messages m \
     WHERE m.conversation_id = $1 AND m.author_user_id IS NOT NULL \
       AND m.author_user_id <> $2 \
     UNION ALL \
     SELECT m.id AS rid, m.created_at AS rat, '",
    "unattributed|",
    "' AS rkey, 'unattributed' AS rway, m.author_user_id AS rauthor \
     FROM chat_messages m \
     WHERE m.conversation_id = $1 AND m.audience_recorded = FALSE \
       AND m.about_user_id IS NULL AND m.role <> 'user' \
       AND COALESCE(m.finish_reason, '') <> $4 \
       AND EXISTS (SELECT 1 FROM chat_messages n \
                   WHERE n.conversation_id = $1 AND n.audience_recorded = FALSE \
                     AND n.about_user_id IS NOT NULL AND n.about_user_id <> $2) \
     ) r \
     JOIN chat_conversations c ON c.id = $1 \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE NOT EXISTS ( \
         SELECT 1 FROM chat_messages d \
         WHERE d.conversation_id = $1 AND d.finish_reason = $4 \
           AND (d.created_at, d.id) > (r.rat, r.rid) \
     ) AND ",
    participant_scope!(),
    " ORDER BY r.rat, r.rid, r.rkey"
);

/// Every read a conversation's rows carry, in any topic, by key, for a
/// participant: the reads whose readers decide what of the whole transcript
/// a reader is served.
pub(crate) const THREAD_READ_KEYS_SQL: &str = concat!(
    "SELECT DISTINCT r.rkey AS rkey FROM ( ",
    thread_row_reads!(),
    " ) r \
     JOIN chat_conversations c ON c.id = $1 \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE ",
    participant_scope!()
);

/// Every row of the conversation `$1` its owner `$2` may file as durable
/// text, oldest first (carnet#1511 A20): the conversation is theirs (its
/// custodian), the row names no legacy member and carries no read of anyone
/// but them, and either it was written with its record (`audience_recorded`)
/// by them (`author_user_id`, which is the sender for the agent's reply too),
/// or it was written before records existed, `$4` says the owner never had
/// anyone else's data within reach (no member history), and no one else
/// takes part in the thread, so its legacy rows are theirs alone.
pub(crate) const DURABLE_MESSAGES_SQL: &str = concat!(
    "SELECT ",
    message_columns!(),
    " FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND ",
    participant_scope!(),
    " AND c.user_id = $2 \
       AND ((m.audience_recorded = TRUE AND m.author_user_id = $2) \
            OR (m.audience_recorded = FALSE AND $4 \
                AND NOT EXISTS (SELECT 1 FROM conversation_participants o \
                                WHERE o.conversation_id = c.id AND o.user_id <> $2))) \
       AND m.about_user_id IS NULL \
       AND NOT EXISTS (SELECT 1 FROM chat_message_members mm \
                       WHERE mm.message_id = m.id AND mm.member_user_id <> $2) \
     ORDER BY m.created_at ASC, m.id ASC"
);

/// The conversation holding the row `$1`, for its participant `$2`.
pub(crate) const MESSAGE_CONVERSATION_SQL: &str = concat!(
    "SELECT m.conversation_id FROM chat_messages m \
     JOIN chat_conversations c ON c.id = m.conversation_id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.id = $1 AND ",
    participant_scope!(),
    " LIMIT 1"
);

/// Whether the row `m` restates a read its reader may not read: a row the
/// platform wrote (not a `user` row) that carries a read whose key is none of
/// the `count` keys bound from `$first` on, or comes after one in the same
/// topic, `$divider` the divider stamp. With `author` (`$author`), a peer or
/// thread read a row the reader wrote carries never withholds it (carnet#1511
/// call 20), and neither does the member a legacy row (written before its
/// record, `audience_recorded` false) names, from the conversation's owner:
/// a row written before records existed is its
/// custodian's (calls 11 and 17), so what it drew on is read by its custodian
/// as it was, never reinterpreted by a peer's withdrawal or the reader
/// leaving (call 20). The statement joins the conversation as `c`. The rule the prompt's replay applies (carnet#1422), over every
/// topic of a transcript instead of the newest: a reply restates the rows
/// before it until a divider starts the model over. What a participant typed
/// restates nothing the model read; the room words it quotes are shown or
/// withheld per reader on their own (`quote_reading`). One placeholder per
/// key; the count is all that is formatted.
pub(crate) fn withheld_member_taint(
    divider: usize,
    first: usize,
    count: usize,
    author: Option<usize>,
) -> String {
    let unreadable = format!("NOT {}", key_in("r.rkey", first, count));
    let exempt = author.map_or_else(String::new, |author| {
        format!(
            " AND NOT ((r.rway IN ('peer', 'thread') AND r.rauthor = ${author}) \
               OR (r.rway = 'about' AND c.user_id = ${author} \
                   AND EXISTS (SELECT 1 FROM chat_messages l \
                               WHERE l.id = r.rid AND l.audience_recorded = FALSE)))"
        )
    });
    format!(
        "EXISTS (SELECT 1 FROM ( {} ) r \
         WHERE {unreadable}{exempt} \
           AND m.role <> 'user' AND (r.rat, r.rid) <= (m.created_at, m.id) \
           AND NOT EXISTS (SELECT 1 FROM chat_messages d \
               WHERE d.conversation_id = $1 AND d.finish_reason = ${divider} \
                 AND (d.created_at, d.id) > (r.rat, r.rid) \
                 AND (d.created_at, d.id) <= (m.created_at, m.id)))",
        thread_row_reads!()
    )
}

/// Record read `$5`..`$10` (member, way, group, coach, class, tier) as one
/// the row `$4` carries (a training read beyond activities stored as
/// `training` with `beyond_activities`, carnet#1511 call 23; a write stored
/// as `coach` with `written`, call 35), for a participant of the conversation `$1` that
/// holds it. Nothing when the row is not theirs, or already carries the
/// member: the writer merges a member's reads into the strictest first
/// ([`CarriedRead::strictest_per_member`]).
///
/// [`CarriedRead::strictest_per_member`]: pierre_core::models::CarriedRead::strictest_per_member
pub(crate) const RECORD_MESSAGE_MEMBER_SQL: &str = concat!(
    "INSERT INTO chat_message_members \
     (message_id, member_user_id, via_kind, via_group, via_coach, data_class, tier, \
      beyond_activities, written) \
     SELECT m.id, $5, CASE WHEN $6 = 'written' THEN 'coach' ELSE $6 END, $7, $8, \
            CASE WHEN $9 = 'beyond_activities' THEN 'training' ELSE $9 END, $10, \
            $9 = 'beyond_activities', $6 = 'written' FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.id = $4 AND m.conversation_id = $1 AND ",
    participant_scope!(),
    " ON CONFLICT (message_id, member_user_id) DO NOTHING"
);

/// Emit the member-row reads for one backend type: the bodies behind
/// [`ChatRepository::get_recent_prompt_messages`],
/// [`ChatRepository::list_member_reads`] and
/// [`ChatRepository::list_thread_read_keys`]. Invoked inside
/// `impl_chat_repository!`, whose row parsers and helpers it names
/// unqualified, and whose shell must `use` this module's statements.
///
/// [`ChatRepository::get_recent_prompt_messages`]: super::chat::ChatRepository::get_recent_prompt_messages
/// [`ChatRepository::list_member_reads`]: super::chat::ChatRepository::list_member_reads
/// [`ChatRepository::list_thread_read_keys`]: super::chat::ChatRepository::list_thread_read_keys
macro_rules! impl_chat_member_rows {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// The newest `limit` rows of a conversation a turn's prompt may replay,
            /// oldest first.
            async fn prompt_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
                readable: TransportPolicy,
                replay: MemberReplay<'_>,
            ) -> AppResult<Vec<MessageRecord>> {
                let sql = recent_prompt_messages_sql(replay.readable_about.len());
                let mut query = sqlx::query(&sql)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .bind(readable.is_first_party_only())
                    .bind(replay.tainted_from)
                    .bind(replay.current)
                    .bind(replay.withhold_all)
                    .bind(replay.recorded_only);
                for member in replay.readable_about {
                    query = query.bind($ids::bind(*member));
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to get the prompt's recent messages: {e}"))
                })?;
                // Newest first from the statement; chronological for the caller.
                let mut messages = rows
                    .iter()
                    .map(message_from_row)
                    .collect::<AppResult<Vec<_>>>()?;
                messages.reverse();
                Ok(messages)
            }

            /// Every read a conversation's rows carry since its newest
            /// divider, oldest first, for a participant.
            async fn member_reads(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<RowRead>> {
                let rows = sqlx::query(MEMBER_READS_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(DIVIDER_FINISH_REASON)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list a thread's member reads: {e}"))
                    })?;
                let column = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read a thread's member read: {e}"))
                };
                rows.iter()
                    .map(|row| {
                        Ok(RowRead {
                            message_id: row.try_get("message_id").map_err(column)?,
                            key: row.try_get("rkey").map_err(column)?,
                        })
                    })
                    .collect()
            }

            /// Every read a conversation's rows carry, by key, for a
            /// participant.
            async fn thread_read_keys(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<String>> {
                let rows = sqlx::query(THREAD_READ_KEYS_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list the reads a thread's rows carry: {e}"
                        ))
                    })?;
                rows.iter()
                    .map(|row| {
                        row.try_get("rkey").map_err(|e| {
                            AppError::database(format!("Failed to read a thread's read: {e}"))
                        })
                    })
                    .collect()
            }

            /// Every row of a conversation its owner may file as durable
            /// text, oldest first.
            async fn durable_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                legacy: bool,
            ) -> AppResult<Vec<MessageRecord>> {
                let rows = sqlx::query(DURABLE_MESSAGES_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(legacy)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a thread's durable rows: {e}"))
                    })?;
                rows.iter().map(message_from_row).collect()
            }

            /// The conversation holding a row, for a participant of it.
            async fn conversation_of_message(
                &self,
                message_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(MESSAGE_CONVERSATION_SQL)
                    .bind(message_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find a row's conversation: {e}"))
                    })?;
                row.map(|row| {
                    row.try_get("conversation_id").map_err(|e| {
                        AppError::database(format!("Failed to read a row's conversation: {e}"))
                    })
                })
                .transpose()
            }
        }
    };
}
pub(crate) use impl_chat_member_rows;
