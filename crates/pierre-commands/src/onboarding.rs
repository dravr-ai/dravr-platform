// ABOUTME: Handler for /pillars — (re)start the guided pillar-onboarding walk on a conversation
// ABOUTME: Activates onboarding mode; `full` / `<pillar>` re-screen by superseding prior onboarding facts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_PILLARS_DM_ONLY, KEY_PILLARS_OPENER, KEY_PILLARS_START_FAILED,
};
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, OnboardingState, Pillar};
use pierre_messaging::commands::CommandResponse;
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/pillars` — (re)build the user's pillar context conversationally.
///
/// - `/pillars` — enter onboarding mode; the next turns walk the North Star +
///   six pillars (coverage is re-derived from the Dossier each turn).
/// - `/pillars full` — re-screen everything: supersede prior onboarding facts
///   (they go stale) then walk all topics again.
/// - `/pillars <pillar>` — re-screen a single pillar (e.g. `fuelling`).
///
/// Superseding is done via `expire_onboarding_facts` (sets `valid_until=now`),
/// never deletion — the GDPR forget path stays separate.
///
/// Direct messages only. Slash replies from a shared room are already rerouted
/// to a private chat, so a group walk would ask the athlete about motivations,
/// sleep and stress in DM while the extraction worker stamped their answers
/// under the *channel* tenant — a fact space disjoint from their own dossier.
pub struct PillarsHandler;

#[async_trait]
impl CommandHandler for PillarsHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();

        if !ctx.is_direct_message {
            info!(
                user_id = %ctx.user_id,
                channel = %ctx.channel_type,
                "/pillars refused outside a direct message"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_PILLARS_DM_ONLY,
                &ctx.locale,
                &[],
            )));
        }

        let conversation_id = ctx.conversation_id.as_deref().ok_or_else(|| {
            AppError::invalid_input("/pillars needs an active conversation to onboard into")
        })?;
        let repos = ctx.ctx.repos();
        let user = ctx.user_id.to_string();

        // Re-screen scope: `full` expires all onboarding facts, a pillar slug
        // expires that pillar's, bare `/pillars` supersedes nothing.
        let arg = ctx.args.first().map(|s| s.trim().to_lowercase());
        match arg.as_deref() {
            Some("full") => {
                let n = repos
                    .memory
                    .expire_onboarding_facts(ctx.tenant_id, &user, None)
                    .await?;
                info!(user_id = %ctx.user_id, superseded = n, "/pillars full re-screen");
            }
            Some(slug) => {
                let pillar = Pillar::parse(slug).ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "unknown pillar '{slug}' — use one of: {}",
                        Pillar::ALL.map(Pillar::as_str).join(", ")
                    ))
                })?;
                let n = repos
                    .memory
                    .expire_onboarding_facts(ctx.tenant_id, &user, Some(pillar))
                    .await?;
                info!(user_id = %ctx.user_id, pillar = pillar.as_str(), superseded = n, "/pillars pillar re-screen");
            }
            None => {}
        }

        // Activate onboarding mode on this conversation. The returned `bool`
        // reports whether a row actually matched: a tenant mismatch updates
        // nothing, and answering with the opener anyway would start a walk that
        // no turn ever runs in.
        let json = OnboardingState::start_now_column();
        let activated = repos
            .chat
            .set_conversation_onboarding_state(conversation_id, Some(&json), ctx.tenant_id)
            .await?;
        if !activated {
            warn!(
                user_id = %ctx.user_id,
                conversation_id,
                "/pillars matched no conversation row; onboarding not activated"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_PILLARS_START_FAILED,
                &ctx.locale,
                &[],
            )));
        }

        info!(user_id = %ctx.user_id, "onboarding mode activated via /pillars");

        let msg = reg.render(KEY_PILLARS_OPENER, &ctx.locale, &[]);

        // Persist the opener as the conversation's first assistant message — a
        // scoped exception to slash-command ephemerality, for flow-opening
        // commands only.
        //
        // Two things depend on it. The coach otherwise receives the athlete's
        // North Star answer with no question attached, and improvises a reply to
        // a question it cannot see; and with no history row, that answer is
        // message #1, which arms the first-turn coach startup prefetch — an
        // activity dump plus the coach's own "build a block" query injected as
        // if the athlete had asked for it (the 2026-07-24 derail).
        //
        // Written under the same tenant as the state above, so
        // `get_conversation_history` — which filters on the conversation
        // tenant — actually sees it.
        let opener = AddMessageParams {
            tenant_id: ctx.tenant_id,
            conversation_id,
            user_id: &user,
            role: "assistant",
            content: &msg,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            structured_content: None,
        };
        if let Err(e) = repos.chat.add_message(&opener).await {
            // The walk is already active and re-derives coverage from the
            // Dossier every turn, so a failed history write degrades the first
            // turn rather than the flow: ask the question anyway.
            warn!(error = %e, "failed to persist /pillars opener as assistant message");
        }

        Ok(CommandResponse::rich_text(msg))
    }
}
