// ABOUTME: SQLite implementation of CoachingGroupRepository for group coaching
// ABOUTME: Handles group CRUD, membership management, and invite tracking with tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, GroupInvite, GroupMember, GroupRole, GroupSummary, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::CoachingGroupRepository;

use chrono::DateTime;

/// Parse a `GroupRole` from a database string value
fn parse_role(s: &str) -> GroupRole {
    GroupRole::from_str_opt(s).unwrap_or(GroupRole::Member)
}

/// Parse a `DateTime<Utc>` from an ISO 8601 string stored in `SQLite`
fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc))
}

/// Parse a `Uuid` from a TEXT column
fn parse_uuid_col(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap_or_default()
}

/// Map a `SQLite` row to `CoachingGroup`
fn row_to_group(r: &SqliteRow) -> CoachingGroup {
    let id_str: String = r.get("id");
    let owner_str: String = r.get("owner_id");
    let created: String = r.get("created_at");
    let updated: String = r.get("updated_at");
    let peer_sharing: i32 = r.get("peer_data_sharing");
    let active: i32 = r.get("is_active");

    CoachingGroup {
        id: parse_uuid_col(&id_str),
        tenant_id: r.get("tenant_id"),
        name: r.get("name"),
        description: r.get("description"),
        coach_id: r.get("coach_id"),
        owner_id: parse_uuid_col(&owner_str),
        peer_data_sharing: peer_sharing != 0,
        max_members: r.get("max_members"),
        is_active: active != 0,
        created_at: parse_dt(&created),
        updated_at: parse_dt(&updated),
    }
}

/// Map a `SQLite` row to `GroupMember`
fn row_to_member(r: &SqliteRow) -> GroupMember {
    let id_str: String = r.get("id");
    let group_str: String = r.get("group_id");
    let user_str: String = r.get("user_id");
    let role_str: String = r.get("role");
    let consent: i32 = r.get("peer_sharing_consent");
    let consent_at: String = r.get("consent_given_at");
    let joined: String = r.get("joined_at");
    let left: Option<String> = r.get("left_at");

    GroupMember {
        id: parse_uuid_col(&id_str),
        group_id: parse_uuid_col(&group_str),
        user_id: parse_uuid_col(&user_str),
        tenant_id: r.get("tenant_id"),
        role: parse_role(&role_str),
        peer_sharing_consent: consent != 0,
        consent_given_at: parse_dt(&consent_at),
        joined_at: parse_dt(&joined),
        left_at: left.map(|s| parse_dt(&s)),
        display_name: r.try_get("display_name").ok(),
    }
}

/// Map a `SQLite` row to `GroupInvite`
fn row_to_invite(r: &SqliteRow) -> GroupInvite {
    let id_str: String = r.get("id");
    let group_str: String = r.get("group_id");
    let creator_str: String = r.get("created_by");
    let created: String = r.get("created_at");
    let expires: Option<String> = r.get("expires_at");
    let active: i32 = r.get("is_active");

    GroupInvite {
        id: parse_uuid_col(&id_str),
        group_id: parse_uuid_col(&group_str),
        tenant_id: r.get("tenant_id"),
        code: r.get("code"),
        created_by: parse_uuid_col(&creator_str),
        expires_at: expires.map(|s| parse_dt(&s)),
        max_uses: r.get("max_uses"),
        use_count: r.get("use_count"),
        is_active: active != 0,
        created_at: parse_dt(&created),
    }
}

#[async_trait]
impl CoachingGroupRepository for Database {
    // -- Group CRUD --

    async fn create_group(
        &self,
        tenant_id: TenantId,
        group: &CoachingGroup,
    ) -> AppResult<CoachingGroup> {
        let id = group.id.to_string();
        let now = Utc::now().to_rfc3339();
        let peer_sharing = i32::from(group.peer_data_sharing);

        sqlx::query(
            r"INSERT INTO coaching_groups (id, tenant_id, name, description, coach_id, owner_id,
              peer_data_sharing, max_members, is_active, created_at, updated_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, $9)",
        )
        .bind(&id)
        .bind(tenant_id.to_string())
        .bind(&group.name)
        .bind(&group.description)
        .bind(&group.coach_id)
        .bind(group.owner_id.to_string())
        .bind(peer_sharing)
        .bind(group.max_members)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create group: {e}")))?;

        self.get_group(&id, tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Group not found after creation"))
    }

    async fn get_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachingGroup>> {
        let row = sqlx::query(
            r"SELECT id, tenant_id, name, description, coach_id, owner_id,
              peer_data_sharing, max_members, is_active, created_at, updated_at
              FROM coaching_groups WHERE id = $1 AND tenant_id = $2",
        )
        .bind(group_id)
        .bind(tenant_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get group: {e}")))?;

        Ok(row.as_ref().map(row_to_group))
    }

    async fn list_groups_for_user(&self, user_id: Uuid) -> AppResult<Vec<GroupSummary>> {
        let rows = sqlx::query(
            r"SELECT g.id, g.name, g.description, g.coach_id, g.is_active, g.peer_data_sharing,
              g.created_at, m.role,
              (SELECT COUNT(*) FROM coaching_group_members m2
               WHERE m2.group_id = g.id AND m2.left_at IS NULL) AS member_count
              FROM coaching_groups g
              JOIN coaching_group_members m ON m.group_id = g.id
              WHERE m.user_id = $1 AND m.left_at IS NULL AND g.is_active = 1
              ORDER BY g.updated_at DESC",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list groups: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                let id_str: String = r.get("id");
                let role_str: String = r.get("role");
                let created: String = r.get("created_at");
                let active: i32 = r.get("is_active");
                let peer: i32 = r.get("peer_data_sharing");

                GroupSummary {
                    id: parse_uuid_col(&id_str),
                    name: r.get("name"),
                    description: r.get("description"),
                    coach_id: r.get("coach_id"),
                    member_count: r.get("member_count"),
                    is_active: active != 0,
                    peer_data_sharing: peer != 0,
                    my_role: parse_role(&role_str),
                    created_at: parse_dt(&created),
                }
            })
            .collect())
    }

    async fn list_groups_for_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>> {
        let rows = sqlx::query(
            r"SELECT id, tenant_id, name, description, coach_id, owner_id,
              peer_data_sharing, max_members, is_active, created_at, updated_at
              FROM coaching_groups
              WHERE coach_id = $1 AND tenant_id = $2 AND is_active = 1
              ORDER BY created_at DESC",
        )
        .bind(coach_id)
        .bind(tenant_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list groups for coach: {e}")))?;

        Ok(rows.iter().map(row_to_group).collect())
    }

    async fn update_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
        request: &UpdateGroupRequest,
    ) -> AppResult<Option<CoachingGroup>> {
        let now = Utc::now().to_rfc3339();

        // Build dynamic UPDATE with only provided fields
        let mut sets = vec!["updated_at = $1".to_owned()];
        let mut param_idx = 3u32; // $1=now, $2=group_id, $3+=dynamic

        if request.name.is_some() {
            sets.push(format!("name = ${param_idx}"));
            param_idx += 1;
        }
        if request.description.is_some() {
            sets.push(format!("description = ${param_idx}"));
            param_idx += 1;
        }
        if request.max_members.is_some() {
            sets.push(format!("max_members = ${param_idx}"));
            param_idx += 1;
        }
        if request.peer_data_sharing.is_some() {
            sets.push(format!("peer_data_sharing = ${param_idx}"));
            param_idx += 1;
        }
        if request.is_active.is_some() {
            sets.push(format!("is_active = ${param_idx}"));
        }

        let sql = format!(
            "UPDATE coaching_groups SET {} WHERE id = $2 AND tenant_id = ${0}",
            sets.join(", "),
        );

        // For simplicity with dynamic params, use a simpler approach
        let result = sqlx::query(
            r"UPDATE coaching_groups SET
              name = COALESCE($1, name),
              description = COALESCE($2, description),
              max_members = COALESCE($3, max_members),
              peer_data_sharing = COALESCE($4, peer_data_sharing),
              is_active = COALESCE($5, is_active),
              updated_at = $6
              WHERE id = $7 AND tenant_id = $8",
        )
        .bind(&request.name)
        .bind(&request.description)
        .bind(request.max_members)
        .bind(request.peer_data_sharing.map(i32::from))
        .bind(request.is_active.map(i32::from))
        .bind(&now)
        .bind(group_id)
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update group: {e}")))?;

        // Drop the unused dynamic SQL variable
        drop(sql);

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_group(group_id, tenant_id).await
    }

    async fn delete_group(&self, group_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"UPDATE coaching_groups SET is_active = 0, updated_at = $1
              WHERE id = $2 AND tenant_id = $3",
        )
        .bind(&now)
        .bind(group_id)
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to delete group: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    // -- Membership --

    async fn add_member(&self, member: &GroupMember) -> AppResult<GroupMember> {
        let id = member.id.to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"INSERT INTO coaching_group_members
              (id, group_id, user_id, tenant_id, role, peer_sharing_consent, consent_given_at, joined_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
        )
        .bind(&id)
        .bind(member.group_id.to_string())
        .bind(member.user_id.to_string())
        .bind(&member.tenant_id)
        .bind(member.role.as_str())
        .bind(i32::from(member.peer_sharing_consent))
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to add member: {e}")))?;

        let tenant_id: TenantId = member
            .tenant_id
            .parse()
            .map_err(|e| AppError::internal(format!("Invalid tenant_id: {e}")))?;
        self.get_member(&member.group_id.to_string(), member.user_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Member not found after creation"))
    }

    async fn remove_member(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"UPDATE coaching_group_members SET left_at = $1
              WHERE group_id = $2 AND user_id = $3 AND tenant_id = $4 AND left_at IS NULL",
        )
        .bind(&now)
        .bind(group_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to remove member: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_member(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<GroupMember>> {
        let row = sqlx::query(
            r"SELECT m.id, m.group_id, m.user_id, m.tenant_id, m.role,
              m.peer_sharing_consent, m.consent_given_at, m.joined_at, m.left_at,
              u.email AS display_name
              FROM coaching_group_members m
              LEFT JOIN users u ON u.id = m.user_id
              WHERE m.group_id = $1 AND m.user_id = $2 AND m.tenant_id = $3 AND m.left_at IS NULL",
        )
        .bind(group_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get member: {e}")))?;

        Ok(row.as_ref().map(row_to_member))
    }

    async fn list_members(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<GroupMember>> {
        let rows = sqlx::query(
            r"SELECT m.id, m.group_id, m.user_id, m.tenant_id, m.role,
              m.peer_sharing_consent, m.consent_given_at, m.joined_at, m.left_at,
              u.email AS display_name
              FROM coaching_group_members m
              LEFT JOIN users u ON u.id = m.user_id
              WHERE m.group_id = $1 AND m.tenant_id = $2 AND m.left_at IS NULL
              ORDER BY m.joined_at ASC",
        )
        .bind(group_id)
        .bind(tenant_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list members: {e}")))?;

        Ok(rows.iter().map(row_to_member).collect())
    }

    async fn update_member_role(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        role: GroupRole,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"UPDATE coaching_group_members SET role = $1
              WHERE group_id = $2 AND user_id = $3 AND tenant_id = $4 AND left_at IS NULL",
        )
        .bind(role.as_str())
        .bind(group_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update member role: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_peer_sharing_consent(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        consent: bool,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"UPDATE coaching_group_members SET peer_sharing_consent = $1, consent_given_at = $2
              WHERE group_id = $3 AND user_id = $4 AND tenant_id = $5 AND left_at IS NULL",
        )
        .bind(i32::from(consent))
        .bind(&now)
        .bind(group_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update peer sharing consent: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn count_members(&self, group_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        let row = sqlx::query(
            r"SELECT COUNT(*) as cnt FROM coaching_group_members
              WHERE group_id = $1 AND tenant_id = $2 AND left_at IS NULL",
        )
        .bind(group_id)
        .bind(tenant_id.to_string())
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count members: {e}")))?;

        Ok(row.get("cnt"))
    }

    // -- Invites --

    async fn create_invite(&self, invite: &GroupInvite) -> AppResult<GroupInvite> {
        let id = invite.id.to_string();
        let now = Utc::now().to_rfc3339();
        let expires = invite.expires_at.map(|dt| dt.to_rfc3339());

        sqlx::query(
            r"INSERT INTO group_invites
              (id, group_id, tenant_id, code, created_by, expires_at, max_uses, use_count, is_active, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 1, $8)",
        )
        .bind(&id)
        .bind(invite.group_id.to_string())
        .bind(&invite.tenant_id)
        .bind(&invite.code)
        .bind(invite.created_by.to_string())
        .bind(&expires)
        .bind(invite.max_uses)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create invite: {e}")))?;

        let row = sqlx::query(
            r"SELECT id, group_id, tenant_id, code, created_by, expires_at, max_uses,
              use_count, is_active, created_at FROM group_invites WHERE id = $1",
        )
        .bind(&id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch invite after creation: {e}")))?;

        Ok(row_to_invite(&row))
    }

    async fn get_invite_by_code(&self, code: &str) -> AppResult<Option<GroupInvite>> {
        let row = sqlx::query(
            r"SELECT id, group_id, tenant_id, code, created_by, expires_at, max_uses,
              use_count, is_active, created_at FROM group_invites
              WHERE code = $1 AND is_active = 1",
        )
        .bind(code)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get invite by code: {e}")))?;

        Ok(row.as_ref().map(row_to_invite))
    }

    async fn increment_invite_use_count(&self, invite_id: &str) -> AppResult<bool> {
        let result =
            sqlx::query(r"UPDATE group_invites SET use_count = use_count + 1 WHERE id = $1")
                .bind(invite_id)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to increment invite use count: {e}"))
                })?;

        Ok(result.rows_affected() > 0)
    }

    async fn deactivate_invite(&self, invite_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"UPDATE group_invites SET is_active = 0
              WHERE id = $1 AND tenant_id = $2",
        )
        .bind(invite_id)
        .bind(tenant_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate invite: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn list_invites(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<GroupInvite>> {
        let rows = sqlx::query(
            r"SELECT id, group_id, tenant_id, code, created_by, expires_at, max_uses,
              use_count, is_active, created_at FROM group_invites
              WHERE group_id = $1 AND tenant_id = $2
              ORDER BY created_at DESC",
        )
        .bind(group_id)
        .bind(tenant_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list invites: {e}")))?;

        Ok(rows.iter().map(row_to_invite).collect())
    }

    // -- Context queries --

    async fn find_groups_for_user_and_coach(
        &self,
        user_id: Uuid,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>> {
        let rows = sqlx::query(
            r"SELECT g.id, g.tenant_id, g.name, g.description, g.coach_id, g.owner_id,
              g.peer_data_sharing, g.max_members, g.is_active, g.created_at, g.updated_at
              FROM coaching_groups g
              JOIN coaching_group_members m ON m.group_id = g.id
              WHERE m.user_id = $1 AND g.coach_id = $2 AND g.tenant_id = $3
              AND m.left_at IS NULL AND g.is_active = 1",
        )
        .bind(user_id.to_string())
        .bind(coach_id)
        .bind(tenant_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to find groups for user+coach: {e}")))?;

        Ok(rows.iter().map(row_to_group).collect())
    }

    async fn count_groups_for_owner(&self, owner_id: Uuid, tenant_id: TenantId) -> AppResult<i64> {
        let row = sqlx::query(
            r"SELECT COUNT(*) as cnt FROM coaching_groups
              WHERE owner_id = $1 AND tenant_id = $2 AND is_active = 1",
        )
        .bind(owner_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count groups: {e}")))?;

        Ok(row.get("cnt"))
    }
}
