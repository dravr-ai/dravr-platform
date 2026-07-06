// ABOUTME: PostgreSQL A2A (Agent-to-Agent) protocol repository implementation
// ABOUTME: Manages A2A clients, sessions, tasks, and usage tracking for inter-agent communication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::A2ARepository;
use super::PostgresDatabase;
use crate::backends::shared;
use crate::database::{A2AUsage, A2AUsageStats};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::a2a::{A2AClient, A2ASession, A2ATask, TaskStatus};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

#[async_trait]
impl A2ARepository for PostgresDatabase {
    // A2A methods
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String> {
        // Hash secrets before storage (never store plaintext credentials)
        let secret_hash = format!("{:x}", Sha256::digest(client_secret.as_bytes()));
        let key_hash = format!("{:x}", Sha256::digest(api_key_id.as_bytes()));

        sqlx::query(
            r"
            INSERT INTO a2a_clients (client_id, user_id, name, description, client_secret_hash,
                                    api_key_hash, capabilities, redirect_uris,
                                    is_active, rate_limit_per_minute, rate_limit_per_day)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ",
        )
        .bind(&client.id)
        .bind(client.user_id)
        .bind(&client.name)
        .bind(&client.description)
        .bind(&secret_hash)
        .bind(&key_hash)
        .bind(&client.capabilities)
        .bind(&client.redirect_uris)
        .bind(client.is_active)
        .bind(100i32) // Default rate limit
        .bind(10000i32) // Default daily rate limit
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create A2A client: {e}")))?;

        Ok(client.id.clone()) // Safe: String ownership for return value
    }

    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT client_id, user_id, name, description, client_secret_hash, capabilities,
                   redirect_uris, contact_email, is_active, rate_limit_per_minute,
                   rate_limit_per_day, created_at, updated_at
            FROM a2a_clients
            WHERE client_id = $1
            ",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A client: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(A2AClient {
                    id: row.get("client_id"),
                    user_id: row.get("user_id"),
                    name: row.get("name"),
                    description: row.get("description"),
                    public_key: String::new(), // Postgres schema does not store public_key separately
                    capabilities: row.get("capabilities"),
                    redirect_uris: row.get("redirect_uris"),
                    is_active: row.get("is_active"),
                    created_at: row.get("created_at"),
                    permissions: vec!["read_activities".into()], // Default permission
                    rate_limit_requests: u32::try_from(
                        row.get::<i32, _>("rate_limit_per_minute").max(0),
                    )
                    .unwrap_or(0),
                    rate_limit_window_seconds: 60, // 1 minute in seconds
                    updated_at: row.get("updated_at"),
                }))
            },
        )
    }

    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT c.client_id, c.user_id, c.name, c.description, c.client_secret_hash, c.capabilities,
                   c.redirect_uris, c.contact_email, c.is_active, c.rate_limit_per_minute,
                   c.rate_limit_per_day, c.created_at, c.updated_at
            FROM a2a_clients c
            INNER JOIN a2a_client_api_keys k ON c.client_id = k.client_id
            WHERE k.api_key_id = $1 AND c.is_active = true
            ",
        )
        .bind(api_key_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A client by API key ID: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(A2AClient {
                    id: row.get("client_id"),
                    user_id: row.get("user_id"),
                    name: row.get("name"),
                    description: row.get("description"),
                    public_key: row.get("client_secret_hash"),
                    capabilities: row.get("capabilities"),
                    redirect_uris: row.get("redirect_uris"),
                    is_active: row.get("is_active"),
                    created_at: row.get("created_at"),
                    permissions: vec!["read_activities".into()],
                    rate_limit_requests: u32::try_from(
                        row.get::<i32, _>("rate_limit_per_minute").max(0),
                    )
                    .unwrap_or(0),
                    rate_limit_window_seconds: 60,
                    updated_at: row.get("updated_at"),
                }))
            },
        )
    }

    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
        let row = sqlx::query(
            r"
            SELECT client_id, user_id, name, description, client_secret_hash, capabilities,
                   redirect_uris, contact_email, is_active, rate_limit_per_minute,
                   rate_limit_per_day, created_at, updated_at
            FROM a2a_clients
            WHERE name = $1
            ",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A client by name: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(A2AClient {
                    id: row.get("client_id"),
                    user_id: row.get("user_id"),
                    name: row.get("name"),
                    description: row.get("description"),
                    public_key: String::new(), // Postgres schema does not store public_key separately
                    capabilities: row.get("capabilities"),
                    redirect_uris: row.get("redirect_uris"),
                    is_active: row.get("is_active"),
                    created_at: row.get("created_at"),
                    permissions: vec!["read_activities".into()], // Default permission
                    rate_limit_requests: u32::try_from(
                        row.get::<i32, _>("rate_limit_per_minute").max(0),
                    )
                    .unwrap_or(0),
                    rate_limit_window_seconds: 60, // 1 minute in seconds
                    updated_at: row.get("updated_at"),
                }))
            },
        )
    }

    async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>> {
        let rows = sqlx::query(
            r"
            SELECT client_id, user_id, name, description, client_secret_hash, capabilities, 
                   redirect_uris, contact_email, is_active, rate_limit_per_minute, 
                   rate_limit_per_day, created_at, updated_at
            FROM a2a_clients
            WHERE user_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list A2A clients: {e}")))?;

        let mut clients = Vec::new();
        for row in rows {
            clients.push(A2AClient {
                id: row.get("client_id"),
                user_id: *user_id, // Use the provided user_id parameter
                name: row.get("name"),
                description: row.get("description"),
                public_key: row.get("client_secret_hash"), // Map client_secret_hash to public_key
                capabilities: row.get("capabilities"),
                redirect_uris: row.get("redirect_uris"),
                is_active: row.get("is_active"),
                created_at: row.get("created_at"),
                permissions: vec!["read_activities".into()], // Default permission
                rate_limit_requests: u32::try_from(
                    row.get::<i32, _>("rate_limit_per_minute").max(0),
                )
                .unwrap_or(0),
                rate_limit_window_seconds: 60, // 1 minute in seconds
                updated_at: row.get("updated_at"),
            });
        }

        Ok(clients)
    }

    async fn deactivate_client(&self, client_id: &str) -> AppResult<()> {
        let query =
            "UPDATE a2a_clients SET is_active = false, updated_at = NOW() WHERE client_id = $1";

        let result = sqlx::query(query)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to deactivate A2A client: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("A2A client {client_id}")));
        }

        Ok(())
    }

    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>> {
        let query = "SELECT client_id, client_secret_hash FROM a2a_clients WHERE client_id = $1 AND is_active = true";

        let row = sqlx::query(query)
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get A2A client credentials: {e}"))
            })?;

        row.map_or_else(
            || Ok(None),
            |row| {
                let id: String = row.get("client_id");
                let secret: String = row.get("client_secret_hash");
                Ok(Some((id, secret)))
            },
        )
    }

    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()> {
        let query =
            "UPDATE a2a_sessions SET expires_at = NOW() - INTERVAL '1 hour' WHERE client_id = $1";

        sqlx::query(query)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to invalidate A2A client sessions: {e}"))
            })?;

        Ok(())
    }

    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
        let query = "UPDATE api_keys SET is_active = false WHERE id IN (SELECT api_key_id FROM a2a_client_api_keys WHERE client_id = $1)";

        sqlx::query(query)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to deactivate client API keys: {e}"))
            })?;

        Ok(())
    }

    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String> {
        let session_token = format!("sess_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::hours(expires_in_hours);
        let scopes: Vec<String> = granted_scopes.to_vec();

        sqlx::query(
            r"
            INSERT INTO a2a_sessions (
                session_token, client_id, user_id, granted_scopes, created_at, expires_at, last_active_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $5)
            ",
        )
        .bind(&session_token)
        .bind(client_id)
        .bind(user_id)
        .bind(&scopes)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create A2A session: {e}")))?;

        Ok(session_token)
    }

    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        let row = sqlx::query(
            r"
            SELECT session_token, client_id, user_id, granted_scopes,
                   expires_at, last_active_at, created_at
            FROM a2a_sessions
            WHERE session_token = $1 AND expires_at > NOW()
            ",
        )
        .bind(session_token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A session: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            // granted_scopes is a canonical TEXT[] column.
            let scopes: Vec<String> = row.try_get("granted_scopes").map_err(|e| {
                AppError::database(format!("Failed to parse granted_scopes column: {e}"))
            })?;

            Ok(Some(A2ASession {
                id: row.try_get("session_token").map_err(|e| {
                    AppError::database(format!("Failed to parse session_token column: {e}"))
                })?,
                client_id: row.try_get("client_id").map_err(|e| {
                    AppError::database(format!("Failed to parse client_id column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                granted_scopes: scopes,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                last_activity: row.try_get("last_active_at").map_err(|e| {
                    AppError::database(format!("Failed to parse last_active_at column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                // requests_count is not tracked in the canonical schema.
                requests_count: 0,
            }))
        } else {
            Ok(None)
        }
    }

    async fn update_session_activity(&self, session_token: &str) -> AppResult<()> {
        sqlx::query("UPDATE a2a_sessions SET last_active_at = NOW() WHERE session_token = $1")
            .bind(session_token)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to update A2A session activity: {e}"))
            })?;

        Ok(())
    }

    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
        let rows = sqlx::query(
            r"
            SELECT session_token, client_id, user_id, granted_scopes,
                   expires_at, last_active_at, created_at
            FROM a2a_sessions
            WHERE client_id = $1 AND expires_at > NOW()
            ORDER BY last_active_at DESC
            ",
        )
        .bind(client_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active A2A sessions: {e}")))?;

        let mut sessions = Vec::new();
        for row in rows {
            // user_id is a canonical NOT NULL UUID column; granted_scopes is TEXT[].
            let user_id: Uuid = row.get("user_id");
            let granted_scopes: Vec<String> = row.get("granted_scopes");

            sessions.push(A2ASession {
                id: row.get("session_token"),
                client_id: row.get("client_id"),
                user_id: Some(user_id),
                granted_scopes,
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                last_activity: row.get("last_active_at"),
                // requests_count is not tracked in the canonical schema.
                requests_count: 0,
            });
        }

        Ok(sessions)
    }

    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> AppResult<String> {
        use uuid::Uuid;

        let uuid = Uuid::new_v4().simple();
        let task_id = format!("task_{uuid}");
        let input_json = serde_json::to_string(input_data)?;

        // The canonical a2a_tasks is session-keyed (no client_id column). Use the
        // session token when provided; otherwise fall back to the client_id as a
        // best-effort session identifier, matching the SQLite backend.
        let session_token = session_id.unwrap_or(client_id);

        sqlx::query(
            r"
            INSERT INTO a2a_tasks
            (task_id, session_token, task_type, parameters, status, created_at)
            VALUES ($1, $2, $3, $4::jsonb, $5, NOW())
            ",
        )
        .bind(&task_id)
        .bind(session_token)
        .bind(task_type)
        .bind(&input_json)
        .bind("pending")
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create A2A task: {e}")))?;

        Ok(task_id)
    }

    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        let row = sqlx::query(
            r"
            SELECT task_id, session_token, task_type, parameters,
                   status, result, created_at, updated_at
            FROM a2a_tasks
            WHERE task_id = $1
            ",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A task: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            // parameters/result are JSONB columns; decode them directly as JSON values.
            let input_data: Value = row.try_get("parameters").map_err(|e| {
                AppError::database(format!("Failed to parse parameters column: {e}"))
            })?;

            // Validate input data structure (log type only, never log raw content)
            if !input_data.is_null() && !input_data.is_object() {
                warn!(
                    task_id = %task_id,
                    value_type = %input_data.as_str().map_or_else(|| "non-object", |_| "string"),
                    "Invalid input data structure for task, expected object"
                );
            }

            let result_data: Option<Value> = row
                .try_get("result")
                .map_err(|e| AppError::database(format!("Failed to parse result column: {e}")))?;

            let status_str: String = row
                .try_get("status")
                .map_err(|e| AppError::database(format!("Failed to parse status column: {e}")))?;
            let status = shared::enums::str_to_task_status(&status_str);

            Ok(Some(A2ATask {
                id: row.try_get("task_id").map_err(|e| {
                    AppError::database(format!("Failed to parse task_id column: {e}"))
                })?,
                status,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                // completed_at column was dropped; reflect completion via updated_at.
                completed_at: row.try_get("updated_at").ok(),
                result: result_data.clone(), // Safe: JSON value ownership for A2ATask struct
                // error/error_message columns were dropped; no longer persisted.
                error: None,
                // a2a_tasks is session-keyed; populate client_id best-effort from session_token.
                client_id: row
                    .try_get("session_token")
                    .unwrap_or_else(|_| "unknown".into()),
                task_type: row.try_get("task_type").map_err(|e| {
                    AppError::database(format!("Failed to parse task_type column: {e}"))
                })?,
                input_data,
                output_data: result_data,
                error_message: None,
                updated_at: row.try_get("updated_at").map_err(|e| {
                    AppError::database(format!("Failed to parse updated_at column: {e}"))
                })?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>> {
        let query = Self::build_a2a_tasks_query(client_id, status_filter, limit, offset)?;

        let mut sql_query = sqlx::query(&query);

        if let Some(client_id_val) = client_id {
            sql_query = sql_query.bind(client_id_val);
        }

        if let Some(status_val) = status_filter {
            let status_str = shared::enums::task_status_to_str(status_val);
            sql_query = sql_query.bind(status_str);
        }

        if let Some(limit_val) = limit {
            sql_query = sql_query.bind(i64::from(limit_val));
        }

        if let Some(offset_val) = offset {
            sql_query = sql_query.bind(i64::from(offset_val));
        }

        let rows = sql_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list A2A tasks: {e}")))?;
        rows.iter()
            .map(Self::parse_a2a_task_from_row)
            .collect::<AppResult<Vec<_>>>()
    }

    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> AppResult<()> {
        let status_str = shared::enums::task_status_to_str(status);

        let result_json = result.map(serde_json::to_string).transpose()?;

        // The canonical a2a_tasks dropped the error/method column; the error
        // argument is therefore not persisted.
        let _ = error;

        sqlx::query(
            r"
            UPDATE a2a_tasks
            SET status = $1, result = $2::jsonb, updated_at = NOW()
            WHERE task_id = $3
            ",
        )
        .bind(status_str)
        .bind(result_json)
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update A2A task status: {e}")))?;

        Ok(())
    }

    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO a2a_usage
            (client_id, session_token, endpoint, status_code,
             response_time_ms, request_size_bytes, response_size_bytes, timestamp,
             method, ip_address, user_agent, protocol_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::inet, $11, $12)
            ",
        )
        .bind(&usage.client_id)
        .bind(&usage.session_token)
        .bind(&usage.tool_name)
        .bind(i32::from(usage.status_code))
        .bind(
            usage
                .response_time_ms
                .map(|x| i32::try_from(x).unwrap_or(i32::MAX)),
        )
        .bind(
            usage
                .request_size_bytes
                .map(|x| i32::try_from(x).unwrap_or(i32::MAX)),
        )
        .bind(
            usage
                .response_size_bytes
                .map(|x| i32::try_from(x).unwrap_or(i32::MAX)),
        )
        .bind(usage.timestamp)
        .bind(None::<String>)
        .bind(&usage.ip_address)
        .bind(&usage.user_agent)
        .bind(&usage.protocol_version)
        .execute(&self.pool)
        .await
        .inspect_err(|e| {
            warn!(
                client_id = %usage.client_id,
                endpoint = %usage.tool_name,
                status_code = usage.status_code,
                error = %e,
                "Failed to record A2A usage tracking (affects billing/analytics)"
            );
        })
        .map_err(|e| AppError::database(format!("Failed to record A2A usage: {e}")))?;

        Ok(())
    }

    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as usage_count
            FROM a2a_usage
            WHERE client_id = $1 AND timestamp >= NOW() - INTERVAL '1 hour'
            ",
        )
        .bind(client_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A client current usage: {e}")))?;

        let count: i64 = row
            .try_get("usage_count")
            .map_err(|e| AppError::database(format!("Failed to parse usage_count column: {e}")))?;
        Ok(u32::try_from(count.max(0)).unwrap_or(0))
    }

    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats> {
        use sqlx::Row;

        let row = sqlx::query(
            r"
            SELECT 
                COUNT(*) as total_requests,
                COUNT(CASE WHEN status_code < 400 THEN 1 END) as successful_requests,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) as failed_requests,
                AVG(response_time_ms)::DOUBLE PRECISION as avg_response_time,
                SUM(request_size_bytes) as total_request_bytes,
                SUM(response_size_bytes) as total_response_bytes
            FROM a2a_usage
            WHERE client_id = $1 AND timestamp BETWEEN $2 AND $3
            ",
        )
        .bind(client_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A usage stats: {e}")))?;

        let total_requests: i64 = row.try_get("total_requests").map_err(|e| {
            AppError::database(format!("Failed to parse total_requests column: {e}"))
        })?;
        let successful_requests: i64 = row.try_get("successful_requests").map_err(|e| {
            AppError::database(format!("Failed to parse successful_requests column: {e}"))
        })?;
        let failed_requests: i64 = row.try_get("failed_requests").map_err(|e| {
            AppError::database(format!("Failed to parse failed_requests column: {e}"))
        })?;
        let avg_response_time: Option<f64> = row.try_get("avg_response_time").map_err(|e| {
            AppError::database(format!("Failed to parse avg_response_time column: {e}"))
        })?;
        let total_request_bytes: Option<i64> = row.try_get("total_request_bytes").map_err(|e| {
            AppError::database(format!("Failed to parse total_request_bytes column: {e}"))
        })?;
        let total_response_bytes: Option<i64> =
            row.try_get("total_response_bytes").map_err(|e| {
                AppError::database(format!("Failed to parse total_response_bytes column: {e}"))
            })?;

        // Log byte usage for monitoring
        if let (Some(req_bytes), Some(resp_bytes)) = (total_request_bytes, total_response_bytes) {
            debug!(
                "A2A client {} usage: {} req bytes, {} resp bytes",
                client_id, req_bytes, resp_bytes
            );
        }

        Ok(A2AUsageStats {
            client_id: client_id.to_owned(),
            period_start: start_date,
            period_end: end_date,
            total_requests: u32::try_from(total_requests.max(0)).unwrap_or(0),
            successful_requests: u32::try_from(successful_requests.max(0)).unwrap_or(0),
            failed_requests: u32::try_from(failed_requests.max(0)).unwrap_or(0),
            avg_response_time_ms: avg_response_time.map(|t| {
                if t.is_nan() || t.is_infinite() || t < 0.0 {
                    0
                } else if t > f64::from(u32::MAX) {
                    u32::MAX
                } else {
                    // Convert to integer via string to avoid casting issues
                    let rounded = t.round();
                    let as_string = format!("{rounded:.0}");
                    as_string.parse::<u32>().unwrap_or(0)
                }
            }),
            total_request_bytes: total_request_bytes.map(|b| u64::try_from(b.max(0)).unwrap_or(0)),
            total_response_bytes: total_response_bytes
                .map(|b| u64::try_from(b.max(0)).unwrap_or(0)),
        })
    }

    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>> {
        // Compute the cutoff in Rust and bind a timestamp (mirrors the SQLite
        // impl): a placeholder inside an INTERVAL string literal is never
        // substituted, so the extra bind made PG reject every execution.
        let start_time = Utc::now() - Duration::days(i64::from(days));
        let rows = sqlx::query(
            r"
            SELECT
                DATE_TRUNC('day', timestamp) as day,
                COUNT(CASE WHEN status_code < 400 THEN 1 END) as success_count,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) as error_count
            FROM a2a_usage
            WHERE client_id = $1
              AND timestamp >= $2
            GROUP BY DATE_TRUNC('day', timestamp)
            ORDER BY day
            ",
        )
        .bind(client_id)
        .bind(start_time)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get A2A client usage history: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            use sqlx::Row;
            let day: DateTime<Utc> = row
                .try_get("day")
                .map_err(|e| AppError::database(format!("Failed to parse day column: {e}")))?;
            let success_count: i64 = row.try_get("success_count").map_err(|e| {
                AppError::database(format!("Failed to parse success_count column: {e}"))
            })?;
            let error_count: i64 = row.try_get("error_count").map_err(|e| {
                AppError::database(format!("Failed to parse error_count column: {e}"))
            })?;

            result.push((
                day,
                u32::try_from(success_count.max(0)).unwrap_or(0),
                u32::try_from(error_count.max(0)).unwrap_or(0),
            ));
        }

        Ok(result)
    }
}
