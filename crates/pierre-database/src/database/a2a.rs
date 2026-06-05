// ABOUTME: A2A (Agent-to-Agent) database operations
// ABOUTME: Manages agent client registration and authentication for enterprise APIs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use super::Database;
use crate::backends::shared::transactions::SqliteTransactionGuard;
use crate::backends::shared::{enums, mappers};
use crate::repositories::A2ARepository;
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
pub use pierre_core::models::a2a::{
    A2AClient, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

/// Helper functions for safe type conversions
fn safe_u32_to_i32(value: u32) -> AppResult<i32> {
    i32::try_from(value).map_err(|e| {
        warn!(
            value = value,
            max_i32 = i32::MAX,
            error = %e,
            "Type conversion failed: u32 to i32"
        );
        AppError::invalid_input(format!("Value {value} too large to convert to i32: {e}"))
    })
}

/// Safely convert i32 to u32, returning an error if negative
fn safe_i32_to_u32(value: i32) -> AppResult<u32> {
    u32::try_from(value).map_err(|e| {
        warn!(
            value = value,
            error = %e,
            "Type conversion failed: negative i32 cannot convert to u32"
        );
        AppError::invalid_input(format!("Cannot convert negative value {value} to u32: {e}"))
    })
}

/// Safely convert i64 to u64, returning an error if negative
fn safe_i64_to_u64(value: i64) -> AppResult<u64> {
    u64::try_from(value).map_err(|_| {
        AppError::invalid_input(format!("Cannot convert negative value {value} to u64"))
    })
}

/// Safely convert f64 to u32, clamping to u32 range
fn safe_f64_to_u32(value: f64) -> u32 {
    if value.is_nan() || value < 0.0 {
        0
    } else if value > f64::from(u32::MAX) {
        u32::MAX
    } else {
        // Safe: value range checked above to be within u32 bounds
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            value as u32
        }
    }
}

impl Database {
    /// Create a new A2A client
    ///
    /// Uses a transaction to ensure atomicity - if the API key association fails,
    /// the client insertion is rolled back to prevent orphaned client records.
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON serialization fails
    pub async fn create_a2a_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String> {
        // Begin transaction for atomic client + API key association
        let tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
        let mut guard = SqliteTransactionGuard::new(tx);

        // Hash the client secret before storage (never store plaintext secrets)
        let secret_hash = format!("{:x}", Sha256::digest(client_secret.as_bytes()));

        // Insert A2A client within transaction. The model's public_key maps to the
        // canonical api_key_hash column, rate_limit_requests to rate_limit_per_minute,
        // and rate_limit_window_seconds to rate_limit_per_day; permissions is dropped.
        sqlx::query(
            r"
            INSERT INTO a2a_clients (
                client_id, user_id, name, description, api_key_hash, client_secret_hash,
                capabilities, redirect_uris,
                rate_limit_per_minute, rate_limit_per_day, is_active,
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ",
        )
        .bind(&client.id)
        .bind(client.user_id.to_string())
        .bind(&client.name)
        .bind(&client.description)
        .bind(&client.public_key)
        .bind(&secret_hash)
        .bind(serde_json::to_string(&client.capabilities)?)
        .bind(serde_json::to_string(&client.redirect_uris)?)
        .bind(safe_u32_to_i32(client.rate_limit_requests)?)
        .bind(safe_u32_to_i32(client.rate_limit_window_seconds)?)
        .bind(client.is_active)
        .bind(client.created_at)
        .bind(client.updated_at)
        .execute(guard.executor()?)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert A2A client: {e}")))?;

        // Associate A2A client with API key within same transaction
        sqlx::query(
            r"
            INSERT INTO a2a_client_api_keys (client_id, api_key_id, created_at)
            VALUES ($1, $2, $3)
            ",
        )
        .bind(&client.id)
        .bind(api_key_id)
        .bind(Utc::now())
        .execute(guard.executor()?)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to insert A2A client API key association: {e}"
            ))
        })?;

        // Commit transaction - if not reached, guard will auto-rollback on drop
        guard.commit().await?;

        debug!(
            "Created A2A client {} with API key {} association",
            client.id, api_key_id
        );

        Ok(client.id.clone()) // Safe: String ownership needed for return value
    }

    /// Get an A2A client by ID
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON deserialization fails
    pub async fn get_a2a_client_impl(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT client_id, user_id, name, description, api_key_hash, capabilities, redirect_uris,
                   rate_limit_per_minute, rate_limit_per_day, is_active,
                   created_at, updated_at
            FROM a2a_clients
            WHERE client_id = $1
            ",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A client: {e}")))?;

        if let Some(row) = row {
            Ok(Some(Self::a2a_client_from_row(&row, "get_a2a_client")?))
        } else {
            Ok(None)
        }
    }

    /// Build an `A2AClient` from a SQLite row using canonical column names.
    ///
    /// The canonical schema renamed `public_key` to `api_key_hash`, dropped the
    /// `permissions` column (defaulted here), and renamed the rate-limit columns
    /// to `rate_limit_per_minute` / `rate_limit_per_day`. The model field names
    /// stay unchanged; only the column keys differ.
    ///
    /// # Errors
    /// Returns an error if required columns are missing or type conversion fails.
    fn a2a_client_from_row(row: &sqlx::sqlite::SqliteRow, operation: &str) -> AppResult<A2AClient> {
        let capabilities_json: String = row.get("capabilities");
        let capabilities = serde_json::from_str(&capabilities_json).unwrap_or_else(|e| {
            warn!(
                client_id = ?row.get::<String, _>("client_id"),
                error = %e,
                operation = operation,
                "A2A client capabilities JSON parsing failed, using empty array"
            );
            vec![]
        });

        let redirect_uris_json: String = row.get("redirect_uris");
        let redirect_uris = serde_json::from_str(&redirect_uris_json).unwrap_or_else(|e| {
            warn!(
                client_id = ?row.get::<String, _>("client_id"),
                error = %e,
                operation = operation,
                "A2A client redirect_uris JSON parsing failed, using empty array"
            );
            vec![]
        });

        Ok(A2AClient {
            id: row.get("client_id"),
            user_id: Uuid::parse_str(&row.get::<String, _>("user_id"))?,
            name: row.get("name"),
            description: row.get("description"),
            public_key: row.get("api_key_hash"),
            capabilities,
            redirect_uris,
            is_active: row.get("is_active"),
            created_at: row.get("created_at"),
            // permissions column dropped from the canonical schema; default it like
            // the Postgres backend does (it has no permissions column either).
            permissions: vec!["read_activities".to_string()],
            rate_limit_requests: safe_i32_to_u32(row.get::<i32, _>("rate_limit_per_minute"))?,
            rate_limit_window_seconds: safe_i32_to_u32(row.get::<i32, _>("rate_limit_per_day"))?,
            updated_at: row.get("updated_at"),
        })
    }

    /// Get A2A client by API key ID
    ///
    /// # Errors
    /// Returns an error if database query fails
    pub async fn get_a2a_client_by_api_key_id_impl(
        &self,
        api_key_id: &str,
    ) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT c.client_id, c.user_id, c.name, c.description, c.api_key_hash, c.capabilities,
                   c.redirect_uris, c.rate_limit_per_minute, c.rate_limit_per_day, c.is_active,
                   c.created_at, c.updated_at
            FROM a2a_clients c
            INNER JOIN a2a_client_api_keys k ON c.client_id = k.client_id
            WHERE k.api_key_id = $1 AND c.is_active = 1
            ",
        )
        .bind(api_key_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A client by API key: {e}")))?;

        if let Some(row) = row {
            Ok(Some(Self::a2a_client_from_row(
                &row,
                "get_a2a_client_by_api_key_id",
            )?))
        } else {
            Ok(None)
        }
    }

    /// List all A2A clients for a user (or all clients if `user_id` is nil)
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON deserialization fails
    pub async fn list_a2a_clients_impl(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>> {
        let rows = if user_id == &Uuid::nil() {
            // Admin/system-wide query - list all active A2A clients
            let query = r"
                SELECT c.client_id, c.user_id, c.name, c.description, c.api_key_hash, c.capabilities, c.redirect_uris,
                       c.rate_limit_per_minute, c.rate_limit_per_day, c.is_active,
                       c.created_at, c.updated_at
                FROM a2a_clients c
                WHERE c.is_active = 1
                ORDER BY c.created_at DESC
            ";

            sqlx::query(query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to list A2A clients: {e}")))?
        } else {
            // User-specific query - filter directly by c.user_id (the A2A client owner)
            let query = r"
                SELECT c.client_id, c.user_id, c.name, c.description, c.api_key_hash, c.capabilities, c.redirect_uris,
                       c.rate_limit_per_minute, c.rate_limit_per_day, c.is_active,
                       c.created_at, c.updated_at
                FROM a2a_clients c
                WHERE c.is_active = 1 AND c.user_id = ?
                ORDER BY c.created_at DESC
            ";

            sqlx::query(query)
                .bind(user_id.to_string())
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to query A2A clients: {e}")))?
        };

        let mut clients = Vec::new();
        for row in rows {
            clients.push(Self::a2a_client_from_row(&row, "list_a2a_clients")?);
        }

        Ok(clients)
    }

    /// Deactivate an A2A client
    ///
    /// # Errors
    /// Returns an error if database operations fail or client not found
    pub async fn deactivate_a2a_client_impl(&self, client_id: &str) -> AppResult<()> {
        let query = "UPDATE a2a_clients SET is_active = 0, updated_at = ? WHERE client_id = ?";
        let now = Utc::now();

        let result = sqlx::query(query)
            .bind(now)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to deactivate A2A client: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("A2A client: {client_id}")));
        }

        Ok(())
    }

    /// Get client credentials for authentication
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn get_a2a_client_credentials(
        &self,
        client_id: &str,
    ) -> AppResult<Option<(String, String)>> {
        let query =
            "SELECT client_id, client_secret_hash FROM a2a_clients WHERE client_id = ? AND is_active = 1";

        let row = sqlx::query(query)
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to query A2A client credentials: {e}"))
            })?;

        Ok(row.map_or_else(
            || None,
            |row| {
                let id: String = row.get("client_id");
                let secret: String = row.get("client_secret_hash");
                Some((id, secret))
            },
        ))
    }

    /// Invalidate all active sessions for a client
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn invalidate_a2a_client_sessions_impl(&self, client_id: &str) -> AppResult<()> {
        let query =
            "UPDATE a2a_sessions SET expires_at = datetime('now', '-1 hour') WHERE client_id = ?";

        sqlx::query(query)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to invalidate A2A client sessions: {e}"))
            })?;

        Ok(())
    }

    /// Deactivate all API keys associated with a client
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn deactivate_client_api_keys_impl(&self, client_id: &str) -> AppResult<()> {
        // Get API keys associated with the client through the a2a_clients table
        let query = "UPDATE api_keys SET is_active = 0 WHERE id IN (SELECT api_key_id FROM a2a_client_api_keys WHERE client_id = ?)";

        sqlx::query(query)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to deactivate client API keys: {e}"))
            })?;

        Ok(())
    }

    /// Get A2A client by name
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON deserialization fails
    pub async fn get_a2a_client_by_name_impl(&self, name: &str) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT client_id, user_id, name, description, api_key_hash, capabilities, redirect_uris,
                   rate_limit_per_minute, rate_limit_per_day, is_active,
                   created_at, updated_at
            FROM a2a_clients
            WHERE name = $1
            ",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A client by name: {e}")))?;

        if let Some(row) = row {
            Ok(Some(Self::a2a_client_from_row(
                &row,
                "get_a2a_client_by_name",
            )?))
        } else {
            Ok(None)
        }
    }

    /// Create a new A2A session
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn create_a2a_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String> {
        let session_token = format!("sess_{}", Uuid::new_v4());
        let now = Utc::now();
        let expires_at = now + Duration::hours(expires_in_hours);

        sqlx::query(
            r"
            INSERT INTO a2a_sessions (
                session_token, client_id, user_id, granted_scopes,
                expires_at, last_active_at, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(&session_token)
        .bind(client_id)
        .bind(user_id.map(ToString::to_string))
        .bind(granted_scopes.join(","))
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create A2A session: {e}")))?;

        Ok(session_token)
    }

    /// Get an A2A session by token
    ///
    /// # Errors
    /// Returns an error if database operations fail or UUID parsing fails
    pub async fn get_a2a_session_impl(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        let row = sqlx::query(
            r"
            SELECT session_token, client_id, user_id, granted_scopes,
                   expires_at, last_active_at, created_at
            FROM a2a_sessions
            WHERE session_token = $1 AND expires_at > CURRENT_TIMESTAMP
            ",
        )
        .bind(session_token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A session: {e}")))?;

        if let Some(row) = row {
            let user_id_str: Option<String> = row.get("user_id");
            let user_id = user_id_str
                .as_ref()
                .map(|s| Uuid::parse_str(s))
                .transpose()?;

            let granted_scopes_str: String = row.get("granted_scopes");
            let granted_scopes = granted_scopes_str.split(',').map(str::to_owned).collect();

            Ok(Some(A2ASession {
                id: row.get("session_token"),
                client_id: row.get("client_id"),
                user_id,
                granted_scopes,
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                last_activity: row.get("last_active_at"),
                // requests_count column dropped from the canonical schema; the model
                // field is retained but no longer persisted, so default it to zero.
                requests_count: 0,
            }))
        } else {
            Ok(None)
        }
    }

    /// Update A2A session activity timestamp
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn update_a2a_session_activity_impl(&self, session_token: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE a2a_sessions
            SET last_active_at = datetime('now')
            WHERE session_token = $1
            ",
        )
        .bind(session_token)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update A2A session activity: {e}")))?;

        Ok(())
    }

    /// Get active sessions for a specific client
    ///
    /// # Errors
    /// Returns an error if database operations fail or UUID parsing fails
    pub async fn get_active_a2a_sessions_impl(
        &self,
        client_id: &str,
    ) -> AppResult<Vec<A2ASession>> {
        let rows = sqlx::query(
            r"
            SELECT session_token, client_id, user_id, granted_scopes,
                   expires_at, last_active_at, created_at
            FROM a2a_sessions
            WHERE client_id = $1 AND expires_at > CURRENT_TIMESTAMP
            ORDER BY last_active_at DESC
            ",
        )
        .bind(client_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query active A2A sessions: {e}")))?;

        let mut sessions = Vec::new();
        for row in rows {
            let user_id_str: Option<String> = row.get("user_id");
            let user_id = user_id_str
                .as_ref()
                .map(|s| Uuid::parse_str(s))
                .transpose()?;

            let granted_scopes_str: String = row.get("granted_scopes");
            let granted_scopes = granted_scopes_str.split(',').map(str::to_owned).collect();

            sessions.push(A2ASession {
                id: row.get("session_token"),
                client_id: row.get("client_id"),
                user_id,
                granted_scopes,
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                last_activity: row.get("last_active_at"),
                // requests_count column dropped from the canonical schema; default it.
                requests_count: 0,
            });
        }

        Ok(sessions)
    }

    /// Create a new A2A task
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON serialization fails
    pub async fn create_a2a_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> AppResult<String> {
        let task_id = format!("task_{}", Uuid::new_v4());
        let now = Utc::now();

        // The canonical a2a_tasks is session-keyed (no client_id column). Use the
        // session token when provided; otherwise fall back to the client_id as a
        // best-effort session identifier, mirroring the schema-rebuild migration.
        let session_token = session_id.unwrap_or(client_id);

        sqlx::query(
            r"
            INSERT INTO a2a_tasks (
                task_id, session_token, task_type, parameters,
                status, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(&task_id)
        .bind(session_token)
        .bind(task_type)
        .bind(serde_json::to_string(input_data)?)
        .bind(enums::task_status_to_str(&TaskStatus::Pending))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create A2A task: {e}")))?;

        Ok(task_id)
    }

    /// List A2A tasks with optional filtering
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON deserialization fails
    pub async fn list_a2a_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>> {
        let mut query = String::from(
            r"
            SELECT task_id, session_token, task_type, parameters, result,
                   status, created_at, updated_at
            FROM a2a_tasks
            ",
        );

        let mut conditions = Vec::new();
        let mut bind_count = 0;

        if client_id.is_some() {
            // a2a_tasks is session-keyed; the client filter matches the session_token
            // (which carries the client_id for client-keyed tasks created without a session).
            bind_count += 1;
            conditions.push(format!("session_token = ${bind_count}"));
        }

        if status_filter.is_some() {
            bind_count += 1;
            conditions.push(format!("status = ${bind_count}"));
        }

        if !conditions.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&conditions.join(" AND "));
        }

        query.push_str(" ORDER BY created_at DESC");

        if limit.is_some() {
            bind_count += 1;
            if write!(query, " LIMIT ${bind_count}").is_err() {
                return Err(AppError::internal("Failed to write LIMIT clause to query"));
            }
        }

        if offset.is_some() {
            bind_count += 1;
            if write!(query, " OFFSET ${bind_count}").is_err() {
                return Err(AppError::internal("Failed to write OFFSET clause to query"));
            }
        }

        let mut sql_query = sqlx::query(&query);

        if let Some(client_id_val) = client_id {
            sql_query = sql_query.bind(client_id_val);
        }

        if let Some(status_val) = status_filter {
            sql_query = sql_query.bind(status_val.to_string());
        }

        if let Some(limit_val) = limit {
            sql_query = sql_query.bind(safe_u32_to_i32(limit_val)?);
        }

        if let Some(offset_val) = offset {
            sql_query = sql_query.bind(safe_u32_to_i32(offset_val)?);
        }

        let rows = sql_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to query A2A tasks: {e}")))?;

        let tasks: Vec<A2ATask> = rows
            .iter()
            .map(mappers::parse_a2a_task_from_row)
            .collect::<AppResult<Vec<_>>>()?;

        Ok(tasks)
    }

    /// Get an A2A task by ID
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON deserialization fails
    pub async fn get_a2a_task_impl(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        let row = sqlx::query(
            r"
            SELECT task_id, session_token, task_type, parameters, result,
                   status, created_at, updated_at
            FROM a2a_tasks
            WHERE task_id = $1
            ",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A task: {e}")))?;

        if let Some(row) = row {
            let task = mappers::parse_a2a_task_from_row(&row)?;
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// Update A2A task status
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON serialization fails
    pub async fn update_a2a_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> AppResult<()> {
        let result_json = result.map(serde_json::to_string).transpose()?;

        // The canonical a2a_tasks dropped error_message and completed_at; the
        // error argument is therefore not persisted. updated_at advances to mark
        // completion (read back into A2ATask.completed_at by the row mapper).
        let _ = error;

        sqlx::query(
            r"
            UPDATE a2a_tasks
            SET status = $2, result = $3, updated_at = datetime('now')
            WHERE task_id = $1
            ",
        )
        .bind(task_id)
        .bind(status.to_string())
        .bind(result_json)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update A2A task status: {e}")))?;

        Ok(())
    }

    /// Record A2A usage for rate limiting and analytics
    ///
    /// # Errors
    /// Returns an error if database operations fail or JSON serialization fails
    pub async fn record_a2a_usage_impl(&self, usage: &A2AUsage) -> AppResult<()> {
        // The model's tool_name maps to the canonical endpoint column; error_message
        // is no longer persisted, and the new method column is left NULL here.
        sqlx::query(
            r"
            INSERT INTO a2a_usage (
                id, client_id, session_token, timestamp, endpoint, response_time_ms,
                status_code, method, request_size_bytes, response_size_bytes,
                ip_address, user_agent, protocol_version, client_capabilities, granted_scopes
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&usage.client_id)
        .bind(&usage.session_token)
        .bind(usage.timestamp)
        .bind(&usage.tool_name)
        .bind(usage.response_time_ms.map(safe_u32_to_i32).transpose()?)
        .bind(i32::from(usage.status_code))
        .bind(None::<String>) // method (not tracked by A2AUsage model)
        .bind(usage.request_size_bytes.map(safe_u32_to_i32).transpose()?)
        .bind(usage.response_size_bytes.map(safe_u32_to_i32).transpose()?)
        .bind(&usage.ip_address)
        .bind(&usage.user_agent)
        .bind(&usage.protocol_version)
        .bind(serde_json::to_string(&usage.client_capabilities)?)
        .bind(serde_json::to_string(&usage.granted_scopes)?)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record A2A usage: {e}")))?;

        Ok(())
    }

    /// Get current usage count for an A2A client (for rate limiting)
    ///
    /// # Errors
    /// Returns an error if database operations fail or client not found
    pub async fn get_a2a_client_current_usage_impl(&self, client_id: &str) -> AppResult<u32> {
        // Get the client to determine its rate limit window
        let client = self
            .get_a2a_client(client_id)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get A2A client for usage check: {e}"))
            })?
            .ok_or_else(|| AppError::not_found(format!("A2A client: {client_id}")))?;

        let window_start =
            Utc::now() - Duration::seconds(i64::from(client.rate_limit_window_seconds));

        let count: i32 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM a2a_usage
            WHERE client_id = $1 AND timestamp > $2
            ",
        )
        .bind(client_id)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A client usage count: {e}")))?;

        safe_i32_to_u32(count)
    }

    /// Get A2A usage statistics for a client
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn get_a2a_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats> {
        let stats = sqlx::query(
            r"
            SELECT 
                COUNT(*) as total_requests,
                COUNT(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 END) as successful_requests,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) as failed_requests,
                AVG(response_time_ms) as avg_response_time,
                SUM(request_size_bytes) as total_request_bytes,
                SUM(response_size_bytes) as total_response_bytes
            FROM a2a_usage
            WHERE client_id = $1 AND timestamp >= $2 AND timestamp <= $3
            ",
        )
        .bind(client_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A usage stats: {e}")))?;

        let total_requests: i32 = stats.get(0);
        let successful_requests: i32 = stats.get(1);
        let failed_requests: i32 = stats.get(2);
        let avg_response_time: Option<f64> = stats.get(3);
        let total_request_bytes: Option<i64> = stats.get(4);
        let total_response_bytes: Option<i64> = stats.get(5);

        Ok(A2AUsageStats {
            client_id: client_id.to_owned(),
            period_start: start_date,
            period_end: end_date,
            total_requests: safe_i32_to_u32(total_requests)?,
            successful_requests: safe_i32_to_u32(successful_requests)?,
            failed_requests: safe_i32_to_u32(failed_requests)?,
            avg_response_time_ms: avg_response_time.map(safe_f64_to_u32),
            total_request_bytes: total_request_bytes.map(safe_i64_to_u64).transpose()?,
            total_response_bytes: total_response_bytes.map(safe_i64_to_u64).transpose()?,
        })
    }

    /// Get A2A client usage history (daily aggregates with success/error counts)
    ///
    /// # Errors
    /// Returns an error if database operations fail or date parsing fails
    ///
    /// # Panics
    /// Panics if the date string from database is not in expected YYYY-MM-DD format
    pub async fn get_a2a_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>> {
        let start_date = Utc::now() - Duration::days(i64::from(days));

        let rows = sqlx::query(
            r"
            SELECT 
                date(timestamp) as usage_date,
                COUNT(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 END) as success_count,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) as error_count
            FROM a2a_usage
            WHERE client_id = $1 AND timestamp >= $2
            GROUP BY date(timestamp)
            ORDER BY usage_date DESC
            ",
        )
        .bind(client_id)
        .bind(start_date)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query A2A client usage history: {e}")))?;

        let mut history = Vec::new();
        for row in rows {
            let date_str: String = row.get("usage_date");
            let success_count: i32 = row.get("success_count");
            let error_count: i32 = row.get("error_count");

            // Parse date string (YYYY-MM-DD format from SQLite date())
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")?
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "Failed to create datetime from date {date_str}"
                    ))
                })?
                .and_utc();

            history.push((
                date,
                safe_i32_to_u32(success_count)?,
                safe_i32_to_u32(error_count)?,
            ));
        }

        Ok(history)
    }
    // Public wrapper methods (delegate to _impl versions)

    /// Deactivate A2A client (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn deactivate_a2a_client(&self, client_id: &str) -> AppResult<()> {
        self.deactivate_a2a_client_impl(client_id).await
    }

    /// Deactivate client API keys (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
        self.deactivate_client_api_keys_impl(client_id).await
    }

    /// Get A2A client by ID (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        self.get_a2a_client_impl(client_id).await
    }

    /// Get A2A client by API key ID (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_client_by_api_key_id(
        &self,
        api_key_id: &str,
    ) -> AppResult<Option<A2AClient>> {
        self.get_a2a_client_by_api_key_id_impl(api_key_id).await
    }

    /// Get A2A client by name (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
        self.get_a2a_client_by_name_impl(name).await
    }

    /// Get A2A client current usage (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_client_current_usage(&self, client_id: &str) -> AppResult<u32> {
        self.get_a2a_client_current_usage_impl(client_id).await
    }

    /// Get A2A session by token (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        self.get_a2a_session_impl(session_token).await
    }

    /// Get A2A task by ID (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_a2a_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        self.get_a2a_task_impl(task_id).await
    }

    /// Get active A2A sessions (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_active_a2a_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
        self.get_active_a2a_sessions_impl(client_id).await
    }

    /// Invalidate A2A client sessions (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn invalidate_a2a_client_sessions(&self, client_id: &str) -> AppResult<()> {
        self.invalidate_a2a_client_sessions_impl(client_id).await
    }

    /// List A2A clients for user (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn list_a2a_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>> {
        self.list_a2a_clients_impl(user_id).await
    }

    /// Record A2A usage (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn record_a2a_usage(&self, usage: &A2AUsage) -> AppResult<()> {
        self.record_a2a_usage_impl(usage).await
    }

    /// Update A2A session activity (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn update_a2a_session_activity(&self, session_token: &str) -> AppResult<()> {
        self.update_a2a_session_activity_impl(session_token).await
    }
}

#[async_trait]
impl A2ARepository for Database {
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String> {
        Self::create_a2a_client(self, client, client_secret, api_key_id).await
    }
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        Self::get_a2a_client_impl(self, client_id).await
    }
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>> {
        Self::get_a2a_client_by_api_key_id_impl(self, api_key_id).await
    }
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
        Self::get_a2a_client_by_name_impl(self, name).await
    }
    async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>> {
        Self::list_a2a_clients_impl(self, user_id).await
    }
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()> {
        Self::deactivate_a2a_client_impl(self, client_id).await
    }
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>> {
        Self::get_a2a_client_credentials(self, client_id).await
    }
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()> {
        Self::invalidate_a2a_client_sessions_impl(self, client_id).await
    }
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
        Self::deactivate_client_api_keys_impl(self, client_id).await
    }
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String> {
        Self::create_a2a_session(self, client_id, user_id, granted_scopes, expires_in_hours).await
    }
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        Self::get_a2a_session_impl(self, session_token).await
    }
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()> {
        Self::update_a2a_session_activity_impl(self, session_token).await
    }
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
        Self::get_active_a2a_sessions_impl(self, client_id).await
    }
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> AppResult<String> {
        Self::create_a2a_task(self, client_id, session_id, task_type, input_data).await
    }
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        Self::get_a2a_task_impl(self, task_id).await
    }
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>> {
        Self::list_a2a_tasks(self, client_id, status_filter, limit, offset).await
    }
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> AppResult<()> {
        Self::update_a2a_task_status(self, task_id, status, result, error).await
    }
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()> {
        Self::record_a2a_usage_impl(self, usage).await
    }
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32> {
        Self::get_a2a_client_current_usage_impl(self, client_id).await
    }
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats> {
        Self::get_a2a_usage_stats(self, client_id, start_date, end_date).await
    }
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>> {
        Self::get_a2a_client_usage_history(self, client_id, days).await
    }
}
