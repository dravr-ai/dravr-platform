// ABOUTME: The one ChatRepository implementation, emitted per backend by impl_chat_repository! with that backend's id codec
// ABOUTME: Row parsers and the trait body over the statements in repositories/chat.rs; the shells are one line each
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit the whole [`ChatRepository`](super::chat::ChatRepository) implementation
/// for one backend type.
/// The body is written once here; each backend's shell invokes it with its
/// own type, its driver's row type, and its uuid codec from
/// [`super::uuid_columns`] — how that backend binds an id the caller holds
/// as text against its `user_id`/`group_id`/`added_by` columns and reads one
/// back. The row parsers are emitted inside the macro because those id
/// reads are the one thing in them that differs per driver; sqlx resolves
/// the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_chat_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
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
            })
        }

        /// Decode one list row. The three `last_*` columns are all `NULL` or
        /// all set — they come from the same newest row — so the preview is
        /// keyed on the role.
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
                    Ok(ConversationLastMessage {
                        content_head: opt("last_content_head")?.unwrap_or_default(),
                        role,
                        created_at: last_created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                    })
                })
                .transpose()?;
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

        #[async_trait::async_trait]
        impl ChatRepository for $ty {
            async fn create_conversation(
                &self,
                user_id: &str,
                tenant_id: TenantId,
                title: &str,
                model: &str,
                agent_id: Option<&str>,
                group_id: Option<&str>,
            ) -> AppResult<ConversationRecord> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let user = $ids::bind_text(user_id)?;
                let group = $ids::bind_text_opt(group_id)?;
                let tenant = tenant_id.to_string();

                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to create conversation: {e}"))
                })?;

                sqlx::query(CREATE_CONVERSATION_SQL)
                    .bind(&id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(title)
                    .bind(model)
                    .bind(agent_id)
                    .bind(group)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create conversation: {e}"))
                    })?;

                sqlx::query(CREATE_OWNER_PARTICIPANT_SQL)
                    .bind(&id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(ParticipantRole::Owner.as_str())
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to add conversation owner: {e}"))
                    })?;

                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to create conversation: {e}"))
                })?;

                Ok(ConversationRecord {
                    id,
                    user_id: user_id.to_owned(),
                    tenant_id: tenant,
                    title: title.to_owned(),
                    model: model.to_owned(),
                    agent_id: agent_id.map(ToOwned::to_owned),
                    session_id: None,
                    total_tokens: 0,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                    group_id: group_id.map(ToOwned::to_owned),
                    // The INSERT above does not name the column, so the stored
                    // value is the schema default. Callers that mean another
                    // channel stamp it afterwards through `set_conversation_channel`.
                    channel_type: CHANNEL_TYPE_WEB.to_owned(),
                    onboarding_state: None,
                })
            }

            async fn get_conversation(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<ConversationRecord>> {
                let row = sqlx::query(GET_CONVERSATION_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get conversation: {e}")))?;
                row.as_ref().map(conversation_from_row).transpose()
            }

            async fn list_conversations(
                &self,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
                offset: i64,
            ) -> AppResult<ConversationPage> {
                let rows = sqlx::query(LIST_CONVERSATIONS_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .bind(offset)
                    .bind(CONTENT_HEAD_CHARS)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list conversations: {e}"))
                    })?;

                let total = self
                    .count_participating_conversations(user_id, tenant_id)
                    .await?;

                Ok(ConversationPage {
                    items: rows
                        .iter()
                        .map(summary_from_row)
                        .collect::<AppResult<_>>()?,
                    total,
                })
            }

            async fn count_participating_conversations(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<i64> {
                sqlx::query_scalar(COUNT_PARTICIPATING_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count participating conversations: {e}"
                        ))
                    })
            }

            async fn mark_conversation_read(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                up_to_message_id: Option<&str>,
            ) -> AppResult<bool> {
                let user = $ids::bind_text(user_id)?;
                let tenant = tenant_id.to_string();
                let row = sqlx::query(READ_TARGET_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(up_to_message_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to resolve read marker: {e}"))
                    })?;

                // No row: not a participant. A row with no target: either a
                // named message that is not in this conversation (refused), or
                // nothing written yet (nothing to mark, and nothing wrong).
                let Some(row) = row else {
                    return Ok(false);
                };
                let target: Option<DateTime<Utc>> = row
                    .try_get("target")
                    .map_err(|e| chat_column_error("target", &e))?;
                let Some(target) = target else {
                    return Ok(up_to_message_id.is_none());
                };

                sqlx::query(ADVANCE_READ_MARKER_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(target)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to advance read marker: {e}"))
                    })?;
                Ok(true)
            }

            async fn clear_conversation_read_marker(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLEAR_READ_MARKER_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to clear read marker: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn update_conversation_title(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                title: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_TITLE_SQL)
                    .bind(title)
                    .bind(Utc::now())
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update conversation title: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn set_conversation_channel(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                channel_type: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_CHANNEL_SQL)
                    .bind(channel_type)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set conversation channel: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn delete_conversation(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_CONVERSATION_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete conversation: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let tenant = params.tenant_id.to_string();

                let result = sqlx::query(ADD_MESSAGE_SQL)
                    .bind(&id)
                    .bind(params.conversation_id)
                    .bind(params.role)
                    .bind(params.content)
                    .bind(params.token_count.map(i64::from))
                    .bind(params.finish_reason)
                    .bind(now)
                    .bind(params.prompt_tokens.map(i64::from))
                    .bind(params.model)
                    .bind($ids::bind_text(params.user_id)?)
                    .bind(&tenant)
                    .bind(params.content_blocks)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add message: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(
                        "Conversation not found or access denied",
                    ));
                }

                // Touch the conversation and count the tokens; membership was
                // verified by the insert above.
                if let Some(tokens) = params.token_count {
                    sqlx::query(BUMP_CONVERSATION_TOKENS_SQL)
                        .bind(now)
                        .bind(i64::from(tokens))
                        .bind(params.conversation_id)
                        .bind(&tenant)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to update conversation tokens: {e}"))
                        })?;
                } else {
                    sqlx::query(TOUCH_CONVERSATION_SQL)
                        .bind(now)
                        .bind(params.conversation_id)
                        .bind(&tenant)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to update conversation timestamp: {e}"
                            ))
                        })?;
                }

                Ok(MessageRecord {
                    id,
                    conversation_id: params.conversation_id.to_owned(),
                    role: params.role.to_owned(),
                    content: params.content.to_owned(),
                    token_count: params.token_count.map(i64::from),
                    prompt_tokens: params.prompt_tokens.map(i64::from),
                    model: params.model.map(ToOwned::to_owned),
                    finish_reason: params.finish_reason.map(ToOwned::to_owned),
                    content_blocks: params.content_blocks.map(ToOwned::to_owned),
                    created_at: now.to_rfc3339(),
                })
            }

            async fn get_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<MessageRecord>> {
                let rows = sqlx::query(GET_MESSAGES_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get messages: {e}")))?;
                rows.iter().map(message_from_row).collect()
            }

            async fn get_recent_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<MessageRecord>> {
                let rows = sqlx::query(GET_RECENT_MESSAGES_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
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

            async fn get_message_count(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<i64> {
                sqlx::query_scalar(MESSAGE_COUNT_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get message count: {e}")))
            }

            async fn upsert_message_feedback(
                &self,
                params: &UpsertMessageFeedbackParams<'_>,
            ) -> AppResult<MessageFeedbackRecord> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let user = $ids::bind_text(params.user_id)?;
                let tenant = params.tenant_id.to_string();

                sqlx::query(UPSERT_FEEDBACK_SQL)
                    .bind(&id)
                    .bind(params.message_id)
                    .bind(params.conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(params.rating)
                    .bind(params.comment)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert message feedback: {e}"))
                    })?;

                let row = sqlx::query(GET_FEEDBACK_SQL)
                    .bind(params.message_id)
                    .bind(&user)
                    .bind(&tenant)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read back message feedback: {e}"))
                    })?;
                row.as_ref()
                    .map(feedback_from_row)
                    .transpose()?
                    .ok_or_else(|| AppError::not_found("Message not found or access denied"))
            }

            async fn delete_message_feedback(
                &self,
                message_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_FEEDBACK_SQL)
                    .bind(message_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete message feedback: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn get_conversation_feedback(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<MessageFeedbackRecord>> {
                let rows = sqlx::query(CONVERSATION_FEEDBACK_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get conversation feedback: {e}"))
                    })?;
                rows.iter().map(feedback_from_row).collect()
            }

            async fn count_conversations(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<i64> {
                sqlx::query_scalar(COUNT_CONVERSATIONS_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count conversations: {e}")))
            }

            async fn delete_all_user_conversations(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<i64> {
                let result = sqlx::query(DELETE_USER_CONVERSATIONS_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete user conversations: {e}"))
                    })?;
                Ok(i64::try_from(result.rows_affected()).unwrap_or(i64::MAX))
            }

            async fn add_participant(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
                user_id: &str,
                added_by: &str,
            ) -> AppResult<ConversationParticipant> {
                let user = $ids::bind_text(user_id)?;
                let tenant = tenant_id.to_string();

                sqlx::query(ADD_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(ParticipantRole::Member.as_str())
                    .bind($ids::bind_text(added_by)?)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add participant: {e}")))?;

                let row = sqlx::query(GET_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read participant: {e}")))?;
                row.as_ref()
                    .map(participant_from_row)
                    .transpose()?
                    .ok_or_else(|| AppError::not_found("Conversation not found"))
            }

            async fn remove_participant(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(ParticipantRole::Member.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove participant: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn list_participants(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<ConversationParticipant>> {
                let rows = sqlx::query(LIST_PARTICIPANTS_SQL)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list participants: {e}")))?;
                rows.iter().map(participant_from_row).collect()
            }

            async fn get_recent_conversations_admin(
                &self,
                limit: i64,
            ) -> AppResult<Vec<ConversationRecord>> {
                let rows = sqlx::query(RECENT_CONVERSATIONS_ADMIN_SQL)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query recent conversations: {e}"))
                    })?;
                rows.iter().map(conversation_from_row).collect()
            }

            async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64> {
                sqlx::query_scalar(COUNT_ACTIVE_SINCE_SQL)
                    .bind(instant_from_rfc3339(since)?)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count active conversations: {e}"))
                    })
            }

            async fn set_conversation_session_id(
                &self,
                conversation_id: &str,
                session_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_SESSION_ID_SQL)
                    .bind(session_id)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set conversation session_id: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn set_conversation_onboarding_state(
                &self,
                conversation_id: &str,
                onboarding_state: Option<&str>,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_ONBOARDING_STATE_SQL)
                    .bind(onboarding_state)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to set conversation onboarding_state: {e}"
                        ))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn compare_and_set_conversation_onboarding_state(
                &self,
                conversation_id: &str,
                expected: Option<&str>,
                onboarding_state: Option<&str>,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(CAS_ONBOARDING_STATE_SQL)
                    .bind(onboarding_state)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .bind(expected)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to compare-and-set conversation onboarding_state: {e}"
                        ))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn list_user_onboarding_states(
                &self,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<String>> {
                sqlx::query_scalar(LIST_ONBOARDING_STATES_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user onboarding states: {e}"))
                    })
            }

            async fn set_conversation_group_id(
                &self,
                conversation_id: &str,
                group_id: Option<&str>,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_GROUP_ID_SQL)
                    .bind($ids::bind_text_opt(group_id)?)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set conversation group_id: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn set_conversation_agent_id(
                &self,
                conversation_id: &str,
                agent_id: Option<&str>,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_AGENT_ID_SQL)
                    .bind(agent_id)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set conversation agent_id: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn has_agent_introduction(
                &self,
                thread_id: &str,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let row = sqlx::query(HAS_AGENT_INTRODUCTION_SQL)
                    .bind(tenant_id.to_string())
                    .bind(thread_id)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read agent introduction: {e}"))
                    })?;
                Ok(row.is_some())
            }

            async fn record_agent_introduction(
                &self,
                thread_id: &str,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<()> {
                sqlx::query(RECORD_AGENT_INTRODUCTION_SQL)
                    .bind(tenant_id.to_string())
                    .bind(thread_id)
                    .bind(agent_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record agent introduction: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_chat_repository;
