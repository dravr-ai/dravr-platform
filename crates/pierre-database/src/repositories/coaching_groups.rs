// ABOUTME: Shared statements, row decoders and body for coaching groups, members, invites, transcript, digest weeks
// ABOUTME: One SQL text per operation; each backend shell supplies its type, row type and uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaching groups, written once.
//!
//! The five tables (`coaching_groups`, `coaching_group_members`,
//! `group_invites`, `group_transcript_entries`, `group_digest_deliveries`)
//! key on uuid columns that are `uuid` on Postgres and hyphenated `TEXT` on
//! `SQLite`, so every id bind and read goes through a [`super::uuid_columns`]
//! codec, the macro's one per-backend argument besides the driver's row type.
//! `tenant_id` is `TEXT` on both and binds as a string.
//!
//! Booleans bind as `bool` and are spelled `TRUE`/`FALSE` in the statements:
//! Postgres has the type, `SQLite` stores 1/0 for both and reads them back as
//! `bool`. Timestamps bind as [`DateTime<Utc>`] on both; sqlx-sqlite encodes
//! one as `to_rfc3339_opts(AutoSi, false)`, the bytes `to_rfc3339()` writes,
//! so the `SQLite` TEXT columns hold what they always held.
//!
//! Every group read selects the same column list, so a listing can never
//! hand back a group with fewer fields than the by-id read does.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

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

/// The columns every member read decodes, with the user's email joined in
/// as the display name.
macro_rules! member_columns {
    () => {
        "m.id, m.group_id, m.user_id, m.tenant_id, m.role,
              m.peer_sharing_consent, m.consent_given_at, m.joined_at, m.left_at,
              u.email AS display_name"
    };
}

/// The columns every invite read decodes.
macro_rules! invite_columns {
    () => {
        "id, group_id, tenant_id, code, kind, created_by, expires_at, max_uses,
              use_count, is_active, created_at"
    };
}

// ============================================================================
// coaching_groups
// ============================================================================

/// One group, active from the start; `$13` serves both timestamps.
pub(crate) const INSERT_GROUP_SQL: &str = r"INSERT INTO coaching_groups (id, tenant_id, name, description, agent_id, owner_id,
              peer_data_sharing, max_members, is_active, channel_type, channel_chat_id,
              respond_mode, digest_mode, created_at, updated_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, $9, $10, $11, $12, $13, $13)";

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

/// The active group bound to one chat of one channel within a tenant.
pub(crate) const GET_GROUP_BY_CHANNEL_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE tenant_id = $1 AND channel_type = $2 AND channel_chat_id = $3
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

/// Every active group of one agent within a tenant, newest first.
pub(crate) const LIST_GROUPS_FOR_AGENT_SQL: &str = concat!(
    "SELECT ",
    group_columns!(""),
    "
              FROM coaching_groups
              WHERE agent_id = $1 AND tenant_id = $2 AND is_active = TRUE
              ORDER BY created_at DESC"
);

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
              max_members = COALESCE($4, max_members),
              peer_data_sharing = COALESCE($5, peer_data_sharing),
              respond_mode = COALESCE($6, respond_mode),
              digest_mode = COALESCE($7, digest_mode),
              is_active = COALESCE($8, is_active),
              updated_at = $9
              WHERE id = $10 AND tenant_id = $11";

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

// ============================================================================
// coaching_group_members
// ============================================================================

/// One membership; `$7` serves both the consent and the join instant.
pub(crate) const INSERT_MEMBER_SQL: &str = r"INSERT INTO coaching_group_members
              (id, group_id, user_id, tenant_id, role, peer_sharing_consent, consent_given_at, joined_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $7)";

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

/// Live members of a group, counted by group alone: they may span tenants.
pub(crate) const COUNT_MEMBERS_SQL: &str = r"SELECT COUNT(*) as cnt FROM coaching_group_members
              WHERE group_id = $1 AND left_at IS NULL";

// ============================================================================
// group_invites
// ============================================================================

/// One invite, unused and active from the start.
pub(crate) const INSERT_INVITE_SQL: &str = r"INSERT INTO group_invites
              (id, group_id, tenant_id, code, kind, created_by, expires_at, max_uses, use_count, is_active, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, TRUE, $9)";

/// The invite just created, read back under its tenant.
pub(crate) const GET_INVITE_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE id = $1 AND tenant_id = $2"
);

/// An active invite by code. Cross-tenant: codes are globally unique for
/// the join flow.
pub(crate) const GET_INVITE_BY_CODE_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE code = $1 AND is_active = TRUE"
);

/// Count one more redemption of an invite.
pub(crate) const INCREMENT_INVITE_USE_COUNT_SQL: &str =
    "UPDATE group_invites SET use_count = use_count + 1 WHERE id = $1";

/// Retire one invite, scoped by group so an admin cannot retire another
/// group's invite.
pub(crate) const DEACTIVATE_INVITE_SQL: &str =
    "UPDATE group_invites SET is_active = FALSE WHERE id = $1 AND group_id = $2";

/// Every invite of a group, newest first. No tenant filter: invites belong
/// to the group, and admins view cross-tenant.
pub(crate) const LIST_INVITES_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE group_id = $1
              ORDER BY created_at DESC"
);

// ============================================================================
// Context queries and the transcript
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

/// One transcript entry.
pub(crate) const INSERT_TRANSCRIPT_ENTRY_SQL: &str = r"INSERT INTO group_transcript_entries
              (id, group_id, tenant_id, author_user_id, speaker, content,
               source_conversation_id, source_message_id, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)";

/// The transcript a viewer may see, newest first. No tenant filter:
/// membership is cross-tenant and the caller verified the viewer's
/// membership. Consent-gated like the peer-grounding fetch: a peer's entries
/// need the group kill-switch AND that member's own standing consent; the
/// viewer always sees their own entries.
pub(crate) const LIST_TRANSCRIPT_VISIBLE_TO_SQL: &str = r"SELECT e.id, e.group_id, e.tenant_id, e.author_user_id, e.speaker,
              e.content, e.source_conversation_id, e.source_message_id, e.created_at,
              u.email AS author_display_name
              FROM group_transcript_entries e
              JOIN coaching_groups g ON g.id = e.group_id
              LEFT JOIN users u ON u.id = e.author_user_id
              WHERE e.group_id = $1
                AND (
                  e.author_user_id = $2
                  OR (
                    g.peer_data_sharing = TRUE
                    AND EXISTS (
                      SELECT 1 FROM coaching_group_members gm
                      WHERE gm.group_id = e.group_id
                        AND gm.user_id = e.author_user_id
                        AND gm.peer_sharing_consent = TRUE
                        AND gm.left_at IS NULL
                    )
                  )
                )
              ORDER BY e.created_at DESC, e.id DESC
              LIMIT $3";

// ============================================================================
// group_digest_deliveries
// ============================================================================

/// A missing row inserts (nobody has claimed the week); an existing one takes
/// the new lease only when the week was never finished and the old lease has
/// lapsed. Both drivers report zero rows changed when the WHERE fails — that
/// zero is the losing claim.
pub(crate) const CLAIM_GROUP_DIGEST_SQL: &str = r"INSERT INTO group_digest_deliveries
              (tenant_id, group_id, week_key, leased_until_ms, finished_at_ms)
              VALUES ($1, $2, $3, $4, 0)
              ON CONFLICT(tenant_id, group_id, week_key) DO UPDATE
              SET leased_until_ms = excluded.leased_until_ms
              WHERE group_digest_deliveries.finished_at_ms = 0
                AND group_digest_deliveries.leased_until_ms <= $5";

/// Close one group's week: stamp it finished and release the lease.
pub(crate) const FINISH_GROUP_DIGEST_SQL: &str = r"UPDATE group_digest_deliveries
              SET finished_at_ms = $1, leased_until_ms = 0
              WHERE tenant_id = $2 AND group_id = $3 AND week_key = $4";

/// Read one column by name via `try_get`, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error naming the column rather than a panic.
pub(crate) fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("coaching_groups column `{name}`: {e}")))
}

/// Read a timestamp column.
pub(crate) fn instant<'r, R>(row: &'r R, name: &str) -> AppResult<DateTime<Utc>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    column(row, name)
}

/// Emit the whole `CoachingGroupRepository` implementation for one backend
/// type, together with its four row decoders.
///
/// `$row` is the driver's row type and `$ids` the backend's uuid codec from
/// [`super::uuid_columns`] (`SqliteRow` + `TextUuid`, `PgRow` + `NativeUuid`).
/// The decoders read every uuid through the codec and everything else through
/// sqlx, so they are written once here and specialised per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_coaching_group_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// A `GroupRole` from its stored text; an unknown value is a member.
        fn parse_role(s: &str) -> GroupRole {
            GroupRole::from_str_opt(s).unwrap_or(GroupRole::Member)
        }

        /// Decode a `coaching_groups` row. An unknown `respond_mode` or
        /// `digest_mode` value reads as that mode's default.
        fn row_to_group(r: &$row) -> AppResult<CoachingGroup> {
            let respond_mode: String = column(r, "respond_mode")?;
            let digest_mode: String = column(r, "digest_mode")?;
            Ok(CoachingGroup {
                id: $ids::read(r, "id")?,
                tenant_id: column(r, "tenant_id")?,
                name: column(r, "name")?,
                description: column(r, "description")?,
                agent_id: column(r, "agent_id")?,
                owner_id: $ids::read(r, "owner_id")?,
                coach_user_id: $ids::read_opt(r, "coach_user_id")?,
                peer_data_sharing: column(r, "peer_data_sharing")?,
                respond_mode: GroupRespondMode::from_str_opt(&respond_mode).unwrap_or_default(),
                digest_mode: GroupDigestMode::from_str_opt(&digest_mode).unwrap_or_default(),
                max_members: column(r, "max_members")?,
                is_active: column(r, "is_active")?,
                channel_type: column(r, "channel_type")?,
                channel_chat_id: column(r, "channel_chat_id")?,
                created_at: instant(r, "created_at")?,
                updated_at: instant(r, "updated_at")?,
            })
        }

        /// Decode a `coaching_group_members` row joined to the user's email.
        fn row_to_member(r: &$row) -> AppResult<GroupMember> {
            let role: String = column(r, "role")?;
            Ok(GroupMember {
                id: $ids::read(r, "id")?,
                group_id: $ids::read(r, "group_id")?,
                user_id: $ids::read(r, "user_id")?,
                tenant_id: column(r, "tenant_id")?,
                role: parse_role(&role),
                peer_sharing_consent: column(r, "peer_sharing_consent")?,
                consent_given_at: instant(r, "consent_given_at")?,
                joined_at: instant(r, "joined_at")?,
                left_at: column(r, "left_at")?,
                display_name: column(r, "display_name")?,
            })
        }

        /// Decode a `group_transcript_entries` row joined to the author's email.
        fn row_to_transcript_entry(r: &$row) -> AppResult<GroupTranscriptEntry> {
            let speaker: String = column(r, "speaker")?;
            Ok(GroupTranscriptEntry {
                id: $ids::read(r, "id")?,
                group_id: $ids::read(r, "group_id")?,
                tenant_id: column(r, "tenant_id")?,
                author_user_id: $ids::read(r, "author_user_id")?,
                speaker: TranscriptSpeaker::from_str_opt(&speaker).ok_or_else(|| {
                    AppError::database(format!(
                        "group_transcript_entries column `speaker` holds unknown value `{speaker}`"
                    ))
                })?,
                content: column(r, "content")?,
                source_conversation_id: column(r, "source_conversation_id")?,
                source_message_id: column(r, "source_message_id")?,
                created_at: instant(r, "created_at")?,
                author_display_name: column(r, "author_display_name")?,
            })
        }

        /// Decode a `group_invites` row. An unknown `kind` reads as the default.
        fn row_to_invite(r: &$row) -> AppResult<GroupInvite> {
            let kind: String = column(r, "kind")?;
            Ok(GroupInvite {
                id: $ids::read(r, "id")?,
                group_id: $ids::read(r, "group_id")?,
                tenant_id: column(r, "tenant_id")?,
                code: column(r, "code")?,
                kind: GroupInviteKind::from_str_opt(&kind).unwrap_or_default(),
                created_by: $ids::read(r, "created_by")?,
                expires_at: column(r, "expires_at")?,
                max_uses: column(r, "max_uses")?,
                use_count: column(r, "use_count")?,
                is_active: column(r, "is_active")?,
                created_at: instant(r, "created_at")?,
            })
        }

        /// Decode one row of the per-user listing.
        fn row_to_summary(r: &$row) -> AppResult<GroupSummary> {
            let role: String = column(r, "role")?;
            Ok(GroupSummary {
                id: $ids::read(r, "id")?,
                name: column(r, "name")?,
                description: column(r, "description")?,
                agent_id: column(r, "agent_id")?,
                member_count: column(r, "member_count")?,
                is_active: column(r, "is_active")?,
                peer_data_sharing: column(r, "peer_data_sharing")?,
                my_role: parse_role(&role),
                created_at: instant(r, "created_at")?,
            })
        }

        #[async_trait::async_trait]
        impl CoachingGroupRepository for $ty {
            // -- Group CRUD --

            async fn create_group(
                &self,
                tenant_id: TenantId,
                group: &CoachingGroup,
            ) -> AppResult<CoachingGroup> {
                sqlx::query(INSERT_GROUP_SQL)
                    .bind($ids::bind(group.id))
                    .bind(tenant_id.to_string())
                    .bind(&group.name)
                    .bind(&group.description)
                    .bind(&group.agent_id)
                    .bind($ids::bind(group.owner_id))
                    .bind(group.peer_data_sharing)
                    .bind(group.max_members)
                    .bind(&group.channel_type)
                    .bind(&group.channel_chat_id)
                    .bind(group.respond_mode.as_str())
                    .bind(group.digest_mode.as_str())
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create group: {e}")))?;

                self.get_group(&group.id.to_string(), tenant_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Group not found after creation"))
            }

            async fn get_group(
                &self,
                group_id: &str,
                _tenant_id: TenantId,
            ) -> AppResult<Option<CoachingGroup>> {
                let row = sqlx::query(GET_GROUP_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get group: {e}")))?;

                row.as_ref().map(row_to_group).transpose()
            }

            async fn get_group_by_channel(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_chat_id: &str,
            ) -> AppResult<Option<CoachingGroup>> {
                let row = sqlx::query(GET_GROUP_BY_CHANNEL_SQL)
                    .bind(tenant_id.to_string())
                    .bind(channel_type)
                    .bind(channel_chat_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get group by channel: {e}"))
                    })?;

                row.as_ref().map(row_to_group).transpose()
            }

            async fn list_groups_for_user(&self, user_id: Uuid) -> AppResult<Vec<GroupSummary>> {
                let rows = sqlx::query(LIST_GROUPS_FOR_USER_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list groups: {e}")))?;

                rows.iter().map(row_to_summary).collect()
            }

            async fn list_groups_for_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CoachingGroup>> {
                let rows = sqlx::query(LIST_GROUPS_FOR_AGENT_SQL)
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list groups for coach: {e}"))
                    })?;

                rows.iter().map(row_to_group).collect()
            }

            async fn list_groups_coached_by(
                &self,
                coach_user_id: Uuid,
            ) -> AppResult<Vec<CoachingGroup>> {
                let rows = sqlx::query(LIST_GROUPS_COACHED_BY_SQL)
                    .bind($ids::bind(coach_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list groups coached by user: {e}"))
                    })?;

                rows.iter().map(row_to_group).collect()
            }

            async fn list_active_groups_for_tenant(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CoachingGroup>> {
                let rows = sqlx::query(LIST_ACTIVE_GROUPS_FOR_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list active groups for tenant: {e}"))
                    })?;

                rows.iter().map(row_to_group).collect()
            }

            async fn update_group(
                &self,
                group_id: &str,
                tenant_id: TenantId,
                request: &UpdateGroupRequest,
            ) -> AppResult<Option<CoachingGroup>> {
                let result = sqlx::query(UPDATE_GROUP_SQL)
                    .bind(&request.name)
                    .bind(&request.description)
                    .bind(&request.agent_id)
                    .bind(request.max_members)
                    .bind(request.peer_data_sharing)
                    .bind(request.respond_mode.map(|m| m.as_str()))
                    .bind(request.digest_mode.map(|m| m.as_str()))
                    .bind(request.is_active)
                    .bind(Utc::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update group: {e}")))?;

                if result.rows_affected() == 0 {
                    return Ok(None);
                }

                self.get_group(group_id, tenant_id).await
            }

            async fn delete_group(&self, group_id: &str, tenant_id: TenantId) -> AppResult<bool> {
                let now = Utc::now();
                // Archiving the group is the authoritative, tenant-scoped step, so it
                // runs first: a caller whose tenant does not own the group affects no
                // rows and leaves the invites alone. Once it lands, `join_group` and
                // `redeem_coach_invite` already refuse the group, and deactivating its
                // invites and releasing its members brings the stored data in line with
                // the confirm dialog's promise that the members are removed.
                let result = sqlx::query(ARCHIVE_GROUP_SQL)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete group: {e}")))?;

                if result.rows_affected() == 0 {
                    return Ok(false);
                }

                sqlx::query(DEACTIVATE_GROUP_INVITES_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deactivate group invites: {e}"))
                    })?;

                sqlx::query(RELEASE_GROUP_MEMBERS_SQL)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release group members: {e}"))
                    })?;

                Ok(true)
            }

            async fn set_group_coach_user(
                &self,
                group_id: &str,
                coach_user_id: Option<Uuid>,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_GROUP_COACH_USER_SQL)
                    .bind($ids::bind_opt(coach_user_id))
                    .bind(Utc::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to set group coach: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            // -- Membership --

            async fn add_member(&self, member: &GroupMember) -> AppResult<GroupMember> {
                sqlx::query(INSERT_MEMBER_SQL)
                    .bind($ids::bind(member.id))
                    .bind($ids::bind(member.group_id))
                    .bind($ids::bind(member.user_id))
                    .bind(&member.tenant_id)
                    .bind(member.role.as_str())
                    .bind(member.peer_sharing_consent)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add member: {e}")))?;

                self.get_member(&member.group_id.to_string(), member.user_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Member not found after creation"))
            }

            async fn remove_member(&self, group_id: &str, user_id: Uuid) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_MEMBER_SQL)
                    .bind(Utc::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to remove member: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn get_member(
                &self,
                group_id: &str,
                user_id: Uuid,
            ) -> AppResult<Option<GroupMember>> {
                let row = sqlx::query(GET_MEMBER_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get member: {e}")))?;

                row.as_ref().map(row_to_member).transpose()
            }

            async fn list_members(&self, group_id: &str) -> AppResult<Vec<GroupMember>> {
                let rows = sqlx::query(LIST_MEMBERS_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list members: {e}")))?;

                rows.iter().map(row_to_member).collect()
            }

            async fn update_member_role(
                &self,
                group_id: &str,
                user_id: Uuid,
                role: GroupRole,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_MEMBER_ROLE_SQL)
                    .bind(role.as_str())
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update member role: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn update_peer_sharing_consent(
                &self,
                group_id: &str,
                user_id: Uuid,
                consent: bool,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_PEER_SHARING_CONSENT_SQL)
                    .bind(consent)
                    .bind(Utc::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update peer sharing consent: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn count_members(&self, group_id: &str) -> AppResult<i64> {
                let row = sqlx::query(COUNT_MEMBERS_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count members: {e}")))?;

                column(&row, "cnt")
            }

            // -- Invites --

            async fn create_invite(&self, invite: &GroupInvite) -> AppResult<GroupInvite> {
                sqlx::query(INSERT_INVITE_SQL)
                    .bind($ids::bind(invite.id))
                    .bind($ids::bind(invite.group_id))
                    .bind(&invite.tenant_id)
                    .bind(&invite.code)
                    .bind(invite.kind.as_str())
                    .bind($ids::bind(invite.created_by))
                    .bind(invite.expires_at)
                    .bind(invite.max_uses)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create invite: {e}")))?;

                let row = sqlx::query(GET_INVITE_SQL)
                    .bind($ids::bind(invite.id))
                    .bind(&invite.tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch invite after creation: {e}"))
                    })?;

                row_to_invite(&row)
            }

            async fn get_invite_by_code(&self, code: &str) -> AppResult<Option<GroupInvite>> {
                let row = sqlx::query(GET_INVITE_BY_CODE_SQL)
                    .bind(code)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get invite by code: {e}"))
                    })?;

                row.as_ref().map(row_to_invite).transpose()
            }

            async fn increment_invite_use_count(&self, invite_id: &str) -> AppResult<bool> {
                let result = sqlx::query(INCREMENT_INVITE_USE_COUNT_SQL)
                    .bind($ids::bind_text(invite_id)?)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to increment invite use count: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn deactivate_invite(&self, group_id: &str, invite_id: &str) -> AppResult<bool> {
                let result = sqlx::query(DEACTIVATE_INVITE_SQL)
                    .bind($ids::bind_text(invite_id)?)
                    .bind($ids::bind_text(group_id)?)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to deactivate invite: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn list_invites(&self, group_id: &str) -> AppResult<Vec<GroupInvite>> {
                let rows = sqlx::query(LIST_INVITES_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list invites: {e}")))?;

                rows.iter().map(row_to_invite).collect()
            }

            // -- Context queries --

            async fn find_groups_for_user_and_agent(
                &self,
                user_id: Uuid,
                agent_id: &str,
            ) -> AppResult<Vec<CoachingGroup>> {
                let rows = sqlx::query(FIND_GROUPS_FOR_USER_AND_AGENT_SQL)
                    .bind($ids::bind(user_id))
                    .bind(agent_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find groups for user+coach: {e}"))
                    })?;

                rows.iter().map(row_to_group).collect()
            }

            async fn count_groups_for_owner(
                &self,
                owner_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<i64> {
                let row = sqlx::query(COUNT_GROUPS_FOR_OWNER_SQL)
                    .bind($ids::bind(owner_id))
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count groups: {e}")))?;

                column(&row, "cnt")
            }

            async fn append_transcript_entry(
                &self,
                entry: &NewGroupTranscriptEntry<'_>,
            ) -> AppResult<()> {
                sqlx::query(INSERT_TRANSCRIPT_ENTRY_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind($ids::bind_text(entry.group_id)?)
                    .bind(entry.tenant_id)
                    .bind($ids::bind(entry.author_user_id))
                    .bind(entry.speaker.as_str())
                    .bind(entry.content)
                    .bind(entry.source_conversation_id)
                    .bind(entry.source_message_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to append transcript entry: {e}"))
                    })?;

                Ok(())
            }

            async fn list_transcript_visible_to(
                &self,
                group_id: &str,
                viewer_user_id: Uuid,
                limit: i64,
            ) -> AppResult<Vec<GroupTranscriptEntry>> {
                let rows = sqlx::query(LIST_TRANSCRIPT_VISIBLE_TO_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(viewer_user_id))
                    .bind(limit.clamp(1, 500))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list transcript entries: {e}"))
                    })?;

                rows.iter().map(row_to_transcript_entry).collect()
            }

            async fn claim_group_digest(
                &self,
                tenant_id: TenantId,
                group_id: Uuid,
                week_key: &str,
                now_ms: i64,
                lease_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_GROUP_DIGEST_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(group_id))
                    .bind(week_key)
                    .bind(now_ms.saturating_add(lease_ms))
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim group digest: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_group_digest(
                &self,
                tenant_id: TenantId,
                group_id: Uuid,
                week_key: &str,
                now_ms: i64,
            ) -> AppResult<()> {
                sqlx::query(FINISH_GROUP_DIGEST_SQL)
                    .bind(now_ms)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(group_id))
                    .bind(week_key)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish group digest: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_coaching_group_repository;
