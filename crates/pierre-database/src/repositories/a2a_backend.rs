// ABOUTME: The one A2ARepository implementation, emitted per backend by impl_a2a_repository! with that backend's codecs and literals
// ABOUTME: Row parsers and the trait body over the statements in repositories/a2a.rs; the shells are one invocation each
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::a2a::TaskStatus;

/// The error a column that will not decode surfaces as, named after the
/// column so a corrupt row is locatable from the message.
pub fn a2a_column_error(column: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("a2a column {column}: {e}"))
}

/// A `u32` the schema stores in an `INTEGER` column.
///
/// # Errors
/// Returns an invalid-input error when the value does not fit an `i32`.
pub fn i32_from_u32(value: u32) -> AppResult<i32> {
    i32::try_from(value)
        .map_err(|e| AppError::invalid_input(format!("Value {value} too large for i32: {e}")))
}

/// A count the engine returns as `i64`, as the `u32` the model carries.
///
/// # Errors
/// Returns a database error when the count is negative or past `u32::MAX`.
pub fn u32_from_count(value: i64, column: &str) -> AppResult<u32> {
    u32::try_from(value)
        .map_err(|e| AppError::database(format!("a2a count {column} = {value}: {e}")))
}

/// A byte total the engine returns as `i64`, as the `u64` the model carries.
///
/// # Errors
/// Returns a database error when the total is negative.
pub fn u64_from_sum(value: i64, column: &str) -> AppResult<u64> {
    u64::try_from(value).map_err(|e| AppError::database(format!("a2a sum {column} = {value}: {e}")))
}

/// An average response time rounded to whole milliseconds; NaN, infinities
/// and negatives read as zero, and anything past `u32::MAX` saturates.
#[must_use]
pub fn average_millis(value: f64) -> u32 {
    if value.is_nan() || value < 0.0 {
        0
    } else if value >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        // Range checked above: the rounded value lies within u32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            value.round() as u32
        }
    }
}

/// The filters one `list_tasks` call carries, each present only when the
/// caller filters on it.
#[derive(Clone, Copy)]
pub struct TaskListing<'a> {
    /// The client, matched on the session token that keys the task (it
    /// carries the client id for a task created without a session).
    pub client_id: Option<&'a str>,
    /// The status to match.
    pub status: Option<&'a TaskStatus>,
    /// The context to match.
    pub context_id: Option<&'a str>,
    /// Only tasks whose status moved after this instant.
    pub updated_after: Option<DateTime<Utc>>,
    /// Page size.
    pub limit: Option<u32>,
    /// Page offset.
    pub offset: Option<u32>,
}

/// The `WHERE`, `ORDER BY`, `LIMIT` and `OFFSET` of a task listing over
/// `select`, in the bind order [`impl_a2a_repository!`]'s `list_tasks`
/// applies: client, status, context, cutoff, limit, offset.
///
/// # Errors
/// Returns an internal error if the clause cannot be written, which a
/// `String` never refuses.
pub fn list_tasks_sql(select: &str, listing: &TaskListing<'_>) -> AppResult<String> {
    let mut query = String::from(select);
    let mut conditions = Vec::new();
    let mut bind_count = 0;

    for (present, column) in [
        (listing.client_id.is_some(), "session_token = $"),
        (listing.status.is_some(), "status = $"),
        (listing.context_id.is_some(), "context_id = $"),
        (listing.updated_after.is_some(), "updated_at > $"),
    ] {
        if present {
            bind_count += 1;
            conditions.push(format!("{column}{bind_count}"));
        }
    }
    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }

    // A2A 1.0 ListTasks: ordered by status timestamp, most recent first.
    query.push_str(" ORDER BY updated_at DESC");

    for (present, clause) in [
        (listing.limit.is_some(), " LIMIT $"),
        (listing.offset.is_some(), " OFFSET $"),
    ] {
        if present {
            bind_count += 1;
            write!(query, "{clause}{bind_count}")
                .map_err(|e| AppError::internal(format!("Failed to write task listing: {e}")))?;
        }
    }
    Ok(query)
}

/// Emit the whole [`A2ARepository`](super::a2a::A2ARepository) implementation
/// for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, its driver's row type, its uuid codec from
/// [`super::uuid_columns`], its list codec from [`super::list_columns`], and
/// the three spellings only it knows: the UTC calendar day of a usage row
/// (`day`), the expression that mints a usage row's id (`usage_id`), and
/// the cast an IP address needs (`inet`). The row parsers are emitted inside
/// the macro because the id and list reads are what differs per driver;
/// sqlx resolves the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_a2a_repository {
    (
        $ty:ty, $row:ty, $ids:ident, $lists:ident,
        day = $day:literal, usage_id = $usage_id:literal, inet = $inet:literal
    ) => {
        /// A client row. `permissions` has no column and reads as its
        /// default; the rate-limit pair is stored in `rate_limit_per_minute`
        /// / `rate_limit_per_day`.
        fn client_from_row(row: &$row) -> AppResult<A2AClient> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            let per_minute: i32 = row
                .try_get("rate_limit_per_minute")
                .map_err(|e| a2a_column_error("rate_limit_per_minute", &e))?;
            let per_day: i32 = row
                .try_get("rate_limit_per_day")
                .map_err(|e| a2a_column_error("rate_limit_per_day", &e))?;
            Ok(A2AClient {
                id: col("client_id")?,
                user_id: $ids::read(row, "user_id")?,
                name: col("name")?,
                description: col("description")?,
                public_key: col("api_key_hash")?,
                capabilities: $lists::read_json(row, "capabilities")?,
                redirect_uris: $lists::read_json(row, "redirect_uris")?,
                is_active: row
                    .try_get("is_active")
                    .map_err(|e| a2a_column_error("is_active", &e))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| a2a_column_error("created_at", &e))?,
                permissions: vec!["read_activities".to_owned()],
                rate_limit_requests: u32_from_count(i64::from(per_minute), "rate_limit_per_minute")?,
                rate_limit_window_seconds: u32_from_count(
                    i64::from(per_day),
                    "rate_limit_per_day",
                )?,
                updated_at: row
                    .try_get("updated_at")
                    .map_err(|e| a2a_column_error("updated_at", &e))?,
            })
        }

        /// A session row. `requests_count` has no column and reads as zero.
        fn session_from_row(row: &$row) -> AppResult<A2ASession> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            let stamp = |name: &str| -> AppResult<DateTime<Utc>> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            Ok(A2ASession {
                id: col("session_token")?,
                client_id: col("client_id")?,
                user_id: $ids::read_opt(row, "user_id")?,
                granted_scopes: $lists::read_csv(row, "granted_scopes")?,
                created_at: stamp("created_at")?,
                expires_at: stamp("expires_at")?,
                last_activity: stamp("last_active_at")?,
                requests_count: 0,
            })
        }

        /// A task row. The JSON columns decode as [`Value`] on both engines;
        /// `client_id` has no column and reads from the session token, which
        /// carries the client id for a task created without a session.
        fn task_from_row(row: &$row) -> AppResult<A2ATask> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            let json = |name: &str| -> AppResult<Option<Value>> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            let status = col("status")?;
            Ok(A2ATask {
                id: col("task_id")?,
                client_id: col("session_token")?,
                task_type: col("task_type")?,
                input_data: row
                    .try_get("parameters")
                    .map_err(|e| a2a_column_error("parameters", &e))?,
                status: str_to_task_status(&status),
                result: json("result")?,
                context_id: row
                    .try_get("context_id")
                    .map_err(|e| a2a_column_error("context_id", &e))?,
                status_message: json("status_message")?,
                history: json("history")?,
                artifacts: json("artifacts")?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| a2a_column_error("created_at", &e))?,
                updated_at: row
                    .try_get("updated_at")
                    .map_err(|e| a2a_column_error("updated_at", &e))?,
            })
        }

        /// A push-notification config row.
        fn push_config_from_row(row: &$row) -> AppResult<A2APushNotificationConfig> {
            let col = |name: &str| -> AppResult<String> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            let opt = |name: &str| -> AppResult<Option<String>> {
                row.try_get(name).map_err(|e| a2a_column_error(name, &e))
            };
            Ok(A2APushNotificationConfig {
                config_id: col("config_id")?,
                task_id: col("task_id")?,
                url: col("url")?,
                token: opt("token")?,
                auth_scheme: opt("auth_scheme")?,
                auth_credentials: opt("auth_credentials")?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| a2a_column_error("created_at", &e))?,
                updated_at: row
                    .try_get("updated_at")
                    .map_err(|e| a2a_column_error("updated_at", &e))?,
            })
        }

        #[async_trait::async_trait]
        impl A2ARepository for $ty {
            async fn create_client(
                &self,
                client: &A2AClient,
                client_secret: &str,
                api_key_id: &str,
            ) -> AppResult<String> {
                // One transaction: the client row and its API-key association
                // land together or not at all, so API-key auth can never find
                // a client the association does not name.
                let tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
                let mut guard = TransactionGuard::new(tx);

                // Hash the client secret before storage (never store plaintext secrets).
                let secret_hash = format!("{:x}", Sha256::digest(client_secret.as_bytes()));

                sqlx::query(INSERT_CLIENT_SQL)
                    .bind(&client.id)
                    .bind($ids::bind(client.user_id))
                    .bind(&client.name)
                    .bind(&client.description)
                    .bind(&client.public_key)
                    .bind(&secret_hash)
                    .bind($lists::bind_json(&client.capabilities))
                    .bind($lists::bind_json(&client.redirect_uris))
                    .bind(i32_from_u32(client.rate_limit_requests)?)
                    .bind(i32_from_u32(client.rate_limit_window_seconds)?)
                    .bind(client.is_active)
                    .bind(client.created_at)
                    .bind(client.updated_at)
                    .execute(guard.executor()?)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert A2A client: {e}")))?;

                sqlx::query(INSERT_CLIENT_API_KEY_SQL)
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

                guard.commit().await?;

                debug!(
                    client_id = %client.id,
                    api_key_id = %api_key_id,
                    "Created A2A client with API key association"
                );
                Ok(client.id.clone())
            }

            async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
                let row = sqlx::query(GET_CLIENT_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to query A2A client: {e}")))?;
                row.as_ref().map(client_from_row).transpose()
            }

            async fn get_client_by_api_key_id(
                &self,
                api_key_id: &str,
            ) -> AppResult<Option<A2AClient>> {
                let row = sqlx::query(GET_CLIENT_BY_API_KEY_SQL)
                    .bind(api_key_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query A2A client by API key: {e}"))
                    })?;
                row.as_ref().map(client_from_row).transpose()
            }

            async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
                let row = sqlx::query(GET_CLIENT_BY_NAME_SQL)
                    .bind(name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query A2A client by name: {e}"))
                    })?;
                row.as_ref().map(client_from_row).transpose()
            }

            async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>> {
                // The nil uuid is the system-wide listing
                // (ClientManager::list_all_clients).
                let rows = if user_id == &Uuid::nil() {
                    sqlx::query(LIST_ALL_CLIENTS_SQL).fetch_all(self.pool()).await
                } else {
                    sqlx::query(LIST_USER_CLIENTS_SQL)
                        .bind($ids::bind(*user_id))
                        .fetch_all(self.pool())
                        .await
                }
                .map_err(|e| AppError::database(format!("Failed to list A2A clients: {e}")))?;
                rows.iter().map(client_from_row).collect()
            }

            async fn deactivate_client(&self, client_id: &str) -> AppResult<()> {
                let result = sqlx::query(DEACTIVATE_CLIENT_SQL)
                    .bind(Utc::now())
                    .bind(client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deactivate A2A client: {e}"))
                    })?;
                (result.rows_affected() > 0)
                    .ok_or_else(|| AppError::not_found(format!("A2A client: {client_id}")))
            }

            async fn get_client_credentials(
                &self,
                client_id: &str,
            ) -> AppResult<Option<(String, String)>> {
                let row = sqlx::query(CLIENT_CREDENTIALS_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query A2A client credentials: {e}"))
                    })?;
                row.as_ref()
                    .map(|r| -> AppResult<(String, String)> {
                        let id: String = r
                            .try_get("client_id")
                            .map_err(|e| a2a_column_error("client_id", &e))?;
                        let secret: String = r
                            .try_get("client_secret_hash")
                            .map_err(|e| a2a_column_error("client_secret_hash", &e))?;
                        Ok((id, secret))
                    })
                    .transpose()
            }

            async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()> {
                sqlx::query(INVALIDATE_CLIENT_SESSIONS_SQL)
                    .bind(Utc::now() - Duration::hours(1))
                    .bind(client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to invalidate A2A client sessions: {e}"))
                    })?;
                Ok(())
            }

            async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
                sqlx::query(DEACTIVATE_CLIENT_API_KEYS_SQL)
                    .bind(client_id)
                    .execute(self.pool())
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
                let session_token = format!("sess_{}", Uuid::new_v4());
                let now = Utc::now();
                sqlx::query(CREATE_SESSION_SQL)
                    .bind(&session_token)
                    .bind(client_id)
                    .bind($ids::bind_opt(user_id.copied()))
                    .bind($lists::bind_csv(granted_scopes))
                    .bind(now + Duration::hours(expires_in_hours))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create A2A session: {e}")))?;
                Ok(session_token)
            }

            async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
                let row = sqlx::query(GET_SESSION_SQL)
                    .bind(session_token)
                    .bind(Utc::now())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to query A2A session: {e}")))?;
                row.as_ref().map(session_from_row).transpose()
            }

            async fn update_session_activity(&self, session_token: &str) -> AppResult<()> {
                sqlx::query(TOUCH_SESSION_SQL)
                    .bind(Utc::now())
                    .bind(session_token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update A2A session activity: {e}"))
                    })?;
                Ok(())
            }

            async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
                let rows = sqlx::query(ACTIVE_SESSIONS_SQL)
                    .bind(client_id)
                    .bind(Utc::now())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query active A2A sessions: {e}"))
                    })?;
                rows.iter().map(session_from_row).collect()
            }

            async fn create_task(
                &self,
                client_id: &str,
                session_id: Option<&str>,
                task_type: &str,
                input_data: &Value,
                context_id: Option<&str>,
            ) -> AppResult<String> {
                let task_id = format!("task_{}", Uuid::new_v4());
                // a2a_tasks is session-keyed: the session token when there is
                // one, else the client id stands in for it.
                let session_token = session_id.unwrap_or(client_id);
                sqlx::query(CREATE_TASK_SQL)
                    .bind(&task_id)
                    .bind(session_token)
                    .bind(task_type)
                    .bind(input_data)
                    .bind(task_status_to_str(&TaskStatus::Submitted))
                    .bind(context_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create A2A task: {e}")))?;
                Ok(task_id)
            }

            async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
                let row = sqlx::query(GET_TASK_SQL)
                    .bind(task_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to query A2A task: {e}")))?;
                row.as_ref().map(task_from_row).transpose()
            }

            async fn list_tasks(
                &self,
                client_id: Option<&str>,
                status_filter: Option<&TaskStatus>,
                context_id: Option<&str>,
                updated_after: Option<DateTime<Utc>>,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<A2ATask>> {
                let listing = TaskListing {
                    client_id,
                    status: status_filter,
                    context_id,
                    updated_after,
                    limit,
                    offset,
                };
                let sql = list_tasks_sql(
                    concat!("SELECT ", task_columns!(), " FROM a2a_tasks"),
                    &listing,
                )?;
                let mut query = sqlx::query(&sql);
                if let Some(client_id) = client_id {
                    query = query.bind(client_id);
                }
                if let Some(status) = status_filter {
                    query = query.bind(task_status_to_str(status));
                }
                if let Some(context_id) = context_id {
                    query = query.bind(context_id);
                }
                if let Some(updated_after) = updated_after {
                    query = query.bind(updated_after);
                }
                if let Some(limit) = limit {
                    query = query.bind(i64::from(limit));
                }
                if let Some(offset) = offset {
                    query = query.bind(i64::from(offset));
                }
                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to query A2A tasks: {e}")))?;
                rows.iter().map(task_from_row).collect()
            }

            async fn update_task_status(
                &self,
                task_id: &str,
                status: &TaskStatus,
                result: Option<&Value>,
                status_message: Option<&Value>,
            ) -> AppResult<()> {
                sqlx::query(UPDATE_TASK_STATUS_SQL)
                    .bind(task_id)
                    .bind(task_status_to_str(status))
                    .bind(result)
                    .bind(status_message)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update A2A task status: {e}"))
                    })?;
                Ok(())
            }

            async fn update_task_wire_state(
                &self,
                task_id: &str,
                history: Option<&Value>,
                artifacts: Option<&Value>,
            ) -> AppResult<()> {
                sqlx::query(UPDATE_TASK_WIRE_STATE_SQL)
                    .bind(task_id)
                    .bind(history)
                    .bind(artifacts)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update A2A task wire state: {e}"))
                    })?;
                Ok(())
            }

            async fn create_push_config(&self, config: &A2APushNotificationConfig) -> AppResult<()> {
                sqlx::query(UPSERT_PUSH_CONFIG_SQL)
                    .bind(&config.config_id)
                    .bind(&config.task_id)
                    .bind(&config.url)
                    .bind(&config.token)
                    .bind(&config.auth_scheme)
                    .bind(&config.auth_credentials)
                    .bind(config.created_at)
                    .bind(config.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create A2A push config: {e}"))
                    })?;
                Ok(())
            }

            async fn get_push_config(
                &self,
                task_id: &str,
                config_id: &str,
            ) -> AppResult<Option<A2APushNotificationConfig>> {
                let row = sqlx::query(GET_PUSH_CONFIG_SQL)
                    .bind(task_id)
                    .bind(config_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get A2A push config: {e}")))?;
                row.as_ref().map(push_config_from_row).transpose()
            }

            async fn list_push_configs(
                &self,
                task_id: &str,
            ) -> AppResult<Vec<A2APushNotificationConfig>> {
                let rows = sqlx::query(LIST_PUSH_CONFIGS_SQL)
                    .bind(task_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list A2A push configs: {e}"))
                    })?;
                rows.iter().map(push_config_from_row).collect()
            }

            async fn delete_push_config(&self, task_id: &str, config_id: &str) -> AppResult<bool> {
                let result = sqlx::query(DELETE_PUSH_CONFIG_SQL)
                    .bind(task_id)
                    .bind(config_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete A2A push config: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()> {
                sqlx::query(record_usage_sql!($usage_id, $inet))
                    .bind(&usage.client_id)
                    .bind(&usage.session_token)
                    .bind(usage.timestamp)
                    .bind(&usage.tool_name)
                    .bind(usage.response_time_ms.map(i32_from_u32).transpose()?)
                    .bind(i32::from(usage.status_code))
                    .bind(None::<String>)
                    .bind(usage.request_size_bytes.map(i32_from_u32).transpose()?)
                    .bind(usage.response_size_bytes.map(i32_from_u32).transpose()?)
                    .bind(&usage.ip_address)
                    .bind(&usage.user_agent)
                    .bind(&usage.protocol_version)
                    .bind($lists::bind_json(&usage.client_capabilities))
                    .bind($lists::bind_json(&usage.granted_scopes))
                    .execute(self.pool())
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
                // The window is the client's own rate_limit_window_seconds.
                let client = A2ARepository::get_client(self, client_id)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("A2A client: {client_id}")))?;
                let window_start =
                    Utc::now() - Duration::seconds(i64::from(client.rate_limit_window_seconds));
                let count: i64 = sqlx::query_scalar(CLIENT_USAGE_SINCE_SQL)
                    .bind(client_id)
                    .bind(window_start)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query A2A client usage count: {e}"))
                    })?;
                u32_from_count(count, "current_usage")
            }

            async fn get_usage_stats(
                &self,
                client_id: &str,
                start_date: DateTime<Utc>,
                end_date: DateTime<Utc>,
            ) -> AppResult<A2AUsageStats> {
                let row = sqlx::query(USAGE_STATS_SQL)
                    .bind(client_id)
                    .bind(start_date)
                    .bind(end_date)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to query A2A usage stats: {e}")))?;
                let count = |name: &str| -> AppResult<u32> {
                    let value: i64 = row.try_get(name).map_err(|e| a2a_column_error(name, &e))?;
                    u32_from_count(value, name)
                };
                let sum = |name: &str| -> AppResult<Option<u64>> {
                    let value: Option<i64> =
                        row.try_get(name).map_err(|e| a2a_column_error(name, &e))?;
                    value.map(|v| u64_from_sum(v, name)).transpose()
                };
                let avg_response_time: Option<f64> = row
                    .try_get("avg_response_time")
                    .map_err(|e| a2a_column_error("avg_response_time", &e))?;
                Ok(A2AUsageStats {
                    client_id: client_id.to_owned(),
                    period_start: start_date,
                    period_end: end_date,
                    total_requests: count("total_requests")?,
                    successful_requests: count("successful_requests")?,
                    failed_requests: count("failed_requests")?,
                    avg_response_time_ms: avg_response_time.map(average_millis),
                    total_request_bytes: sum("total_request_bytes")?,
                    total_response_bytes: sum("total_response_bytes")?,
                })
            }

            async fn get_client_usage_history(
                &self,
                client_id: &str,
                days: u32,
            ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>> {
                let rows = sqlx::query(usage_history_sql!($day))
                    .bind(client_id)
                    .bind(Utc::now() - Duration::days(i64::from(days)))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query A2A client usage history: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        let day: NaiveDate = row
                            .try_get("usage_date")
                            .map_err(|e| a2a_column_error("usage_date", &e))?;
                        let count = |name: &str| -> AppResult<u32> {
                            let value: i64 =
                                row.try_get(name).map_err(|e| a2a_column_error(name, &e))?;
                            u32_from_count(value, name)
                        };
                        Ok((
                            day.and_time(NaiveTime::MIN).and_utc(),
                            count("success_count")?,
                            count("error_count")?,
                        ))
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_a2a_repository;
