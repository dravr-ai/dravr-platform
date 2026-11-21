// ABOUTME: Repository trait, statements and shared body for admin tokens, their usage audit and the provisioned-key ledger
// ABOUTME: Admin-console authentication; written once, emitted for each backend by macro with the inet cast as its one argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminAction, AdminPermissions, AdminToken, AdminTokenUsage, CreateAdminTokenRequest,
    GeneratedAdminToken, DELETED_ACCOUNT_ADMIN,
};
use pierre_core::admin::{AdminJwtManager, TokenScope};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use secrecy::SecretString;
use tracing::debug;
use uuid::Uuid;

/// What a caller settled before it asked for a token to be rotated.
///
/// The caller reads the token, decides on that row whether it may hand its
/// replacement out (the admin API holds a caller without the super-admin
/// flag to the permissions it holds itself), and passes the row it decided
/// on. [`AdminRepository::rotate_token`] mints only from the row it claims,
/// and refuses when that row's flag or permissions are not the ones checked
/// here: the replacement never carries more than the caller was held to.
#[derive(Clone, Copy)]
pub struct TokenRotation<'checked> {
    /// The token as the caller read it and checked it.
    pub checked: &'checked AdminToken,
    /// The replacement's lifetime in days from now; `0` for one that never
    /// expires.
    pub expires_in_days: u64,
    /// The admin whose session rotates, when one does: the admin a device
    /// login's replacement keeps acting as only when it is theirs
    /// ([`CreateAdminTokenRequest::rotation_of`]).
    pub rotated_by: Option<Uuid>,
}

/// Prints the checked token by its id alone. Its row holds the hash a
/// presented token is verified against and the hash of the secret it was
/// signed under, and a debug line is no place for either.
impl fmt::Debug for TokenRotation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenRotation")
            .field("checked", &self.checked.id)
            .field("expires_in_days", &self.expires_in_days)
            .field("rotated_by", &self.rotated_by)
            .finish()
    }
}

/// What became of a token someone asked to revoke
/// ([`AdminRepository::deactivate_token`]).
///
/// A rotation retires a token as a revocation does, and mints a replacement
/// that is still live: revoking the old token again ends nothing. The three
/// answers are what the admin API and `pierre-cli token revoke` tell apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenRevocation {
    /// The token was live, and this call retired it.
    Revoked,
    /// No live token has that id and none names a replacement: the token
    /// was revoked earlier, withdrawn with its admin's standing, or rotated
    /// before `admin_tokens.rotated_to_token_id` existed, which cannot be
    /// told from a revocation; or the id is no token's, which the store
    /// answers without an error. Nothing was written.
    AlreadyRevoked,
    /// A rotation retired the token and minted `replacement`, which this
    /// call left as it is. Nothing was written.
    Rotated {
        /// The id of the token the rotation minted in its place.
        replacement: String,
    },
}

/// Admin token management repository.
///
/// A permission surface, and a global one: admin tokens authenticate the
/// admin console and the CLI, so they are keyed by token id and prefix
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
    /// List all admin tokens (super admin only)
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>>;
    /// Revoke a token: retire it while it is live, in one guarded statement
    /// (`… WHERE id = $1 AND is_active = TRUE`), and say what the call
    /// found ([`TokenRevocation`]).
    ///
    /// A token that was not live is read after the statement that left it
    /// alone, so a rotation that commits between a caller's own read and
    /// this call is answered as a rotation: the caller is told the
    /// replacement instead of being told it revoked something. An id that
    /// is no token's is no error here: a caller that owes its own caller a
    /// 404 reads the token first, as the admin API and `pierre-cli` do.
    ///
    /// # Errors
    /// Returns a database error when a statement fails.
    async fn deactivate_token(&self, token_id: &str) -> AppResult<TokenRevocation>;
    /// Rotate a token: retire it and mint its replacement, in one
    /// transaction, so there is no moment in which the old token is gone
    /// and the new one is not there, and none in which both are live.
    ///
    /// The transaction claims the old row only while it is live (one
    /// `UPDATE … WHERE id = $1 AND is_active = TRUE`), writing on it the id
    /// the replacement is about to be minted under
    /// (`rotated_to_token_id`), reads it back as claimed, and builds the
    /// replacement from that row, never from
    /// `rotation.checked`, through the one definition of what a rotation
    /// carries over ([`CreateAdminTokenRequest::rotation_of`]). So a name an
    /// account deletion rewrote since the caller's read is the name the
    /// replacement gets, and a row retired since then is not claimed at
    /// all. The replacement is signed and hashed as
    /// [`Self::create_token`] signs one.
    ///
    /// A replacement is minted only at the level the caller checked: when
    /// the claimed row's super-admin flag or permissions differ from
    /// `rotation.checked`'s, the rotation is refused and the claim rolled
    /// back with everything else.
    ///
    /// # Against a concurrent account deletion
    ///
    /// The deletion retires the device logins the account approved,
    /// rewrites the account's id out of every other token name, and commits
    /// only when its own read finds no `admin_tokens` row naming the
    /// account (`super::account_deletion`). A replacement names the account
    /// exactly when the row it replaces does, so the two transactions must
    /// run one after the other, in either order:
    ///
    /// - the deletion first: the old row is retired, the claim finds it
    ///   inactive, and the rotation is refused with nothing minted; or the
    ///   old row only carried the id in its name, and the replacement takes
    ///   the rewritten name;
    /// - the rotation first: the deletion's statements find the replacement
    ///   beside the old row, and retire it (it acts as the admin) or
    ///   rename it (it carries the name).
    ///
    /// On `SQLite` both open with `BEGIN IMMEDIATE`, which takes the one
    /// write lock: the second waits for the first to commit before it reads
    /// anything.
    ///
    /// On `PostgreSQL` the claim's row lock orders the two only halfway.
    /// The rotation's claim over a row the deletion rewrote waits for the
    /// deletion to commit and decides on the row as it was left, which is
    /// the first order. But a deletion's `UPDATE` that waits on a claimed
    /// row re-checks that one row when the rotation commits, under the
    /// snapshot it started with: it retires the old row and never sees the
    /// replacement inserted beside it, and the deletion's own read then
    /// finds a row naming the account and refuses to commit. So before it
    /// claims anything the rotation takes the lock an account deletion
    /// opens with, as does every write that ends an admin's standing: the
    /// admins' `users` rows, in id order (`admin_removal_lock_sql!`). The
    /// deletion of an admin, or of any account while an admin exists, then
    /// waits at its first statement for a rotation that got there first,
    /// and reads every token after the rotation's commit; a rotation that
    /// came second waits there for the deletion's. Both take the user rows
    /// before any token row, so neither can hold what the other waits for.
    ///
    /// Where no account holds the admin role there is no row to share, and
    /// the row lock is all that orders them: a deletion that loses that
    /// race is refused by its own read and commits nothing.
    ///
    /// # Errors
    /// Returns a not-found error for an unknown token, the invalid-input
    /// error of [`CreateAdminTokenRequest::rotation_of`] for one that is not
    /// live, a conflict (`ErrorCode::ResourceLocked`) for one whose flag or
    /// permissions changed since the caller checked them, and a database
    /// error when a statement fails. Nothing is committed then: the old
    /// token is as it was.
    async fn rotate_token(
        &self,
        rotation: &TokenRotation<'_>,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken>;
    /// Record admin token usage for audit trail.
    ///
    /// The address and the user agent are the caller's, and one retirement
    /// clears them: the deletion of the account a device login acted as,
    /// which takes both out of the log it keeps. A login retired that way
    /// between a request's authentication and this write still gets its
    /// row, with what the request did and when, and without who sent it.
    /// Every other token keeps both, a revoked one included: the request in
    /// flight when a leaked token is revoked is the one whose address
    /// matters.
    ///
    /// # Against a concurrent account deletion
    ///
    /// The write and the deletion are ordered on the token's row, so no row
    /// is left with a caller under a deleted account's login whichever
    /// comes first. The deletion retires the login, then scrubs the log
    /// under it, in one transaction (`super::account_deletion`). This
    /// statement reads the token's row under a lock that waits for a
    /// retirement in flight (`admin_statements_sql!`'s `$share`):
    ///
    /// - it reaches the row first: it commits its row, caller included,
    ///   before the retirement can start, and the scrub that follows the
    ///   retirement clears it;
    /// - the retirement reaches the row first: this waits for the deletion
    ///   to end and reads the row as it was left, so a committed deletion
    ///   gets a row without a caller and a rolled-back one a row with it.
    ///
    /// On `SQLite` a writer holds the whole database, so the two never
    /// overlap and no clause is needed.
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()>;
    /// Count one authenticated use of a token on its own row: one more
    /// request, used at `used_at`, from `ip_address` when the edge named one.
    /// A deleted account's device login counts no use: its row keeps the
    /// count it had, and the address the deletion cleared stays cleared.
    async fn record_token_use(
        &self,
        token_id: &str,
        used_at: DateTime<Utc>,
        ip_address: Option<&str>,
    ) -> AppResult<()>;
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
}

/// The sixteen columns every read of `admin_tokens` returns, in the order
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
             tenant_id, operator_user_id, rotated_to_token_id, created_at, expires_at, \
             last_used_at, ",
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
                tenant_id, created_at, expires_at, usage_count, operator_user_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ";

/// Revoke a token: retire it, only while it is live. Whether a row was
/// affected is whether this statement is what retired it; a token already
/// retired, by a revocation or by a rotation, is left exactly as it is.
/// `FALSE` and `TRUE` are the boolean spellings both engines accept.
pub(crate) const DEACTIVATE_ADMIN_TOKEN_SQL: &str =
    "UPDATE admin_tokens SET is_active = FALSE WHERE id = $1 AND is_active = TRUE";

/// Claim a token for rotation: retire it, only while it is live, and name
/// on it the replacement the same transaction mints (`$2`, the new token's
/// id; `rotated_to_token_id` is `TEXT` on both engines, as `id` is).
/// Whether a row was affected is whether the rotation holds it. `AND
/// is_active` is what refuses a token revoked, or retired with its admin,
/// since the caller read it. A rotation that fails further on rolls the
/// name back with the claim.
pub(crate) const CLAIM_ADMIN_TOKEN_SQL: &str = "UPDATE admin_tokens \
     SET is_active = FALSE, rotated_to_token_id = $2 WHERE id = $1 AND is_active = TRUE";

/// Retire every live token acting as one admin, naming the ones retired:
/// the device logins an account loses with its admin standing, run by the
/// user write that ends it, in that write's transaction (`admin_standing`).
/// `operator_user_id` is text on both engines, so the id binds as a string.
pub(crate) const DEACTIVATE_ADMIN_TOKENS_SQL: &str = "UPDATE admin_tokens SET is_active = FALSE \
     WHERE operator_user_id = $1 AND is_active = TRUE RETURNING id";

/// Whether an `admin_tokens` row still takes a caller's address: every row
/// but the device login of a deleted account, which acts as no account
/// (`$deleted` is the placeholder `DELETED_ACCOUNT_ADMIN` binds to, as
/// text). The deletion cleared the address under that row and its log, and
/// a request still in flight must not write it back.
///
/// `operator_user_id` is `NULL` on a service token, and `NULL <> x` is not
/// true, so the service token is named first.
macro_rules! token_takes_a_caller_sql {
    ($deleted:literal) => {
        concat!(
            "(operator_user_id IS NULL OR operator_user_id <> ",
            $deleted,
            ")"
        )
    };
}
pub(crate) use token_takes_a_caller_sql;

/// What the `$deleted` placeholder of [`token_takes_a_caller_sql!`] binds:
/// what a deleted account's device logins act as
/// (`DELETED_ACCOUNT_ADMIN`), as the text `operator_user_id` holds on
/// both engines.
pub(crate) fn deleted_account_column() -> String {
    DELETED_ACCOUNT_ADMIN.to_string()
}

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
///
/// `$share` is the row lock the log write reads its token's row under:
/// `" FOR SHARE"` on Postgres, `""` on `SQLite`. An account deletion retires
/// a device login with an `UPDATE` of that row; a plain read would not wait
/// for it, would see the row as it was before the uncommitted retirement,
/// and would log a caller under a login the deletion then leaves scrubbed
/// of every other. `FOR SHARE` conflicts with that `UPDATE`'s lock: the
/// read waits for the deletion to end and is evaluated again on the row as
/// it was left, and a deletion that arrives second waits for the log write
/// to commit. Log writes share the lock among themselves, and each holds it
/// for its one statement. `SQLite` has one writer and no such clause.
macro_rules! admin_statements_sql {
    ($inet:literal, $ip_text:literal, $share:literal) => {
        /// One token by its id.
        const GET_ADMIN_TOKEN_BY_ID_SQL: &str = concat!(
            "SELECT ",
            admin_token_columns!($ip_text),
            " FROM admin_tokens WHERE id = $1"
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

        /// Append one audit row. `id` is assigned by the engine on both
        /// backends (`SERIAL` there, `INTEGER PRIMARY KEY AUTOINCREMENT`
        /// here) and `method` stays NULL, so neither is named. The address
        /// and the user agent are read through the token's row: a scalar
        /// subquery that finds no row taking a caller is NULL
        /// ([`token_takes_a_caller_sql!`], `$10` a deleted account's mark).
        /// Each reads the row under `$share`, so it waits for an account
        /// deletion that is retiring the token and reads what it left.
        const RECORD_ADMIN_TOKEN_USAGE_SQL: &str = concat!(
            "INSERT INTO admin_token_usage ( \
                admin_token_id, timestamp, action, target_resource, \
                ip_address, user_agent, request_size_bytes, success, \
                response_time_ms \
            ) VALUES ($1, $2, $3, $4, \
                (SELECT $5",
            $inet,
            " FROM admin_tokens WHERE id = $1 AND ",
            token_takes_a_caller_sql!("$10"),
            $share,
            "), (SELECT CAST($6 AS TEXT) FROM admin_tokens WHERE id = $1 AND ",
            token_takes_a_caller_sql!("$10"),
            $share,
            "), $7, $8, $9)"
        );

        /// One authenticated use of a token: the request count, the time
        /// and the address the token list shows. An absent address keeps
        /// the last one known. No row that takes no caller is written
        /// ([`token_takes_a_caller_sql!`], `$4` a deleted account's mark).
        /// An `UPDATE` of the row waits for a retirement in flight by
        /// itself and checks the row again as it was left.
        const RECORD_ADMIN_TOKEN_USE_SQL: &str = concat!(
            "UPDATE admin_tokens SET usage_count = usage_count + 1, last_used_at = $1, \
             last_used_ip = COALESCE($2",
            $inet,
            ", last_used_ip) WHERE id = $3 AND ",
            token_takes_a_caller_sql!("$4")
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

/// The token id shape: `admin_` plus a simple uuid.
pub(crate) fn new_admin_token_id() -> String {
    format!("admin_{}", Uuid::new_v4().simple())
}

/// A token signed for a request and not yet stored: what its holder is
/// handed, and the three values only its row keeps. It does not print:
/// two of those values are the hash a presented token is verified against
/// and the hash of the secret it was signed under.
pub(crate) struct MintedAdminToken {
    /// The token and what it carries, handed to its holder once.
    pub(crate) generated: GeneratedAdminToken,
    /// The hash the row verifies a presented token against.
    pub(crate) token_hash: String,
    /// The hash of the secret the token was minted under.
    pub(crate) jwt_secret_hash: String,
    /// The permissions as the row stores them.
    pub(crate) permissions_json: String,
}

/// Sign the token `request` asks for, under `token_id`
/// ([`new_admin_token_id`]): the one place a token is minted, for a new
/// token and for a rotation's replacement alike. The id is the caller's so
/// that a rotation can name its replacement on the row it claims before the
/// replacement is signed. Nothing is stored here; the backend inserts the
/// row.
///
/// # Errors
/// Returns an invalid-input error for a lifetime past any date, and an
/// internal error when the token cannot be signed or hashed, or its
/// permissions cannot be encoded.
pub(crate) fn mint_admin_token(
    request: &CreateAdminTokenRequest,
    token_id: String,
    admin_jwt_secret: &str,
    jwks_manager: &dyn JwtSigner,
) -> AppResult<MintedAdminToken> {
    debug!("Creating admin token with RS256 asymmetric signing");

    let permissions = request.granted_permissions();

    let expires_at = request.expires_at(Utc::now())?;

    let jwt_token = AdminJwtManager::new()
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
        .map_err(|e| AppError::internal(format!("Failed to generate JWT token: {e}")))?;

    let token_prefix = AdminJwtManager::generate_token_prefix(&jwt_token);
    let token_hash = AdminJwtManager::hash_token_for_storage(&jwt_token)
        .map_err(|e| AppError::internal(format!("Failed to hash JWT token: {e}")))?;
    let jwt_secret_hash = AdminJwtManager::hash_secret(admin_jwt_secret);
    let permissions_json = permissions.to_json()?;

    Ok(MintedAdminToken {
        generated: GeneratedAdminToken {
            token_id,
            service_name: request.service_name.clone(),
            jwt_token: SecretString::from(jwt_token),
            token_prefix,
            permissions,
            is_super_admin: request.is_super_admin,
            expires_at,
            created_at: Utc::now(),
        },
        token_hash,
        jwt_secret_hash,
        permissions_json,
    })
}

/// The request a rotation mints its replacement from: `row` is the token as
/// the rotation's transaction read it back after its claim, and `claimed`
/// whether the claim took it.
///
/// The claim clears `is_active`, so the row reads as retired either way;
/// `claimed` is what it was before. A row the claim did not take was not
/// live, and [`CreateAdminTokenRequest::rotation_of`] refuses it in its own
/// words.
///
/// # Errors
/// Returns that refusal for a row that was not live, and a conflict
/// (`ErrorCode::ResourceLocked`) when the row's super-admin flag or
/// permissions are not the ones the caller checked.
pub(crate) fn replacement_request(
    rotation: &TokenRotation<'_>,
    row: AdminToken,
    claimed: bool,
) -> AppResult<CreateAdminTokenRequest> {
    let existing = AdminToken {
        is_active: claimed,
        ..row
    };
    let as_checked = existing.is_super_admin == rotation.checked.is_super_admin
        && existing.permissions == rotation.checked.permissions;
    let token_id = existing.id.clone();
    let request = CreateAdminTokenRequest::rotation_of(
        existing,
        rotation.expires_in_days,
        rotation.rotated_by,
    )?;
    if !as_checked {
        return Err(AppError::new(
            ErrorCode::ResourceLocked,
            format!(
                "Admin token {token_id} changed while it was being rotated; nothing was rotated"
            ),
        ));
    }
    Ok(request)
}

/// Saturate a caller-supplied count into the 4-byte `INTEGER` both schemas
/// declare for it.
pub(crate) fn count_to_column(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
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
        operator_user_id: row
            .try_get::<Option<String>, _>("operator_user_id")
            .map_err(|e| {
                AppError::database(format!("Failed to get column 'operator_user_id': {e}"))
            })?
            .map(|id| Uuid::parse_str(&id))
            .transpose()
            .map_err(|e| {
                AppError::database(format!("Failed to parse column 'operator_user_id': {e}"))
            })?,
        rotated_to_token_id: row.try_get("rotated_to_token_id").map_err(|e| {
            AppError::database(format!("Failed to get column 'rotated_to_token_id': {e}"))
        })?,
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
/// `$row` is the driver's row type and `$ids` its
/// [`uuid_columns`](super::uuid_columns) codec. `$begin` is the statement a
/// rotation's transaction opens with and `$lock` the row lock it takes on
/// the admins' `users` rows before it claims a token, the two a write that
/// can remove an admin uses (see `impl_user_repository!`): `BEGIN IMMEDIATE`
/// and no clause on `SQLite`, whose write lock is taken at the `BEGIN`; a
/// plain `BEGIN` and `FOR NO KEY UPDATE` on `PostgreSQL`.
macro_rules! impl_admin_repository {
    ($ty:ty, $row:ty, $ids:ident, $begin:literal, $lock:literal) => {
        impl $ty {
            /// Store the row of the token [`mint_admin_token`] signed for
            /// `request`, on `conn`: the pool's own connection for a new
            /// token, a rotation's transaction for a replacement.
            async fn insert_admin_token(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                request: &CreateAdminTokenRequest,
                minted: &MintedAdminToken,
            ) -> AppResult<()> {
                sqlx::query(CREATE_ADMIN_TOKEN_SQL)
                    .bind(&minted.generated.token_id)
                    .bind(&request.service_name)
                    .bind(&request.service_description)
                    .bind(&minted.token_hash)
                    .bind(&minted.generated.token_prefix)
                    .bind(&minted.jwt_secret_hash)
                    .bind(&minted.permissions_json)
                    .bind(request.is_super_admin)
                    .bind(true)
                    .bind(&request.tenant_id)
                    .bind(minted.generated.created_at)
                    .bind(minted.generated.expires_at)
                    .bind(0i64)
                    .bind(request.operator_user_id.map(|id| id.to_string()))
                    .execute(conn)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create admin token: {e}"))
                    })?;
                Ok(())
            }
        }

        #[async_trait::async_trait]
        impl AdminRepository for $ty {
            async fn create_token(
                &self,
                request: &CreateAdminTokenRequest,
                admin_jwt_secret: &str,
                jwks_manager: &dyn JwtSigner,
            ) -> AppResult<GeneratedAdminToken> {
                let minted = mint_admin_token(
                    request,
                    new_admin_token_id(),
                    admin_jwt_secret,
                    jwks_manager,
                )?;
                let mut conn = self.pool().acquire().await.map_err(|e| {
                    AppError::database(format!("Failed to create admin token: {e}"))
                })?;
                Self::insert_admin_token(&mut conn, request, &minted).await?;
                Ok(minted.generated)
            }

            async fn rotate_token(
                &self,
                rotation: &TokenRotation<'_>,
                admin_jwt_secret: &str,
                jwks_manager: &dyn JwtSigner,
            ) -> AppResult<GeneratedAdminToken> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to rotate admin token: {e}"))
                };
                let token_id = rotation.checked.id.as_str();
                let tx = self.pool().begin_with($begin).await.map_err(failed)?;
                let mut guard = TransactionGuard::new(tx);

                // The lock an account deletion opens with, held before any
                // token is read. Its `$1` is the account a write is about
                // to remove; a rotation removes none, and the nil id is no
                // account's.
                sqlx::query(admin_removal_lock_sql!($lock))
                    .bind($ids::bind(Uuid::nil()))
                    .execute(guard.executor()?)
                    .await
                    .map_err(failed)?;

                // The replacement's id is chosen before the claim, which
                // writes it on the old row: a rotated token names what
                // replaced it from the statement that retires it.
                let replacement_id = new_admin_token_id();
                let claimed = sqlx::query(CLAIM_ADMIN_TOKEN_SQL)
                    .bind(token_id)
                    .bind(&replacement_id)
                    .execute(guard.executor()?)
                    .await
                    .map_err(failed)?
                    .rows_affected()
                    == 1;
                let row = sqlx::query(GET_ADMIN_TOKEN_BY_ID_SQL)
                    .bind(token_id)
                    .fetch_optional(guard.executor()?)
                    .await
                    .map_err(failed)?
                    .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))?;
                let request = replacement_request(rotation, admin_token_from_row(&row)?, claimed)?;

                let minted =
                    mint_admin_token(&request, replacement_id, admin_jwt_secret, jwks_manager)?;
                Self::insert_admin_token(guard.executor()?, &request, &minted).await?;
                guard.commit().await?;
                Ok(minted.generated)
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

            async fn deactivate_token(&self, token_id: &str) -> AppResult<TokenRevocation> {
                let revoked = sqlx::query(DEACTIVATE_ADMIN_TOKEN_SQL)
                    .bind(token_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deactivate admin token: {e}"))
                    })?
                    .rows_affected()
                    == 1;
                if revoked {
                    return Ok(TokenRevocation::Revoked);
                }
                // Not live when the statement ran: read what retired it.
                let replacement = self
                    .get_token_by_id(token_id)
                    .await?
                    .and_then(|retired| retired.rotated_to_token_id);
                Ok(
                    replacement.map_or(TokenRevocation::AlreadyRevoked, |replacement| {
                        TokenRevocation::Rotated { replacement }
                    }),
                )
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
                    .bind(deleted_account_column())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record admin token usage: {e}"))
                    })?;

                Ok(())
            }

            async fn record_token_use(
                &self,
                token_id: &str,
                used_at: DateTime<Utc>,
                ip_address: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_ADMIN_TOKEN_USE_SQL)
                    .bind(used_at)
                    .bind(ip_address)
                    .bind(token_id)
                    .bind(deleted_account_column())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record admin token use: {e}"))
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
                    .bind(normalize_email(user_email))
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
        }
    };
}
pub(crate) use impl_admin_repository;
