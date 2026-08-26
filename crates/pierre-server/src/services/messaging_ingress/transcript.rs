// ABOUTME: Ambient group-chatter capture into the shared group transcript read model
// ABOUTME: The one path unaddressed room messages take into the surface-neutral room view
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use tracing::warn;
use uuid::Uuid;

use super::{content_body_text, ResolvedSession};
use crate::mcp::resources::ServerContext;
use pierre_core::models::groups::{NewGroupTranscriptEntry, TranscriptSpeaker};
use pierre_core::models::messaging::IncomingMessage;
use pierre_core::models::TenantId;

/// Append an ambient (unaddressed) group message to the group's shared room
/// transcript.
///
/// Ambient chatter never runs a turn, so the chat-pipeline fan-out that
/// records addressed messages never sees it — this is its one path into the
/// surface-neutral read model that web and mobile members (and the ambient
/// prompt block) read. Best-effort like the surrounding capture: no reply
/// was owed on this message, so a failure is a transcript gap, not a
/// dropped answer.
pub(super) async fn append_ambient_transcript_entry(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    session: &ResolvedSession,
    message: &IncomingMessage,
) {
    let Some(body) = content_body_text(&message.content).filter(|b| !b.is_empty()) else {
        return;
    };
    let conversation = match resources
        .common
        .repos
        .chat
        .get_conversation(&session.conversation, &session.user_id, tenant_id)
        .await
    {
        Ok(Some(conv)) => conv,
        Ok(None) => return,
        Err(e) => {
            warn!(
                error = %e,
                conversation_id = %session.conversation,
                "ambient transcript append: conversation lookup failed; room entry lost"
            );
            return;
        }
    };
    let Some(group_id) = conversation.group_id.as_deref() else {
        return;
    };
    let Ok(author_user_id) = Uuid::parse_str(&session.user_id) else {
        warn!(
            conversation_id = %session.conversation,
            "ambient transcript append: session user id is not a UUID; room entry lost"
        );
        return;
    };
    let tenant_str = tenant_id.to_string();
    let entry = NewGroupTranscriptEntry {
        group_id,
        tenant_id: &tenant_str,
        author_user_id,
        speaker: TranscriptSpeaker::Member,
        content: &body,
        source_conversation_id: None,
        source_message_id: Some(&message.channel_message_id),
    };
    if let Err(e) = resources
        .common
        .repos
        .groups
        .append_transcript_entry(&entry)
        .await
    {
        warn!(
            error = %e,
            group_id = %group_id,
            "ambient transcript append failed; room entry lost"
        );
    }
}
