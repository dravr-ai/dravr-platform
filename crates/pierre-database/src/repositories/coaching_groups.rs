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
//! `bool`. Timestamps bind as [`Timestamp`] on both; sqlx-sqlite encodes
//! one as `to_rfc3339_opts(AutoSi, false)`, the bytes `to_rfc3339()` writes,
//! so the `SQLite` TEXT columns hold what they always held.
//!
//! Every group read selects the same column list, so a listing can never
//! hand back a group with fewer fields than the by-id read does.

mod sql;

pub(crate) use sql::{
    ARCHIVE_GROUP_SQL, ATTACH_COACH_TO_COACHLESS_GROUP_SQL, COUNT_GROUPS_FOR_OWNER_SQL,
    DEACTIVATE_GROUP_INVITES_SQL, FIND_GROUPS_FOR_USER_AND_AGENT_SQL, GET_GROUP_BY_CHANNEL_SQL,
    GET_GROUP_SQL, INSERT_GROUP_IF_ABSENT_SQL, INSERT_GROUP_SQL, LIST_ACTIVE_GROUPS_FOR_TENANT_SQL,
    LIST_ATHLETES_COACHED_BY_SQL, LIST_COACHLESS_GROUPS_OWNED_BY_SQL, LIST_GROUPS_COACHED_BY_SQL,
    LIST_GROUPS_FOR_USER_SQL, LIST_GROUPS_HELD_BY_SQL, RELEASE_GROUP_MEMBERS_SQL,
    SET_GROUP_COACH_USER_SQL, UPDATE_GROUP_SQL,
};

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};

/// The name a person reads for the user joined in as `u`: their display name
/// when it is not blank, else their email — the rule
/// `pierre_services::delegated_connections::person_name` applies to a user
/// row already in hand. `NULL` only when the user row is missing.
macro_rules! person_name_column {
    () => {
        person_name_column!("u")
    };
    ($alias:literal) => {
        concat!(
            "COALESCE(NULLIF(TRIM(",
            $alias,
            ".display_name), ''), ",
            $alias,
            ".email)"
        )
    };
}
pub(crate) use person_name_column;

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
pub(crate) fn instant<'r, R>(row: &'r R, name: &str) -> AppResult<Timestamp>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Timestamp: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
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
                chat_connection: chat_connection(r)?,
                created_at: instant(r, "created_at")?,
                updated_at: instant(r, "updated_at")?,
            })
        }

        /// The web-app connection a `coaching_groups` row records, when its
        /// chat binding was made that way: `channel_connected_at` and the
        /// serving tenant are both set. A row the chat created leaves both
        /// NULL and reads as `None`.
        fn chat_connection(r: &$row) -> AppResult<Option<GroupChatConnection>> {
            let connected_at: Option<Timestamp> = column(r, "channel_connected_at")?;
            let serving: Option<String> = column(r, "channel_tenant_id")?;
            let (Some(connected_at), Some(serving)) = (connected_at, serving) else {
                return Ok(None);
            };
            let channel_tenant_id = TenantId::parse_str(&serving).map_err(|e| {
                AppError::database(format!("coaching_groups column `channel_tenant_id`: {e}"))
            })?;
            Ok(Some(GroupChatConnection {
                channel_tenant_id,
                chat_title: column(r, "channel_chat_title")?,
                connected_at,
            }))
        }

        /// Decode a `coaching_group_members` row joined to the member's name.
        fn row_to_member(r: &$row) -> AppResult<GroupMember> {
            let role: String = column(r, "role")?;
            Ok(GroupMember {
                id: $ids::read(r, "id")?,
                group_id: $ids::read(r, "group_id")?,
                user_id: $ids::read(r, "user_id")?,
                tenant_id: column(r, "tenant_id")?,
                role: parse_role(&role),
                peer_sharing_consent: column(r, "peer_sharing_consent")?,
                coach_sharing_consent: column(r, "coach_sharing_consent")?,
                coach_sharing_consent_at: column(r, "coach_sharing_consent_at")?,
                coach_consents: CoachConsents {
                    calendar_write: column(r, "coach_calendar_write")?,
                    calendar_write_at: column(r, "coach_calendar_write_at")?,
                    wellness_read: column(r, "coach_wellness_read")?,
                    wellness_read_at: column(r, "coach_wellness_read_at")?,
                    assistant_read: column(r, "coach_assistant_read")?,
                    assistant_read_at: column(r, "coach_assistant_read_at")?,
                    missed_session_check: column(r, "coach_missed_session_check")?,
                    missed_session_check_at: column(r, "coach_missed_session_check_at")?,
                    routine_detection: column(r, "coach_routine_detection")?,
                    routine_detection_at: column(r, "coach_routine_detection_at")?,
                    sensitive_t2_read: column(r, "coach_sensitive_t2_read")?,
                    sensitive_t2_read_at: column(r, "coach_sensitive_t2_read_at")?,
                    sensitive_t2_read_until: column(r, "coach_sensitive_t2_read_until")?,
                },
                consent_given_at: instant(r, "consent_given_at")?,
                joined_at: instant(r, "joined_at")?,
                left_at: column(r, "left_at")?,
                display_name: column(r, "display_name")?,
            })
        }

        /// Decode a `group_transcript_entries` row joined to the author's name.
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
                transport_policy: TransportPolicy::from_first_party_only(column(
                    r,
                    "first_party_only",
                )?),
                author_coached: column(r, "author_coached")?,
                room_reads: column(r, "room_reads")?,
            })
        }

        /// Decode one row of a room page. A withheld row carries NULL for
        /// everything but its place in the room, so only a visible one is
        /// read past its id, speaker and time.
        fn row_to_room_entry(r: &$row) -> AppResult<RoomTranscriptEntry> {
            let speaker: String = column(r, "speaker")?;
            let visible: bool = column(r, "visible")?;
            let body = if visible {
                let corrected_by = match $ids::read_opt(r, "corrector_user_id")? {
                    Some(user_id) => Some(RoomCorrector {
                        user_id,
                        display_name: column(r, "corrector_display_name")?,
                    }),
                    None => None,
                };
                let corrected_content: Option<String> = column(r, "corrects_content")?;
                let corrects = match ($ids::read_opt(r, "corrects_entry_id")?, corrected_content) {
                    (Some(entry_id), Some(content)) => Some(CorrectedReply {
                        entry_id,
                        message_id: column(r, "corrects_message_id")?,
                        content,
                    }),
                    _ => None,
                };
                RoomEntryBody::Shared(Box::new(SharedRoomEntry {
                    author_user_id: $ids::read(r, "author_user_id")?,
                    author_display_name: column(r, "author_display_name")?,
                    content: column(r, "content")?,
                    source_conversation_id: column(r, "source_conversation_id")?,
                    source_message_id: column(r, "source_message_id")?,
                    corrects,
                    corrected_by,
                }))
            } else {
                RoomEntryBody::Withheld
            };
            Ok(RoomTranscriptEntry {
                id: $ids::read(r, "id")?,
                speaker: TranscriptSpeaker::from_str_opt(&speaker).ok_or_else(|| {
                    AppError::database(format!(
                        "group_transcript_entries column `speaker` holds unknown value `{speaker}`"
                    ))
                })?,
                created_at: instant(r, "created_at")?,
                body,
                transport_policy: TransportPolicy::from_first_party_only(column(
                    r,
                    "first_party_only",
                )?),
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
                approves_signup: column(r, "approves_signup")?,
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

        impl $ty {
            /// Insert `group` under its own id with `sql` ([`INSERT_GROUP_SQL`]
            /// or [`INSERT_GROUP_IF_ABSENT_SQL`]): how many rows it wrote.
            async fn insert_group(
                &self,
                sql: &str,
                tenant_id: TenantId,
                group: &CoachingGroup,
            ) -> AppResult<u64> {
                Ok(sqlx::query(sql)
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
                    .bind(Timestamp::now())
                    .bind($ids::bind_opt(group.coach_user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create group: {e}")))?
                    .rows_affected())
            }
        }

        #[async_trait::async_trait]
        impl CoachingGroupRepository for $ty {
            // -- Group CRUD --

            async fn create_group(
                &self,
                tenant_id: TenantId,
                group: &CoachingGroup,
            ) -> AppResult<CoachingGroup> {
                self.insert_group(INSERT_GROUP_SQL, tenant_id, group)
                    .await?;
                self.get_group(&group.id.to_string(), tenant_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Group not found after creation"))
            }

            async fn create_group_if_absent(
                &self,
                tenant_id: TenantId,
                group: &CoachingGroup,
            ) -> AppResult<bool> {
                Ok(self
                    .insert_group(INSERT_GROUP_IF_ABSENT_SQL, tenant_id, group)
                    .await?
                    > 0)
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

            async fn list_coachless_groups_owned_by(
                &self,
                owner_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CoachingGroup>> {
                let rows = sqlx::query(LIST_COACHLESS_GROUPS_OWNED_BY_SQL)
                    .bind($ids::bind(owner_id))
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list the coachless groups a user owns: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_group).collect()
            }

            async fn list_athletes_coached_by(&self, coach_user_id: Uuid) -> AppResult<Vec<Uuid>> {
                let rows = sqlx::query(LIST_ATHLETES_COACHED_BY_SQL)
                    .bind($ids::bind(coach_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list athletes coached by user: {e}"))
                    })?;

                rows.iter().map(|r| $ids::read(r, "user_id")).collect()
            }

            async fn list_groups_held_by(&self, user_id: Uuid) -> AppResult<Vec<Uuid>> {
                let rows = sqlx::query(LIST_GROUPS_HELD_BY_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list the groups a user holds: {e}"))
                    })?;

                rows.iter().map(|r| $ids::read(r, "group_id")).collect()
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
                    .bind(request.peer_data_sharing)
                    .bind(request.respond_mode.map(|m| m.as_str()))
                    .bind(request.digest_mode.map(|m| m.as_str()))
                    .bind(request.is_active)
                    .bind(Timestamp::now())
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
                let now = Timestamp::now();
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
                    .bind(Timestamp::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to set group coach: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn attach_coach_to_coachless_group(
                &self,
                group_id: &str,
                coach_user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(ATTACH_COACH_TO_COACHLESS_GROUP_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind(Timestamp::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to attach a coachless group's coach: {e}"
                        ))
                    })?;

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
                    .bind(Timestamp::now())
                    .bind(member.coach_sharing_consent)
                    .bind(member.coach_consents.calendar_write)
                    .bind(member.coach_consents.calendar_write_at)
                    .bind(member.coach_consents.wellness_read)
                    .bind(member.coach_consents.wellness_read_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add member: {e}")))?;

                self.get_member(&member.group_id.to_string(), member.user_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Member not found after creation"))
            }

            async fn remove_member(&self, group_id: &str, user_id: Uuid) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_MEMBER_SQL)
                    .bind(Timestamp::now())
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
                    .bind(Timestamp::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update peer sharing consent: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn update_coach_sharing_consent(
                &self,
                group_id: &str,
                user_id: Uuid,
                consent: bool,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_COACH_SHARING_CONSENT_SQL)
                    .bind(consent)
                    .bind(Timestamp::now())
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update coach sharing consent: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn set_coach_sensitive_reads(
                &self,
                group_id: &str,
                user_id: Uuid,
                calendar_write: Option<bool>,
                wellness_read: Option<bool>,
                t2_read: Option<(bool, Option<Timestamp>)>,
                now: Timestamp,
            ) -> AppResult<Option<GroupMember>> {
                let result = sqlx::query(SET_COACH_CONSENTS_SQL)
                    .bind(calendar_write)
                    .bind(wellness_read)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .bind(t2_read.map(|(read, _)| read))
                    .bind(t2_read.and_then(|(_, until)| until))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set a member's coach consents: {e}"))
                    })?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                self.get_member(group_id, user_id).await
            }

            async fn set_coach_missed_session_check(
                &self,
                group_id: &str,
                user_id: Uuid,
                on: bool,
                now: Timestamp,
            ) -> AppResult<Option<GroupMember>> {
                let result = sqlx::query(SET_COACH_MISSED_SESSION_CHECK_SQL)
                    .bind(on)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to set a member's missed-session check: {e}"
                        ))
                    })?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                self.get_member(group_id, user_id).await
            }

            async fn set_coach_routine_detection(
                &self,
                group_id: &str,
                user_id: Uuid,
                on: bool,
                now: Timestamp,
            ) -> AppResult<Option<GroupMember>> {
                let result = sqlx::query(SET_COACH_ROUTINE_DETECTION_SQL)
                    .bind(on)
                    .bind(now)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to set a member's routine detection: {e}"
                        ))
                    })?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                self.get_member(group_id, user_id).await
            }

            async fn count_members(&self, group_id: &str) -> AppResult<i64> {
                let row = sqlx::query(COUNT_MEMBERS_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count members: {e}")))?;

                column(&row, "cnt")
            }

            async fn setup_timeline(
                &self,
                group_id: &str,
                tenant_id: &str,
            ) -> AppResult<Option<GroupSetupTimeline>> {
                let row = sqlx::query(GROUP_SETUP_TIMELINE_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read group setup timeline: {e}"))
                    })?;
                row.map(|r| {
                    Ok(GroupSetupTimeline {
                        group_created_at: instant(&r, "group_created_at")?,
                        owner_created_at: instant(&r, "owner_created_at")?,
                        first_member_id: $ids::read_opt(&r, "first_member_id")?,
                    })
                })
                .transpose()
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
                    .bind(invite.approves_signup)
                    .bind(Timestamp::now())
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

            async fn find_invite_by_code(&self, code: &str) -> AppResult<Option<GroupInvite>> {
                let row = sqlx::query(FIND_INVITE_BY_CODE_SQL)
                    .bind(code)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find invite by code: {e}"))
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

            async fn claim_signup_invite(
                &self,
                code: &str,
                now: Timestamp,
            ) -> AppResult<Option<GroupInvite>> {
                let row = sqlx::query(CLAIM_SIGNUP_INVITE_SQL)
                    .bind(code)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to claim invite: {e}")))?;
                row.as_ref().map(row_to_invite).transpose()
            }

            async fn get_invite_by_id(&self, invite_id: &str) -> AppResult<Option<GroupInvite>> {
                let row = sqlx::query(GET_INVITE_BY_ID_SQL)
                    .bind($ids::bind_text(invite_id)?)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get invite: {e}")))?;
                row.as_ref().map(row_to_invite).transpose()
            }

            async fn list_signup_invites(&self) -> AppResult<Vec<GroupInvite>> {
                let rows = sqlx::query(LIST_SIGNUP_INVITES_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list invites: {e}")))?;
                rows.iter().map(row_to_invite).collect()
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
                let id = Uuid::new_v4();
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin a transcript entry: {e}"))
                })?;
                sqlx::query(INSERT_TRANSCRIPT_ENTRY_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind_text(entry.group_id)?)
                    .bind(entry.tenant_id)
                    .bind($ids::bind(entry.author_user_id))
                    .bind(entry.speaker.as_str())
                    .bind(entry.content)
                    .bind(entry.source_conversation_id)
                    .bind(entry.source_message_id)
                    .bind(Timestamp::now())
                    .bind(entry.transport_policy.is_first_party_only())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to append transcript entry: {e}"))
                    })?;
                // One link row per member other than the author, the
                // strictest of their reads (carnet#1511 call 19).
                for read in CarriedRead::strictest_per_member(entry.carried)
                    .into_iter()
                    .filter(|read| read.member != entry.author_user_id)
                {
                    sqlx::query(INSERT_TRANSCRIPT_ENTRY_MEMBER_SQL)
                        .bind($ids::bind(id))
                        .bind($ids::bind(read.member))
                        .bind(read.way.as_str())
                        .bind($ids::bind_opt(read.group))
                        .bind($ids::bind_opt(read.coach))
                        .bind(&read.class)
                        .bind(read.tier.as_deref())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to record a member a transcript entry carries: {e}"
                            ))
                        })?;
                }
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit a transcript entry: {e}"))
                })?;

                Ok(())
            }

            async fn list_transcript_visible_to(
                &self,
                group_id: &str,
                viewer_user_id: Uuid,
                limit: i64,
                readable: TranscriptReadable<'_>,
            ) -> AppResult<Vec<GroupTranscriptEntry>> {
                let sql =
                    list_transcript_visible_to_sql(readable.viewer.len(), readable.room.len());
                let mut query = sqlx::query(&sql)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(viewer_user_id))
                    .bind(limit.clamp(1, 500));
                let keys = readable.viewer.iter().chain(readable.room);
                for key in keys {
                    query = query.bind(key);
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to list transcript entries: {e}"))
                })?;

                rows.iter().map(row_to_transcript_entry).collect()
            }

            async fn list_room_transcript_for(
                &self,
                group_id: &str,
                viewer_user_id: Uuid,
                before: Option<Uuid>,
                limit: i64,
                readable: &[String],
            ) -> AppResult<Vec<RoomTranscriptEntry>> {
                let sql = list_room_transcript_sql(readable.len());
                let mut query = sqlx::query(&sql)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(viewer_user_id))
                    .bind(limit.clamp(1, 500))
                    .bind($ids::bind_opt(before));
                for key in readable {
                    query = query.bind(key);
                }
                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list the room: {e}")))?;

                rows.iter().map(row_to_room_entry).collect()
            }

            async fn get_room_transcript_entry_for(
                &self,
                group_id: &str,
                viewer_user_id: Uuid,
                entry_id: Uuid,
                tenants: [TenantId; 2],
                readable: &[String],
            ) -> AppResult<Option<RoomTranscriptEntry>> {
                let [first_tenant, second_tenant] = tenants;
                let sql = get_room_transcript_entry_sql(readable.len());
                let mut query = sqlx::query(&sql)
                    .bind($ids::bind_text(group_id)?)
                    .bind($ids::bind(viewer_user_id))
                    .bind($ids::bind(entry_id))
                    .bind(first_tenant.to_string())
                    .bind(second_tenant.to_string());
                for key in readable {
                    query = query.bind(key);
                }
                let row = query
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read a room entry: {e}")))?;

                row.as_ref().map(row_to_room_entry).transpose()
            }

            async fn list_group_read_keys(&self, group_id: &str) -> AppResult<Vec<String>> {
                let rows = sqlx::query(GROUP_READ_KEYS_SQL)
                    .bind($ids::bind_text(group_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list a room's reads: {e}"))
                    })?;
                rows.iter().map(|row| column(row, "rkey")).collect()
            }

            async fn list_transcript_entry_reads(
                &self,
                entry_ids: &[Uuid],
            ) -> AppResult<Vec<(Uuid, String)>> {
                if entry_ids.is_empty() {
                    return Ok(Vec::new());
                }
                let sql = transcript_entry_reads_sql(entry_ids.len());
                let mut query = sqlx::query(&sql);
                for id in entry_ids {
                    query = query.bind($ids::bind(*id));
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to list a room entry's reads: {e}"))
                })?;
                rows.iter()
                    .map(|row| Ok(($ids::read(row, "entry_id")?, column(row, "rkey")?)))
                    .collect()
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
