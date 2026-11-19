// ABOUTME: The page of a participant's conversations and its total: the list statements, emitted per backend
// ABOUTME: An outside client is listed no coach's thread about a member who does not share with the coach (EFVP 9.4c)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::chat::message_unread;

/// Whether row `c`, a conversation, is listed under the subject rule: `$n`,
/// bound to [`SubjectThreads::lists_every_subject`](super::chat_subject::SubjectThreads::lists_every_subject),
/// lists every thread; otherwise a coach's thread bound to one athlete is
/// listed only while that athlete is a live member of the thread's group who
/// shares their training with the coach. A thread bound to no athlete is
/// always listed.
macro_rules! subject_shared {
    ($n:literal) => {
        concat!(
            "(",
            $n,
            " OR c.subject_user_id IS NULL OR EXISTS ( \
               SELECT 1 FROM coaching_group_members sm \
               WHERE sm.group_id = c.subject_group_id AND sm.user_id = c.subject_user_id \
                 AND sm.left_at IS NULL AND sm.coach_sharing_consent = TRUE))"
        )
    };
}

/// One page of a participant's conversations plus the facts each row shows.
///
/// The statement asks the row-level questions inline — count of turns, count
/// of unread turns, and the newest turn — as correlated scalar subqueries over
/// `chat_messages`, each served by the `(conversation_id, created_at)` index.
/// A `GROUP BY` over a `LEFT JOIN chat_messages` counted tool rows and could
/// not say which row was newest; three scoped subqueries can.
///
/// `p` is the caller's own participant row, which is where their read marker
/// lives: "unread" is a question about one participant, so it is answered
/// from that row and never from the conversation.
///
/// A thread bound to one athlete names them through the same join a group
/// row names its group by: their display name and email come back for the
/// row parser to apply the roster naming rule to, so a page of athlete
/// threads costs no read per row.
pub(crate) const LIST_CONVERSATIONS_SQL: &str = concat!(
    r"
    SELECT c.id, c.title, c.model, c.total_tokens, c.agent_id, c.channel_type,
           c.created_at, c.updated_at, c.group_id, c.archived_at,
           c.personal, c.subject_user_id, c.subject_group_id,
           su.display_name AS subject_display_name, su.email AS subject_email,
           g.name AS group_name, co.slug AS agent_handle, co.title AS agent_title,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)) AS message_count,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)
               AND ",
    message_unread!(),
    r") AS unread_count,
           (SELECT SUBSTR(m.content, 1, $5) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_content_head,
           (SELECT m.role FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_role,
           (SELECT m.created_at FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_created_at,
           (SELECT m.first_party_only FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (m.first_party_only = FALSE OR $6)
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_first_party_only
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    LEFT JOIN coaching_groups g ON g.id = c.group_id
    LEFT JOIN agents co ON co.id = c.agent_id
    LEFT JOIN users su ON su.id = c.subject_user_id
    WHERE p.user_id = $1
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $2 OR c.group_id IS NOT NULL)
      AND ($6 OR NOT EXISTS (
        SELECT 1 FROM activity_conversations l
        WHERE l.conversation_id = c.id AND l.first_party_only = TRUE
      ))
      AND ",
    subject_shared!("$7"),
    r"
    ORDER BY c.updated_at DESC, c.id DESC
    LIMIT $3 OFFSET $4",
);

/// The participant's total — the same membership, readability and subject
/// predicates as the page.
pub(crate) const COUNT_PARTICIPATING_SQL: &str = concat!(
    r"
    SELECT COUNT(*)
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    WHERE p.user_id = $1
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $2 OR c.group_id IS NOT NULL)
      AND ($3 OR NOT EXISTS (
        SELECT 1 FROM activity_conversations l
        WHERE l.conversation_id = c.id AND l.first_party_only = TRUE
      ))
      AND ",
    subject_shared!("$4"),
);
