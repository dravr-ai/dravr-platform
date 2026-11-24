// ABOUTME: Repository trait, statements and shared body for a member's record of fellow members reading their data under peer consent
// ABOUTME: One row per read (a group view once a day), tenant-scoped under the member's tenant, listed in their feed, purged with coach reads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Peer data reads (carnet#1543).
//!
//! A group member who shares their training with their peers is read by a
//! fellow member through Dravr's own surfaces (a peer's sharing never reaches
//! an outside assistant, carnet#1511): `get_group_member_activities`, and the
//! views built from every member's training (the room's member cards, the
//! group's stats, report and health flags, a manager's copy of the weekly
//! digest).
//! Every such read is recorded here, under a tenant the member belongs to,
//! and the member's "who read my data" feed lists their own rows: one row per
//! read, or for a group view at most one a UTC day
//! ([`PeerDataReadRepository::record_daily_peer_read`]). A read by the
//! group's human coach is the member's `coach_data_reads` record instead.
//! Rows older than the coach reads' retention are purged by the same hourly
//! sweep.

use std::fmt::Display;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use uuid::Uuid;

use super::coach_data_reads::RecordedTransport;

/// One read of a member's data by a fellow member, as it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPeerDataRead<'read> {
    /// A tenant the member whose data was read belongs to.
    pub tenant: TenantId,
    /// The member whose data was read.
    pub member: Uuid,
    /// The fellow member who read it.
    pub peer: Uuid,
    /// The group whose peer sharing let the read through.
    pub group: Uuid,
    /// The tool the reader's agent read through, or the group view's label
    /// (`CoachReadSurface::as_str`).
    pub tool: &'read str,
    /// When it was read.
    pub read_at: Timestamp,
    /// The transport the read was served over; `None` when it declared none.
    pub transport: Option<Transport>,
    /// The admin whose support session made the read while acting as the
    /// reader; `None` for the reader's own.
    pub impersonator: Option<Uuid>,
}

/// One recorded read, as the member's feed lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDataRead {
    /// The fellow member who read; `None` once that account is deleted.
    pub peer: Option<Uuid>,
    /// The group whose peer sharing let the read through.
    pub group: Uuid,
    /// The tool or group view the read was made through.
    pub tool: String,
    /// When it was read.
    pub read_at: Timestamp,
    /// What the row says the read was served over.
    pub transport: RecordedTransport,
    /// The admin whose support session made the read acting as the reader.
    pub impersonator: Option<Uuid>,
    /// The read is a group view recorded at most once a UTC day, so it stands
    /// for every view of that source that day.
    pub once_a_day: bool,
}

/// Record and list a member's record of their peers' reads.
#[async_trait]
pub trait PeerDataReadRepository: Send + Sync {
    /// Record one read.
    async fn record_peer_read(&self, read: &NewPeerDataRead<'_>) -> AppResult<()>;

    /// Record one read at most once a UTC day: a read of the same member by
    /// the same peer in the same group through the same source, already
    /// recorded on `read.read_at`'s UTC day, writes nothing. `true` when this
    /// call wrote the row, `false` on such a repeat, which is no failure.
    async fn record_daily_peer_read(&self, read: &NewPeerDataRead<'_>) -> AppResult<bool>;

    /// `member`'s recorded reads under `tenant`, made at or after `since`,
    /// newest first, at most `limit`.
    async fn list_peer_reads_since(
        &self,
        tenant: TenantId,
        member: Uuid,
        since: Timestamp,
        limit: u32,
    ) -> AppResult<Vec<PeerDataRead>>;

    /// Delete every read recorded before `cutoff`, in every tenant, returning
    /// how many went.
    async fn purge_peer_reads_before(&self, cutoff: Timestamp) -> AppResult<u64>;
}

/// Record one read. `$n` placeholders serve both backends; the ids bind
/// through the backend's uuid codec (see [`super::uuid_columns`]), the
/// tenant, group and day are `TEXT` on both.
///
/// `$7` is the `read_day`: the UTC day of a group view recorded once a day,
/// `NULL` for a read recorded each time. The conflict target is the unique
/// index `idx_peer_data_reads_daily`, which holds the day: a repeat that day
/// writes nothing, and a `NULL` day never conflicts.
pub(crate) const RECORD_PEER_READ_SQL: &str = r"
            INSERT INTO peer_data_reads
                (id, tenant_id, user_id, peer_user_id, group_id, tool_name, read_day, read_at,
                 transport, impersonator_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (tenant_id, user_id, peer_user_id, group_id, tool_name, read_day)
            DO NOTHING
            ";

/// The member's reads under one tenant since `$3`, newest first.
pub(crate) const LIST_PEER_READS_SINCE_SQL: &str = r"
            SELECT peer_user_id, group_id, tool_name, read_at, read_day, transport,
                   impersonator_user_id
            FROM peer_data_reads
            WHERE tenant_id = $1 AND user_id = $2 AND read_at >= $3
            ORDER BY read_at DESC
            LIMIT $4
            ";

/// The retention purge.
///
/// ISOLATION EXEMPTION: the purge spans every tenant on purpose, as the
/// coach reads' does: it serves the one retention every tenant's members are
/// under, carries no data out and selects by age alone.
pub(crate) const PURGE_PEER_READS_SQL: &str = r"
            DELETE FROM peer_data_reads WHERE read_at < $1
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn peer_read_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} peer data read: {e}"))
}

/// Emit the whole [`PeerDataReadRepository`] implementation for one backend
/// type, whose rows are `$row`. `$ids` is the codec in
/// [`super::uuid_columns`] for how that backend's id columns bind and read.
macro_rules! impl_peer_data_read_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// One listed row as the read it records.
        fn peer_read_from_row(row: &$row) -> AppResult<PeerDataRead> {
            let text = |col: &str| -> AppResult<String> {
                row.try_get(col).map_err(|e| peer_read_error("decode", e))
            };
            let optional = |col: &str| -> AppResult<Option<String>> {
                row.try_get(col).map_err(|e| peer_read_error("decode", e))
            };
            Ok(PeerDataRead {
                peer: $ids::read_opt(row, "peer_user_id")?,
                group: Uuid::parse_str(&text("group_id")?)
                    .map_err(|e| peer_read_error("decode", e))?,
                tool: text("tool_name")?,
                read_at: row
                    .try_get("read_at")
                    .map_err(|e| peer_read_error("decode", e))?,
                transport: parse_transport(optional("transport")?.as_deref())?,
                impersonator: $ids::read_opt(row, "impersonator_user_id")?,
                once_a_day: optional("read_day")?.is_some(),
            })
        }

        impl $ty {
            /// Insert `read`, keyed on `day` when it is recorded once a day.
            async fn insert_peer_read(
                &self,
                read: &NewPeerDataRead<'_>,
                day: Option<String>,
            ) -> AppResult<u64> {
                let done = sqlx::query(RECORD_PEER_READ_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(read.tenant.to_string())
                    .bind($ids::bind(read.member))
                    .bind($ids::bind(read.peer))
                    .bind(read.group.to_string())
                    .bind(read.tool)
                    .bind(day)
                    .bind(read.read_at)
                    .bind(Transport::label_of(read.transport))
                    .bind($ids::bind_opt(read.impersonator))
                    .execute(self.pool())
                    .await
                    .map_err(|e| peer_read_error("record", e))?;
                Ok(done.rows_affected())
            }
        }

        #[async_trait::async_trait]
        impl PeerDataReadRepository for $ty {
            async fn record_peer_read(&self, read: &NewPeerDataRead<'_>) -> AppResult<()> {
                self.insert_peer_read(read, None).await.map(drop)
            }

            async fn record_daily_peer_read(&self, read: &NewPeerDataRead<'_>) -> AppResult<bool> {
                let day = read.read_at.utc_date().to_string();
                Ok(self.insert_peer_read(read, Some(day)).await? > 0)
            }

            async fn list_peer_reads_since(
                &self,
                tenant: TenantId,
                member: Uuid,
                since: Timestamp,
                limit: u32,
            ) -> AppResult<Vec<PeerDataRead>> {
                let rows = sqlx::query(LIST_PEER_READS_SINCE_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(member))
                    .bind(since)
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| peer_read_error("list", e))?;
                rows.iter().map(peer_read_from_row).collect()
            }

            async fn purge_peer_reads_before(&self, cutoff: Timestamp) -> AppResult<u64> {
                let done = sqlx::query(PURGE_PEER_READS_SQL)
                    .bind(cutoff)
                    .execute(self.pool())
                    .await
                    .map_err(|e| peer_read_error("purge", e))?;
                Ok(done.rows_affected())
            }
        }
    };
}
pub(crate) use impl_peer_data_read_repository;

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns between the first two parentheses after `marker` in `sql`.
    fn columns_after<'a>(sql: &'a str, marker: &str) -> Vec<&'a str> {
        let start = sql.find(marker).expect("the marker") + marker.len();
        let open = start + sql[start..].find('(').expect("an opening parenthesis") + 1;
        let close = open + sql[open..].find(')').expect("a closing parenthesis");
        sql[open..close].split(',').map(str::trim).collect()
    }

    #[test]
    fn the_daily_conflict_target_is_the_unique_index_on_both_engines() {
        let target = columns_after(RECORD_PEER_READ_SQL, "ON CONFLICT");
        // Reads the migrations as data: a parity check between the statement
        // and both backends' migrations. Only the PostgreSQL lane runs the
        // statement against the PostgreSQL index, so this catches a drift on
        // that backend in every build.
        for migration in [
            include_str!("../../migrations/20261122120003_peer_data_reads.sql"),
            include_str!("../../migrations_pg/20261122120003_peer_data_reads.sql"),
        ] {
            assert_eq!(
                columns_after(
                    migration,
                    "idx_peer_data_reads_daily\n    ON peer_data_reads"
                ),
                target,
                "bug: ON CONFLICT names no unique index, so a repeat view errors instead of \
                 writing nothing"
            );
        }
    }
}
