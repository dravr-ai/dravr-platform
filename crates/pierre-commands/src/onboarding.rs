// ABOUTME: Handler for /context — (re)start the guided pillar-onboarding walk on a conversation
// ABOUTME: Activates onboarding mode; `full` / `<pillar>` re-screen by superseding prior onboarding facts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::models::{OnboardingState, Pillar};
use pierre_messaging::commands::CommandResponse;
use tracing::info;

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/context` — (re)build the user's pillar context conversationally.
///
/// - `/context` — enter onboarding mode; the next turns walk the North Star +
///   six pillars (coverage is re-derived from the Dossier each turn).
/// - `/context full` — re-screen everything: supersede prior onboarding facts
///   (they go stale) then walk all topics again.
/// - `/context <pillar>` — re-screen a single pillar (e.g. `fuelling`).
///
/// Superseding is done via `expire_onboarding_facts` (sets `valid_until=now`),
/// never deletion — the GDPR forget path stays separate.
pub struct ContextHandler;

#[async_trait]
impl CommandHandler for ContextHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let conversation_id = ctx.conversation_id.as_deref().ok_or_else(|| {
            AppError::invalid_input("/context needs an active conversation to onboard into")
        })?;
        let repos = ctx.ctx.repos();
        let user = ctx.user_id.to_string();
        let is_fr = ctx.locale.starts_with("fr");

        // Re-screen scope: `full` expires all onboarding facts, a pillar slug
        // expires that pillar's, bare `/context` supersedes nothing.
        let arg = ctx.args.first().map(|s| s.trim().to_lowercase());
        match arg.as_deref() {
            Some("full") => {
                let n = repos
                    .memory
                    .expire_onboarding_facts(ctx.tenant_id, &user, None)
                    .await?;
                info!(user_id = %ctx.user_id, superseded = n, "/context full re-screen");
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
                info!(user_id = %ctx.user_id, pillar = pillar.as_str(), superseded = n, "/context pillar re-screen");
            }
            None => {}
        }

        // Activate onboarding mode on this conversation.
        let json = OnboardingState::start_now_column();
        repos
            .chat
            .set_conversation_onboarding_state(conversation_id, Some(&json), ctx.tenant_id)
            .await?;

        info!(user_id = %ctx.user_id, "onboarding mode activated via /context");

        let msg = if is_fr {
            "On va construire ton profil ensemble — je vais te poser quelques questions, une à la fois. Pour commencer : qu'est-ce qui te motive profondément à t'entraîner (ton « North Star ») ?"
        } else {
            "Let's build your profile together — I'll ask about a few areas, one at a time. To start: what's the deeper reason you train — your North Star?"
        };
        Ok(CommandResponse::rich_text(msg.to_owned()))
    }
}
