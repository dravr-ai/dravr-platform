// ABOUTME: Per-backend seed codecs, row helpers and the shared SeederRepository body over the statements in seeder.rs
// ABOUTME: Each backend shell invokes impl_seeder_repository! with its type, uuid codec, seed codec and usage-id clause
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The seeder's body.
//!
//! The statements it binds and the trait it implements are in
//! [`super::seeder`], whose module doc explains the three things the two
//! backends store differently; the codecs that carry two of them are here.

use pierre_core::errors::{AppError, AppResult};
use sqlx::query::Query;
use sqlx::sqlite::{Sqlite, SqliteArguments};
use uuid::Uuid;

#[cfg(feature = "postgresql")]
use sqlx::postgres::{PgArguments, Postgres};

// ============================================================================
// Per-backend seed codecs
// ============================================================================

/// The list a seed's `capabilities` JSON must parse to.
fn capabilities_list(json: &str) -> AppResult<Vec<String>> {
    serde_json::from_str(json)
        .map_err(|e| AppError::invalid_input(format!("seed capabilities are not a JSON list: {e}")))
}

/// The `SQLite` seed codec: the usage tables key on the seed's uuid and
/// `a2a_clients.capabilities` is JSON text.
pub struct TextSeedColumns;

impl TextSeedColumns {
    /// `api_key_usage.id` / `a2a_usage.id` are `TEXT PRIMARY KEY` with no
    /// default here, so the seed's uuid is bound as the row's key.
    pub(crate) fn bind_usage_id<'q>(
        query: Query<'q, Sqlite, SqliteArguments<'q>>,
        id: Uuid,
    ) -> Query<'q, Sqlite, SqliteArguments<'q>> {
        query.bind(id.to_string())
    }

    /// `a2a_clients.capabilities` is `TEXT` holding the JSON list: the seed's
    /// text binds as it is, once it has been checked to be a list, so a
    /// malformed seed is refused here exactly as [`NativeSeedColumns`]
    /// refuses it.
    ///
    /// # Errors
    /// Returns an invalid-input error when the text is not a JSON list.
    pub(crate) fn bind_capabilities(json: &str) -> AppResult<&str> {
        capabilities_list(json).map(|_| json)
    }
}

/// The Postgres seed codec: the usage tables mint their own `SERIAL` key
/// and `a2a_clients.capabilities` is a native `TEXT[]`.
///
/// Gated with the backend that uses it: without the `postgresql` feature the
/// Postgres shell is not compiled, so neither is its half of the seam.
#[cfg(feature = "postgresql")]
pub struct NativeSeedColumns;

#[cfg(feature = "postgresql")]
impl NativeSeedColumns {
    /// `api_key_usage.id` / `a2a_usage.id` are `SERIAL` here: the table
    /// mints the key, so the seed's uuid is not sent and the query is
    /// returned as it came.
    pub(crate) const fn bind_usage_id(
        query: Query<'_, Postgres, PgArguments>,
        _: Uuid,
    ) -> Query<'_, Postgres, PgArguments> {
        query
    }

    /// `a2a_clients.capabilities` is `TEXT[]`: the seed's JSON list binds as
    /// the parsed list.
    ///
    /// # Errors
    /// Returns an invalid-input error when the text is not a JSON list.
    pub(crate) fn bind_capabilities(json: &str) -> AppResult<Vec<String>> {
        capabilities_list(json)
    }
}

// ============================================================================
// Row helpers
// ============================================================================

/// Read one column by name via `try_get`, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error naming the column.
pub(crate) fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("seed column {name}: {e}")))
}

/// Parse a text id column into a [`Uuid`]: the api-key and A2A client ids
/// are text on both backends.
pub(crate) fn uuid_text_column<R>(row: &R, name: &str, what: &str) -> AppResult<Uuid>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let raw: String = column(row, name)?;
    raw.parse::<Uuid>()
        .map_err(|e| AppError::database(format!("Failed to parse {what} ID: {e}")))
}

/// The `(first, content_hash)` pair the two agent lookups return.
pub(crate) fn text_and_hash<R>(row: &R, first: &str) -> AppResult<(String, Option<String>)>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok((column(row, first)?, column(row, "content_hash")?))
}

/// The seeder's `(id, slug)` catalogue listing row.
pub(crate) fn id_and_slug<R>(row: &R) -> AppResult<(String, String)>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok((column(row, "id")?, column(row, "slug")?))
}

/// Serialise one seed field to the JSON text its column holds.
pub(crate) fn json_field<T: serde::Serialize>(value: &T, name: &str) -> AppResult<String> {
    serde_json::to_string(value)
        .map_err(|e| AppError::internal(format!("Failed to serialize {name}: {e}")))
}

/// A seed's unsigned count as the `INTEGER` the column holds on both.
pub(crate) fn int_column(value: u32, name: &str) -> AppResult<i32> {
    i32::try_from(value)
        .map_err(|e| AppError::invalid_input(format!("seed {name} does not fit INTEGER: {e}")))
}

/// A seed's HTTP status as the `SMALLINT` Postgres declares; `SQLite`'s
/// `INTEGER` takes the same value.
pub(crate) fn status_code_column(value: i32) -> AppResult<i16> {
    i16::try_from(value).map_err(|e| {
        AppError::invalid_input(format!(
            "seed status_code {value} is not an HTTP status: {e}"
        ))
    })
}

// ============================================================================
// The shared body
// ============================================================================

/// Emit the whole [`SeederRepository`] implementation for one backend type.
///
/// - `$ids` is the backend's uuid codec from [`super::uuid_columns`]
///   (`TextUuid` on `SQLite`, `NativeUuid` on Postgres);
/// - `$seed` is its seed codec ([`TextSeedColumns`] / [`NativeSeedColumns`]);
/// - `$usage_id_col` / `$usage_id_val` are the usage-id clause literals
///   [`insert_api_key_usage_sql!`] and [`insert_a2a_usage_sql!`] take.
///
/// The body is written once here; each backend's shell invokes it with its
/// own arguments, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts, helpers and types unqualified, so
/// the invoking shell must `use` every one of them.
macro_rules! impl_seeder_repository {
    ($ty:ty, $ids:ident, $seed:ident, $usage_id_col:literal, $usage_id_val:literal) => {
        #[async_trait::async_trait]
        impl SeederRepository for $ty {
            async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64> {
                let sql = format!("DELETE FROM {}", table.table_name());
                let result = sqlx::query(&sql).execute(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to reset table {}: {e}", table.table_name()))
                })?;
                Ok(result.rows_affected())
            }

            async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64> {
                let sql = format!("SELECT COUNT(*) as cnt FROM {}", table.table_name());
                let row = sqlx::query(&sql)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count table {}: {e}",
                            table.table_name()
                        ))
                    })?;
                column(&row, "cnt")
            }

            async fn seed_upsert_stretching_exercise(
                &self,
                exercise: &StretchingExercise,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_STRETCHING_EXERCISE_SQL)
                    .bind(&exercise.id)
                    .bind(&exercise.name)
                    .bind(&exercise.description)
                    .bind(exercise.category.as_str())
                    .bind(exercise.difficulty.as_str())
                    .bind(json_field(&exercise.primary_muscles, "primary_muscles")?)
                    .bind(json_field(
                        &exercise.secondary_muscles,
                        "secondary_muscles",
                    )?)
                    .bind(int_column(exercise.duration_seconds, "duration_seconds")?)
                    .bind(
                        exercise
                            .repetitions
                            .map(|r| int_column(r, "repetitions"))
                            .transpose()?,
                    )
                    .bind(int_column(exercise.sets, "sets")?)
                    .bind(json_field(
                        &exercise.recommended_for_activities,
                        "recommended_for_activities",
                    )?)
                    .bind(json_field(
                        &exercise.contraindications,
                        "contraindications",
                    )?)
                    .bind(json_field(&exercise.instructions, "instructions")?)
                    .bind(json_field(&exercise.cues, "cues")?)
                    .bind(&exercise.image_url)
                    .bind(&exercise.video_url)
                    .bind(exercise.created_at)
                    .bind(exercise.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert stretching exercise: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()> {
                sqlx::query(UPSERT_YOGA_POSE_SQL)
                    .bind(&pose.id)
                    .bind(&pose.english_name)
                    .bind(&pose.sanskrit_name)
                    .bind(&pose.description)
                    .bind(json_field(&pose.benefits, "benefits")?)
                    .bind(pose.category.as_str())
                    .bind(pose.difficulty.as_str())
                    .bind(pose.pose_type.as_str())
                    .bind(json_field(&pose.primary_muscles, "primary_muscles")?)
                    .bind(json_field(&pose.secondary_muscles, "secondary_muscles")?)
                    .bind(json_field(&pose.chakras, "chakras")?)
                    .bind(int_column(
                        pose.hold_duration_seconds,
                        "hold_duration_seconds",
                    )?)
                    .bind(&pose.breath_guidance)
                    .bind(json_field(
                        &pose.recommended_for_activities,
                        "recommended_for_activities",
                    )?)
                    .bind(json_field(
                        &pose.recommended_for_recovery,
                        "recommended_for_recovery",
                    )?)
                    .bind(json_field(&pose.contraindications, "contraindications")?)
                    .bind(json_field(&pose.instructions, "instructions")?)
                    .bind(json_field(&pose.modifications, "modifications")?)
                    .bind(json_field(&pose.progressions, "progressions")?)
                    .bind(json_field(&pose.cues, "cues")?)
                    .bind(json_field(&pose.warmup_poses, "warmup_poses")?)
                    .bind(json_field(&pose.followup_poses, "followup_poses")?)
                    .bind(&pose.image_url)
                    .bind(&pose.video_url)
                    .bind(pose.created_at)
                    .bind(pose.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to upsert yoga pose: {e}")))?;
                Ok(())
            }

            async fn seed_upsert_activity_mapping(
                &self,
                mapping: &ActivityMuscleMapping,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_ACTIVITY_MAPPING_SQL)
                    .bind(&mapping.id)
                    .bind(&mapping.activity_type)
                    .bind(json_field(&mapping.primary_muscles, "primary_muscles")?)
                    .bind(json_field(&mapping.secondary_muscles, "secondary_muscles")?)
                    .bind(json_field(
                        &mapping.recommended_stretch_categories,
                        "recommended_stretch_categories",
                    )?)
                    .bind(json_field(
                        &mapping.recommended_yoga_categories,
                        "recommended_yoga_categories",
                    )?)
                    .bind(mapping.created_at)
                    .bind(mapping.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert activity mapping: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_get_admin_user(&self) -> AppResult<Option<User>> {
                let email: Option<String> = sqlx::query_scalar(ADMIN_USER_EMAIL_SQL)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to find admin user: {e}")))?;
                match email {
                    Some(email) => self.get_by_email(&email).await,
                    None => Ok(None),
                }
            }

            async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>> {
                let row = sqlx::query(USER_TENANT_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user tenant: {e}")))?;
                row.as_ref()
                    .map(|r| column::<_, Option<String>>(r, "tenant_id"))
                    .transpose()
                    .map(Option::flatten)
            }

            async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>> {
                self.get_by_email(email).await
            }

            async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>> {
                let rows = sqlx::query(NON_ADMIN_USER_IDS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get non-admin user IDs: {e}"))
                    })?;
                rows.iter().map(|r| $ids::read(r, "id")).collect()
            }

            async fn seed_count_non_admin_users(&self) -> AppResult<i64> {
                let row = sqlx::query(COUNT_NON_ADMIN_USERS_SQL)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count non-admin users: {e}"))
                    })?;
                column(&row, "cnt")
            }

            async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64> {
                let result = sqlx::query(DELETE_LLM_USAGE_BY_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete LLM usage by tenant: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()> {
                sqlx::query(INSERT_LLM_USAGE_SQL)
                    .bind(record.id.to_string())
                    .bind(record.tenant_id.to_string())
                    .bind(record.user_id.to_string())
                    .bind(&record.conversation_id)
                    .bind(&record.provider)
                    .bind(&record.model)
                    .bind(record.prompt_tokens)
                    .bind(record.completion_tokens)
                    .bind(record.total_tokens)
                    .bind(&record.call_type)
                    .bind(record.tool_calls_count)
                    .bind(record.execution_time_ms)
                    .bind(record.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert LLM usage record: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64> {
                let result = sqlx::query(DELETE_SYNTHETIC_BY_USER_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to delete synthetic activities by user: {e}"
                        ))
                    })?;
                Ok(result.rows_affected())
            }

            async fn seed_insert_synthetic_activity(
                &self,
                activity: &SeedSyntheticActivity,
            ) -> AppResult<()> {
                sqlx::query(INSERT_SYNTHETIC_ACTIVITY_SQL)
                    .bind(activity.id.to_string())
                    .bind($ids::bind(activity.user_id))
                    .bind(activity.tenant_id.to_string())
                    .bind(&activity.name)
                    .bind(&activity.sport_type)
                    .bind(activity.start_date)
                    .bind(activity.duration_seconds)
                    .bind(activity.distance_meters)
                    .bind(activity.elevation_gain)
                    .bind(activity.average_heart_rate)
                    .bind(activity.max_heart_rate)
                    .bind(activity.average_speed)
                    .bind(activity.max_speed)
                    .bind(activity.calories)
                    .bind(&activity.city)
                    .bind(&activity.region)
                    .bind(&activity.country)
                    .bind(activity.created_at)
                    .bind(activity.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert synthetic activity: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_upsert_provider_connection(
                &self,
                conn: &SeedProviderConnection,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_PROVIDER_CONNECTION_SQL)
                    .bind(conn.id.to_string())
                    .bind(conn.user_id.to_string())
                    .bind(conn.tenant_id.to_string())
                    .bind(&conn.provider)
                    .bind(&conn.connection_type)
                    .bind(conn.connected_at)
                    .bind(&conn.metadata)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert provider connection: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>> {
                let row = sqlx::query(USER_ID_BY_EMAIL_SQL)
                    .bind(email)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check user existence: {e}"))
                    })?;
                row.as_ref().map(|r| $ids::read(r, "id")).transpose()
            }

            async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()> {
                // A suspended demo account is inactive; every other status is
                // live, and an active one is approved from its creation.
                let is_active = user.status != "suspended";
                let approved_at = (user.status == "active").then_some(user.created_at);

                // The only user any seeder marks is_admin is the bootstrap operator, and the
                // operator must be super_admin: cookie_admin_middleware derives console
                // permissions from the role column, and super_admin is the tier that unlocks
                // config/user/impersonation/device-login approval. This mirrors the local
                // setup script, which creates the operator with `user create --super-admin`.
                let role = if user.is_admin { "super_admin" } else { "user" };

                sqlx::query(UPSERT_DEMO_USER_SQL)
                    .bind($ids::bind(user.id))
                    .bind(&user.email)
                    .bind(&user.display_name)
                    .bind(&user.password_hash)
                    .bind(&user.tier)
                    .bind(is_active)
                    .bind(&user.status)
                    .bind(user.is_admin)
                    .bind(role)
                    .bind(approved_at)
                    .bind(user.created_at)
                    .bind(&user.locale)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert demo user: {e}")))?;
                Ok(())
            }

            async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()> {
                sqlx::query(UPSERT_TENANT_SQL)
                    .bind($ids::bind(tenant.id))
                    .bind(&tenant.name)
                    .bind(&tenant.slug)
                    .bind(&tenant.plan)
                    .bind(tenant.created_at)
                    .bind(tenant.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert tenant: {e}")))?;
                Ok(())
            }

            async fn seed_insert_tenant_user(
                &self,
                id: Uuid,
                tenant_id: Uuid,
                user_id: Uuid,
                now: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(INSERT_TENANT_USER_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind(tenant_id))
                    .bind($ids::bind(user_id))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert tenant user: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_update_user_tenant(
                &self,
                user_id: Uuid,
                tenant_id: Uuid,
            ) -> AppResult<()> {
                sqlx::query(UPDATE_USER_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user tenant: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
                let row = sqlx::query(API_KEY_ID_BY_NAME_SQL)
                    .bind(name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check API key by name: {e}"))
                    })?;
                row.as_ref()
                    .map(|r| uuid_text_column(r, "id", "API key"))
                    .transpose()
            }

            async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()> {
                // A seed without a limit is an unlimited enterprise key, stored as
                // i32::MAX exactly as ApiKeyRepository::create stores u32::MAX; the
                // Postgres column is NOT NULL, so unlimited cannot be NULL there.
                let rate_limit_requests = key.rate_limit.unwrap_or(i32::MAX);
                sqlx::query(INSERT_API_KEY_SQL)
                    .bind(key.id.to_string())
                    .bind($ids::bind(key.user_id))
                    .bind(&key.name)
                    .bind(&key.description)
                    .bind(&key.key_hash)
                    .bind(&key.key_prefix)
                    .bind(&key.tier)
                    .bind(rate_limit_requests)
                    .bind(key.expires_at)
                    .bind(key.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert API key: {e}")))?;
                Ok(())
            }

            async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
                let row = sqlx::query(A2A_CLIENT_ID_BY_NAME_SQL)
                    .bind(name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check A2A client by name: {e}"))
                    })?;
                row.as_ref()
                    .map(|r| uuid_text_column(r, "client_id", "A2A client"))
                    .transpose()
            }

            async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()> {
                sqlx::query(INSERT_A2A_CLIENT_SQL)
                    .bind(client.id.to_string())
                    .bind($ids::bind(client.user_id))
                    .bind(&client.name)
                    .bind(&client.description)
                    .bind(&client.public_key)
                    .bind(&client.client_secret)
                    .bind($seed::bind_capabilities(&client.capabilities)?)
                    .bind(client.created_at)
                    .bind(client.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert A2A client: {e}")))?;
                Ok(())
            }

            async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()> {
                let query = sqlx::query(insert_api_key_usage_sql!($usage_id_col, $usage_id_val))
                    .bind(usage.api_key_id.to_string())
                    .bind(usage.timestamp)
                    .bind(&usage.tool_name)
                    .bind(status_code_column(usage.status_code)?)
                    .bind(usage.response_time_ms);
                $seed::bind_usage_id(query, usage.id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert API key usage: {e}"))
                    })?;
                Ok(())
            }

            async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()> {
                let query = sqlx::query(insert_a2a_usage_sql!($usage_id_col, $usage_id_val))
                    .bind(usage.client_id.to_string())
                    .bind(usage.timestamp)
                    .bind(&usage.tool_name)
                    .bind(status_code_column(usage.status_code)?)
                    .bind(usage.response_time_ms);
                $seed::bind_usage_id(query, usage.id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert A2A usage: {e}")))?;
                Ok(())
            }

            async fn seed_find_agent_by_slug(
                &self,
                slug: &str,
                tenant_id: &str,
            ) -> AppResult<Option<(String, Option<String>)>> {
                let row = sqlx::query(AGENT_BY_SLUG_SQL)
                    .bind(slug)
                    .bind($ids::bind_text(tenant_id)?)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find coach by slug: {e}"))
                    })?;
                row.as_ref().map(|r| text_and_hash(r, "id")).transpose()
            }

            async fn seed_find_agent_drift_info(
                &self,
                slug: &str,
            ) -> AppResult<Option<(String, Option<String>)>> {
                let row = sqlx::query(AGENT_DRIFT_INFO_SQL)
                    .bind(slug)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch coach drift info: {e}"))
                    })?;
                row.as_ref().map(|r| text_and_hash(r, "source")).transpose()
            }

            async fn seed_list_catalogue_agents(
                &self,
                tenant_id: &str,
            ) -> AppResult<Vec<(String, String)>> {
                let rows = sqlx::query(CATALOGUE_AGENTS_SQL)
                    .bind($ids::bind_text(tenant_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list catalogue coaches: {e}"))
                    })?;
                rows.iter().map(id_and_slug).collect()
            }

            async fn seed_repoint_agent_references(
                &self,
                retired_agent_id: &str,
                successor_agent_id: &str,
            ) -> AppResult<u64> {
                let mut moved = 0u64;
                for statement in AGENT_POINTER_REWRITES.iter().chain(&AGENT_POINTER_MERGES) {
                    let result = sqlx::query(statement)
                        .bind(successor_agent_id)
                        .bind(retired_agent_id)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to re-point agent references: {e}"))
                        })?;
                    moved += result.rows_affected();
                }
                sqlx::query(AGENT_INSTALL_COUNT_RESYNC)
                    .bind(successor_agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to resync the agent install count: {e}"))
                    })?;
                Ok(moved)
            }

            async fn seed_repoint_agent_slug_references(
                &self,
                retired_slug: &str,
                successor_slug: &str,
            ) -> AppResult<u64> {
                let mut moved = 0u64;
                for statement in AGENT_SLUG_REWRITES {
                    let result = sqlx::query(statement)
                        .bind(successor_slug)
                        .bind(retired_slug)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to re-point agent slug references: {e}"
                            ))
                        })?;
                    moved += result.rows_affected();
                }
                Ok(moved)
            }

            async fn seed_detach_agent_conversations(
                &self,
                retired_agent_id: &str,
            ) -> AppResult<u64> {
                let result = sqlx::query(DETACH_AGENT_CONVERSATIONS_SQL)
                    .bind(retired_agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to detach coach conversations: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn seed_take_catalogue_ownership(&self, tenant_id: &str) -> AppResult<u64> {
                let result = sqlx::query(TAKE_CATALOGUE_OWNERSHIP_SQL)
                    .bind($ids::bind_text(tenant_id)?)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to take catalogue ownership: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn seed_list_catalogue_slugs(&self) -> AppResult<Vec<String>> {
                let rows = sqlx::query(CATALOGUE_SLUGS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list catalogue slugs: {e}"))
                    })?;
                rows.iter().map(|r| column(r, "slug")).collect()
            }

            async fn seed_insert_agent(&self, agent: &SeedAgent) -> AppResult<()> {
                sqlx::query(INSERT_AGENT_SQL)
                    .bind(&agent.id)
                    .bind($ids::bind(agent.user_id))
                    .bind(agent.tenant_id)
                    .bind(&agent.title)
                    .bind(&agent.description)
                    .bind(&agent.system_prompt)
                    .bind(&agent.category)
                    .bind(&agent.tags_json)
                    .bind(&agent.sample_prompts_json)
                    .bind(agent.token_count)
                    .bind(agent.created_at)
                    .bind(agent.updated_at)
                    .bind(&agent.visibility)
                    .bind(&agent.slug)
                    .bind(&agent.purpose)
                    .bind(&agent.when_to_use)
                    .bind(&agent.instructions)
                    .bind(&agent.example_inputs)
                    .bind(&agent.example_outputs)
                    .bind(&agent.success_criteria)
                    .bind(&agent.prerequisites_json)
                    .bind(&agent.source_file)
                    .bind(&agent.content_hash)
                    .bind(&agent.startup_query)
                    .bind(&agent.data_requirements)
                    .bind(&agent.visuals)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert coach: {e}")))?;
                Ok(())
            }

            async fn seed_update_agent(&self, agent: &SeedAgent) -> AppResult<()> {
                sqlx::query(UPDATE_AGENT_SQL)
                    .bind(&agent.title)
                    .bind(&agent.description)
                    .bind(&agent.system_prompt)
                    .bind(&agent.category)
                    .bind(&agent.tags_json)
                    .bind(&agent.sample_prompts_json)
                    .bind(agent.token_count)
                    .bind(agent.updated_at)
                    .bind(&agent.visibility)
                    .bind(&agent.purpose)
                    .bind(&agent.when_to_use)
                    .bind(&agent.instructions)
                    .bind(&agent.example_inputs)
                    .bind(&agent.example_outputs)
                    .bind(&agent.success_criteria)
                    .bind(&agent.prerequisites_json)
                    .bind(&agent.source_file)
                    .bind(&agent.content_hash)
                    .bind(&agent.startup_query)
                    .bind(&agent.data_requirements)
                    .bind(&agent.visuals)
                    .bind(&agent.id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;
                Ok(())
            }

            async fn seed_insert_agent_relation_if_absent(
                &self,
                relation: &SeedAgentRelation,
            ) -> AppResult<bool> {
                let result = sqlx::query(INSERT_AGENT_RELATION_SQL)
                    .bind(&relation.id)
                    .bind(&relation.agent_id)
                    .bind(&relation.related_agent_id)
                    .bind(&relation.relation_type)
                    .bind(relation.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert coach relation: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn seed_upsert_agent_author(
                &self,
                author: &SeedAgentAuthor,
            ) -> AppResult<String> {
                let existing: Option<String> = sqlx::query_scalar(AGENT_AUTHOR_ID_SQL)
                    .bind($ids::bind(author.user_id))
                    .bind(&author.tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check coach author: {e}"))
                    })?;
                if let Some(existing_id) = existing {
                    return Ok(existing_id);
                }

                sqlx::query(INSERT_AGENT_AUTHOR_SQL)
                    .bind(&author.id)
                    .bind($ids::bind(author.user_id))
                    .bind(&author.tenant_id)
                    .bind(&author.display_name)
                    .bind(author.created_at)
                    .bind(author.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert coach author: {e}"))
                    })?;
                Ok(author.id.clone())
            }

            async fn seed_insert_store_listing_if_absent(
                &self,
                listing: &SeedStoreListing,
            ) -> AppResult<bool> {
                let result = sqlx::query(INSERT_STORE_LISTING_SQL)
                    .bind(&listing.id)
                    .bind(&listing.agent_id)
                    .bind(listing.tenant_id.to_string())
                    .bind(listing.created_at)
                    .bind(&listing.author_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert store listing: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn seed_upsert_agent_translation(
                &self,
                translation: &SeedAgentTranslation,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_AGENT_TRANSLATION_SQL)
                    .bind(&translation.agent_id)
                    .bind(&translation.locale)
                    .bind(&translation.title)
                    .bind(&translation.description)
                    .bind(&translation.purpose)
                    .bind(&translation.instructions)
                    .bind(&translation.source_sha)
                    .bind(
                        translation
                            .tags
                            .as_ref()
                            .map(|tags| json_field(tags, "tags"))
                            .transpose()?,
                    )
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert coach translation: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_seeder_repository;
