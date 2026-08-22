// ABOUTME: Pre-dispatch chat quota enforcement for messaging turns (QuotaGate impl)
// ABOUTME: Scoped to the USER's tenant so Telegram/WhatsApp/Slack usage depletes the same budget web reads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The messaging implementation of the chat pipeline's
//! [`QuotaGate`](pierre_chat_pipeline::hooks::QuotaGate) hook.
//!
//! Web chat enforces the [`pierre_core::models::TierQuotaConfig`] matrix
//! inline before dispatch; messaging ingress historically enforced nothing —
//! a Telegram user bypassed every message/token cap
//! (registre#9). Both paths now funnel into the same policy function,
//! [`check_pre_chat_quotas_scoped`], so there is one quota system with two
//! call sites rather than two systems: this hook is plumbing, not policy.
//!
//! Scoping: quotas are checked (and, in
//! `dispatch::increment_messaging_usage_counters`, recorded) under the
//! **user's own tenant** — `TurnContext::tool_tenant_id`, which messaging
//! dispatch sets from `PendingDispatch::user_tenant_id` — never the bot
//! tenant. Counting under the bot's tenant let a user's Telegram usage
//! deplete a budget web chat never read.

use async_trait::async_trait;
use pierre_chat_pipeline::hooks::{QuotaGate, QuotaWarning};
use pierre_chat_pipeline::turn::TurnContext;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use std::sync::Arc;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::chat::quotas::{check_pre_chat_quotas_scoped, PreChatScope};

/// Pre-dispatch quota gate for messaging turns.
pub(super) struct MessagingQuotaGate {
    /// Shared server context (Arc-cloned per dispatch; the gate borrows for
    /// the turn's lifetime through `PipelineHooks`).
    pub(super) resources: Arc<ServerContext>,
}

#[async_trait]
impl QuotaGate for MessagingQuotaGate {
    async fn pre_check(&self, ctx: &TurnContext) -> AppResult<Option<QuotaWarning>> {
        let user_uuid = Uuid::parse_str(&ctx.user_id).map_err(|e| {
            AppError::new(
                ErrorCode::InvalidInput,
                format!("messaging quota gate: user id is not a UUID: {e}"),
            )
        })?;

        // The per-coach cap needs the conversation's coach, resolved the same
        // way web chat does it — from the conversation row, which lives under
        // the session/conversation tenant.
        let coach_id = self
            .resources
            .common
            .repos
            .chat
            .get_conversation(
                &ctx.conversation_id,
                &ctx.user_id,
                ctx.conversation_tenant_id,
            )
            .await
            .ok()
            .flatten()
            .and_then(|c| c.coach_id);
        let scope = PreChatScope {
            conversation_id: Some(ctx.conversation_id.as_str()),
            coach_id: coach_id.as_deref(),
        };

        let warning = check_pre_chat_quotas_scoped(
            &self.resources,
            &ctx.tool_tenant_id.to_string(),
            &ctx.user_id,
            user_uuid,
            &scope,
        )
        .await?;

        Ok(
            warning.map(|(code, current, limit, resets_at)| QuotaWarning {
                code: code.to_owned(),
                message: format!("{code}: {current}/{limit}, resets at {resets_at}"),
            }),
        )
    }
}
