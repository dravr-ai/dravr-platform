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
//!
//! A third keeps out only some rows: one about a group member — a tool round
//! that read them, the reply drawn from it, a summary of those — replays
//! only while the turn's sender may still read that member, by the member's
//! consent in the conversation's group ([`readable_about`], carnet#1174).
//! The history load and the compaction summaries apply it.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::member_reads;
use pierre_tool_runtime::derived_content;
use tracing::warn;
use uuid::Uuid;

use super::subject_athlete::{self, SubjectTurn};
use crate::{ChatPipelineContext, TurnInput};

/// The group members whose rows `conv` may replay into `sender`'s turn: of
/// the members its rows are about, those `sender` may still read through
/// the conversation's group — the room it is bound to, or the group a
/// coach's athlete thread was opened from — or, in a conversation bound to
/// neither, through any group they share (carnet#1174).
///
/// A member who withdrew their consent, left the group, or deleted their
/// account is not returned, and neither is anyone when the members cannot be
/// read: a row about a member the turn cannot vouch for stays out.
pub(crate) async fn readable_about(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    sender: &str,
    tenant: TenantId,
) -> Vec<Uuid> {
    let resolved = async {
        let stamped = repos
            .chat
            .list_message_subjects(&conv.id, sender, tenant)
            .await?;
        if stamped.is_empty() {
            return Ok(Vec::new());
        }
        let reader = parse_uuid(sender)?;
        let within = match conv.group_id.as_deref() {
            Some(group) => Some(parse_uuid(group)?),
            None => conv.subject().map(|subject| subject.group_id),
        };
        member_reads::readable_members(repos, reader, within, &stamped).await
    };
    let result: AppResult<Vec<Uuid>> = resolved.await;
    result.unwrap_or_else(|e| {
        warn!(
            error = %e,
            "the members this thread's rows are about could not be checked; none of those rows is replayed"
        );
        Vec::new()
    })
}

/// Keep of the loaded `history` what the turn may read — only the row
/// `current` names, the turn's own message, when either rule applies — and
/// answer the turn's standing in a coach's athlete thread.
///
/// # Errors
///
/// Returns the error reading the thread's transport policy, or an unparsable
/// sender id.
pub(crate) async fn scope_history(
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
