// ABOUTME: Repository trait, statements and shared body for the athlete's statutory data access requests
// ABOUTME: Every read is the athlete's own in their tenant, save the daily sweep's, which claims each reminder once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Data access requests (carnet#1646, Owl row 60).
//!
//! An athlete asks a company for the personal information it holds about them,
//! under Quebec's P-39.1 or the GDPR. Dravr writes the letter and tracks the
//! legal deadline; the athlete sends it. A row here is one such request: who
//! it is addressed to, under which law and scope, and the dates the deadline
//! is counted from.
//!
//! The stored status is `drafted`, `sent`, `answered` or `closed`. Whether a
//! sent request is *due soon* or *overdue* is derived from `due_on` when it is
//! read, never stored. `due_on` itself is stored so the sweep has a column to
//! index: the service recomputes it from `sent_on`, the law and
//! `extended_until` on every write.
//!
//! Every statement is scoped to the tenant and the athlete, save the sweep's
//! read ([`DataAccessRequestRepository::due_for_reminder`]), which reads due
//! rows across tenants as retention does. Each reminder it sends is claimed
//! first by a conditional update
//! ([`DataAccessRequestRepository::claim_reminder`]) that matches only while
//! the stage's stamp is NULL, so two instances sweeping at once send it once.
//!
//! `user_id` is `uuid` on Postgres and `TEXT` on `SQLite`; it binds and reads
//! through the shell's codec (`$ids`). `id` and `tenant_id` are `TEXT` on
//! both. The four dates are `DATE` on Postgres and ISO `TEXT` on `SQLite`,
//! which compares in the same order. `$n` placeholders throughout.

use async_trait::async_trait;
use chrono::{DateTime, Days, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The law a request is made under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataAccessLaw {
    /// Quebec's Act respecting the protection of personal information in the
    /// private sector (CQLR c. P-39.1), as amended by Law 25.
    QuebecP39_1,
    /// Regulation (EU) 2016/679, the GDPR.
    Gdpr,
}

impl DataAccessLaw {
    /// The stored and wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuebecP39_1 => "quebec_p39_1",
            Self::Gdpr => "gdpr",
        }
    }

    /// The law `text` names, or `None` for any other text.
    #[must_use]
    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "quebec_p39_1" => Some(Self::QuebecP39_1),
            "gdpr" => Some(Self::Gdpr),
            _ => None,
        }
    }
}

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataAccessScope {
    /// A copy of the personal information held.
    Access,
    /// The copy, and the information in a structured format another service
    /// can read (P-39.1 art. 27 para. 3, GDPR Art. 20).
    AccessPortability,
}

impl DataAccessScope {
    /// The stored and wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::AccessPortability => "access_portability",
        }
    }

    /// The scope `text` names, or `None` for any other text.
    #[must_use]
    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "access" => Some(Self::Access),
            "access_portability" => Some(Self::AccessPortability),
            _ => None,
        }
    }
}

/// Where a request stands, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataAccessStatus {
    /// Written, not sent yet; the athlete may still edit it.
    Drafted,
    /// Sent on `sent_on`; the deadline runs.
    Sent,
    /// The company answered on `answered_on`.
    Answered,
    /// The athlete gave up, or the request no longer matters.
    Closed,
}

impl DataAccessStatus {
    /// The stored and wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drafted => "drafted",
            Self::Sent => "sent",
            Self::Answered => "answered",
            Self::Closed => "closed",
        }
    }

    /// The status `text` names, or `None` for any other text.
    #[must_use]
    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "drafted" => Some(Self::Drafted),
            "sent" => Some(Self::Sent),
            "answered" => Some(Self::Answered),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// The two reminders a sent request may raise, at most one of each per due
/// date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderStage {
    /// Three days or fewer before the due date.
    DueSoon,
    /// The due date passed with no answer.
    Overdue,
}

/// One data access request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAccessRequest {
    /// Its id.
    pub id: Uuid,
    /// The tenant of the session that wrote it.
    pub tenant_id: TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// `DRV-<year>-<6 base32 characters>`, quoted in the letter.
    pub reference: String,
    /// The company the request is addressed to.
    pub controller: String,
    /// The company's privacy address, as the athlete entered it.
    pub controller_contact: String,
    /// The account or email address the athlete uses with the company.
    pub athlete_identifier: String,
    /// The law the request is made under.
    pub law: DataAccessLaw,
    /// What it asks for.
    pub scope: DataAccessScope,
    /// The language the letter is written in.
    pub letter_locale: String,
    /// Where it stands.
    pub status: DataAccessStatus,
    /// The day the athlete sent it.
    pub sent_on: Option<NaiveDate>,
    /// The day the company must answer by, once sent.
    pub due_on: Option<NaiveDate>,
    /// The later date a GDPR controller notified it would answer by.
    pub extended_until: Option<NaiveDate>,
    /// The day the company answered.
    pub answered_on: Option<NaiveDate>,
    /// When the due-soon reminder was claimed.
    pub due_soon_notified_at: Option<DateTime<Utc>>,
    /// When the overdue reminder was claimed.
    pub overdue_notified_at: Option<DateTime<Utc>>,
    /// When it was written.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// The athlete's data access requests, and the sweep's claims on them.
#[async_trait]
pub trait DataAccessRequestRepository: Send + Sync {
    /// Insert `request`. `false` when its reference is already taken, and
    /// nothing is written: the caller draws another.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn insert(&self, request: &DataAccessRequest) -> AppResult<bool>;

    /// The athlete's request `id` in `tenant_id`; `None` when it is not theirs.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn get(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
    ) -> AppResult<Option<DataAccessRequest>>;

    /// The athlete's requests in `tenant_id`, newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<Vec<DataAccessRequest>>;

    /// Write `request`'s editable columns, its status and its dates, while
    /// the stored row is still in `expected`. With `reset_reminders` both
    /// reminder stamps are cleared, because the due date moved; otherwise the
    /// stored stamps stand, so a claim the sweep made since the caller read
    /// the row is kept. `false` when the row is not the athlete's or no
    /// longer in `expected`, which is left as it is.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn update(
        &self,
        request: &DataAccessRequest,
        expected: DataAccessStatus,
        reset_reminders: bool,
    ) -> AppResult<bool>;

    /// Sent requests of every athlete whose `stage` reminder is due on
    /// `today` and not yet claimed, earliest due first, at most `limit`: due
    /// within three days for [`ReminderStage::DueSoon`], past due for
    /// [`ReminderStage::Overdue`]. The sweep's read, across tenants by design.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn due_for_reminder(
        &self,
        stage: ReminderStage,
        today: NaiveDate,
        limit: i64,
    ) -> AppResult<Vec<DataAccessRequest>>;

    /// Claim the `stage` reminder of the request `id` for the due date
    /// `due_on`, stamping it `at`. `true` for the one caller that claimed it;
    /// `false` when the stage was claimed already, or the request is no
    /// longer sent, or its due date moved since it was read.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn claim_reminder(
        &self,
        stage: ReminderStage,
        request: &DataAccessRequest,
        due_on: NaiveDate,
        at: DateTime<Utc>,
    ) -> AppResult<bool>;
}

/// How many days before its due date a request is due soon.
pub const DUE_SOON_DAYS: u64 = 3;

/// The columns every read decodes.
macro_rules! data_access_request_columns {
    () => {
        "id, tenant_id, user_id, reference, controller, controller_contact, athlete_identifier, \
         law, scope, letter_locale, status, sent_on, due_on, extended_until, answered_on, \
         due_soon_notified_at, overdue_notified_at, created_at, updated_at"
    };
}

/// Insert a request; a taken reference writes nothing.
pub(crate) const INSERT_SQL: &str = concat!(
    "INSERT INTO data_access_requests (",
    data_access_request_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
     ON CONFLICT DO NOTHING"
);

/// One of the athlete's requests.
pub(crate) const GET_SQL: &str = concat!(
    "SELECT ",
    data_access_request_columns!(),
    " FROM data_access_requests WHERE tenant_id = $1 AND user_id = $2 AND id = $3"
);

/// The athlete's requests, newest first.
pub(crate) const LIST_SQL: &str = concat!(
    "SELECT ",
    data_access_request_columns!(),
    " FROM data_access_requests WHERE tenant_id = $1 AND user_id = $2 \
     ORDER BY created_at DESC, id DESC"
);

/// Write a request while it is still in the status the caller read.
pub(crate) const UPDATE_SQL: &str = "UPDATE data_access_requests SET \
     controller = $5, controller_contact = $6, athlete_identifier = $7, law = $8, scope = $9, \
     letter_locale = $10, status = $11, sent_on = $12, due_on = $13, extended_until = $14, \
     answered_on = $15, updated_at = $16, \
     due_soon_notified_at = CASE WHEN $17 THEN NULL ELSE due_soon_notified_at END, \
     overdue_notified_at = CASE WHEN $17 THEN NULL ELSE overdue_notified_at END \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND status = $4";

/// The sweep: sent requests due within the window and not yet reminded.
pub(crate) const DUE_SOON_SQL: &str = concat!(
    "SELECT ",
    data_access_request_columns!(),
    " FROM data_access_requests WHERE status = 'sent' AND due_soon_notified_at IS NULL \
     AND due_on >= $1 AND due_on <= $2 ORDER BY due_on ASC, id ASC LIMIT $3"
);

/// The sweep: sent requests past due and not yet reminded.
pub(crate) const OVERDUE_SQL: &str = concat!(
    "SELECT ",
    data_access_request_columns!(),
    " FROM data_access_requests WHERE status = 'sent' AND overdue_notified_at IS NULL \
     AND due_on < $1 ORDER BY due_on ASC, id ASC LIMIT $2"
);

/// Claim the due-soon reminder; matches only while it is unclaimed.
pub(crate) const CLAIM_DUE_SOON_SQL: &str = "UPDATE data_access_requests \
     SET due_soon_notified_at = $5 \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND due_on = $4 \
     AND status = 'sent' AND due_soon_notified_at IS NULL";

/// Claim the overdue reminder; matches only while it is unclaimed.
pub(crate) const CLAIM_OVERDUE_SQL: &str = "UPDATE data_access_requests \
     SET overdue_notified_at = $5 \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND due_on = $4 \
     AND status = 'sent' AND overdue_notified_at IS NULL";

/// The last day of the due-soon window that opens on `today`.
pub(crate) fn due_soon_horizon(today: NaiveDate) -> NaiveDate {
    today
        .checked_add_days(Days::new(DUE_SOON_DAYS))
        .unwrap_or(NaiveDate::MAX)
}

/// Decode the backend-neutral columns of a request row; the user uuid arrives
/// already read through the backend's codec. `try_get` throughout, so a
/// corrupt row surfaces as a recoverable error.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// an id that is not a uuid, or a law, scope or status no write stores.
pub(crate) fn request_from_row<R>(row: &R, user_id: Uuid) -> AppResult<DataAccessRequest>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<NaiveDate>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("data_access_requests {col}: {e}"));
    let text = |col: &str| -> AppResult<String> { row.try_get(col).map_err(|e| column(col, e)) };
    let date = |col: &str| -> AppResult<Option<NaiveDate>> {
        row.try_get(col).map_err(|e| column(col, e))
    };
    let id = text("id")?;
    let tenant = text("tenant_id")?;
    let law = text("law")?;
    let scope = text("scope")?;
    let status = text("status")?;
    Ok(DataAccessRequest {
        id: Uuid::parse_str(&id)
            .map_err(|e| AppError::database(format!("data_access_requests id '{id}': {e}")))?,
        tenant_id: TenantId::parse_str(&tenant)
            .map_err(|e| AppError::database(format!("data_access_requests tenant_id: {e}")))?,
        user_id,
        reference: text("reference")?,
        controller: text("controller")?,
        controller_contact: text("controller_contact")?,
        athlete_identifier: text("athlete_identifier")?,
        law: DataAccessLaw::from_stored(&law)
            .ok_or_else(|| AppError::database(format!("Unknown data access law: {law}")))?,
        scope: DataAccessScope::from_stored(&scope)
            .ok_or_else(|| AppError::database(format!("Unknown data access scope: {scope}")))?,
        letter_locale: text("letter_locale")?,
        status: DataAccessStatus::from_stored(&status)
            .ok_or_else(|| AppError::database(format!("Unknown data access status: {status}")))?,
        sent_on: date("sent_on")?,
        due_on: date("due_on")?,
        extended_until: date("extended_until")?,
        answered_on: date("answered_on")?,
        due_soon_notified_at: row
            .try_get("due_soon_notified_at")
            .map_err(|e| column("due_soon_notified_at", e))?,
        overdue_notified_at: row
            .try_get("overdue_notified_at")
            .map_err(|e| column("overdue_notified_at", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Emit the whole [`DataAccessRequestRepository`] implementation for one
/// backend type. `$row` is the driver's row type and `$ids` the backend's
/// codec in [`super::uuid_columns`]. The body names its consts and helpers
/// unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_data_access_request_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Decode a request row through the backend's uuid codec.
            fn data_access_row(row: &$row) -> AppResult<DataAccessRequest> {
                request_from_row(row, $ids::read(row, "user_id")?)
            }
        }

        #[async_trait::async_trait]
        impl DataAccessRequestRepository for $ty {
            async fn insert(&self, request: &DataAccessRequest) -> AppResult<bool> {
                let result = sqlx::query(INSERT_SQL)
                    .bind(request.id.to_string())
                    .bind(request.tenant_id.to_string())
                    .bind($ids::bind(request.user_id))
                    .bind(&request.reference)
                    .bind(&request.controller)
                    .bind(&request.controller_contact)
                    .bind(&request.athlete_identifier)
                    .bind(request.law.as_str())
                    .bind(request.scope.as_str())
                    .bind(&request.letter_locale)
                    .bind(request.status.as_str())
                    .bind(request.sent_on)
                    .bind(request.due_on)
                    .bind(request.extended_until)
                    .bind(request.answered_on)
                    .bind(request.due_soon_notified_at)
                    .bind(request.overdue_notified_at)
                    .bind(request.created_at)
                    .bind(request.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert data access request: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn get(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
            ) -> AppResult<Option<DataAccessRequest>> {
                let row = sqlx::query(GET_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(user_id))
                    .bind(id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read data access request: {e}"))
                    })?;
                row.map(|row| Self::data_access_row(&row)).transpose()
            }

            async fn list(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Vec<DataAccessRequest>> {
                let rows = sqlx::query(LIST_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list data access requests: {e}"))
                    })?;
                rows.iter().map(Self::data_access_row).collect()
            }

            async fn update(
                &self,
                request: &DataAccessRequest,
                expected: DataAccessStatus,
                reset_reminders: bool,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_SQL)
                    .bind(request.tenant_id.to_string())
                    .bind($ids::bind(request.user_id))
                    .bind(request.id.to_string())
                    .bind(expected.as_str())
                    .bind(&request.controller)
                    .bind(&request.controller_contact)
                    .bind(&request.athlete_identifier)
                    .bind(request.law.as_str())
                    .bind(request.scope.as_str())
                    .bind(&request.letter_locale)
                    .bind(request.status.as_str())
                    .bind(request.sent_on)
                    .bind(request.due_on)
                    .bind(request.extended_until)
                    .bind(request.answered_on)
                    .bind(request.updated_at)
                    .bind(reset_reminders)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update data access request: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn due_for_reminder(
                &self,
                stage: ReminderStage,
                today: NaiveDate,
                limit: i64,
            ) -> AppResult<Vec<DataAccessRequest>> {
                let query = match stage {
                    ReminderStage::DueSoon => sqlx::query(DUE_SOON_SQL)
                        .bind(today)
                        .bind(due_soon_horizon(today))
                        .bind(limit),
                    ReminderStage::Overdue => sqlx::query(OVERDUE_SQL).bind(today).bind(limit),
                };
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to read due data access requests: {e}"))
                })?;
                rows.iter().map(Self::data_access_row).collect()
            }

            async fn claim_reminder(
                &self,
                stage: ReminderStage,
                request: &DataAccessRequest,
                due_on: NaiveDate,
                at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let statement = match stage {
                    ReminderStage::DueSoon => CLAIM_DUE_SOON_SQL,
                    ReminderStage::Overdue => CLAIM_OVERDUE_SQL,
                };
                let result = sqlx::query(statement)
                    .bind(request.tenant_id.to_string())
                    .bind($ids::bind(request.user_id))
                    .bind(request.id.to_string())
                    .bind(due_on)
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to claim a data access request reminder: {e}"
                        ))
                    })?;
                Ok(result.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_data_access_request_repository;
