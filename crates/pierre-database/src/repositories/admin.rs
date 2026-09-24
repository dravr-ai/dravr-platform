// ABOUTME: Repository trait, statements and shared body for admin tokens, their usage audit and the provisioned-key ledger
// ABOUTME: Operator-console authentication; written once, emitted for each backend by macro with the inet cast as its one argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminAction, AdminPermissions, AdminToken, AdminTokenUsage, CreateAdminTokenRequest,
    GeneratedAdminToken,
};
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// Admin token management repository.
///
/// A permission surface, and a global one: admin tokens authenticate the
/// operator console and the CLI, so they are keyed by token id and prefix
/// rather than by tenant — the row itself carries the `tenant_id` claim the
/// token was minted with, and the auth service enforces it on every call.
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// Create a new admin token
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken>;
    /// Get admin token by ID
    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>>;
    /// Get admin token by prefix for fast lookup
    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>>;
    /// List all admin tokens (super admin only)
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>>;
    /// Deactivate admin token
    async fn deactivate_token(&self, token_id: &str) -> AppResult<()>;
    /// Update admin token last used timestamp
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()>;
    /// Record admin token usage for audit trail
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()>;
    /// Get admin token usage history
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>>;
    /// Record API key provisioning by admin
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()>;
    /// Get admin provisioned keys history
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<Value>>;
}

/// The fifteen columns every read of `admin_tokens` returns, in the order
/// [`admin_token_from_row`] reads them. One list, so a column added to
/// `AdminToken` reaches every read at once.
///
/// `$ip_text` is the function that renders the stored `last_used_ip` as
/// text: `host` on Postgres, whose column is `INET`, and nothing on
/// `SQLite`, whose column already is text — there the expansion is the bare
/// parenthesised column.
macro_rules! admin_token_columns {
    ($ip_text:literal) => {
        concat!(
            "id, service_name, service_description, token_hash, token_prefix, \
             jwt_secret_hash, permissions, is_super_admin, is_active, \
             tenant_id, created_at, expires_at, last_used_at, ",
            $ip_text,
            "(last_used_ip) AS last_used_ip, usage_count"
        )
    };
}
pub(crate) use admin_token_columns;

/// Mint a token row.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. Every bind here is a plain `&str`, `Option<&str>`, `bool`,
/// `DateTime<Utc>`, `Option<DateTime<Utc>>` or `i64` both drivers encode
/// alike — the boolean columns are `BOOLEAN` on Postgres and 0/1 `INTEGER`
/// on `SQLite`, and a `bool` binds and decodes as both.
pub(crate) const CREATE_ADMIN_TOKEN_SQL: &str = r"
            INSERT INTO admin_tokens (
                id, service_name, service_description, token_hash, token_prefix,
                jwt_secret_hash, permissions, is_super_admin, is_active,
                tenant_id, created_at, expires_at, usage_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ";

/// Retire a token. `FALSE` is the boolean spelling both engines accept.
pub(crate) const DEACTIVATE_ADMIN_TOKEN_SQL: &str =
    "UPDATE admin_tokens SET is_active = FALSE WHERE id = $1";

/// The statements that touch an address column, resolved once per backend.
///
/// The caller's address is the one thing the two schemas spell apart: an
/// `INET` column on Postgres, `TEXT` on `SQLite`. `$inet` is the cast a text
/// bind needs on the way in (`"::inet"` there, `""` here — a
/// `CAST(… AS INET)` would give `SQLite`'s unknown type NUMERIC affinity and
/// turn `127.0.0.1` into `127.0`), and `$ip_text` the function that renders
/// the column on the way out (`"host"` there, `""` here; see
/// [`admin_token_columns!`]). Every other statement on these tables is
/// shared verbatim. The expansion is the six `const`s the shared body
/// reads, emitted into the shell that invokes it.
macro_rules! admin_statements_sql {
    ($inet:literal, $ip_text:literal) => {
        /// One token by its id.
        const GET_ADMIN_TOKEN_BY_ID_SQL: &str = concat!(
            "SELECT ",
            admin_token_columns!($ip_text),
            " FROM admin_tokens WHERE id = $1"
        );

        /// One token by its clear prefix, the fast path on every
        /// authenticated call.
        const GET_ADMIN_TOKEN_BY_PREFIX_SQL: &str = concat!(
            "SELECT ",
            admin_token_columns!($ip_text),
            " FROM admin_tokens WHERE token_prefix = $1"
        );

        /// Every token, newest first.
        const LIST_ADMIN_TOKENS_SQL: &str = concat!(
            "SELECT ",
            admin_token_columns!($ip_text),
            " FROM admin_tokens ORDER BY created_at DESC"
        );

        /// Every live token, newest first. `TRUE` is the boolean spelling
        /// both engines accept.
        const LIST_ACTIVE_ADMIN_TOKENS_SQL: &str = concat!(
            "SELECT ",
            admin_token_columns!($ip_text),
            " FROM admin_tokens WHERE is_active = TRUE ORDER BY created_at DESC"
        );

        /// Stamp a use: the moment, the caller's address, one more on the
        /// counter.
        const TOUCH_ADMIN_TOKEN_SQL: &str = concat!(
            "UPDATE admin_tokens \
             SET last_used_at = CURRENT_TIMESTAMP, last_used_ip = $1",
            $inet,
            ", usage_count = usage_count + 1 \
             WHERE id = $2"
        );

        /// Append one audit row. `id` is assigned by the engine on both
        /// backends (`SERIAL` there, `INTEGER PRIMARY KEY AUTOINCREMENT`
        /// here) and `method` stays NULL, so neither is named.
        const RECORD_ADMIN_TOKEN_USAGE_SQL: &str = concat!(
            "INSERT INTO admin_token_usage ( \
                admin_token_id, timestamp, action, target_resource, \
                ip_address, user_agent, request_size_bytes, success, \
                response_time_ms \
            ) VALUES ($1, $2, $3, $4, $5",
            $inet,
            ", $6, $7, $8, $9)"
        );

        /// A token's audit rows inside a window, newest first. `id` is
        /// `SERIAL` (4 bytes) on Postgres and a rowid on `SQLite`; the
        /// BIGINT cast makes both decode as `i64`.
        const GET_ADMIN_TOKEN_USAGE_SQL: &str = concat!(
            "SELECT CAST(id AS BIGINT) AS id, admin_token_id, timestamp, action, target_resource, ",
            $ip_text,
            "(ip_address) AS ip_address, user_agent, request_size_bytes, success, \
             response_time_ms \
             FROM admin_token_usage \
             WHERE admin_token_id = $1 AND timestamp BETWEEN $2 AND $3 \
             ORDER BY timestamp DESC"
        );
    };
}
pub(crate) use admin_statements_sql;

/// Append one ledger row for a key an admin token provisioned.
pub(crate) const RECORD_PROVISIONED_KEY_SQL: &str = r"
            INSERT INTO admin_provisioned_keys (
                admin_token_id, api_key_id, user_email, requested_tier,
                provisioned_at, provisioned_by_service, rate_limit_requests,
                rate_limit_period, key_status
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ";

/// The columns every read of `admin_provisioned_keys` returns, in the order
/// [`provisioned_key_from_row`] reads them.
macro_rules! provisioned_key_columns {
    () => {
        "id, admin_token_id, api_key_id, user_email, requested_tier, \
         provisioned_at, provisioned_by_service, rate_limit_requests, \
         rate_limit_period, key_status, revoked_at, revoked_reason"
    };
}

/// One token's ledger rows inside a window, newest first.
pub(crate) const GET_PROVISIONED_KEYS_FOR_TOKEN_SQL: &str = concat!(
    "SELECT ",
    provisioned_key_columns!(),
    " FROM admin_provisioned_keys \
     WHERE admin_token_id = $1 AND provisioned_at BETWEEN $2 AND $3 \
     ORDER BY provisioned_at DESC"
);

/// Every ledger row inside a window, newest first.
pub(crate) const GET_ALL_PROVISIONED_KEYS_SQL: &str = concat!(
    "SELECT ",
    provisioned_key_columns!(),
    " FROM admin_provisioned_keys \
     WHERE provisioned_at BETWEEN $1 AND $2 \
     ORDER BY provisioned_at DESC"
);

/// The token id shape: `admin_` plus a simple uuid.
pub(crate) fn new_admin_token_id() -> String {
    format!("admin_{}", uuid::Uuid::new_v4().simple())
}

/// Saturate a caller-supplied count into the 4-byte `INTEGER` both schemas
/// declare for it.
pub(crate) fn count_to_column(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// Decode one `admin_provisioned_keys` row into the JSON the admin API
/// serves, via `try_get` only.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn provisioned_key_from_row<R>(row: &R) -> AppResult<Value>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get column '{name}': {e}"))
    };
    Ok(serde_json::json!({
        "id": row.try_get::<i32, _>("id").map_err(|e| col("id", e))?,
        "admin_token_id": row.try_get::<String, _>("admin_token_id").map_err(|e| col("admin_token_id", e))?,
        "api_key_id": row.try_get::<String, _>("api_key_id").map_err(|e| col("api_key_id", e))?,
        "user_email": row.try_get::<String, _>("user_email").map_err(|e| col("user_email", e))?,
        "requested_tier": row.try_get::<String, _>("requested_tier").map_err(|e| col("requested_tier", e))?,
        "provisioned_at": row.try_get::<DateTime<Utc>, _>("provisioned_at").map_err(|e| col("provisioned_at", e))?,
        "provisioned_by_service": row.try_get::<String, _>("provisioned_by_service").map_err(|e| col("provisioned_by_service", e))?,
        "rate_limit_requests": row.try_get::<i32, _>("rate_limit_requests").map_err(|e| col("rate_limit_requests", e))?,
        "rate_limit_period": row.try_get::<String, _>("rate_limit_period").map_err(|e| col("rate_limit_period", e))?,
        "key_status": row.try_get::<String, _>("key_status").map_err(|e| col("key_status", e))?,
        "revoked_at": row.try_get::<Option<DateTime<Utc>>, _>("revoked_at").map_err(|e| col("revoked_at", e))?,
        "revoked_reason": row.try_get::<Option<String>, _>("revoked_reason").map_err(|e| col("revoked_reason", e))?,
    }))
}

/// Decode one `admin_tokens` row via `try_get` only, so a corrupt row
/// surfaces as a recoverable error rather than a panic. Every column read
/// here decodes alike on both drivers once `last_used_ip` is rendered as
/// text.
///
/// # Errors
/// Returns error if required fields are missing or have invalid types.
pub(crate) fn admin_token_from_row<R>(row: &R) -> AppResult<AdminToken>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let permissions_json: String = row
        .try_get("permissions")
        .map_err(|e| AppError::database(format!("Failed to get column 'permissions': {e}")))?;
    let permissions = AdminPermissions::from_json(&permissions_json)?;

    Ok(AdminToken {
        id: row
            .try_get("id")
            .map_err(|e| AppError::database(format!("Failed to get column 'id': {e}")))?,
        service_name: row
            .try_get("service_name")
            .map_err(|e| AppError::database(format!("Failed to get column 'service_name': {e}")))?,
        service_description: row.try_get("service_description").map_err(|e| {
            AppError::database(format!("Failed to get column 'service_description': {e}"))
        })?,
        token_hash: row
            .try_get("token_hash")
            .map_err(|e| AppError::database(format!("Failed to get column 'token_hash': {e}")))?,
        token_prefix: row
            .try_get("token_prefix")
            .map_err(|e| AppError::database(format!("Failed to get column 'token_prefix': {e}")))?,
        jwt_secret_hash: row.try_get("jwt_secret_hash").map_err(|e| {
            AppError::database(format!("Failed to get column 'jwt_secret_hash': {e}"))
        })?,
        permissions,
        is_super_admin: row.try_get("is_super_admin").map_err(|e| {
            AppError::database(format!("Failed to get column 'is_super_admin': {e}"))
        })?,
        is_active: row
            .try_get("is_active")
            .map_err(|e| AppError::database(format!("Failed to get column 'is_active': {e}")))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("Failed to get column 'tenant_id': {e}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("Failed to get column 'created_at': {e}")))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| AppError::database(format!("Failed to get column 'expires_at': {e}")))?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(|e| AppError::database(format!("Failed to get column 'last_used_at': {e}")))?,
        last_used_ip: row
            .try_get("last_used_ip")
            .map_err(|e| AppError::database(format!("Failed to get column 'last_used_ip': {e}")))?,
        #[allow(clippy::cast_sign_loss)]
        usage_count: u64::try_from(
            row.try_get::<i64, _>("usage_count")
                .map_err(|e| {
                    AppError::database(format!("Failed to get column 'usage_count': {e}"))
                })?
                .max(0),
        )
        .unwrap_or(0),
    })
}

/// Decode one `admin_token_usage` row via `try_get` only. `id` is selected
/// as a BIGINT and `ip_address` rendered as text so both drivers hand back
/// the same types.
///
/// # Errors
/// Returns error if required fields are missing or have invalid types.
pub(crate) fn admin_token_usage_from_row<R>(row: &R) -> AppResult<AdminTokenUsage>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let action_str: String = row
        .try_get("action")
        .map_err(|e| AppError::database(format!("Failed to get column 'action': {e}")))?;
    let action = action_str
        .parse::<AdminAction>()
        .unwrap_or(AdminAction::ProvisionKey);

    Ok(AdminTokenUsage {
        id: Some(
            row.try_get::<i64, _>("id")
                .map_err(|e| AppError::database(format!("Failed to get column 'id': {e}")))?,
        ),
        admin_token_id: row.try_get("admin_token_id").map_err(|e| {
            AppError::database(format!("Failed to get column 'admin_token_id': {e}"))
        })?,
        timestamp: row
            .try_get("timestamp")
            .map_err(|e| AppError::database(format!("Failed to get column 'timestamp': {e}")))?,
        action,
        target_resource: row.try_get("target_resource").map_err(|e| {
            AppError::database(format!("Failed to get column 'target_resource': {e}"))
        })?,
        ip_address: row
            .try_get("ip_address")
            .map_err(|e| AppError::database(format!("Failed to get column 'ip_address': {e}")))?,
        user_agent: row
            .try_get("user_agent")
            .map_err(|e| AppError::database(format!("Failed to get column 'user_agent': {e}")))?,
        #[allow(clippy::cast_sign_loss)]
        request_size_bytes: row
            .try_get::<Option<i32>, _>("request_size_bytes")
            .map_err(|e| {
                AppError::database(format!("Failed to get column 'request_size_bytes': {e}"))
            })?
            .map(|v| u32::try_from(v.max(0)).unwrap_or(0)),
        success: row
            .try_get("success")
            .map_err(|e| AppError::database(format!("Failed to get column 'success': {e}")))?,
        error_message: None,
        #[allow(clippy::cast_sign_loss)]
        response_time_ms: row
            .try_get::<Option<i32>, _>("response_time_ms")
            .map_err(|e| {
                AppError::database(format!("Failed to get column 'response_time_ms': {e}"))
            })?
            .map(|v| u32::try_from(v.max(0)).unwrap_or(0)),
    })
}

/// Emit the whole [`AdminRepository`] implementation for one backend type.
/// The body is written once here; each backend's shell invokes it with its
/// own type after expanding [`admin_statements_sql!`] with its address
/// spellings, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_admin_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl AdminRepository for $ty {
            async fn create_token(
                &self,
                request: &CreateAdminTokenRequest,
                admin_jwt_secret: &str,
                jwks_manager: &dyn JwtSigner,
            ) -> AppResult<GeneratedAdminToken> {
                let token_id = new_admin_token_id();

                debug!("Creating admin token with RS256 asymmetric signing");

                let jwt_manager = AdminJwtManager::new();

                let permissions = request.permissions.as_ref().map_or_else(
                    || {
                        if request.is_super_admin {
                            AdminPermissions::super_admin()
                        } else {
                            AdminPermissions::default_admin()
                        }
                    },
                    |perms| AdminPermissions::new(perms.clone()),
                );

                // 0 days means never expires.
                let expires_at = request.expires_in_days.and_then(|days| {
                    (days != 0)
                        .then(|| Utc::now() + Duration::days(i64::try_from(days).unwrap_or(365)))
                });

                let jwt_token = jwt_manager
                    .generate_token(
                        &token_id,
                        &request.service_name,
                        &permissions,
                        &TokenScope {
                            is_super_admin: request.is_super_admin,
                            expires_at,
                            tenant_id: request.tenant_id.as_deref(),
                        },
                        jwks_manager,
                    )
                    .map_err(|e| {
                        AppError::internal(format!("Failed to generate JWT token: {e}"))
                    })?;

                let token_prefix = AdminJwtManager::generate_token_prefix(&jwt_token);
                let token_hash = AdminJwtManager::hash_token_for_storage(&jwt_token)
                    .map_err(|e| AppError::internal(format!("Failed to hash JWT token: {e}")))?;
                let jwt_secret_hash = AdminJwtManager::hash_secret(admin_jwt_secret);

                let permissions_json = permissions.to_json()?;
                let created_at = Utc::now();

                sqlx::query(CREATE_ADMIN_TOKEN_SQL)
                    .bind(&token_id)
                    .bind(&request.service_name)
                    .bind(&request.service_description)
                    .bind(&token_hash)
                    .bind(&token_prefix)
                    .bind(&jwt_secret_hash)
                    .bind(&permissions_json)
                    .bind(request.is_super_admin)
                    .bind(true)
                    .bind(&request.tenant_id)
                    .bind(created_at)
                    .bind(expires_at)
                    .bind(0i64)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create admin token: {e}"))
                    })?;

                Ok(GeneratedAdminToken {
                    token_id,
                    service_name: request.service_name.clone(),
                    jwt_token,
                    token_prefix,
                    permissions,
                    is_super_admin: request.is_super_admin,
                    expires_at,
                    created_at,
                })
            }

            async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>> {
                let row = sqlx::query(GET_ADMIN_TOKEN_BY_ID_SQL)
                    .bind(token_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get admin token by ID: {e}"))
                    })?;

                row.as_ref().map(admin_token_from_row).transpose()
            }

            async fn get_token_by_prefix(
                &self,
                token_prefix: &str,
            ) -> AppResult<Option<AdminToken>> {
                let row = sqlx::query(GET_ADMIN_TOKEN_BY_PREFIX_SQL)
                    .bind(token_prefix)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get admin token by prefix: {e}"))
                    })?;

                row.as_ref().map(admin_token_from_row).transpose()
            }

            async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>> {
                let query = if include_inactive {
                    LIST_ADMIN_TOKENS_SQL
                } else {
                    LIST_ACTIVE_ADMIN_TOKENS_SQL
                };

                let rows = sqlx::query(query)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list admin tokens: {e}")))?;

                rows.iter().map(admin_token_from_row).collect()
            }

            async fn deactivate_token(&self, token_id: &str) -> AppResult<()> {
                sqlx::query(DEACTIVATE_ADMIN_TOKEN_SQL)
                    .bind(token_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deactivate admin token: {e}"))
                    })?;

                Ok(())
            }

            async fn update_token_last_used(
                &self,
                token_id: &str,
                ip_address: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(TOUCH_ADMIN_TOKEN_SQL)
                    .bind(ip_address)
                    .bind(token_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update admin token last used: {e}"))
                    })?;

                Ok(())
            }

            async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()> {
                sqlx::query(RECORD_ADMIN_TOKEN_USAGE_SQL)
                    .bind(&usage.admin_token_id)
                    .bind(usage.timestamp)
                    .bind(usage.action.to_string())
                    .bind(&usage.target_resource)
                    .bind(&usage.ip_address)
                    .bind(&usage.user_agent)
                    .bind(usage.request_size_bytes.map(count_to_column))
                    .bind(usage.success)
                    .bind(usage.response_time_ms.map(count_to_column))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record admin token usage: {e}"))
                    })?;

                Ok(())
            }

            async fn get_token_usage_history(
                &self,
                token_id: &str,
                start_date: DateTime<Utc>,
                end_date: DateTime<Utc>,
            ) -> AppResult<Vec<AdminTokenUsage>> {
                let rows = sqlx::query(GET_ADMIN_TOKEN_USAGE_SQL)
                    .bind(token_id)
                    .bind(start_date)
                    .bind(end_date)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get admin token usage history: {e}"))
                    })?;

                rows.iter().map(admin_token_usage_from_row).collect()
            }

            async fn record_provisioned_key(
                &self,
                admin_token_id: &str,
                api_key_id: &str,
                user_email: &str,
                tier: &str,
                rate_limit_requests: u32,
                rate_limit_period: &str,
            ) -> AppResult<()> {
                let service_name = self
                    .get_token_by_id(admin_token_id)
                    .await?
                    .map_or_else(|| "unknown".to_owned(), |token| token.service_name);

                sqlx::query(RECORD_PROVISIONED_KEY_SQL)
                    .bind(admin_token_id)
                    .bind(api_key_id)
                    .bind(user_email)
                    .bind(tier)
                    .bind(Utc::now())
                    .bind(service_name)
                    .bind(count_to_column(rate_limit_requests))
                    .bind(rate_limit_period)
                    .bind("active")
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record admin provisioned key: {e}"))
                    })?;

                Ok(())
            }

            async fn get_provisioned_keys(
                &self,
                admin_token_id: Option<&str>,
                start_date: DateTime<Utc>,
                end_date: DateTime<Utc>,
            ) -> AppResult<Vec<Value>> {
                let rows = if let Some(token_id) = admin_token_id {
                    sqlx::query(GET_PROVISIONED_KEYS_FOR_TOKEN_SQL)
                        .bind(token_id)
                        .bind(start_date)
                        .bind(end_date)
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to get admin provisioned keys: {e}"))
                        })?
                } else {
                    sqlx::query(GET_ALL_PROVISIONED_KEYS_SQL)
                        .bind(start_date)
                        .bind(end_date)
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to get all admin provisioned keys: {e}"
                            ))
                        })?
                };

                rows.iter().map(provisioned_key_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_admin_repository;
