// ABOUTME: Repository trait, shared statements and body for delegated connections: a member's provider read through their coach
// ABOUTME: One SQL text per operation; each backend shell supplies its type, row type and uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delegated connections, written once.
//!
//! A row ties an athlete on a coach's provider roster to a live member of a
//! coaching group that user coaches. The coach proposes, the member confirms,
//! and either side or the group lifecycle ends it; an ended row is stamped
//! `revoked` with its reason and stays for audit. The partial unique indexes
//! cover live rows only, so a new proposal after an end is a new row.
//!
//! Every uuid column (ids, users, tenants, the group) is `uuid` on Postgres
//! and hyphenated `TEXT` on `SQLite`, so every id bind and read goes through a
//! [`super::uuid_columns`] codec, the macro's one per-backend argument besides
//! the driver's row type. Timestamps bind and decode as [`DateTime<Utc>`] on
//! both: `TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`.
//!
//! Groups span tenants, so no statement filters by a group's tenant. Each is
//! scoped by a user key instead (the coach, the member, or either participant),
//! and the member's read path adds the member's tenant. The lifecycle ends
//! that take only a group id are called after a tenant-scoped write on that
//! group succeeded.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DelegatedConnection, DelegationEndReason, DelegationStatus, TenantId};
use uuid::Uuid;

/// Delegated-connection persistence.
#[async_trait]
pub trait DelegatedConnectionRepository: Send + Sync {
    /// Store a proposal. `link` must be one [`DelegatedConnection::propose`]
    /// built: status `proposed` and no confirmation or end recorded. Returns
    /// the stored row, or `None` when a live link already holds the member
    /// in that group for that provider, or the coach's athlete.
    ///
    /// # Errors
    /// Returns an invalid-input error for a link that is not a fresh proposal,
    /// and a database error when the insert fails for any other reason.
    async fn propose(&self, link: &DelegatedConnection) -> AppResult<Option<DelegatedConnection>>;

    /// One link of a group, whatever its status, when `user_id` is its coach
    /// or its member.
    async fn get_for_participant(
        &self,
        id: Uuid,
        group_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<DelegatedConnection>>;

    /// The live links a coach holds in one group, newest proposal first.
    async fn list_live_for_coach_in_group(
        &self,
        group_id: Uuid,
        coach_user_id: Uuid,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// Every live link one coach's `provider` session serves in one tenant,
    /// across the coach's groups, newest proposal first.
    async fn list_live_for_coach(
        &self,
        coach_user_id: Uuid,
        coach_tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// The live links naming one member in one group, newest proposal first.
    async fn list_live_for_member_in_group(
        &self,
        group_id: Uuid,
        member_user_id: Uuid,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// Every live link naming one member, across groups: the confirmed one
    /// first, then proposals, newest first.
    async fn list_live_for_member(
        &self,
        member_user_id: Uuid,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// The member confirms a proposal, in the tenant their delegated provider
    /// connection will live in. Returns the confirmed row, or `None` when no
    /// proposal with that id names the member.
    ///
    /// # Errors
    /// Returns an already-exists error with `details.reason` `already_linked`
    /// when the member already has a confirmed link for the provider, and a
    /// database error for any other failure.
    async fn confirm(
        &self,
        id: Uuid,
        member_user_id: Uuid,
        member_tenant_id: TenantId,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DelegatedConnection>>;

    /// End one live link. `participant_user_id` scopes the statement: it must
    /// be the link's coach or member. `actor` is recorded as `revoked_by`;
    /// `None` when the system ends it. Returns the ended row, or `None` when
    /// no live link with that id names the participant.
    async fn end_one(
        &self,
        id: Uuid,
        participant_user_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DelegatedConnection>>;

    /// End every live link naming one member in one group. Returns exactly
    /// the rows it ended.
    async fn end_for_group_member(
        &self,
        group_id: Uuid,
        member_user_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// End every live link of one group. Returns exactly the rows it ended.
    async fn end_for_group(
        &self,
        group_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// End every live link served by one coach's session in one tenant for
    /// one provider. Returns exactly the rows it ended.
    async fn end_for_coach(
        &self,
        coach_user_id: Uuid,
        coach_tenant_id: TenantId,
        provider: &str,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// End the member's confirmed link for one provider in one tenant.
    /// Returns exactly the rows it ended: at most one, since a member holds
    /// one confirmed link per provider.
    async fn end_confirmed_for_member(
        &self,
        member_user_id: Uuid,
        member_tenant_id: TenantId,
        provider: &str,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// The member's live links for one provider that the group relation still
    /// backs, the confirmed one first, then proposals, newest first.
    ///
    /// The relation is the one [`Self::find_active_for_member`] reads through.
    /// What a status
    /// surface shows the member, so it cannot name a link the read path would
    /// refuse.
    async fn list_backed_for_member(
        &self,
        member_user_id: Uuid,
        provider: &str,
    ) -> AppResult<Vec<DelegatedConnection>>;

    /// The confirmed link a member's provider reads go through, in the tenant
    /// the read runs in. Fails closed on the group relation as well as the
    /// row: `None` once the member has left the group, the group is archived,
    /// or the group's coach is no longer the link's coach, even when no end
    /// was recorded.
    async fn find_active_for_member(
        &self,
        member_user_id: Uuid,
        member_tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DelegatedConnection>>;
}

/// The columns every read decodes, each prefixed with `$p` (`""` for the bare
/// table, `"d."` inside a join), in the order the decoder reads them.
macro_rules! delegated_connection_columns {
    ($p:literal) => {
        concat!(
            $p,
            "id, ",
            $p,
            "provider, ",
            $p,
            "group_id, ",
            $p,
            "coach_user_id, ",
            $p,
            "coach_tenant_id, ",
            $p,
            "member_user_id, ",
            $p,
            "member_tenant_id, ",
            $p,
            "provider_athlete_id, ",
            $p,
            "provider_athlete_name, ",
            $p,
            "status, ",
            $p,
            "proposed_at, ",
            $p,
            "confirmed_at, ",
            $p,
            "revoked_at, ",
            $p,
            "revoked_by, ",
            $p,
            "revoke_reason"
        )
    };
}

/// The stamp every end writes, `$1`..`$3`; the scoping keys follow from `$4`.
macro_rules! end_links_sql {
    ($filter:literal) => {
        concat!(
            "UPDATE delegated_connections
               SET status = 'revoked', revoked_at = $1, revoked_by = $2, revoke_reason = $3
             WHERE ",
            $filter,
            "
             RETURNING ",
            delegated_connection_columns!("")
        )
    };
}

/// Insert a proposal. Any live-link index conflict inserts nothing and
/// returns no row.
pub(crate) const PROPOSE_SQL: &str = concat!(
    "INSERT INTO delegated_connections
        (id, provider, group_id, coach_user_id, coach_tenant_id, member_user_id,
         member_tenant_id, provider_athlete_id, provider_athlete_name, status, proposed_at,
         confirmed_at, revoked_at, revoked_by, revoke_reason)
     VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, 'proposed', $9, NULL, NULL, NULL, NULL)
     ON CONFLICT DO NOTHING
     RETURNING ",
    delegated_connection_columns!("")
);

/// One link of a group, when `$3` is its coach or its member.
pub(crate) const GET_FOR_PARTICIPANT_SQL: &str = concat!(
    "SELECT ",
    delegated_connection_columns!(""),
    " FROM delegated_connections
     WHERE id = $1 AND group_id = $2 AND (coach_user_id = $3 OR member_user_id = $3)"
);

/// A coach's live links in one group.
pub(crate) const LIST_LIVE_FOR_COACH_IN_GROUP_SQL: &str = concat!(
    "SELECT ",
    delegated_connection_columns!(""),
    " FROM delegated_connections
     WHERE group_id = $1 AND coach_user_id = $2 AND status IN ('proposed', 'confirmed')
     ORDER BY proposed_at DESC, id"
);

/// Every live link one coach's session serves in one tenant.
pub(crate) const LIST_LIVE_FOR_COACH_SQL: &str = concat!(
    "SELECT ",
    delegated_connection_columns!(""),
    " FROM delegated_connections
     WHERE coach_user_id = $1 AND coach_tenant_id = $2 AND provider = $3
       AND status IN ('proposed', 'confirmed')
     ORDER BY proposed_at DESC, id"
);

/// A member's live links in one group.
pub(crate) const LIST_LIVE_FOR_MEMBER_IN_GROUP_SQL: &str = concat!(
    "SELECT ",
    delegated_connection_columns!(""),
    " FROM delegated_connections
     WHERE group_id = $1 AND member_user_id = $2 AND status IN ('proposed', 'confirmed')
     ORDER BY proposed_at DESC, id"
);

/// A member's live links across groups, the confirmed one first.
pub(crate) const LIST_LIVE_FOR_MEMBER_SQL: &str = concat!(
    "SELECT ",
    delegated_connection_columns!(""),
    " FROM delegated_connections
     WHERE member_user_id = $1 AND status IN ('proposed', 'confirmed')
     ORDER BY CASE WHEN status = 'confirmed' THEN 0 ELSE 1 END, proposed_at DESC, id"
);

/// The member confirms their own proposal.
pub(crate) const CONFIRM_SQL: &str = concat!(
    "UPDATE delegated_connections
        SET status = 'confirmed', confirmed_at = $1, member_tenant_id = $2
      WHERE id = $3 AND member_user_id = $4 AND status = 'proposed'
      RETURNING ",
    delegated_connection_columns!("")
);

/// End one live link, scoped to one of its participants.
pub(crate) const END_ONE_SQL: &str = end_links_sql!(
    "id = $4 AND (coach_user_id = $5 OR member_user_id = $5)
               AND status IN ('proposed', 'confirmed')"
);

/// End every live link naming one member in one group.
pub(crate) const END_FOR_GROUP_MEMBER_SQL: &str =
    end_links_sql!("group_id = $4 AND member_user_id = $5 AND status IN ('proposed', 'confirmed')");

/// End every live link of one group.
pub(crate) const END_FOR_GROUP_SQL: &str =
    end_links_sql!("group_id = $4 AND status IN ('proposed', 'confirmed')");

/// End every live link one coach's session serves in one tenant.
pub(crate) const END_FOR_COACH_SQL: &str = end_links_sql!(
    "coach_user_id = $4 AND coach_tenant_id = $5 AND provider = $6
               AND status IN ('proposed', 'confirmed')"
);

/// End the member's confirmed link for one provider in one tenant.
pub(crate) const END_CONFIRMED_FOR_MEMBER_SQL: &str = end_links_sql!(
    "member_user_id = $4 AND member_tenant_id = $5 AND provider = $6
               AND status = 'confirmed'"
);

/// Links the group relation still backs.
///
/// The group is active, the member is still in it, and the group's coach is
/// still the link's. `$filter` narrows
/// the links (aliased `d`) and `$order` closes the statement. Every read that
/// says whether a link stands goes through this one relation.
macro_rules! backed_links_sql {
    ($filter:literal, $order:literal) => {
        concat!(
            "SELECT ",
            delegated_connection_columns!("d."),
            " FROM delegated_connections d
     JOIN coaching_groups g ON g.id = d.group_id
     JOIN coaching_group_members m ON m.group_id = d.group_id AND m.user_id = d.member_user_id
     WHERE ",
            $filter,
            "
       AND g.is_active = TRUE AND g.coach_user_id = d.coach_user_id
       AND m.left_at IS NULL",
            $order
        )
    };
}

/// A member's backed live links for one provider, the confirmed one first.
pub(crate) const LIST_BACKED_FOR_MEMBER_SQL: &str = backed_links_sql!(
    "d.member_user_id = $1 AND d.provider = $2 AND d.status IN ('proposed', 'confirmed')",
    "
     ORDER BY CASE WHEN d.status = 'confirmed' THEN 0 ELSE 1 END, d.proposed_at DESC, d.id"
);

/// The confirmed link a member reads through, only while the relation backs
/// it.
pub(crate) const FIND_ACTIVE_FOR_MEMBER_SQL: &str = backed_links_sql!(
    "d.member_user_id = $1 AND d.member_tenant_id = $2 AND d.provider = $3
       AND d.status = 'confirmed'",
    ""
);

/// Read one column by name via `try_get`, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error naming the column rather than a panic.
pub(crate) fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("delegated_connections column `{name}`: {e}")))
}

/// A stored text outside a column's vocabulary.
pub(crate) fn unknown_value(col: &str, value: &str) -> AppError {
    AppError::database(format!(
        "delegated_connections column `{col}` holds unknown value `{value}`"
    ))
}

/// The error a confirm reports when the member already has a confirmed link
/// for the provider: `details.reason` is what a client branches on.
pub(crate) fn already_linked() -> AppError {
    let mut err = AppError::already_exists("A confirmed link for this member and provider");
    err.details = Some(Box::new(serde_json::json!({ "reason": "already_linked" })));
    err
}

/// Refuse a link [`DelegatedConnectionRepository::propose`] must not store.
///
/// # Errors
/// Returns an invalid-input error when the link is not a fresh proposal.
pub(crate) fn require_fresh_proposal(link: &DelegatedConnection) -> AppResult<()> {
    let fresh = link.status == DelegationStatus::Proposed
        && link.member_tenant_id.is_none()
        && link.confirmed_at.is_none()
        && link.revoked_at.is_none()
        && link.revoked_by.is_none()
        && link.revoke_reason.is_none();
    if fresh {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "Only a fresh proposal can be stored as a delegated connection",
        ))
    }
}

/// Emit the whole [`DelegatedConnectionRepository`] implementation for one
/// backend type, together with its row decoder.
///
/// `$row` is the driver's row type and `$ids` the backend's uuid codec from
/// [`super::uuid_columns`] (`SqliteRow` + `TextUuid`, `PgRow` + `NativeUuid`).
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_delegated_connection_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode one `delegated_connections` row. An unknown status or end
        /// reason is a database error: the CHECK constraints admit neither.
        fn row_to_delegated_connection(r: &$row) -> AppResult<DelegatedConnection> {
            let status: String = column(r, "status")?;
            let reason: Option<String> = column(r, "revoke_reason")?;
            Ok(DelegatedConnection {
                id: $ids::read(r, "id")?,
                provider: column(r, "provider")?,
                group_id: $ids::read(r, "group_id")?,
                coach_user_id: $ids::read(r, "coach_user_id")?,
                coach_tenant_id: TenantId::from_uuid($ids::read(r, "coach_tenant_id")?),
                member_user_id: $ids::read(r, "member_user_id")?,
                member_tenant_id: $ids::read_opt(r, "member_tenant_id")?.map(TenantId::from_uuid),
                provider_athlete_id: column(r, "provider_athlete_id")?,
                provider_athlete_name: column(r, "provider_athlete_name")?,
                status: DelegationStatus::from_str_opt(&status)
                    .ok_or_else(|| unknown_value("status", &status))?,
                proposed_at: column(r, "proposed_at")?,
                confirmed_at: column(r, "confirmed_at")?,
                revoked_at: column(r, "revoked_at")?,
                revoked_by: $ids::read_opt(r, "revoked_by")?,
                revoke_reason: reason
                    .map(|text| {
                        DelegationEndReason::from_str_opt(&text)
                            .ok_or_else(|| unknown_value("revoke_reason", &text))
                    })
                    .transpose()?,
            })
        }

        #[async_trait::async_trait]
        impl DelegatedConnectionRepository for $ty {
            async fn propose(
                &self,
                link: &DelegatedConnection,
            ) -> AppResult<Option<DelegatedConnection>> {
                require_fresh_proposal(link)?;
                let row = sqlx::query(PROPOSE_SQL)
                    .bind($ids::bind(link.id))
                    .bind(&link.provider)
                    .bind($ids::bind(link.group_id))
                    .bind($ids::bind(link.coach_user_id))
                    .bind($ids::bind(link.coach_tenant_id.as_uuid()))
                    .bind($ids::bind(link.member_user_id))
                    .bind(&link.provider_athlete_id)
                    .bind(&link.provider_athlete_name)
                    .bind(link.proposed_at)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to propose delegated connection: {e}"))
                    })?;

                row.as_ref().map(row_to_delegated_connection).transpose()
            }

            async fn get_for_participant(
                &self,
                id: Uuid,
                group_id: Uuid,
                user_id: Uuid,
            ) -> AppResult<Option<DelegatedConnection>> {
                let row = sqlx::query(GET_FOR_PARTICIPANT_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind(group_id))
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get delegated connection: {e}"))
                    })?;

                row.as_ref().map(row_to_delegated_connection).transpose()
            }

            async fn list_live_for_coach_in_group(
                &self,
                group_id: Uuid,
                coach_user_id: Uuid,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(LIST_LIVE_FOR_COACH_IN_GROUP_SQL)
                    .bind($ids::bind(group_id))
                    .bind($ids::bind(coach_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list a coach's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn list_live_for_coach(
                &self,
                coach_user_id: Uuid,
                coach_tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(LIST_LIVE_FOR_COACH_SQL)
                    .bind($ids::bind(coach_user_id))
                    .bind($ids::bind(coach_tenant_id.as_uuid()))
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list a coach's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn list_live_for_member_in_group(
                &self,
                group_id: Uuid,
                member_user_id: Uuid,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(LIST_LIVE_FOR_MEMBER_IN_GROUP_SQL)
                    .bind($ids::bind(group_id))
                    .bind($ids::bind(member_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list a member's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn list_live_for_member(
                &self,
                member_user_id: Uuid,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(LIST_LIVE_FOR_MEMBER_SQL)
                    .bind($ids::bind(member_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list a member's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn list_backed_for_member(
                &self,
                member_user_id: Uuid,
                provider: &str,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(LIST_BACKED_FOR_MEMBER_SQL)
                    .bind($ids::bind(member_user_id))
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list a member's backed delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn confirm(
                &self,
                id: Uuid,
                member_user_id: Uuid,
                member_tenant_id: TenantId,
                now: DateTime<Utc>,
            ) -> AppResult<Option<DelegatedConnection>> {
                let row = sqlx::query(CONFIRM_SQL)
                    .bind(now)
                    .bind($ids::bind(member_tenant_id.as_uuid()))
                    .bind($ids::bind(id))
                    .bind($ids::bind(member_user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        // The member already reads this provider through
                        // another confirmed link: the partial unique index on
                        // (member, provider) for confirmed rows, reported by
                        // the driver as such.
                        if e.as_database_error()
                            .is_some_and(|db| db.is_unique_violation())
                        {
                            already_linked()
                        } else {
                            AppError::database(format!(
                                "Failed to confirm delegated connection: {e}"
                            ))
                        }
                    })?;

                row.as_ref().map(row_to_delegated_connection).transpose()
            }

            async fn end_one(
                &self,
                id: Uuid,
                participant_user_id: Uuid,
                actor: Option<Uuid>,
                reason: DelegationEndReason,
                now: DateTime<Utc>,
            ) -> AppResult<Option<DelegatedConnection>> {
                let row = sqlx::query(END_ONE_SQL)
                    .bind(now)
                    .bind($ids::bind_opt(actor))
                    .bind(reason.as_str())
                    .bind($ids::bind(id))
                    .bind($ids::bind(participant_user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to end delegated connection: {e}"))
                    })?;

                row.as_ref().map(row_to_delegated_connection).transpose()
            }

            async fn end_for_group_member(
                &self,
                group_id: Uuid,
                member_user_id: Uuid,
                actor: Option<Uuid>,
                reason: DelegationEndReason,
                now: DateTime<Utc>,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(END_FOR_GROUP_MEMBER_SQL)
                    .bind(now)
                    .bind($ids::bind_opt(actor))
                    .bind(reason.as_str())
                    .bind($ids::bind(group_id))
                    .bind($ids::bind(member_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to end a member's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn end_for_group(
                &self,
                group_id: Uuid,
                actor: Option<Uuid>,
                reason: DelegationEndReason,
                now: DateTime<Utc>,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(END_FOR_GROUP_SQL)
                    .bind(now)
                    .bind($ids::bind_opt(actor))
                    .bind(reason.as_str())
                    .bind($ids::bind(group_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to end a group's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn end_for_coach(
                &self,
                coach_user_id: Uuid,
                coach_tenant_id: TenantId,
                provider: &str,
                actor: Option<Uuid>,
                reason: DelegationEndReason,
                now: DateTime<Utc>,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(END_FOR_COACH_SQL)
                    .bind(now)
                    .bind($ids::bind_opt(actor))
                    .bind(reason.as_str())
                    .bind($ids::bind(coach_user_id))
                    .bind($ids::bind(coach_tenant_id.as_uuid()))
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to end a coach's delegated connections: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn end_confirmed_for_member(
                &self,
                member_user_id: Uuid,
                member_tenant_id: TenantId,
                provider: &str,
                actor: Option<Uuid>,
                reason: DelegationEndReason,
                now: DateTime<Utc>,
            ) -> AppResult<Vec<DelegatedConnection>> {
                let rows = sqlx::query(END_CONFIRMED_FOR_MEMBER_SQL)
                    .bind(now)
                    .bind($ids::bind_opt(actor))
                    .bind(reason.as_str())
                    .bind($ids::bind(member_user_id))
                    .bind($ids::bind(member_tenant_id.as_uuid()))
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to end a member's confirmed delegated connection: {e}"
                        ))
                    })?;

                rows.iter().map(row_to_delegated_connection).collect()
            }

            async fn find_active_for_member(
                &self,
                member_user_id: Uuid,
                member_tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<DelegatedConnection>> {
                let row = sqlx::query(FIND_ACTIVE_FOR_MEMBER_SQL)
                    .bind($ids::bind(member_user_id))
                    .bind($ids::bind(member_tenant_id.as_uuid()))
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to find a member's delegated connection: {e}"
                        ))
                    })?;

                row.as_ref().map(row_to_delegated_connection).transpose()
            }
        }
    };
}
pub(crate) use impl_delegated_connection_repository;
