// ABOUTME: Verifies the group room entry a coach's message quotes into their thread about one athlete (carnet#1249)
// ABOUTME: The thread's own coaching gates, then the room's consent rule; the quote is copied from the room, never the client

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A coach drafting an athlete's plan privately sends their "Only Dravr"
//! turns to their thread about that athlete (carnet#1070), and "Adjust plan
//! from this" on the athlete's room message sends that message along as a
//! quote. The client names the room entry by id; everything the quote says
//! is read here, from the room, under the rule the room itself applies — so a
//! quote is the athlete's words as the coach may read them, or nothing.
//!
//! Refusals carry `details.reason`:
//!
//! - 400 `quote_not_allowed` — the conversation is not the caller's own
//!   thread about an athlete, or the message is a slash command or a
//!   suggestion's postback, which store no question to carry it;
//! - the coaching gates' own refusals, as opening the thread gives them
//!   (403 `not_group_coach`, 404 `not_a_member`, 403 `coach_sharing_revoked`,
//!   409 `no_tenant` / `several_tenants`);
//! - 404 `quote_not_found` — the thread's group holds no entry by that id (an
//!   entry of another group included);
//! - 403 `quote_withheld` — the room's consent rule withholds the entry from
//!   the caller, or the transport may not serve it;
//! - 400 `quote_not_athlete` — the entry is not the thread's athlete's own
//!   words: another member's, or an agent reply or a correction attributed to
//!   the athlete.
//!
//! A quote stays the athlete's words after it is sent: a read of the thread
//! shows it only while the same gates admit the coach ([`quote_reading`]),
//! so a thread never shows what the room has come to hide.

use pierre_chat_pipeline::suggestions::is_postback;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{RoomEntryBody, TranscriptSpeaker};
use pierre_core::models::{RoomQuote, TenantId};
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::coached_athlete::{coached_athlete_access, CoachedAthleteRefusal};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::common::{refusal, with_reason};
use super::dto::RoomQuoteResponse;

/// The room words `entry_id` names, as the quote `caller`'s message in
/// `conversation` may carry.
///
/// `content` is the message being sent: a slash command or a postback is
/// answered without a question row to store a quote on, so it carries none.
///
/// # Errors
///
/// Every refusal the module docs list, and the repository's error from a
/// failed read.
pub(super) async fn verify_room_quote(
    repos: &RepositoryRegistry,
    conversation: &ConversationRecord,
    caller: Uuid,
    entry_id: &str,
    content: &str,
) -> Result<RoomQuote, AppError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('/') || is_postback(content) {
        return Err(refusal(
            ErrorCode::InvalidInput,
            "quote_not_allowed",
            "A quote rides a message, not a command",
        ));
    }
    let subject = conversation
        .subject()
        .filter(|_| conversation.user_id == caller.to_string())
        .ok_or_else(|| {
            refusal(
                ErrorCode::InvalidInput,
                "quote_not_allowed",
                "A room message can be quoted only in your own thread about its athlete",
            )
        })?;
    let access = coached_athlete_access(repos, caller, subject.athlete_user_id, subject.group_id)
        .await?
        .map_err(CoachedAthleteRefusal::into_error)?;

    let not_found = || {
        refusal(
            ErrorCode::ResourceNotFound,
            "quote_not_found",
            "The thread's group holds no such message",
        )
    };
    let entry_id = Uuid::parse_str(entry_id.trim()).map_err(|_| not_found())?;
    let group_tenant = TenantId::parse_str(&access.group.tenant_id)
        .map_err(|e| AppError::internal(format!("Invalid group tenant: {e}")))?;
    // The athlete's room words are written under the group's tenant, by its
    // bound chat's session, or under her own, by her in-app room thread.
    let entry = repos
        .groups
        .get_room_transcript_entry_for(
            &subject.group_id.to_string(),
            caller,
            entry_id,
            [group_tenant, access.tenant],
        )
        .await?
        .ok_or_else(not_found)?;

    let shared = match entry.body {
        RoomEntryBody::Shared(shared) if ai_scope::admit_derived(entry.transport_policy) => shared,
        RoomEntryBody::Shared(_) | RoomEntryBody::Withheld => {
            return Err(with_reason(
                AppError::new(
                    ErrorCode::PermissionDenied,
                    "That message is not shared with you",
                ),
                "quote_withheld",
            ));
        }
    };
    if entry.speaker != TranscriptSpeaker::Member
        || shared.author_user_id != subject.athlete_user_id
    {
        return Err(refusal(
            ErrorCode::InvalidInput,
            "quote_not_athlete",
            "Only the thread's athlete's own messages can be quoted here",
        ));
    }

    info!(
        coach_user_id = %caller,
        athlete_user_id = %subject.athlete_user_id,
        group_id = %subject.group_id,
        entry_id = %entry.id,
        "coach quoted their athlete's room message into their thread"
    );
    Ok(RoomQuote {
        entry_id: entry.id.to_string(),
        author_user_id: subject.athlete_user_id.to_string(),
        author_display_name: access.display_name,
        content: shared.content,
        created_at: entry.created_at.to_rfc3339(),
        transport_policy: entry.transport_policy,
    })
}

/// Whether a response may show the room words a coach's questions quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuoteReading {
    /// The thread's coaching gates admit the caller to the athlete's words.
    Shown,
    /// The gates refuse or could not be read, or the conversation is no
    /// caller's thread about an athlete: no quote leaves.
    Withheld,
}

impl QuoteReading {
    /// The quote `record` carries, as a response under this reading shows it.
    pub(super) fn quote_of(self, record: &MessageRecord) -> Option<RoomQuoteResponse> {
        match self {
            Self::Shown => record.room_quote().map(Into::into),
            Self::Withheld => None,
        }
    }
}

/// How `caller`'s read of `conversation` shows quotes, decided once for the
/// whole read.
///
/// A quote is the athlete's room words, copied when the coach sent it. The
/// room hides them once she stops sharing with the coach, leaves, or the
/// coach is detached, and so does the thread: the same gates it runs on every
/// turn ([`coached_athlete_access`]), asked once per request, never per row.
pub(super) async fn quote_reading(
    repos: &RepositoryRegistry,
    conversation: &ConversationRecord,
    caller: Uuid,
) -> QuoteReading {
    let Some(subject) = conversation
        .subject()
        .filter(|_| conversation.user_id == caller.to_string())
    else {
        return QuoteReading::Withheld;
    };
    match coached_athlete_access(repos, caller, subject.athlete_user_id, subject.group_id).await {
        Ok(Ok(_)) => QuoteReading::Shown,
        Ok(Err(refused)) => {
            debug!(
                athlete_user_id = %subject.athlete_user_id,
                group_id = %subject.group_id,
                reason = refused.reason(),
                "coach's athlete thread read: the coaching gates refuse; its quotes are withheld"
            );
            QuoteReading::Withheld
        }
        Err(e) => {
            warn!(
                athlete_user_id = %subject.athlete_user_id,
                group_id = %subject.group_id,
                error = %e,
                "coach's athlete thread read: the coaching gates could not be read; its quotes are withheld"
            );
            QuoteReading::Withheld
        }
    }
}
