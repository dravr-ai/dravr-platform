// ABOUTME: The personal-thread statements: the thread one is adopted from, the designated one, and designating it
// ABOUTME: Run by impl_chat_repository!; the owner predicate is user_onboarding's personal_conversation!
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::user_onboarding::personal_conversation;

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
    " AND tenant_id = $2 AND archived_at IS NULL \
       AND NOT EXISTS (SELECT 1 FROM activity_conversations l \
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
/// qualifies, nor does a coach's thread about one athlete: the platform posts
/// the owner's own notices into the personal thread. The partial unique index
/// refuses a second personal thread for the same owner and tenant.
pub(crate) const DESIGNATE_PERSONAL_SQL: &str = r"
    UPDATE chat_conversations
    SET personal = TRUE
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND group_id IS NULL
      AND subject_user_id IS NULL";
