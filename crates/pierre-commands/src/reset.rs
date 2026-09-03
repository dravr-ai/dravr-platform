// ABOUTME: Handler for /reset (/nouveau, /new) — rotate the athlete onto a fresh conversation
// ABOUTME: One implementation for every surface: a messaging channel repoints its session, the app follows the id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_contremaitre::messaging_strings::{
    KEY_NEW_CONVERSATION_TITLE_PREFIX, KEY_RESET_CONFIRM, KEY_RESET_WALK_INTERRUPTED,
};
use pierre_core::errors::AppError;
use pierre_core::models::OnboardingState;
use pierre_messaging::commands::CommandResponse;
use pierre_services::coach_selection::CoachSelectionSource;
use pierre_services::conversation_forge::{
    forge_conversation, in_app_title, messaging_title, repoint_messaging_session, ForgeCoach,
    ForgeParams,
};
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/reset` — abandon the current thread and continue on a fresh
/// one.
///
/// A long or derailed conversation is the athlete's to end, on whichever
/// surface they are on. The previous thread is left intact and archived: what
/// changes is only where the next turn is written.
///
/// The rotation is the same everywhere, because a conversation is the same
/// everywhere. What differs is who has to be told: a messaging channel keeps
/// its own pointer at the conversation it writes into, so that pointer moves
/// here; the in-app clients hold theirs in the UI, so the id is handed back on
/// the turn and they open it.
pub struct ResetHandler;

#[async_trait]
impl CommandHandler for ResetHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let user_id = ctx.user_id.to_string();
        let repos = ctx.ctx.repos();

        let previous_id = ctx.conversation_id.as_deref().ok_or_else(|| {
            AppError::invalid_input("/reset needs a conversation to rotate away from")
        })?;
        let previous = repos
            .chat
            .get_conversation(previous_id, &user_id, ctx.conversation_tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Conversation {previous_id}")))?;

        // Read before forging: the fresh row's onboarding_state is NULL, so
        // asking afterwards always answers "no walk" and the athlete is never
        // told the seventh question is not coming.
        let interrupted_walk =
            OnboardingState::from_column(previous.onboarding_state.as_deref()).is_some();

        // The fresh thread names itself. Carrying the old title forward left
        // the list with rows an athlete could not tell apart, and on a
        // messaging channel the thread has only ever been named after its
        // channel.
        let in_app = matches!(ctx.channel_type.as_str(), "web" | "mobile");
        let title = if in_app {
            in_app_title(
                &reg.render(KEY_NEW_CONVERSATION_TITLE_PREFIX, locale, &[]),
                Utc::now(),
            )
        } else {
            messaging_title(&ctx.channel_type)
        };

        let new_id = forge_conversation(
            repos,
            ForgeParams {
                user_id: &user_id,
                // The thread's tenant, not the caller's: a shared room files
                // its rows under the channel's tenant, and forging under the
                // caller's would leave every later turn reading an empty
                // thread.
                tenant_id: ctx.conversation_tenant_id,
                title: &title,
                model: Some(&previous.model),
                // The thread being replaced already names the coach the
                // athlete was talking to; a reset changes the thread, not who
                // they train with.
                coach: ForgeCoach::Explicit(previous.coach_id.as_deref()),
                group_id: previous.group_id.as_deref(),
                channel_type: &ctx.channel_type,
                selection_source: if in_app {
                    CoachSelectionSource::ChatConversation
                } else {
                    CoachSelectionSource::MessagingSession
                },
                guided_flow: ctx.is_direct_message,
            },
        )
        .await?;

        // On a messaging channel this is what makes the rotation stick. In the
        // app there is no session and it finds nothing, which is the correct
        // answer rather than a failure.
        match repoint_messaging_session(repos, ctx.conversation_tenant_id, previous_id, &new_id)
            .await
        {
            Ok(repointed) => info!(
                user_id = %ctx.user_id,
                channel = %ctx.channel_type,
                previous_conversation_id = previous_id,
                conversation_id = %new_id,
                repointed,
                interrupted_walk,
                "Reset command: rotated the athlete onto a fresh conversation"
            ),
            Err(e) => {
                // The fresh conversation exists and the athlete is about to be
                // told so; a channel still writing to the old thread would
                // make that a lie.
                warn!(error = %e, previous_conversation_id = previous_id, "Reset command: the messaging session could not be repointed");
                return Err(e);
            }
        }

        ctx.rotation.record(new_id);

        let confirm = reg.render(KEY_RESET_CONFIRM, locale, &[]);
        let body = if interrupted_walk {
            format!(
                "{confirm}{}",
                reg.render(KEY_RESET_WALK_INTERRUPTED, locale, &[])
            )
        } else {
            confirm
        };
        // Rich, because the interrupted-walk note names `/pillars` as code.
        Ok(CommandResponse::rich_text(body))
    }
}
