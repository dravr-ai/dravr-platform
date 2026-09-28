// ABOUTME: The agent row decoders and column readers both backends share, emitted per backend by macro
// ABOUTME: Read by the agents repository body and by the store listings over the same agents columns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How an `agents` row reads, written once: [`impl_agent_row_decoders`]
//! expands once per backend below, with that driver's row type and its
//! [`super::uuid_columns`] codec.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

/// Read a column, naming it in the error.
pub fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("agents column `{name}`: {e}")))
}

/// Read a timestamp column.
pub fn instant<'r, R>(row: &'r R, name: &str) -> AppResult<DateTime<Utc>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    column(row, name)
}

/// Bind a token count into the `INTEGER` column: a count is bounded far
/// below `i32::MAX` (about 25K tokens for 100K characters).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub const fn token_count_bind(count: u32) -> i32 {
    count as i32
}

/// Emit the agent row decoders for one backend: `row_to_agent`,
/// `row_to_agent_list_item` and `row_to_agent_version`.
///
/// `$row` is the driver's row type and `$ids` the backend's uuid codec. The
/// optional columns read leniently, as a missing column or NULL, because the
/// store listings read agents through the same decoder with a narrower
/// column list.
macro_rules! impl_agent_row_decoders {
    ($row:ty, $ids:ident) => {
        /// Decode an `agents` row.
        ///
        /// # Errors
        /// Returns an error when a required column is missing or unreadable,
        /// or `tags` / `sample_prompts` hold malformed JSON.
        pub fn row_to_agent(row: &$row) -> AppResult<Agent> {
            let id: String = column(row, "id")?;
            let category: String = column(row, "category")?;
            let tags: Option<String> = column(row, "tags")?;
            let token_count: i32 = column(row, "token_count")?;
            let is_system: bool = row.try_get("is_system").unwrap_or(false);
            let visibility: String = row
                .try_get("visibility")
                .unwrap_or_else(|_| "private".to_owned());
            let sample_prompts: Option<String> = row.try_get("sample_prompts").ok().flatten();
            let prerequisites: Option<String> = row.try_get("prerequisites").ok().flatten();
            let forked_from: Option<Uuid> = row
                .try_get::<Option<String>, _>("forked_from")
                .ok()
                .flatten()
                .and_then(|s| Uuid::parse_str(&s).ok());
            let temperature: Option<f64> = row.try_get("temperature").ok().flatten();
            let data_requirements: Option<DataRequirements> = row
                .try_get::<Option<String>, _>("data_requirements")
                .ok()
                .flatten()
                .and_then(|json| serde_json::from_str(&json).ok());

            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(Agent {
                id: Uuid::parse_str(&id)
                    .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
                user_id: $ids::read(row, "user_id")?,
                tenant_id: $ids::read_text(row, "tenant_id")?,
                title: column(row, "title")?,
                description: column(row, "description")?,
                system_prompt: column(row, "system_prompt")?,
                category: AgentCategory::parse(&category),
                tags: tags
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default(),
                sample_prompts: sample_prompts
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default(),
                token_count: token_count as u32,
                created_at: instant(row, "created_at")?,
                updated_at: instant(row, "updated_at")?,
                is_system,
                visibility: AgentVisibility::parse(&visibility),
                prerequisites: prerequisites
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
                forked_from,
                max_tool_iterations: row.try_get("max_tool_iterations").ok().flatten(),
                temperature: temperature.map(|t| t as f32),
                startup_query: row.try_get("startup_query").ok().flatten(),
                data_requirements,
                purpose: row.try_get("purpose").ok().flatten(),
                when_to_use: row.try_get("when_to_use").ok().flatten(),
                instructions: row.try_get("instructions").ok().flatten(),
                example_inputs: row.try_get("example_inputs").ok().flatten(),
                example_outputs: row.try_get("example_outputs").ok().flatten(),
                success_criteria: row.try_get("success_criteria").ok().flatten(),
                source: row
                    .try_get("source")
                    .unwrap_or_else(|_| "custom".to_owned()),
                handle: row.try_get("slug").ok().flatten(),
            })
        }

        /// Decode an agent row joined to the user's assignment state.
        ///
        /// # Errors
        /// Returns an error when the agent columns do not decode.
        pub fn row_to_agent_list_item(row: &$row) -> AppResult<AgentListItem> {
            let agent = row_to_agent(row)?;
            let use_count: i32 = row.try_get("use_count").unwrap_or(0);
            #[allow(clippy::cast_sign_loss)]
            Ok(AgentListItem {
                agent,
                is_assigned: row.try_get("is_assigned").unwrap_or(false),
                is_favorite: row.try_get("is_favorite").unwrap_or(false),
                is_active: row.try_get("is_active").unwrap_or(false),
                use_count: use_count as u32,
                last_used_at: row.try_get("last_used_at").ok().flatten(),
            })
        }

        /// Decode an `agent_versions` row.
        ///
        /// # Errors
        /// Returns an error when a column is missing or the snapshot is not JSON.
        pub fn row_to_agent_version(row: &$row) -> AppResult<AgentVersion> {
            let snapshot: String = column(row, "content_snapshot")?;
            Ok(AgentVersion {
                id: column(row, "id")?,
                agent_id: column(row, "agent_id")?,
                version: column(row, "version")?,
                content_hash: column(row, "content_hash")?,
                content_snapshot: serde_json::from_str(&snapshot).map_err(|e| {
                    AppError::internal(format!("Invalid JSON in version snapshot: {e}"))
                })?,
                change_summary: column(row, "change_summary")?,
                created_at: instant(row, "created_at")?,
                created_by: $ids::read_opt(row, "created_by")?,
            })
        }
    };
}

/// The `SQLite` agent row decoders.
pub mod sqlite {
    use pierre_core::errors::{AppError, AppResult};
    use pierre_core::models::agents::{
        Agent, AgentCategory, AgentListItem, AgentVersion, AgentVisibility, DataRequirements,
    };
    use sqlx::sqlite::SqliteRow;
    use sqlx::Row;
    use uuid::Uuid;

    use super::{column, instant};
    use crate::repositories::uuid_columns::TextUuid;

    impl_agent_row_decoders!(SqliteRow, TextUuid);
}

/// The `PostgreSQL` agent row decoders.
#[cfg(feature = "postgresql")]
pub mod postgres {
    use pierre_core::errors::{AppError, AppResult};
    use pierre_core::models::agents::{
        Agent, AgentCategory, AgentListItem, AgentVersion, AgentVisibility, DataRequirements,
    };
    use sqlx::postgres::PgRow;
    use sqlx::Row;
    use uuid::Uuid;

    use super::{column, instant};
    use crate::repositories::uuid_columns::NativeUuid;

    impl_agent_row_decoders!(PgRow, NativeUuid);
}
