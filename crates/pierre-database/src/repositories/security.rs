// ABOUTME: Repository trait, statements and shared body for the RSA signing keypair and the system secrets
// ABOUTME: One SQL text per operation; the two backends share every bind, so the shell passes only its type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The security store, written once.
//!
//! Two tables: `rsa_keypairs`, holding the keypair that signs user sessions
//! with its private half enveloped under AES-256-GCM (bound by AAD to the
//! key id) and its public half in clear for JWKS; and `system_secrets`, the
//! name → value store the admin JWT secret and the wrapped data-encryption
//! keys live in. Rows written before the envelope existed read as plaintext
//! rather than failing, so an upgrade cannot lock every session out, and the
//! key-rotation sweep seals them.
//!
//! Nothing here differs per backend once the binds are chosen: `$n`
//! placeholders serve both drivers; timestamps bind as [`DateTime<Utc>`]
//! (`TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`, which orders
//! correctly under `ORDER BY created_at DESC` because every value shares one
//! offset and width); `is_active` binds and reads as `bool` (`BOOLEAN` on
//! Postgres, `INTEGER` 0/1 on `SQLite`); `key_size_bits` binds as the `i32`
//! the trait carries. The encryption calls go through
//! [`HasEncryption`](crate::backends::shared::encryption::HasEncryption),
//! which both backends implement.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

use crate::backends::shared::key_material::StoredSecretKind;
use crate::backends::shared::keyring::SealDomain;

/// Security and secret repository
#[async_trait]
pub trait SecurityRepository: Send + Sync {
    /// Save an RSA keypair under its key id, insert-if-absent
    ///
    /// A row that already carries `kid` is never replaced: key material that
    /// signed live sessions must not change under them (carnet#696). A caller
    /// that raced another instance re-reads with [`Self::load_rsa_keypairs`]
    /// and adopts whatever is stored.
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()>;
    /// Load all RSA keypairs from database
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>>;
    /// Get or create system secret (generates if not exists)
    ///
    /// Only a definite not-found mints a value, and the mint is
    /// insert-if-absent followed by a re-read, so racing callers converge on
    /// one stored value. Any other read error is returned.
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Get existing system secret
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ResourceNotFound`](pierre_core::errors::ErrorCode::ResourceNotFound)
    /// when no row carries `secret_type`, and a database error for every
    /// other failure, so a caller can tell an absent secret from a failed read.
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Store `value` under `secret_type` only when no row carries that name,
    /// then return the value the store holds — the caller's when it was
    /// absent, the existing one otherwise. Never replaces a stored secret.
    async fn insert_system_secret_if_absent(
        &self,
        secret_type: &str,
        value: &str,
    ) -> AppResult<String>;
    /// Update system secret (for rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()>;
    /// Count the rows that hold ciphertext sealed under a Database Encryption
    /// Key, across every table that stores one.
    ///
    /// Key management asks before it mints a key: a fresh key on a database
    /// that already holds ciphertext cannot open any of it (carnet#703).
    async fn count_encrypted_rows(&self) -> AppResult<i64>;
    /// Replace the secret `secret_type` with `new_value` only while it still
    /// holds `expected`; whether it did. The keyset document is written only
    /// this way, so two writers never lose each other's update.
    async fn compare_and_swap_system_secret(
        &self,
        secret_type: &str,
        expected: &str,
        new_value: &str,
    ) -> AppResult<bool>;
    /// Delete the secret `secret_type`; deleting an absent one succeeds.
    async fn delete_system_secret(&self, secret_type: &str) -> AppResult<()>;
    /// Write the keyset document and delete the legacy active-version secret
    /// in one transaction: the adoption of a legacy key layout. `false` when
    /// a keyset document already exists, and then nothing is written.
    async fn adopt_keyset(
        &self,
        keyset_secret: &str,
        document: &str,
        legacy_active_version_secret: &str,
    ) -> AppResult<bool>;
    /// Up to `limit` rows of `kind`, in key order after `after`, whose stored
    /// value is not sealed under data version `primary`: what the rotation
    /// sweep re-seals.
    async fn sealed_rows_not_under(
        &self,
        kind: StoredSecretKind,
        primary: u32,
        after: &str,
        limit: i64,
    ) -> AppResult<Vec<SealedRow>>;
    /// Replace one row's stored value with `new_value` only while it still
    /// holds `old_value`, so a concurrent write wins; whether it did.
    async fn reseal_row(
        &self,
        kind: StoredSecretKind,
        row_key: &str,
        old_value: &str,
        new_value: &str,
    ) -> AppResult<bool>;
    /// How many rows of `kind` hold a value sealed under data `version`
    /// (for version 1, untagged values too).
    async fn count_sealed_under(&self, kind: StoredSecretKind, version: u32) -> AppResult<u64>;
    /// How many refresh-token rows unexpired at `now`, of any status, in
    /// both refresh-token tables, carry a hash under index `version`.
    async fn count_live_index_hashes(&self, version: u32, now: DateTime<Utc>) -> AppResult<u64>;
    /// Record one key lifecycle transition in `audit_events` (global: no
    /// tenant, no user), with its family, version and states in `metadata`.
    async fn record_key_lifecycle(
        &self,
        message: &str,
        metadata: serde_json::Value,
        at: DateTime<Utc>,
    ) -> AppResult<()>;
    /// The latest `limit` key lifecycle transitions, newest first.
    async fn list_key_lifecycle(&self, limit: i64) -> AppResult<Vec<KeyLifecycleEvent>>;
    /// Seal `data` for `domain` (see
    /// [`HasEncryption::encrypt_data_with_aad`](crate::backends::shared::encryption::HasEncryption::encrypt_data_with_aad)).
    ///
    /// # Errors
    /// Returns an error if encryption fails or the keyring is stale
    fn encrypt_data_with_aad(&self, domain: SealDomain, data: &str, aad: &str)
        -> AppResult<String>;
    /// Open a value of `domain` (see
    /// [`HasEncryption::decrypt_data_with_aad`](crate::backends::shared::encryption::HasEncryption::decrypt_data_with_aad)).
    ///
    /// # Errors
    /// Returns an error if decryption fails (invalid data, AAD mismatch, tampered data)
    fn decrypt_data_with_aad(
        &self,
        domain: SealDomain,
        encrypted: &str,
        aad: &str,
    ) -> AppResult<String>;
}

/// One key lifecycle transition as `audit_events` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyLifecycleEvent {
    /// When it happened, by the writer's clock.
    pub at: DateTime<Utc>,
    /// What happened, e.g. "data v2 promoted".
    pub message: String,
    /// The family, version and from/to states.
    pub metadata: serde_json::Value,
}

/// One stored secret as the rotation sweep reads it: the row's key, the
/// columns its AAD is built from (see [`StoredSecretKind::row_aad`]), and the
/// stored value.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedRow {
    /// The row's unique key, as text.
    pub row_key: String,
    /// Up to three AAD columns, as text, in the order the kind's AAD takes.
    pub parts: [Option<String>; 3],
    /// The stored value: ciphertext, or a legacy plaintext private key.
    pub sealed: String,
}

impl fmt::Debug for SealedRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedRow")
            .field("row_key", &self.row_key)
            .field("sealed", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Save the keypair under its key id; a row that already carries that id is
/// left untouched (carnet#696).
pub(crate) const SAVE_RSA_KEYPAIR_SQL: &str = r"
            INSERT INTO rsa_keypairs (kid, private_key_pem, public_key_pem, created_at, is_active, key_size_bits)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(kid) DO NOTHING
            ";

/// Every stored keypair, newest first; the key id breaks a timestamp tie so
/// every instance loads the same order and settles on the same active key.
pub(crate) const LOAD_RSA_KEYPAIRS_SQL: &str = "SELECT kid, private_key_pem, public_key_pem, created_at, is_active FROM rsa_keypairs ORDER BY created_at DESC, kid DESC";

/// Create a system secret unless a row already carries the name; an existing
/// secret is never replaced, so racing creators converge on the first write.
pub(crate) const INSERT_SYSTEM_SECRET_IF_ABSENT_SQL: &str =
    "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO NOTHING";

/// The value stored under a secret name.
pub(crate) const GET_SYSTEM_SECRET_SQL: &str =
    "SELECT secret_value FROM system_secrets WHERE secret_type = $1";

/// Store or rotate a system secret: insert when the name is new, otherwise
/// replace the value and stamp `updated_at`.
pub(crate) const UPSERT_SYSTEM_SECRET_SQL: &str = "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO UPDATE SET secret_value = EXCLUDED.secret_value, updated_at = EXCLUDED.updated_at";

/// Rows holding DEK ciphertext, summed over every table that stores one.
///
/// A deployment-wide count with no tenant or user key on purpose: the DEK
/// and the JWT signing keypair are global, so whether any tenant's row holds
/// ciphertext is the question. Only a number leaves the query.
pub(crate) const COUNT_ENCRYPTED_ROWS_SQL: &str = r"
            SELECT (SELECT COUNT(*) FROM user_oauth_tokens)
                 + (SELECT COUNT(*) FROM rsa_keypairs)
                 + (SELECT COUNT(*) FROM tenant_oauth_credentials)
                 + (SELECT COUNT(*) FROM strava_oauth_app_pool)
                 + (SELECT COUNT(*) FROM user_llm_credentials) AS encrypted_rows
            ";

/// Replace a system secret only while it holds the expected value.
pub(crate) const COMPARE_AND_SWAP_SYSTEM_SECRET_SQL: &str = "UPDATE system_secrets \
             SET secret_value = $3, updated_at = $4 \
             WHERE secret_type = $1 AND secret_value = $2";

/// Delete a system secret by name.
pub(crate) const DELETE_SYSTEM_SECRET_SQL: &str =
    "DELETE FROM system_secrets WHERE secret_type = $1";

/// The sweep's statements for one [`StoredSecretKind`]: a page of rows not
/// sealed under the primary (`$1` the primary's `v{N}:%` pattern, `$2` the
/// key to start after, `$3` the page size), the compare-and-set re-seal
/// (`$1` new value, `$2` key, `$3` old value) and the count under one
/// version (`$1` its `v{N}:%` pattern, `$2` whether untagged values count).
/// Every key and AAD column reads back as text on both drivers.
pub(crate) struct SweepSql {
    pub(crate) select: &'static str,
    pub(crate) reseal: &'static str,
    pub(crate) count: &'static str,
}

/// The sweep statements of `kind`.
pub(crate) const fn sweep_sql(kind: StoredSecretKind) -> SweepSql {
    match kind {
        StoredSecretKind::OAuthAccessToken => SweepSql {
            select: "SELECT id AS row_key, tenant_id AS p1, CAST(user_id AS TEXT) AS p2, \
                     provider AS p3, access_token AS sealed FROM user_oauth_tokens \
                     WHERE access_token NOT LIKE $1 AND id > $2 ORDER BY id LIMIT $3",
            reseal: "UPDATE user_oauth_tokens SET access_token = $1 \
                     WHERE id = $2 AND access_token = $3",
            count: "SELECT COUNT(*) AS n FROM user_oauth_tokens \
                    WHERE access_token LIKE $1 OR ($2 AND access_token NOT LIKE '%:%')",
        },
        StoredSecretKind::OAuthRefreshToken => SweepSql {
            select: "SELECT id AS row_key, tenant_id AS p1, CAST(user_id AS TEXT) AS p2, \
                     provider AS p3, refresh_token AS sealed FROM user_oauth_tokens \
                     WHERE refresh_token IS NOT NULL AND refresh_token NOT LIKE $1 AND id > $2 \
                     ORDER BY id LIMIT $3",
            reseal: "UPDATE user_oauth_tokens SET refresh_token = $1 \
                     WHERE id = $2 AND refresh_token = $3",
            count: "SELECT COUNT(*) AS n FROM user_oauth_tokens WHERE refresh_token IS NOT NULL \
                    AND (refresh_token LIKE $1 OR ($2 AND refresh_token NOT LIKE '%:%'))",
        },
        StoredSecretKind::RsaPrivateKey => SweepSql {
            select: "SELECT kid AS row_key, CAST(NULL AS TEXT) AS p1, CAST(NULL AS TEXT) AS p2, \
                     CAST(NULL AS TEXT) AS p3, private_key_pem AS sealed FROM rsa_keypairs \
                     WHERE private_key_pem NOT LIKE $1 AND kid > $2 ORDER BY kid LIMIT $3",
            reseal: "UPDATE rsa_keypairs SET private_key_pem = $1 \
                     WHERE kid = $2 AND private_key_pem = $3",
            count: "SELECT COUNT(*) AS n FROM rsa_keypairs \
                    WHERE private_key_pem LIKE $1 OR ($2 AND private_key_pem NOT LIKE '%:%')",
        },
        StoredSecretKind::TenantOAuthClientSecret => SweepSql {
            select: "SELECT CAST(id AS TEXT) AS row_key, CAST(tenant_id AS TEXT) AS p1, \
                     provider AS p2, CAST(NULL AS TEXT) AS p3, client_secret_encrypted AS sealed \
                     FROM tenant_oauth_credentials \
                     WHERE client_secret_encrypted NOT LIKE $1 AND CAST(id AS TEXT) > $2 \
                     ORDER BY CAST(id AS TEXT) LIMIT $3",
            reseal: "UPDATE tenant_oauth_credentials SET client_secret_encrypted = $1 \
                     WHERE CAST(id AS TEXT) = $2 AND client_secret_encrypted = $3",
            count: "SELECT COUNT(*) AS n FROM tenant_oauth_credentials \
                    WHERE client_secret_encrypted LIKE $1 \
                    OR ($2 AND client_secret_encrypted NOT LIKE '%:%')",
        },
        StoredSecretKind::StravaPoolClientSecret => SweepSql {
            select: "SELECT client_id AS row_key, CAST(NULL AS TEXT) AS p1, \
                     CAST(NULL AS TEXT) AS p2, CAST(NULL AS TEXT) AS p3, \
                     client_secret_encrypted AS sealed FROM strava_oauth_app_pool \
                     WHERE client_secret_encrypted NOT LIKE $1 AND client_id > $2 \
                     ORDER BY client_id LIMIT $3",
            reseal: "UPDATE strava_oauth_app_pool SET client_secret_encrypted = $1 \
                     WHERE client_id = $2 AND client_secret_encrypted = $3",
            count: "SELECT COUNT(*) AS n FROM strava_oauth_app_pool \
                    WHERE client_secret_encrypted LIKE $1 \
                    OR ($2 AND client_secret_encrypted NOT LIKE '%:%')",
        },
        StoredSecretKind::LlmApiKey => SweepSql {
            select: "SELECT id AS row_key, tenant_id AS p1, CAST(user_id AS TEXT) AS p2, \
                     provider AS p3, api_key_encrypted AS sealed FROM user_llm_credentials \
                     WHERE api_key_encrypted NOT LIKE $1 AND id > $2 ORDER BY id LIMIT $3",
            reseal: "UPDATE user_llm_credentials SET api_key_encrypted = $1 \
                     WHERE id = $2 AND api_key_encrypted = $3",
            count: "SELECT COUNT(*) AS n FROM user_llm_credentials \
                    WHERE api_key_encrypted LIKE $1 OR ($2 AND api_key_encrypted NOT LIKE '%:%')",
        },
    }
}

/// Refresh-token rows unexpired at `$1`, of any status, in both tables,
/// whose hash is tagged `$2` (`k{N}:%`), or untagged when `$3`.
pub(crate) const COUNT_LIVE_INDEX_HASHES_SQL: &str = r"
            SELECT (SELECT COUNT(*) FROM oauth2_refresh_tokens
                     WHERE expires_at > $1 AND (token LIKE $2 OR ($3 AND token NOT LIKE '%:%')))
                 + (SELECT COUNT(*) FROM session_refresh_tokens
                     WHERE expires_at > $1
                       AND (token_hash LIKE $2 OR ($3 AND token_hash NOT LIKE '%:%'))) AS n
            ";

/// A key lifecycle transition: global, so tenant and user stay NULL.
pub(crate) const RECORD_KEY_LIFECYCLE_SQL: &str = "INSERT INTO audit_events \
             (id, event_type, severity, message, source, result, tenant_id, user_id, metadata, timestamp) \
             VALUES ($1, 'key_lifecycle', 'info', $2, 'keyset', 'success', NULL, NULL, $3, $4)";

/// The latest key lifecycle transitions, which are global (no tenant).
pub(crate) const LIST_KEY_LIFECYCLE_SQL: &str =
    "SELECT message, metadata, timestamp FROM audit_events \
             WHERE tenant_id IS NULL AND event_type = 'key_lifecycle' \
             ORDER BY timestamp DESC, id DESC LIMIT $1";

/// The `LIKE` pattern of values sealed under data `version`.
pub(crate) fn data_tag_pattern(version: u32) -> String {
    format!("v{version}:%")
}

/// The `LIKE` pattern of hashes computed under index `version`.
pub(crate) fn index_tag_pattern(version: u32) -> String {
    format!("k{version}:%")
}

/// Emit the whole [`SecurityRepository`] implementation for one backend
/// type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_security_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl SecurityRepository for $ty {
            async fn save_rsa_keypair(
                &self,
                kid: &str,
                private_key_pem: &str,
                public_key_pem: &str,
                created_at: DateTime<Utc>,
                is_active: bool,
                key_size_bits: i32,
            ) -> AppResult<()> {
                let encrypted_private_key = encrypt_rsa_private_key(self, kid, private_key_pem)?;

                sqlx::query(SAVE_RSA_KEYPAIR_SQL)
                    .bind(kid)
                    .bind(&encrypted_private_key)
                    .bind(public_key_pem)
                    .bind(created_at)
                    .bind(is_active)
                    .bind(key_size_bits)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                Ok(())
            }

            async fn load_rsa_keypairs(
                &self,
            ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>> {
                let rows = sqlx::query(LOAD_RSA_KEYPAIRS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        $crate::backends::shared::key_material::key_store_query_error(
                            "Database query failed",
                            &e,
                        )
                    })?;

                let mut keypairs = Vec::with_capacity(rows.len());
                for row in rows {
                    let kid: String = row
                        .try_get("kid")
                        .map_err(|e| AppError::database(format!("Failed to get kid: {e}")))?;
                    let stored_private_key: String =
                        row.try_get("private_key_pem").map_err(|e| {
                            AppError::database(format!("Failed to get private_key_pem: {e}"))
                        })?;
                    // A row written before the column carried ciphertext reads
                    // as it is; the rotation sweep seals it.
                    let private_key_pem = if is_plaintext_private_key_pem(&stored_private_key) {
                        stored_private_key
                    } else {
                        decrypt_rsa_private_key(self, &kid, &stored_private_key)?
                    };
                    let public_key_pem: String = row.try_get("public_key_pem").map_err(|e| {
                        AppError::database(format!("Failed to get public_key_pem: {e}"))
                    })?;
                    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|e| {
                        AppError::database(format!("Failed to get created_at: {e}"))
                    })?;
                    let is_active: bool = row
                        .try_get("is_active")
                        .map_err(|e| AppError::database(format!("Failed to get is_active: {e}")))?;

                    keypairs.push((kid, private_key_pem, public_key_pem, created_at, is_active));
                }

                Ok(keypairs)
            }

            async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
                match self.get_system_secret(secret_type).await {
                    Ok(secret) => return Ok(secret),
                    Err(e) if e.code == ErrorCode::ResourceNotFound => {}
                    Err(e) => return Err(e),
                }

                // Only the admin JWT secret is minted here; the data-encryption
                // keys are wrapped and stored by key management under their own
                // names through insert_system_secret_if_absent.
                let secret_value = match secret_type {
                    "admin_jwt_secret" => AdminJwtManager::generate_jwt_secret(),
                    _ => {
                        return Err(AppError::invalid_input(format!(
                            "Unknown secret type: {secret_type}"
                        )))
                    }
                };

                self.insert_system_secret_if_absent(secret_type, &secret_value)
                    .await
            }

            async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
                let row = sqlx::query(GET_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        $crate::backends::shared::key_material::key_store_query_error(
                            "Database query failed",
                            &e,
                        )
                    })?
                    .ok_or_else(|| AppError::not_found(format!("System secret '{secret_type}'")))?;

                row.try_get("secret_value")
                    .map_err(|e| AppError::database(format!("Failed to get secret_value: {e}")))
            }

            async fn insert_system_secret_if_absent(
                &self,
                secret_type: &str,
                value: &str,
            ) -> AppResult<String> {
                let now = Utc::now();
                sqlx::query(INSERT_SYSTEM_SECRET_IF_ABSENT_SQL)
                    .bind(secret_type)
                    .bind(value)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                self.get_system_secret(secret_type).await
            }

            async fn update_system_secret(
                &self,
                secret_type: &str,
                new_value: &str,
            ) -> AppResult<()> {
                let now = Utc::now();
                sqlx::query(UPSERT_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .bind(new_value)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                Ok(())
            }

            async fn count_encrypted_rows(&self) -> AppResult<i64> {
                let row = sqlx::query(COUNT_ENCRYPTED_ROWS_SQL)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        $crate::backends::shared::key_material::key_store_query_error(
                            "Failed to count encrypted rows",
                            &e,
                        )
                    })?;
                row.try_get("encrypted_rows")
                    .map_err(|e| AppError::database(format!("Failed to get encrypted_rows: {e}")))
            }

            async fn compare_and_swap_system_secret(
                &self,
                secret_type: &str,
                expected: &str,
                new_value: &str,
            ) -> AppResult<bool> {
                let done = sqlx::query(COMPARE_AND_SWAP_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .bind(expected)
                    .bind(new_value)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;
                Ok(done.rows_affected() == 1)
            }

            async fn delete_system_secret(&self, secret_type: &str) -> AppResult<()> {
                sqlx::query(DELETE_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;
                Ok(())
            }

            async fn adopt_keyset(
                &self,
                keyset_secret: &str,
                document: &str,
                legacy_active_version_secret: &str,
            ) -> AppResult<bool> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("Failed to adopt the keyset: {e}"));
                let now = Utc::now();
                let mut tx = self.pool().begin().await.map_err(failed)?;
                let inserted = sqlx::query(INSERT_SYSTEM_SECRET_IF_ABSENT_SQL)
                    .bind(keyset_secret)
                    .bind(document)
                    .bind(now)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                if inserted.rows_affected() != 1 {
                    tx.rollback().await.map_err(failed)?;
                    return Ok(false);
                }
                sqlx::query(DELETE_SYSTEM_SECRET_SQL)
                    .bind(legacy_active_version_secret)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(true)
            }

            async fn sealed_rows_not_under(
                &self,
                kind: StoredSecretKind,
                primary: u32,
                after: &str,
                limit: i64,
            ) -> AppResult<Vec<SealedRow>> {
                let rows = sqlx::query(sweep_sql(kind).select)
                    .bind(data_tag_pattern(primary))
                    .bind(after)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read {} rows to re-seal: {e}",
                            kind.as_str()
                        ))
                    })?;
                rows.iter()
                    .map(|row| {
                        let text = |name: &str| -> AppResult<Option<String>> {
                            row.try_get(name).map_err(|e| {
                                AppError::database(format!(
                                    "Failed to get {name} of a {} row: {e}",
                                    kind.as_str()
                                ))
                            })
                        };
                        Ok(SealedRow {
                            row_key: text("row_key")?.unwrap_or_default(),
                            parts: [text("p1")?, text("p2")?, text("p3")?],
                            sealed: text("sealed")?.unwrap_or_default(),
                        })
                    })
                    .collect()
            }

            async fn reseal_row(
                &self,
                kind: StoredSecretKind,
                row_key: &str,
                old_value: &str,
                new_value: &str,
            ) -> AppResult<bool> {
                let done = sqlx::query(sweep_sql(kind).reseal)
                    .bind(new_value)
                    .bind(row_key)
                    .bind(old_value)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to re-seal a {} row: {e}",
                            kind.as_str()
                        ))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn count_sealed_under(
                &self,
                kind: StoredSecretKind,
                version: u32,
            ) -> AppResult<u64> {
                let row = sqlx::query(sweep_sql(kind).count)
                    .bind(data_tag_pattern(version))
                    .bind(version == 1)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count {} rows under v{version}: {e}",
                            kind.as_str()
                        ))
                    })?;
                let n: i64 = row
                    .try_get("n")
                    .map_err(|e| AppError::database(format!("Failed to get the count: {e}")))?;
                Ok(u64::try_from(n).unwrap_or(0))
            }

            async fn count_live_index_hashes(
                &self,
                version: u32,
                now: DateTime<Utc>,
            ) -> AppResult<u64> {
                let row = sqlx::query(COUNT_LIVE_INDEX_HASHES_SQL)
                    .bind(now)
                    .bind(index_tag_pattern(version))
                    .bind(version == 1)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count refresh tokens under k{version}: {e}"
                        ))
                    })?;
                let n: i64 = row
                    .try_get("n")
                    .map_err(|e| AppError::database(format!("Failed to get the count: {e}")))?;
                Ok(u64::try_from(n).unwrap_or(0))
            }

            async fn record_key_lifecycle(
                &self,
                message: &str,
                metadata: serde_json::Value,
                at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_KEY_LIFECYCLE_SQL)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(message)
                    .bind(metadata)
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a key lifecycle event: {e}"))
                    })?;
                Ok(())
            }

            async fn list_key_lifecycle(&self, limit: i64) -> AppResult<Vec<KeyLifecycleEvent>> {
                let rows = sqlx::query(LIST_KEY_LIFECYCLE_SQL)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list key lifecycle events: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        let column = |e: sqlx::Error| {
                            AppError::database(format!("Failed to read a key lifecycle event: {e}"))
                        };
                        Ok(KeyLifecycleEvent {
                            at: row.try_get("timestamp").map_err(column)?,
                            message: row.try_get("message").map_err(column)?,
                            metadata: row
                                .try_get::<Option<serde_json::Value>, _>("metadata")
                                .map_err(column)?
                                .unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect()
            }

            fn encrypt_data_with_aad(
                &self,
                domain: SealDomain,
                data: &str,
                aad: &str,
            ) -> AppResult<String> {
                HasEncryption::encrypt_data_with_aad(self, domain, data, aad)
            }

            fn decrypt_data_with_aad(
                &self,
                domain: SealDomain,
                encrypted: &str,
                aad: &str,
            ) -> AppResult<String> {
                HasEncryption::decrypt_data_with_aad(self, domain, encrypted, aad)
            }
        }
    };
}
pub(crate) use impl_security_repository;
