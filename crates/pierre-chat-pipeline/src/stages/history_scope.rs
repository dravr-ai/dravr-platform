// ABOUTME: What of a conversation's loaded history a turn may read before the prompt is built
// ABOUTME: A first-party-only thread on an external turn, or a coach's athlete thread the gates refuse, reads only the new message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History scope.
//!
//! Two rules keep earlier turns out of the model's input, each leaving only
//! the row the turn just wrote:
//!
//! - A thread opened from a first-party-only activity is about that activity
//!   throughout, the athlete's own first question included: an external turn
//!   reads none of it but the question it answers, and a first-party turn is
//!   stamped by it (carnet#769).
//! - A coach's thread about one athlete runs the coaching gates again on every
//!   turn (carnet#1070); once they refuse, the turn reads the coach's new
//!   message alone ([`subject_athlete::resolve_withholding_history`],
//!   carnet#1168).

use pierre_core::errors::AppResult;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_providers::ai_scope;
use pierre_tool_runtime::derived_content;

use super::subject_athlete::{self, SubjectTurn};
use crate::{ChatPipelineContext, TurnInput};

/// Keep of the loaded `history` what the turn may read — only the row
/// `current` names, the turn's own message, when either rule applies — and
/// answer the turn's standing in a coach's athlete thread.
///
/// # Errors
///
/// Returns the error reading the thread's transport policy, or an unparsable
/// sender id.
pub async fn scope_history(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    current: &str,
    history: &mut Vec<MessageRecord>,
) -> AppResult<Option<SubjectTurn>> {
    let thread = derived_content::thread_policy(
        ctx.tool_runtime.as_ref(),
        &input.conversation_tenant_id,
        parse_uuid(&input.user_id)?,
        &input.conversation_id,
    )
    .await?;
    if !ai_scope::admit_derived(thread) {
        history.retain(|row| row.id == current);
    }
    Ok(subject_athlete::resolve_withholding_history(
        &ctx.repos,
        conv,
        &input.user_id,
        history,
        current,
    )
    .await)
}
