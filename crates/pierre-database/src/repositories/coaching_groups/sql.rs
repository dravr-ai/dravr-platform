// ABOUTME: Shared statements for coaching groups — create, read, list, update, archive, coach attachment, context queries
// ABOUTME: A child of coaching_groups.rs, which re-exports them; the repository body there runs them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The columns every group read decodes, each prefixed with `$p` (`""` for a
/// bare table, `"g."` inside a join), in the order the group decoder reads
/// them. One list, so a column added to `CoachingGroup` reaches every
/// statement at once.
macro_rules! group_columns {
    ($p:literal) => {
        concat!(
            $p,
            "id, ",
            $p,
            "tenant_id, ",
            $p,
            "name, ",
            $p,
            "description, ",
            $p,
            "agent_id, ",
            $p,
            "owner_id, ",
            $p,
            "coach_user_id, ",
            $p,
            "peer_data_sharing, ",
            $p,
            "max_members, ",
            $p,
            "is_active, ",
            $p,
            "channel_type, ",
            $p,
            "channel_chat_id, ",
            $p,
            "channel_tenant_id, ",
            $p,
            "channel_chat_title, ",
            $p,
            "channel_connected_at, ",
            $p,
            "respond_mode, ",
            $p,
            "digest_mode, ",
            $p,
            "created_at, ",
            $p,
            "updated_at"
        )
    };
}

// ============================================================================
// coaching_groups
// ============================================================================

/// The insert of one group, shared by [`INSERT_GROUP_SQL`] and
/// [`INSERT_GROUP_IF_ABSENT_SQL`].
macro_rules! insert_group_sql {
    () => {
        r"INSERT INTO coaching_groups (id, tenant_id, name, description, agent_id, owner_id,
              peer_data_sharing, max_members, is_active, channel_type, channel_chat_id,
              respond_mode, digest_mode, created_at, updated_at, coach_user_id)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, $9, $10, $11, $12, $13, $13, $14)"
    };
}

/// One group, active from the start; `$13` serves both timestamps. `$14` is
/// the human coach it starts with, `NULL` until one is attached.
pub(crate) const INSERT_GROUP_SQL: &str = insert_group_sql!();

/// [`INSERT_GROUP_SQL`] under an id the caller chose, unless a group holds
/// that id already: of two writers racing on one id, one inserts and the
/// other writes nothing.
pub(crate) const INSERT_GROUP_IF_ABSENT_SQL: &str =
    concat!(insert_group_sql!(), " ON CONFLICT (id) DO NOTHING");

/// One group by id. Coaching groups are intentionally cross-tenant: members
/// join the same group from different tenants, so the group's globally
/// unique id is the access key, not `tenant_id`. Tenant scoping happens at
/// the membership layer.
pub(crate) const GET_GROUP_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups WHERE id = $1"
);

/// The active group bound to one chat of one channel, served by the bot of
/// tenant `$1`: the tenant recorded when the chat was connected from the web
/// app, else the group's own (a group the chat created lives under its bot's
/// tenant). The served-chat unique index holds the same expression, so at
/// most one row matches.
pub(crate) const GET_GROUP_BY_CHANNEL_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE COALESCE(channel_tenant_id, tenant_id) = $1
                AND channel_type = $2 AND channel_chat_id = $3
                AND is_active = TRUE"
);

/// Every active group a user is a live member of, with their role and the
/// live member count, most recently updated first.
pub(crate) const LIST_GROUPS_FOR_USER_SQL: &str = r"SELECT g.id, g.name, g.description, g.agent_id, g.is_active, g.peer_data_sharing,
              g.created_at, m.role,
              (SELECT COUNT(*) FROM coaching_group_members m2
               WHERE m2.group_id = g.id AND m2.left_at IS NULL) AS member_count
              FROM coaching_groups g
              JOIN coaching_group_members m ON m.group_id = g.id
              WHERE m.user_id = $1 AND m.left_at IS NULL AND g.is_active = TRUE
              ORDER BY g.updated_at DESC";

/// Every active group a human coach holds, across tenants: the coach
/// attachment is the key.
pub(crate) const LIST_GROUPS_COACHED_BY_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE coach_user_id = $1 AND is_active = TRUE
              ORDER BY updated_at DESC"
);

/// Every active group one user owns within one tenant that no human coach
/// holds, oldest first: the groups a coach-access grant attaches its coach to.
pub(crate) const LIST_COACHLESS_GROUPS_OWNED_BY_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE owner_id = $1 AND tenant_id = $2
                AND coach_user_id IS NULL AND is_active = TRUE
              ORDER BY created_at ASC, id ASC"
);

/// Every live member of every active group one human coach holds, each user
/// once, across tenants like [`LIST_GROUPS_COACHED_BY_SQL`].
pub(crate) const LIST_ATHLETES_COACHED_BY_SQL: &str = r"SELECT DISTINCT m.user_id
              FROM coaching_group_members m
              JOIN coaching_groups g ON g.id = m.group_id
              WHERE g.coach_user_id = $1
                AND g.is_active = TRUE
                AND m.left_at IS NULL
              ORDER BY m.user_id";

/// Every group a user coaches, owns, or is a live member of, active or
/// archived, across tenants: each group whose reach the user holds now or
/// gets back when it is switched back on (an archive keeps its coach and its
/// members).
pub(crate) const LIST_GROUPS_HELD_BY_SQL: &str = r"SELECT g.id AS group_id FROM coaching_groups g
              WHERE g.coach_user_id = $1 OR g.owner_id = $1
              UNION
              SELECT m.group_id AS group_id FROM coaching_group_members m
              WHERE m.user_id = $1 AND m.left_at IS NULL";

/// Every active group of a tenant, newest first.
pub(crate) const LIST_ACTIVE_GROUPS_FOR_TENANT_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE tenant_id = $1 AND is_active = TRUE
              ORDER BY created_at DESC"
);

/// Patch the fields a request carries; a NULL bind keeps the column.
pub(crate) const UPDATE_GROUP_SQL: &str = r"UPDATE coaching_groups SET
              name = COALESCE($1, name),
              description = COALESCE($2, description),
              agent_id = COALESCE($3, agent_id),
              peer_data_sharing = COALESCE($4, peer_data_sharing),
              respond_mode = COALESCE($5, respond_mode),
              digest_mode = COALESCE($6, digest_mode),
              is_active = COALESCE($7, is_active),
              updated_at = $8
              WHERE id = $9 AND tenant_id = $10";

/// Archive a group under its tenant: the authoritative, tenant-scoped step
/// of a delete.
pub(crate) const ARCHIVE_GROUP_SQL: &str = r"UPDATE coaching_groups SET is_active = FALSE, updated_at = $1
              WHERE id = $2 AND tenant_id = $3";

/// Retire every invite of a group once the group is archived.
pub(crate) const DEACTIVATE_GROUP_INVITES_SQL: &str =
    "UPDATE group_invites SET is_active = FALSE WHERE group_id = $1";

/// Release every live member of a group once the group is archived.
pub(crate) const RELEASE_GROUP_MEMBERS_SQL: &str = r"UPDATE coaching_group_members SET left_at = $1
              WHERE group_id = $2 AND left_at IS NULL";

/// Attach or detach the human coach of a group under its tenant.
pub(crate) const SET_GROUP_COACH_USER_SQL: &str = r"UPDATE coaching_groups SET coach_user_id = $1, updated_at = $2
              WHERE id = $3 AND tenant_id = $4";

/// Attach a human coach to an active group under its tenant only while it
/// has none: a coach a racing writer attached first is never overwritten.
pub(crate) const ATTACH_COACH_TO_COACHLESS_GROUP_SQL: &str = r"UPDATE coaching_groups SET coach_user_id = $1, updated_at = $2
              WHERE id = $3 AND tenant_id = $4 AND coach_user_id IS NULL AND is_active = TRUE";

// ============================================================================
// Context queries (the transcript's statements: `super::group_transcript`)
// ============================================================================

/// Every active group a user is a live member of that runs one agent. No
/// tenant filter: groups span tenants via cross-tenant membership.
pub(crate) const FIND_GROUPS_FOR_USER_AND_AGENT_SQL: &str = concat!(
    "SELECT ",
    group_columns!("g."),
    "
              FROM coaching_groups g
              JOIN coaching_group_members m ON m.group_id = g.id
              WHERE m.user_id = $1 AND g.agent_id = $2
              AND m.left_at IS NULL AND g.is_active = TRUE"
);

/// Active groups one user owns within a tenant.
pub(crate) const COUNT_GROUPS_FOR_OWNER_SQL: &str = r"SELECT COUNT(*) as cnt FROM coaching_groups
              WHERE owner_id = $1 AND tenant_id = $2 AND is_active = TRUE";
