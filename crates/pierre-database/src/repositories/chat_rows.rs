// ABOUTME: The ChatRepository row parsers, emitted per backend inside impl_chat_repository! with that backend's id codec
// ABOUTME: Decode conversation, list, message, feedback and membership rows through try_get only, never Row::get
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit the row parsers [`impl_chat_repository!`](super::chat_backend::impl_chat_repository)
/// decodes with, for one backend's row type and uuid codec.
///
/// Separate from the trait body only so neither file carries both; it is
/// invoked from inside that macro, and like it names its helpers and types
/// unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_chat_row_parsers {
    ($row:ty, $ids:ident) => {
        /// Decode one conversation row via `try_get` only: `Row::get` is
        /// `try_get().unwrap()`, and an unwind here takes the whole
        /// container down with every in-flight request on it.
        fn conversation_from_row(row: &$row) -> AppResult<ConversationRecord> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let opt = |name: &str| -> AppResult<Option<String>> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            Ok(ConversationRecord {
                id: col("id")?,
                user_id: $ids::read_text(row, "user_id")?,
                tenant_id: col("tenant_id")?,
                title: col("title")?,
                model: col("model")?,
                agent_id: opt("agent_id")?,
                session_id: opt("session_id")?,
                total_tokens: row
                    .try_get("total_tokens")
                    .map_err(|e| chat_column_error("total_tokens", &e))?,
                created_at: stamp_column(row, "created_at")?,
                updated_at: stamp_column(row, "updated_at")?,
                group_id: $ids::read_text_opt(row, "group_id")?,
                channel_type: col("channel_type")?,
                onboarding_state: opt("onboarding_state")?,
                archived_at: row
                    .try_get::<Option<DateTime<Utc>>, _>("archived_at")
                    .map_err(|e| chat_column_error("archived_at", &e))?
                    .map(|t| t.to_rfc3339()),
                personal: row
                    .try_get("personal")
                    .map_err(|e| chat_column_error("personal", &e))?,
                subject_user_id: $ids::read_opt(row, "subject_user_id")?,
                subject_group_id: $ids::read_opt(row, "subject_group_id")?,
            })
        }

        /// Decode one list row. The three `last_*` columns are all `NULL` or
        /// all set — they come from the same newest row — so the preview is
        /// keyed on the role. A thread bound to one athlete is named by the
        /// roster rule from the joined user's display name and email, which
        /// are `NULL` together on every other row.
        fn summary_from_row(row: &$row) -> AppResult<ConversationSummary> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let opt = |name: &str| -> AppResult<Option<String>> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let count = |name: &str| -> AppResult<i64> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let last_message = opt("last_role")?
                .map(|role| -> AppResult<ConversationLastMessage> {
                    let last_created_at: Option<DateTime<Utc>> = row
                        .try_get("last_created_at")
                        .map_err(|e| chat_column_error("last_created_at", &e))?;
                    let last_first_party_only: Option<bool> = row
                        .try_get("last_first_party_only")
                        .map_err(|e| chat_column_error("last_first_party_only", &e))?;
                    Ok(ConversationLastMessage {
                        content_head: opt("last_content_head")?.unwrap_or_default(),
                        role,
                        created_at: last_created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                        transport_policy: TransportPolicy::from_first_party_only(
                            last_first_party_only.unwrap_or(false),
                        ),
                    })
                })
                .transpose()?;
            let subject_display_name = match opt("subject_email")? {
                Some(email) => Some(roster_display_name(
                    opt("subject_display_name")?.as_deref(),
                    &email,
                )),
                None => None,
            };
            Ok(ConversationSummary {
                id: col("id")?,
                title: col("title")?,
                model: col("model")?,
                message_count: count("message_count")?,
                total_tokens: count("total_tokens")?,
                agent_id: opt("agent_id")?,
                agent_handle: opt("agent_handle")?,
                agent_title: opt("agent_title")?,
                group_id: $ids::read_text_opt(row, "group_id")?,
                group_name: opt("group_name")?,
                channel_type: opt("channel_type")?,
                archived_at: row
                    .try_get::<Option<DateTime<Utc>>, _>("archived_at")
                    .map_err(|e| chat_column_error("archived_at", &e))?
                    .map(|t| t.to_rfc3339()),
                personal: row
                    .try_get("personal")
                    .map_err(|e| chat_column_error("personal", &e))?,
                subject_user_id: $ids::read_opt(row, "subject_user_id")?,
                subject_group_id: $ids::read_opt(row, "subject_group_id")?,
                subject_display_name,
                last_message,
                unread_count: count("unread_count")?,
                created_at: stamp_column(row, "created_at")?,
                updated_at: stamp_column(row, "updated_at")?,
            })
        }

        /// Decode one message row.
        fn message_from_row(row: &$row) -> AppResult<MessageRecord> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let opt = |name: &str| -> AppResult<Option<String>> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let tokens = |name: &str| -> AppResult<Option<i64>> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            Ok(MessageRecord {
                id: col("id")?,
                conversation_id: col("conversation_id")?,
                role: col("role")?,
                content: col("content")?,
                token_count: tokens("token_count")?,
                prompt_tokens: tokens("prompt_tokens")?,
                model: opt("model")?,
                finish_reason: opt("finish_reason")?,
                content_blocks: opt("content_blocks")?,
                created_at: stamp_column(row, "created_at")?,
                transport_policy: TransportPolicy::from_first_party_only(
                    row.try_get("first_party_only")
                        .map_err(|e| chat_column_error("first_party_only", &e))?,
                ),
                channel_type: opt("channel_type")?,
            })
        }

        /// Decode one feedback row. `user_id` is a uuid column (matching
        /// `chat_conversations`), surfaced as text for the wire DTO.
        fn feedback_from_row(row: &$row) -> AppResult<MessageFeedbackRecord> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            Ok(MessageFeedbackRecord {
                id: col("id")?,
                message_id: col("message_id")?,
                conversation_id: col("conversation_id")?,
                user_id: $ids::read_text(row, "user_id")?,
                tenant_id: col("tenant_id")?,
                rating: col("rating")?,
                comment: row
                    .try_get("comment")
                    .map_err(|e| chat_column_error("comment", &e))?,
                created_at: stamp_column(row, "created_at")?,
                updated_at: stamp_column(row, "updated_at")?,
            })
        }

        /// Decode one membership row. The role column is CHECK-constrained
        /// to the two known values, so an unknown one is a schema breach
        /// surfaced as a database error rather than silently defaulted.
        fn participant_from_row(row: &$row) -> AppResult<ConversationParticipant> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| chat_column_error(name, &e))
            };
            let role = col("role")?;
            let role = ParticipantRole::from_column(&role).ok_or_else(|| {
                AppError::database(format!(
                    "conversation_participants.role holds unknown value {role}"
                ))
            })?;
            Ok(ConversationParticipant {
                conversation_id: col("conversation_id")?,
                user_id: $ids::read_text(row, "user_id")?,
                tenant_id: col("tenant_id")?,
                role,
                added_by: $ids::read_text(row, "added_by")?,
                added_at: stamp_column(row, "added_at")?,
            })
        }
    };
}
pub(crate) use impl_chat_row_parsers;
