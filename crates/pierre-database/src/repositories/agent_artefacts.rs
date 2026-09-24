// ABOUTME: Repository trait for agent package artefacts — the flavour, skeleton and workout files stored per agent
// ABOUTME: The seeder replaces an agent's set wholesale; the resolver and the admin review read it back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AgentArtefact, ArtefactKind, PackageArtefact};

/// Persistent storage for an agent package's training artefacts.
///
/// Tenant-scoped through the agent row: `agent_artefacts` carries the
/// agent's `tenant_id` so every query includes it, and the caller has
/// already resolved the agent it asks about. A package is written as a set —
/// the files beside one prompt at one seed run — so the write replaces
/// whatever the agent carried before rather than merging: an artefact
/// deleted from the checkout leaves the database on the next seed, the way
/// a retired agent does.
#[async_trait]
pub trait AgentArtefactRepository: Send + Sync {
    /// Replace the agent's artefacts with `artefacts`, in one transaction.
    ///
    /// Returns how many rows the agent carries afterwards. An empty set
    /// clears the package.
    async fn replace_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
        artefacts: &[PackageArtefact],
    ) -> AppResult<usize>;

    /// Every artefact the agent carries, ordered by `(kind, slug)` so two
    /// reads of the same package list it the same way.
    async fn list_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
    ) -> AppResult<Vec<AgentArtefact>>;
}

/// Clear the agent's current set before the replacement is written.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them.
pub(crate) const CLEAR_AGENT_ARTEFACTS_SQL: &str =
    "DELETE FROM agent_artefacts WHERE tenant_id = $1 AND agent_id = $2";

/// Insert one artefact of the replacement set; `$8` carries both timestamps.
///
/// The timestamp binds a `DateTime<Utc>` on both backends: sqlx-sqlite encodes
/// it as `to_rfc3339_opts(AutoSi, false)`, which is what `to_rfc3339()`
/// produces, so the TEXT column keeps the bytes it always held while Postgres
/// gets its native `TIMESTAMPTZ`.
pub(crate) const INSERT_AGENT_ARTEFACT_SQL: &str = r"
                INSERT INTO agent_artefacts
                    (id, agent_id, tenant_id, kind, slug, content, sha256, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
                ";

/// The agent's artefacts, ordered so a package reads back in a stable shape.
pub(crate) const LIST_AGENT_ARTEFACTS_SQL: &str = r"
            SELECT id, agent_id, tenant_id, kind, slug, content, sha256, created_at, updated_at
            FROM agent_artefacts
            WHERE tenant_id = $1 AND agent_id = $2
            ORDER BY kind, slug
            ";

/// Extract an [`AgentArtefact`] from a row of either backend via `try_get`
/// only — `Row::get` is `try_get().unwrap()` and panics the whole read path on
/// a width or NULL surprise.
///
/// `created_at`/`updated_at` decode straight into `DateTime<Utc>` on both:
/// Postgres hands back a native `TIMESTAMPTZ`, and sqlx-sqlite's decoder tries
/// `parse_from_rfc3339` first, which is the format the TEXT column holds.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded, or
/// when `kind` is not a known [`ArtefactKind`].
pub(crate) fn artefact_from_row<R>(row: &R) -> AppResult<AgentArtefact>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("read {name}: {e}")))
    };
    let stamp = |name: &str| -> AppResult<DateTime<Utc>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("read {name}: {e}")))
    };
    let kind_str = col("kind")?;
    let kind = ArtefactKind::parse(&kind_str)
        .ok_or_else(|| AppError::database(format!("coach_artefacts.kind '{kind_str}' unknown")))?;
    Ok(AgentArtefact {
        id: col("id")?,
        agent_id: col("agent_id")?,
        tenant_id: col("tenant_id")?,
        kind,
        slug: col("slug")?,
        content: col("content")?,
        sha256: col("sha256")?,
        created_at: stamp("created_at")?,
        updated_at: stamp("updated_at")?,
    })
}

/// Emit the whole [`AgentArtefactRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_agent_artefact_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl AgentArtefactRepository for $ty {
            async fn replace_agent_artefacts(
                &self,
                tenant_id: &str,
                agent_id: &str,
                artefacts: &[PackageArtefact],
            ) -> AppResult<usize> {
                let mut tx =
                    self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("begin coach_artefacts tx: {e}"))
                    })?;
                sqlx::query(CLEAR_AGENT_ARTEFACTS_SQL)
                    .bind(tenant_id)
                    .bind(agent_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("clear coach_artefacts: {e}")))?;
                let now = Utc::now();
                for artefact in artefacts {
                    sqlx::query(INSERT_AGENT_ARTEFACT_SQL)
                        .bind(Uuid::new_v4().to_string())
                        .bind(agent_id)
                        .bind(tenant_id)
                        .bind(artefact.kind.as_str())
                        .bind(&artefact.slug)
                        .bind(&artefact.content)
                        .bind(&artefact.sha256)
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| AppError::database(format!("insert coach_artefact: {e}")))?;
                }
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit coach_artefacts tx: {e}")))?;
                Ok(artefacts.len())
            }

            async fn list_agent_artefacts(
                &self,
                tenant_id: &str,
                agent_id: &str,
            ) -> AppResult<Vec<AgentArtefact>> {
                let rows = sqlx::query(LIST_AGENT_ARTEFACTS_SQL)
                    .bind(tenant_id)
                    .bind(agent_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list coach_artefacts: {e}")))?;
                rows.iter().map(artefact_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_agent_artefact_repository;
