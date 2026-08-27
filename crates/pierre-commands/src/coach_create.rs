// ABOUTME: Handler for /coach create — drafts a coach from the conversation, parks it, and creates it on confirm
// ABOUTME: The draft lives in the Guardian pending-action store so /deny and the expiry come from the one claim path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/coach create` — a coach drafted from the conversation, confirmed to exist.
//!
//! The last messages of the conversation are handed to the model, which
//! proposes a persona, and nothing exists until the athlete confirms it.
//!
//! The proposal is parked in the Guardian pending-action store under a
//! single-use claim token, the same store `/confirm` and `/deny` resolve.
//! That buys the whole confirmation grammar for free — `/deny <token>`
//! discards a draft, a token can be confirmed once, and an old draft expires
//! on its own — and it means `/confirm <token>` typed out of habit creates
//! the coach too, because the claim path routes on the parked action's kind.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_COACH_CREATE_CARD_TITLE, KEY_COACH_CREATE_CONFIRM_LABEL, KEY_COACH_CREATE_DISCARD_LABEL,
    KEY_COACH_CREATE_DONE, KEY_COACH_CREATE_DONE_UNBOUND, KEY_COACH_CREATE_EMPTY,
    KEY_COACH_CREATE_NO_CONVERSATION, KEY_COACH_CREATE_PROPOSAL_BODY, KEY_COACH_CREATE_QUOTA,
    KEY_COACH_CREATE_USAGE,
};
use pierre_core::errors::AppError;
use pierre_core::models::coaches::{Coach, CoachCategory, CreateCoachRequest};
use pierre_database::repositories::PendingGuardianAction;
use pierre_messaging::commands::{CommandAction, CommandResponse};
use pierre_services::coach_generation::{
    coach_quota, conversation_excerpt, propose_coach, resolve_chat_provider, CoachProposal,
    ExcerptRequest, DEFAULT_MAX_MESSAGES,
};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::coach::{bind_coach, CoachBinding};
use crate::guardian_confirm::resolve_pending;
use crate::{CommandHandler, PlatformCommandContext};

/// `tool_name` a parked coach draft carries in the pending-action store.
///
/// Deliberately not a registered tool: the claim path checks for it before
/// anything is dispatched, so a confirmed draft becomes a coach here rather
/// than a tool call the executor would refuse.
pub const COACH_PROPOSAL_ACTION: &str = "coach.create_proposal";

/// How long a draft waits for its confirmation.
///
/// Longer than the Guardian's five minutes for a destructive call: reading a
/// proposed persona takes a moment, and a stale draft creates nothing on its
/// own.
const PROPOSAL_TTL_MINUTES: i64 = 10;

/// The parked row's `deny_reason` — the store's column names why a row is
/// waiting, and a draft waits for the athlete's yes.
const PROPOSAL_PARK_REASON: &str = "awaiting_confirmation";

/// The draft as stored between `/coach create` and its confirmation.
#[derive(Debug, Serialize, Deserialize)]
struct ParkedProposal {
    title: String,
    description: String,
    system_prompt: String,
    category: String,
    tags: Vec<String>,
}

impl From<CoachProposal> for ParkedProposal {
    fn from(proposal: CoachProposal) -> Self {
        Self {
            title: proposal.title,
            description: proposal.description,
            system_prompt: proposal.system_prompt,
            category: proposal.category,
            tags: proposal.tags,
        }
    }
}

/// Handler for `/coach create` and `/coach create confirm <token>`.
///
/// Listed for every caller: drafting needs nothing but a conversation with
/// messages in it, and creating needs nothing a group standing can see.
pub struct CoachCreateHandler;

#[async_trait]
impl CommandHandler for CoachCreateHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        match ctx.args.as_slice() {
            [] => propose(ctx).await,
            [verb, token] if verb.eq_ignore_ascii_case("confirm") => {
                resolve_pending(ctx, token, "confirmed").await
            }
            _ => Ok(CommandResponse::text(
                ctx.ctx.messaging_strings_registry().render(
                    KEY_COACH_CREATE_USAGE,
                    ctx.locale.as_str(),
                    &[],
                ),
            )),
        }
    }
}

/// Draft a coach from the conversation the command was typed in, park it,
/// and show the athlete what they would be creating.
async fn propose(ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    let Some(conversation_id) = ctx.conversation_id.as_deref() else {
        return Ok(CommandResponse::text(reg.render(
            KEY_COACH_CREATE_NO_CONVERSATION,
            locale,
            &[],
        )));
    };

    // The excerpt first, the model second: an empty conversation is refused
    // before any provider is resolved, so the refusal costs nothing.
    let request = ExcerptRequest {
        conversation_id,
        user_id: ctx.user_id,
        tenant_id: ctx.conversation_tenant_id,
        max_messages: DEFAULT_MAX_MESSAGES,
    };
    let Some(excerpt) = conversation_excerpt(ctx.ctx.repos().chat.as_ref(), &request).await? else {
        return Ok(CommandResponse::text(reg.render(
            KEY_COACH_CREATE_EMPTY,
            locale,
            &[],
        )));
    };
    let provider = resolve_chat_provider(ctx.ctx.chat_provider(), ctx.ctx.llm_provider())?;
    let proposal = propose_coach(&provider, &ctx.ctx.coach_generation_prompt(), &excerpt).await?;

    let tags = proposal.tags.join(", ");
    let body_args = [
        proposal.title.clone(),
        proposal.description.clone(),
        proposal.category.clone(),
        tags,
    ];
    let token = park(ctx, proposal).await?;

    let body = reg.render(
        KEY_COACH_CREATE_PROPOSAL_BODY,
        locale,
        &[
            &body_args[0],
            &body_args[1],
            &body_args[2],
            &body_args[3],
            &token,
        ],
    );
    // Both postbacks stay under Telegram's 64-byte callback-data ceiling:
    // the longest is 22 bytes of verb plus a 32-character token.
    let actions = vec![
        CommandAction {
            label: reg.render(KEY_COACH_CREATE_CONFIRM_LABEL, locale, &[]),
            action_type: "postback".to_owned(),
            value: format!("/coach create confirm {token}"),
        },
        CommandAction {
            label: reg.render(KEY_COACH_CREATE_DISCARD_LABEL, locale, &[]),
            action_type: "postback".to_owned(),
            value: format!("/deny {token}"),
        },
    ];
    Ok(CommandResponse::card(
        reg.render(KEY_COACH_CREATE_CARD_TITLE, locale, &[]),
        body,
        actions,
    ))
}

/// Park the draft under a fresh single-use claim token and return the token.
///
/// Filed under the caller's own tenant — the tenant `/coach create confirm`
/// and `/deny` claim with — even when the conversation lives under a shared
/// room's channel tenant.
async fn park(ctx: &PlatformCommandContext, proposal: CoachProposal) -> Result<String, AppError> {
    let token = Uuid::new_v4().simple().to_string();
    let parked = ParkedProposal::from(proposal);
    let arguments = serde_json::to_value(&parked)
        .map_err(|e| AppError::internal(format!("serialize coach draft: {e}")))?;
    let action = PendingGuardianAction {
        id: token.clone(),
        tenant_id: ctx.tenant_id.to_string(),
        user_id: ctx.user_id.to_string(),
        conversation_id: ctx.conversation_id.clone(),
        tool_name: COACH_PROPOSAL_ACTION.to_owned(),
        arguments,
        deny_reason: PROPOSAL_PARK_REASON.to_owned(),
    };
    ctx.ctx
        .repos()
        .guardian_actions
        .create_pending_action(
            &action,
            Utc::now() + Duration::minutes(PROPOSAL_TTL_MINUTES),
        )
        .await?;
    info!(
        user_id = %ctx.user_id,
        token = %token,
        title = %parked.title,
        "coach draft parked pending confirmation"
    );
    Ok(token)
}

/// Create the coach a claimed draft describes and bind it to the
/// conversation.
///
/// Reached from the claim path once the athlete's `confirm` won the
/// single-use claim. The per-user coach quota is the one `POST /api/coaches`
/// enforces, read at this moment rather than at drafting time so a coach
/// deleted in between counts. The new coach is given its catalogue handle
/// right away — `@handle` and `/coach add @handle` reach it from its first
/// second — and attached exactly the way `/coach add` attaches one.
pub(crate) async fn create_from_claimed_proposal(
    ctx: &PlatformCommandContext,
    action: &PendingGuardianAction,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();
    let repos = ctx.ctx.repos();

    let parked: ParkedProposal = serde_json::from_value(action.arguments.clone())
        .map_err(|e| AppError::internal(format!("parse parked coach draft: {e}")))?;

    let quota = coach_quota(
        ctx.ctx.admin_config().as_deref(),
        repos.coaches.as_ref(),
        ctx.user_id,
        ctx.tenant_id,
    )
    .await?;
    if quota.is_full() {
        return Ok(CommandResponse::text(reg.render(
            KEY_COACH_CREATE_QUOTA,
            locale,
            &[&quota.current.to_string(), &quota.max.to_string()],
        )));
    }

    let request = CreateCoachRequest {
        title: parked.title,
        description: Some(parked.description),
        system_prompt: parked.system_prompt,
        category: CoachCategory::parse(&parked.category),
        tags: parked.tags,
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let created = repos
        .coaches
        .create(ctx.user_id, ctx.tenant_id, &request)
        .await?;
    let coach_id = created.id.to_string();
    let handle = repos
        .store_listings
        .assign_catalogue_handle(&coach_id, ctx.tenant_id)
        .await?;
    let coach = Coach {
        handle: Some(handle.clone()),
        ..created
    };
    info!(
        user_id = %ctx.user_id,
        coach_id = %coach_id,
        handle = %handle,
        "coach created from a conversation via /coach create"
    );

    let key = match bind_coach(ctx, &coach).await? {
        CoachBinding::Personal | CoachBinding::Group(_) => KEY_COACH_CREATE_DONE,
        CoachBinding::Refused => KEY_COACH_CREATE_DONE_UNBOUND,
    };
    Ok(CommandResponse::text(reg.render(
        key,
        locale,
        &[&coach.title, &handle],
    )))
}
