// ABOUTME: A chat row written with the member reads it carries and the conversation's touch, in one transaction
// ABOUTME: The body behind ChatRepository::add_message and add_message_about, emitted per backend inside impl_chat_repository!

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit the message write for one backend type: the body behind
/// [`ChatRepository::add_message`] and [`ChatRepository::add_message_about`].
/// Invoked inside `impl_chat_repository!`, whose shell must `use` the
/// statements it names unqualified.
///
/// [`ChatRepository::add_message`]: super::chat::ChatRepository::add_message
/// [`ChatRepository::add_message_about`]: super::chat::ChatRepository::add_message_about
macro_rules! impl_chat_message_write {
    ($ty:ty, $ids:ident, $begin:literal) => {
        impl $ty {
            /// Write the row, the members it is about and the conversation's
            /// touch in one transaction.
            async fn insert_message(
                &self,
                params: &AddMessageParams<'_>,
                carried: &[CarriedRead],
            ) -> AppResult<MessageRecord> {
                let id = Uuid::new_v4().to_string();
                let now = Timestamp::now();
                let tenant = params.tenant_id.to_string();
                // The row, the reads it carries and the conversation's touch
                // land together or not at all: a row stored without the
                // members it read would be about nobody (carnet#1422).
                let mut tx = self.pool().begin_with($begin).await.map_err(|e| {
                    AppError::database(format!("Failed to open the message write: {e}"))
                })?;

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
                    .bind(params.transport_policy.is_first_party_only())
                    .bind(params.channel_type)
                    .bind($ids::bind_opt(params.about_user_id))
                    .bind(params.turn_attempted_write)
                    .bind(consent_sources_column(params.ai_consent_sources))
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add message: {e}")))?;

                if result.rows_affected() == 0 {
                    // Dropping the transaction rolls it back; nothing was written.
                    return Err(AppError::not_found(
                        "Conversation not found or access denied",
                    ));
                }

                // One link row per member: the strictest of their reads, so
                // no reader the stricter read keeps out reads it (call 19).
                for read in CarriedRead::strictest_per_member(carried) {
                    sqlx::query(RECORD_MESSAGE_MEMBER_SQL)
                        .bind(params.conversation_id)
                        .bind($ids::bind_text(params.user_id)?)
                        .bind(&tenant)
                        .bind(&id)
                        .bind($ids::bind(read.member))
                        .bind(read.way.as_str())
                        .bind($ids::bind_opt(read.group))
                        .bind($ids::bind_opt(read.coach))
                        .bind(&read.class)
                        .bind(read.tier.as_deref())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to name a member a chat row is about: {e}"
                            ))
                        })?;
                }

                // Touch the conversation and count the tokens; membership was
                // verified by the insert above.
                if let Some(tokens) = params.token_count {
                    sqlx::query(BUMP_CONVERSATION_TOKENS_SQL)
                        .bind(now)
                        .bind(i64::from(tokens))
                        .bind(params.conversation_id)
                        .bind(&tenant)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to update conversation tokens: {e}"))
                        })?;
                } else {
                    sqlx::query(TOUCH_CONVERSATION_SQL)
                        .bind(now)
                        .bind(params.conversation_id)
                        .bind(&tenant)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to update conversation timestamp: {e}"
                            ))
                        })?;
                }

                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add message: {e}")))?;

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
                    transport_policy: params.transport_policy,
                    channel_type: params.channel_type.map(ToOwned::to_owned),
                    about_user_id: params.about_user_id,
                    turn_attempted_write: params.turn_attempted_write,
                    ai_consent_sources: params.ai_consent_sources.to_vec(),
                })
            }
        }
    };
}
pub(crate) use impl_chat_message_write;
