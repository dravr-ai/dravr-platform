// ABOUTME: The speaker-labeled ambient transcript a group turn's prompt carries, read consent-gated
// ABOUTME: Bounded in lines and characters so a busy room costs a fixed number of prompt tokens

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use pierre_core::models::TranscriptSpeaker;
use pierre_core::narration::scrub_replayed_narration;
use tracing::warn;
use uuid::Uuid;

use super::PendingDispatch;

/// Maximum ambient-transcript lines injected into a group turn's prompt.
const AMBIENT_TRANSCRIPT_MAX_LINES: usize = 25;

/// Maximum characters kept per ambient-transcript line (grapheme-unaware
/// char truncation is fine for prompt context).
const AMBIENT_TRANSCRIPT_MAX_LINE_CHARS: usize = 240;

/// Build the speaker-labeled ambient transcript for a group turn.
///
/// Reads the group's shared room transcript (`group_transcript_entries`) —
/// the same surface-neutral read model web and mobile members read — through
/// the consent-gated visibility query, with the requesting member as the
/// viewer: an unconsented peer's content never enters this member's prompt.
/// Member rows are labeled with the sender's display name, coach rows
/// "Coach". The triggering message is not yet in the transcript (the turn
/// pipeline fans it out at persistence), so nothing is excluded here.
/// Returns `None` when the room has no other recent messages, so DM-shaped
/// groups cost no prompt tokens.
pub(super) async fn build_group_ambient_context(dispatch: &PendingDispatch) -> Option<String> {
    let conversation = dispatch
        .resources
        .common
        .repos
        .chat
        .get_conversation(
            &dispatch.session.conversation,
            &dispatch.session.user_id,
            dispatch.session_tenant_id,
        )
        .await
        .ok()??;
    let group_id = conversation.group_id?;

    let limit = i64::try_from(AMBIENT_TRANSCRIPT_MAX_LINES).unwrap_or(25);
    let entries = match dispatch
        .resources
        .common
        .repos
        .groups
        .list_transcript_visible_to(&group_id, dispatch.auth_result.user_id, limit)
        .await
    {
        Ok(entries) => entries,
        Err(e) => {
            warn!(error = %e, "ambient transcript load failed; dispatching without it");
            return None;
        }
    };

    // Newest-first from the query; restore chronological order, cap the
    // per-line length.
    let mut lines: Vec<String> = Vec::new();
    let mut label_cache: HashMap<String, String> = HashMap::new();
    for entry in &entries {
        if entry.content.is_empty() {
            continue;
        }
        // A coach line re-enters every member's prompt from here, and the
        // `capability_claim_unverified` stamp lives on the author's row, not
        // here — so the replay scrub runs on the way in (2026-08-30: a consent
        // denial replayed to the peer it named, after he had consented).
        let (label, content) = match entry.speaker {
            TranscriptSpeaker::Coach => (
                "Coach".to_owned(),
                scrub_replayed_narration(&entry.content).cleaned,
            ),
            TranscriptSpeaker::Member => {
                let author = entry.author_user_id.to_string();
                let label = speaker_label(dispatch, &author, &mut label_cache).await;
                (label, entry.content.clone())
            }
        };
        if content.trim().is_empty() {
            continue;
        }
        let truncated: String = content
            .chars()
            .take(AMBIENT_TRANSCRIPT_MAX_LINE_CHARS)
            .collect();
        lines.push(format!("{label}: {truncated}"));
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();

    Some(format!(
        "## Recent group chat\n\
         This conversation happens inside a group chat. The lines below are \
         the room's most recent messages, oldest first, for context only — \
         answer the current message, which follows the conversation history. \
         Never prefix your reply with a name label.\n\n{}",
        lines.join("\n")
    ))
}

/// Resolve a member's display label for the ambient transcript, caching per
/// build. Falls back to the email local-part, then a neutral "Member".
async fn speaker_label(
    dispatch: &PendingDispatch,
    user_id: &str,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(cached) = cache.get(user_id) {
        return cached.clone();
    }
    let label = match Uuid::parse_str(user_id) {
        Ok(uuid) => match dispatch.resources.common.repos.users.get_global(uuid).await {
            Ok(Some(user)) => user
                .display_name
                .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Member").to_owned()),
            _ => "Member".to_owned(),
        },
        Err(_) => "Member".to_owned(),
    };
    cache.insert(user_id.to_owned(), label.clone());
    label
}
