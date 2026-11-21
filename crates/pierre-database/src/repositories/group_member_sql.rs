// ABOUTME: Shared statements for group memberships — join, leave, read, list, roles, consents, counts
// ABOUTME: Split from coaching_groups.rs so it stays under the size ceiling; the repository body there runs them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::coaching_groups::person_name_column;

/// The columns every member read decodes, with the member's name joined in
/// as the display name.
macro_rules! member_columns {
    () => {
        concat!(
            "m.id, m.group_id, m.user_id, m.tenant_id, m.role,
              m.peer_sharing_consent, m.coach_sharing_consent, m.coach_sharing_consent_at,
              m.consent_given_at,
              m.coach_calendar_write, m.coach_calendar_write_at,
              m.coach_wellness_read, m.coach_wellness_read_at,
              m.coach_assistant_read, m.coach_assistant_read_at,
              m.coach_sensitive_t2_read, m.coach_sensitive_t2_read_at, m.coach_sensitive_t2_read_until,
              m.joined_at, m.left_at,
              ",
            person_name_column!(),
            " AS display_name"
        )
    };
}

// ============================================================================
// coaching_group_members
// ============================================================================

/// One membership; `$7` serves the consent, the join and the coach-sharing
/// instants, `$8` is the coach-sharing grant joining carries, and `$9`..`$12`
/// the coach consents the membership starts with (none, for a member who
/// just joined).
pub(crate) const INSERT_MEMBER_SQL: &str = r"INSERT INTO coaching_group_members
              (id, group_id, user_id, tenant_id, role, peer_sharing_consent, consent_given_at, joined_at,
               coach_sharing_consent, coach_sharing_consent_at, coach_calendar_write, coach_calendar_write_at,
               coach_wellness_read, coach_wellness_read_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $7, $9, $10, $11, $12)";

/// Mark a live membership as left.
pub(crate) const REMOVE_MEMBER_SQL: &str = r"UPDATE coaching_group_members SET left_at = $1
              WHERE group_id = $2 AND user_id = $3 AND left_at IS NULL";

/// One live membership.
pub(crate) const GET_MEMBER_SQL: &str = concat!(
    "SELECT ",
    member_columns!(),
    "
              FROM coaching_group_members m
              LEFT JOIN users u ON u.id = m.user_id
              WHERE m.group_id = $1 AND m.user_id = $2 AND m.left_at IS NULL"
);

/// Every live member of a group, in join order. No tenant filter: members
/// join cross-tenant via invite codes.
pub(crate) const LIST_MEMBERS_SQL: &str = concat!(
    "SELECT ",
    member_columns!(),
    "
              FROM coaching_group_members m
              LEFT JOIN users u ON u.id = m.user_id
              WHERE m.group_id = $1 AND m.left_at IS NULL
              ORDER BY m.joined_at ASC"
);

/// Change a live member's role.
pub(crate) const UPDATE_MEMBER_ROLE_SQL: &str = r"UPDATE coaching_group_members SET role = $1
              WHERE group_id = $2 AND user_id = $3 AND left_at IS NULL";

/// Record a live member's peer-sharing decision and when it was made.
pub(crate) const UPDATE_PEER_SHARING_CONSENT_SQL: &str = r"UPDATE coaching_group_members SET peer_sharing_consent = $1, consent_given_at = $2
              WHERE group_id = $3 AND user_id = $4 AND left_at IS NULL";

/// Record a live member's coach-sharing decision and when it was made.
pub(crate) const UPDATE_COACH_SHARING_CONSENT_SQL: &str = r"UPDATE coaching_group_members SET coach_sharing_consent = $1, consent_given_at = $2,
              coach_sharing_consent_at = $2
              WHERE group_id = $3 AND user_id = $4 AND left_at IS NULL";

/// When a live member answered the notice of their coach's AI tools, NULL
/// while they have not.
pub(crate) const GET_ASSISTANT_NOTICE_ANSWERED_SQL: &str = r"SELECT assistant_notice_answered_at FROM coaching_group_members
              WHERE group_id = $1 AND user_id = $2 AND left_at IS NULL";

/// Record that a live member answered the notice of their coach's AI tools,
/// at `$1`.
pub(crate) const ANSWER_ASSISTANT_NOTICE_SQL: &str = r"UPDATE coaching_group_members SET assistant_notice_answered_at = $1
              WHERE group_id = $2 AND user_id = $3 AND left_at IS NULL";

/// A live member sets what their coach may do beyond reading their training:
/// `$1` calendar write, `$2` wellness read and `$6` sharing their health
/// records (T2), NULL to leave one as it stands, in one statement so a
/// change lands whole or not at all. A calendar consent whose value changes
/// is stamped `$3`, and so is every wellness grant, a standing one included:
/// `coach_wellness_read_at` is when the member last agreed to the wellness
/// consent as it reads now. Every health-records grant or revoke is stamped
/// `$3`; a grant lapses at `$7` (NULL: until revoked), a renewed grant
/// included, and a revoke clears the lapse instant. Every SET expression
/// reads the row as it stood before the update, on both engines.
pub(crate) const SET_COACH_CONSENTS_SQL: &str = r"UPDATE coaching_group_members
        SET coach_calendar_write = COALESCE($1, coach_calendar_write),
            coach_calendar_write_at = CASE WHEN $1 IS NULL OR $1 = coach_calendar_write
                                           THEN coach_calendar_write_at ELSE $3 END,
            coach_wellness_read = COALESCE($2, coach_wellness_read),
            coach_wellness_read_at = CASE WHEN $2 THEN $3
                                          WHEN $2 IS NULL OR $2 = coach_wellness_read
                                          THEN coach_wellness_read_at ELSE $3 END,
            coach_sensitive_t2_read = COALESCE($6, coach_sensitive_t2_read),
            coach_sensitive_t2_read_at = CASE WHEN $6 IS NULL
                                              THEN coach_sensitive_t2_read_at ELSE $3 END,
            coach_sensitive_t2_read_until = CASE WHEN $6 IS NULL THEN coach_sensitive_t2_read_until
                                                 WHEN $6 THEN $7 ELSE NULL END
      WHERE group_id = $4 AND user_id = $5 AND left_at IS NULL";

/// A live member turns on or off `$1` their human coach's AI assistant
/// including them in the roster tools, stamped `$2` when the value changes.
pub(crate) const SET_COACH_ASSISTANT_READ_SQL: &str = r"UPDATE coaching_group_members
        SET coach_assistant_read = $1,
            coach_assistant_read_at = CASE WHEN $1 = coach_assistant_read
                                           THEN coach_assistant_read_at ELSE $2 END
      WHERE group_id = $3 AND user_id = $4 AND left_at IS NULL";

/// Live members of a group, counted by group alone: they may span tenants.
pub(crate) const COUNT_MEMBERS_SQL: &str = r"SELECT COUNT(*) as cnt FROM coaching_group_members
              WHERE group_id = $1 AND left_at IS NULL";

/// A group's start, its owner's signup and its earliest non-owner membership
/// ever. The group is scoped by tenant; its members, like every member read,
/// are not, since they may span tenants.
pub(crate) const GROUP_SETUP_TIMELINE_SQL: &str = r"SELECT g.created_at AS group_created_at, u.created_at AS owner_created_at,
              (SELECT m.id FROM coaching_group_members m WHERE m.group_id = g.id AND m.role <> 'owner'
               ORDER BY m.joined_at ASC, m.id ASC LIMIT 1) AS first_member_id
              FROM coaching_groups g JOIN users u ON u.id = g.owner_id WHERE g.id = $1 AND g.tenant_id = $2";
