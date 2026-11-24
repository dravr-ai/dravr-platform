// ABOUTME: Repository trait, shared statements and body for admin signup codes and the one redemption each account holds
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user columns need (carnet#1128)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Signup codes, written once.
//!
//! A code an admin mints is a bearer secret: only its SHA-256 reaches the
//! table, and a use is claimed by one conditional `UPDATE` that checks
//! revocation, expiry and the use limit in the same statement, so two
//! accounts entering the last use cannot both pass.
//!
//! A redemption is keyed by the account (`user_id` primary key): an account
//! holds one, naming either an admin's code or a coach's approving group
//! invite. `signup_codes.id` is `TEXT` on both backends; `group_invites.id`
//! is `TEXT` on `SQLite` and `uuid` on PostgreSQL, so `group_invite_id`, like
//! `user_id`, `created_by` and `granted_by`, goes through the shell's
//! [`uuid_columns`](super::uuid_columns) codec.

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    GroupInviteFunnelRow, GroupInviteHold, SignupCode, SignupCodeRedemption, SignupCodeSource,
};
use uuid::Uuid;

/// Admin signup codes and the accounts that redeemed one, and the group
/// invite an account holds until it can join (carnet#1648).
#[async_trait]
pub trait SignupCodeRepository: Send + Sync {
    /// Store a freshly minted code by its hash.
    async fn mint(
        &self,
        code_hash: &str,
        code_hint: &str,
        note: Option<&str>,
        created_by: Option<Uuid>,
        expires_at: Option<Timestamp>,
        max_uses: Option<i32>,
    ) -> AppResult<SignupCode>;
    /// The live code with this hash at `now`, without claiming a use.
    async fn find_live(&self, code_hash: &str, now: Timestamp) -> AppResult<Option<SignupCode>>;
    /// Claim one use of the live code with this hash, atomically: `None` when
    /// it is unknown, revoked, expired or used up.
    async fn claim(&self, code_hash: &str, now: Timestamp) -> AppResult<Option<SignupCode>>;
    /// Give back a use claimed for an account that could not record it.
    async fn release(&self, code_id: Uuid) -> AppResult<()>;
    /// Every code, newest first.
    async fn list(&self) -> AppResult<Vec<SignupCode>>;
    /// Withdraw a code. Returns whether a live one was revoked.
    async fn revoke(&self, code_id: Uuid, now: Timestamp) -> AppResult<bool>;
    /// Record that `user_id` redeemed `source`. Returns `false` when the
    /// account already holds a redemption (the new one is not recorded).
    async fn record_redemption(
        &self,
        user_id: Uuid,
        source: SignupCodeSource,
        granted_by: Option<Uuid>,
        now: Timestamp,
    ) -> AppResult<bool>;
    /// The redemption an account holds.
    async fn redemption_for(&self, user_id: Uuid) -> AppResult<Option<SignupCodeRedemption>>;
    /// Mark an account's redemption as having activated it.
    async fn mark_activated(&self, user_id: Uuid, now: Timestamp) -> AppResult<bool>;
    /// Every redemption, newest first, for the admin listing.
    async fn list_redemptions(&self) -> AppResult<Vec<SignupCodeRedemption>>;
    /// Hold `invite_id` for `user_id` until the account joins, replacing the
    /// invite it held before.
    async fn hold_group_invite(
        &self,
        user_id: Uuid,
        invite_id: Uuid,
        now: Timestamp,
    ) -> AppResult<()>;
    /// The group invite an account holds.
    async fn held_group_invite(&self, user_id: Uuid) -> AppResult<Option<GroupInviteHold>>;
    /// Drop the account's hold on `invite_id`. A hold the account took on
    /// another invite since is kept. Returns whether one was dropped.
    async fn release_group_invite_hold(&self, user_id: Uuid, invite_id: Uuid) -> AppResult<bool>;
    /// Stamp the account's hold on `invite_id` as refused because the group
    /// is full, keeping it for a retry (carnet#1650). A hold the account
    /// took on another invite since is left alone. Returns whether one was
    /// stamped.
    async fn mark_group_invite_hold_full(
        &self,
        user_id: Uuid,
        invite_id: Uuid,
        now: Timestamp,
    ) -> AppResult<bool>;
    /// Count one opening of the invite's public card: `open_count` up by one,
    /// `first_opened_at` set once, `last_opened_at` at `now`. Scoped to the
    /// invite's own tenant. Returns whether an invite was counted.
    async fn record_group_invite_open(
        &self,
        invite_id: Uuid,
        tenant_id: &str,
        now: Timestamp,
    ) -> AppResult<bool>;
    /// The funnel of every invite of `group_id` in `tenant_id`, one row per
    /// invite, in one query. Holds taken before `live_since` have lapsed and
    /// are not counted.
    async fn group_invite_funnels(
        &self,
        group_id: Uuid,
        tenant_id: &str,
        live_since: Timestamp,
    ) -> AppResult<Vec<GroupInviteFunnelRow>>;
}

/// The columns every code read decodes.
macro_rules! signup_code_columns {
    () => {
        "id, code_hint, note, created_by, expires_at, max_uses, use_count, revoked_at, created_at"
    };
}

/// The columns every redemption read decodes.
macro_rules! redemption_columns {
    () => {
        "user_id, signup_code_id, group_invite_id, granted_by, redeemed_at, activated_at"
    };
}

/// One code, unused from the start.
pub(crate) const INSERT_SIGNUP_CODE_SQL: &str = r"
            INSERT INTO signup_codes
                (id, code_hash, code_hint, note, created_by, expires_at, max_uses, use_count, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8)
            ";

/// The live code with a hash at `$2`, unclaimed.
pub(crate) const FIND_LIVE_SIGNUP_CODE_SQL: &str = concat!(
    "SELECT ",
    signup_code_columns!(),
    " FROM signup_codes
            WHERE code_hash = $1 AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > $2)
              AND (max_uses IS NULL OR use_count < max_uses)"
);

/// Claim one use: the liveness check and the count are one statement.
pub(crate) const CLAIM_SIGNUP_CODE_SQL: &str = concat!(
    "UPDATE signup_codes SET use_count = use_count + 1
            WHERE code_hash = $1 AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > $2)
              AND (max_uses IS NULL OR use_count < max_uses)
            RETURNING ",
    signup_code_columns!()
);

/// Give back one claimed use.
pub(crate) const RELEASE_SIGNUP_CODE_SQL: &str =
    "UPDATE signup_codes SET use_count = use_count - 1 WHERE id = $1 AND use_count > 0";

/// Every code, newest first.
pub(crate) const LIST_SIGNUP_CODES_SQL: &str = concat!(
    "SELECT ",
    signup_code_columns!(),
    " FROM signup_codes ORDER BY created_at DESC"
);

/// Withdraw a live code.
pub(crate) const REVOKE_SIGNUP_CODE_SQL: &str =
    "UPDATE signup_codes SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL";

/// Record one redemption; an account that holds one keeps it.
pub(crate) const INSERT_REDEMPTION_SQL: &str = r"
            INSERT INTO signup_code_redemptions
                (user_id, signup_code_id, group_invite_id, granted_by, redeemed_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id) DO NOTHING
            ";

/// The redemption an account holds.
pub(crate) const GET_REDEMPTION_SQL: &str = concat!(
    "SELECT ",
    redemption_columns!(),
    " FROM signup_code_redemptions WHERE user_id = $1"
);

/// Mark a redemption as having activated its account, once.
pub(crate) const MARK_REDEMPTION_ACTIVATED_SQL: &str =
    "UPDATE signup_code_redemptions SET activated_at = $1
            WHERE user_id = $2 AND activated_at IS NULL";

/// Every redemption, newest first.
pub(crate) const LIST_REDEMPTIONS_SQL: &str = concat!(
    "SELECT ",
    redemption_columns!(),
    " FROM signup_code_redemptions ORDER BY redeemed_at DESC"
);

/// Hold an invite for an account, replacing the one it held.
pub(crate) const HOLD_GROUP_INVITE_SQL: &str = r"
            INSERT INTO group_invite_holds (user_id, invite_id, held_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id) DO UPDATE SET invite_id = excluded.invite_id, held_at = excluded.held_at,
                refused_full_at = NULL
            ";

/// The invite an account holds.
pub(crate) const GET_GROUP_INVITE_HOLD_SQL: &str =
    "SELECT user_id, invite_id, held_at, refused_full_at FROM group_invite_holds WHERE user_id = $1";

/// Drop an account's hold, only while it still names the same invite.
pub(crate) const RELEASE_GROUP_INVITE_HOLD_SQL: &str =
    "DELETE FROM group_invite_holds WHERE user_id = $1 AND invite_id = $2";

/// Keep a hold a full group refused, stamped, while it still names the same
/// invite.
pub(crate) const MARK_GROUP_INVITE_HOLD_FULL_SQL: &str =
    "UPDATE group_invite_holds SET refused_full_at = $1
            WHERE user_id = $2 AND invite_id = $3";

/// Count one opening of an invite's public card, under the invite's tenant.
pub(crate) const RECORD_GROUP_INVITE_OPEN_SQL: &str = "UPDATE group_invites
            SET open_count = open_count + 1,
                first_opened_at = COALESCE(first_opened_at, $1),
                last_opened_at = $1
            WHERE id = $2 AND tenant_id = $3";

/// One funnel row per invite of a group: the open counter, and the live holds
/// (taken at or after `$3`) that name it.
pub(crate) const GROUP_INVITE_FUNNELS_SQL: &str = r"
            SELECT i.id AS invite_id, i.open_count, i.last_opened_at,
                   COUNT(h.user_id) AS held_count,
                   MAX(h.held_at) AS last_held_at,
                   COUNT(h.refused_full_at) AS full_count
            FROM group_invites i
            LEFT JOIN group_invite_holds h ON h.invite_id = i.id AND h.held_at >= $3
            WHERE i.group_id = $1 AND i.tenant_id = $2
            GROUP BY i.id, i.open_count, i.last_opened_at
            ";

/// Read a `TEXT` id column as a uuid; `try_get` so a corrupt row is an error.
pub(crate) fn text_uuid_opt<R>(row: &R, col: &str) -> AppResult<Option<Uuid>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let raw: Option<String> = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("signup codes {col}: {e}")))?;
    raw.map(|value| {
        Uuid::parse_str(&value)
            .map_err(|e| AppError::database(format!("signup codes {col} is not a uuid: {e}")))
    })
    .transpose()
}

/// Decode a code row. `created_by` is decoded by the caller through its
/// backend's uuid codec and handed in.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn signup_code_from_row<R>(row: &R, created_by: Option<Uuid>) -> AppResult<SignupCode>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str| AppError::database(format!("signup_codes {name}"));
    Ok(SignupCode {
        id: text_uuid_opt(row, "id")?.ok_or_else(|| column("id"))?,
        code_hint: row.try_get("code_hint").map_err(|_| column("code_hint"))?,
        note: row.try_get("note").map_err(|_| column("note"))?,
        created_by,
        expires_at: row
            .try_get("expires_at")
            .map_err(|_| column("expires_at"))?,
        max_uses: row.try_get("max_uses").map_err(|_| column("max_uses"))?,
        use_count: row.try_get("use_count").map_err(|_| column("use_count"))?,
        revoked_at: row
            .try_get("revoked_at")
            .map_err(|_| column("revoked_at"))?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| column("created_at"))?,
    })
}

/// Decode a redemption row. The uuid columns are decoded by the caller
/// through its backend's codec and handed in.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn redemption_from_row<R>(
    row: &R,
    user_id: Uuid,
    granted_by: Option<Uuid>,
    group_invite_id: Option<Uuid>,
) -> AppResult<SignupCodeRedemption>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str| AppError::database(format!("signup_code_redemptions {name}"));
    let source = match (text_uuid_opt(row, "signup_code_id")?, group_invite_id) {
        (Some(code), _) => Some(SignupCodeSource::AdminCode(code)),
        (None, Some(invite)) => Some(SignupCodeSource::CoachInvite(invite)),
        (None, None) => None,
    };
    Ok(SignupCodeRedemption {
        user_id,
        source,
        granted_by,
        redeemed_at: row
            .try_get("redeemed_at")
            .map_err(|_| column("redeemed_at"))?,
        activated_at: row
            .try_get("activated_at")
            .map_err(|_| column("activated_at"))?,
    })
}

/// The two id columns a source binds: exactly one is set. The invite id is
/// bound through the backend's uuid codec by the caller.
pub(crate) fn source_columns(source: SignupCodeSource) -> (Option<String>, Option<Uuid>) {
    match source {
        SignupCodeSource::AdminCode(id) => (Some(id.to_string()), None),
        SignupCodeSource::CoachInvite(id) => (None, Some(id)),
    }
}

/// Emit the whole [`SignupCodeRepository`] implementation for one backend
/// type. `$ids` is that backend's [`uuid_columns`](super::uuid_columns)
/// codec. The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_signup_code_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl SignupCodeRepository for $ty {
            async fn mint(
                &self,
                code_hash: &str,
                code_hint: &str,
                note: Option<&str>,
                created_by: Option<Uuid>,
                expires_at: Option<Timestamp>,
                max_uses: Option<i32>,
            ) -> AppResult<SignupCode> {
                let id = Uuid::new_v4();
                let now = Timestamp::now();
                sqlx::query(INSERT_SIGNUP_CODE_SQL)
                    .bind(id.to_string())
                    .bind(code_hash)
                    .bind(code_hint)
                    .bind(note)
                    .bind($ids::bind_opt(created_by))
                    .bind(expires_at)
                    .bind(max_uses)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to mint signup code: {e}")))?;
                Ok(SignupCode {
                    id,
                    code_hint: code_hint.to_owned(),
                    note: note.map(str::to_owned),
                    created_by,
                    expires_at,
                    max_uses,
                    use_count: 0,
                    revoked_at: None,
                    created_at: now,
                })
            }

            async fn find_live(
                &self,
                code_hash: &str,
                now: Timestamp,
            ) -> AppResult<Option<SignupCode>> {
                let row = sqlx::query(FIND_LIVE_SIGNUP_CODE_SQL)
                    .bind(code_hash)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read signup code: {e}")))?;
                row.as_ref()
                    .map(|row| signup_code_from_row(row, $ids::read_opt(row, "created_by")?))
                    .transpose()
            }

            async fn claim(
                &self,
                code_hash: &str,
                now: Timestamp,
            ) -> AppResult<Option<SignupCode>> {
                let row = sqlx::query(CLAIM_SIGNUP_CODE_SQL)
                    .bind(code_hash)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to claim signup code: {e}")))?;
                row.as_ref()
                    .map(|row| signup_code_from_row(row, $ids::read_opt(row, "created_by")?))
                    .transpose()
            }

            async fn release(&self, code_id: Uuid) -> AppResult<()> {
                sqlx::query(RELEASE_SIGNUP_CODE_SQL)
                    .bind(code_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release signup code: {e}"))
                    })?;
                Ok(())
            }

            async fn list(&self) -> AppResult<Vec<SignupCode>> {
                let rows = sqlx::query(LIST_SIGNUP_CODES_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list signup codes: {e}")))?;
                rows.iter()
                    .map(|row| signup_code_from_row(row, $ids::read_opt(row, "created_by")?))
                    .collect()
            }

            async fn revoke(&self, code_id: Uuid, now: Timestamp) -> AppResult<bool> {
                let result = sqlx::query(REVOKE_SIGNUP_CODE_SQL)
                    .bind(now)
                    .bind(code_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to revoke signup code: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn record_redemption(
                &self,
                user_id: Uuid,
                source: SignupCodeSource,
                granted_by: Option<Uuid>,
                now: Timestamp,
            ) -> AppResult<bool> {
                let (code_id, invite_id) = source_columns(source);
                let result = sqlx::query(INSERT_REDEMPTION_SQL)
                    .bind($ids::bind(user_id))
                    .bind(code_id)
                    .bind($ids::bind_opt(invite_id))
                    .bind($ids::bind_opt(granted_by))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record signup code redemption: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn redemption_for(
                &self,
                user_id: Uuid,
            ) -> AppResult<Option<SignupCodeRedemption>> {
                let row = sqlx::query(GET_REDEMPTION_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read signup code redemption: {e}"))
                    })?;
                row.as_ref()
                    .map(|row| {
                        redemption_from_row(
                            row,
                            user_id,
                            $ids::read_opt(row, "granted_by")?,
                            $ids::read_opt(row, "group_invite_id")?,
                        )
                    })
                    .transpose()
            }

            async fn mark_activated(&self, user_id: Uuid, now: Timestamp) -> AppResult<bool> {
                let result = sqlx::query(MARK_REDEMPTION_ACTIVATED_SQL)
                    .bind(now)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark redemption activated: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn list_redemptions(&self) -> AppResult<Vec<SignupCodeRedemption>> {
                let rows = sqlx::query(LIST_REDEMPTIONS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list signup code redemptions: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        redemption_from_row(
                            row,
                            $ids::read(row, "user_id")?,
                            $ids::read_opt(row, "granted_by")?,
                            $ids::read_opt(row, "group_invite_id")?,
                        )
                    })
                    .collect()
            }

            async fn hold_group_invite(
                &self,
                user_id: Uuid,
                invite_id: Uuid,
                now: Timestamp,
            ) -> AppResult<()> {
                sqlx::query(HOLD_GROUP_INVITE_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(invite_id))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to hold group invite: {e}")))?;
                Ok(())
            }

            async fn held_group_invite(&self, user_id: Uuid) -> AppResult<Option<GroupInviteHold>> {
                use sqlx::Row;

                let row = sqlx::query(GET_GROUP_INVITE_HOLD_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read held group invite: {e}"))
                    })?;
                row.as_ref()
                    .map(|row| {
                        Ok(GroupInviteHold {
                            user_id,
                            invite_id: $ids::read(row, "invite_id")?,
                            held_at: row.try_get("held_at").map_err(|e| {
                                AppError::database(format!("group_invite_holds held_at: {e}"))
                            })?,
                            refused_full_at: row.try_get("refused_full_at").map_err(|e| {
                                AppError::database(format!(
                                    "group_invite_holds refused_full_at: {e}"
                                ))
                            })?,
                        })
                    })
                    .transpose()
            }

            async fn release_group_invite_hold(
                &self,
                user_id: Uuid,
                invite_id: Uuid,
            ) -> AppResult<bool> {
                let result = sqlx::query(RELEASE_GROUP_INVITE_HOLD_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(invite_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release held group invite: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn mark_group_invite_hold_full(
                &self,
                user_id: Uuid,
                invite_id: Uuid,
                now: Timestamp,
            ) -> AppResult<bool> {
                let result = sqlx::query(MARK_GROUP_INVITE_HOLD_FULL_SQL)
                    .bind(now)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(invite_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark held group invite full: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn record_group_invite_open(
                &self,
                invite_id: Uuid,
                tenant_id: &str,
                now: Timestamp,
            ) -> AppResult<bool> {
                let result = sqlx::query(RECORD_GROUP_INVITE_OPEN_SQL)
                    .bind(now)
                    .bind($ids::bind(invite_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count a group invite opening: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn group_invite_funnels(
                &self,
                group_id: Uuid,
                tenant_id: &str,
                live_since: Timestamp,
            ) -> AppResult<Vec<GroupInviteFunnelRow>> {
                use sqlx::Row;

                let rows = sqlx::query(GROUP_INVITE_FUNNELS_SQL)
                    .bind($ids::bind(group_id))
                    .bind(tenant_id)
                    .bind(live_since)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read group invite funnels: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        let column = |name: &'static str| {
                            move |e: sqlx::Error| {
                                AppError::database(format!("group invite funnel {name}: {e}"))
                            }
                        };
                        Ok(GroupInviteFunnelRow {
                            invite_id: $ids::read(row, "invite_id")?,
                            // An `INTEGER` column: INT4 on PostgreSQL, which
                            // decodes only as `i32`. The counts are `COUNT()`s,
                            // INT8 on both backends.
                            open_count: row
                                .try_get::<i32, _>("open_count")
                                .map(i64::from)
                                .map_err(column("open_count"))?,
                            last_opened_at: row
                                .try_get("last_opened_at")
                                .map_err(column("last_opened_at"))?,
                            held_count: row.try_get("held_count").map_err(column("held_count"))?,
                            last_held_at: row
                                .try_get("last_held_at")
                                .map_err(column("last_held_at"))?,
                            full_count: row.try_get("full_count").map_err(column("full_count"))?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_signup_code_repository;
