// ABOUTME: The personal and plan-thread statements: the thread one is adopted from, the designated ones, and designating them
// ABOUTME: Run by impl_chat_repository!; the owner predicate is user_onboarding's personal_conversation!
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::user_onboarding::personal_conversation;

/// No messaging session but a direct message's writes into the
/// `chat_conversations` row in scope: a group chat's thread, and one whose
/// session has not recorded its chat, is never adopted or designated as the
/// owner's personal thread (carnet#1422). A group chat whose group binding
/// failed keeps no group, exactly as a DM's thread does, so the session is
/// what tells them apart. The sessions read are the row's own tenant's,
/// where its sessions live (`messaging_sessions.tenant_id`).
macro_rules! written_by_no_room {
    () => {
        "NOT EXISTS (SELECT 1 FROM messaging_sessions s \
         WHERE s.pierre_conversation_id = chat_conversations.id \
         AND CAST(s.tenant_id AS TEXT) = chat_conversations.tenant_id \
         AND s.is_direct_message IS NOT TRUE)"
    };
}

/// The newest active conversation that is the user's alone in a tenant and
/// about no single activity — the thread a personal one is adopted from. The
/// predicate addresses `chat_conversations` unaliased, so it runs in a
/// subquery of its own and the outer read projects the usual columns. `$1` is
/// the user id as text, `$2` the tenant.
pub(crate) const LATEST_PERSONAL_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     WHERE c.id = ( \
       SELECT id FROM chat_conversations \
       WHERE ",
    personal_conversation!(),
    " AND tenant_id = $2 AND archived_at IS NULL AND ",
    written_by_no_room!(),
    " AND NOT EXISTS (SELECT 1 FROM activity_conversations l \
         WHERE l.conversation_id = chat_conversations.id) \
       ORDER BY updated_at DESC \
       LIMIT 1 \
     )"
);

/// The user's personal thread in a tenant, once one is designated.
pub(crate) const PERSONAL_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     WHERE c.user_id = $1 AND c.tenant_id = $2 AND c.personal = TRUE"
);

/// Designate one of the owner's threads their personal one. A room never
/// qualifies, a group chat's thread included, nor does a coach's thread about
/// one athlete: the platform posts the owner's own notices into the personal
/// thread. The partial unique index
/// refuses a second personal thread for the same owner and tenant, and a
/// thread already designated is not designated again, so of two requests
/// adopting the same thread only one is told it did.
pub(crate) const DESIGNATE_PERSONAL_SQL: &str = concat!(
    "UPDATE chat_conversations \
     SET personal = TRUE \
     WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND group_id IS NULL \
       AND subject_user_id IS NULL AND NOT personal AND NOT plan_thread AND ",
    written_by_no_room!()
);

/// The user's own plan thread in a tenant, once one is designated
/// (carnet#1841).
pub(crate) const PLAN_THREAD_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     WHERE c.user_id = $1 AND c.tenant_id = $2 AND c.plan_thread = TRUE"
);
