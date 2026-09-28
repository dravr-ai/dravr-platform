// ABOUTME: PersonalBestRepository — an athlete's all-time best per standard distance, the runs scanned, and the seed's progress
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Personal bests at the standard running distances.
//!
//! `personal_best_efforts` keeps, per athlete and tenant, the fastest elapsed
//! time any scanned run covered each standard distance in, with the run that
//! set it. `best_effort_scans` records every run whose best efforts were
//! computed, so a run is measured once however many syncs list it again.
//! `personal_best_seeds` records how far the one-time walk of an athlete's
//! history at a provider has got, and when it reached their first activity:
//! until then the stored bests cover part of the history only.
//!
//! Every per-athlete statement is keyed by both `user_id` and `tenant_id`. The
//! one exception is [`PersonalBestRepository::list_personal_best_seed_candidates`],
//! which the seed worker runs across tenants to find whose walk is owed, and
//! which returns each athlete with the tenant their token lives in. `$n`
//! placeholders throughout: sqlx accepts them on `SQLite` as well as Postgres.
//! `achieved_at`, `updated_at`, `scanned_at` and `completed_at` bind as
//! [`DateTime<Utc>`], which sqlx stores as RFC 3339 text on `SQLite` and as
//! `TIMESTAMPTZ` on Postgres.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// An athlete's all-time best effort at one standard distance.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonalBest {
    /// The distance's catalogue code: `5k`, `10k`, `half_marathon` or `marathon`.
    pub distance: String,
    /// Elapsed time of the fastest window covering the distance, in seconds.
    pub elapsed_seconds: f64,
    /// The provider the run came from.
    pub provider: String,
    /// The provider's id for the run that set the best.
    pub activity_id: String,
    /// When that run started.
    pub achieved_at: DateTime<Utc>,
}

/// How far the one-time walk of an athlete's history at one provider has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PersonalBestSeed {
    /// The exclusive upper bound, in unix seconds, of the next page the walk
    /// lists, newest activity first: every activity that started at or after
    /// it was measured or skipped. `None` before the first page is listed.
    pub cursor_before: Option<i64>,
    /// When a listing past the athlete's oldest activity came back empty, so
    /// every past run was measured; `None` while the walk is under way.
    pub completed_at: Option<DateTime<Utc>>,
}

/// An athlete whose history walk at a provider is owed, with the tenant the
/// provider token lives in, as the token row spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalBestSeedCandidate {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant their provider token is stored under.
    pub tenant_id: String,
}

/// An athlete's personal bests and the runs already scanned for them.
#[async_trait]
pub trait PersonalBestRepository: Send + Sync {
    /// Every stored best of `user_id` in `tenant_id`, one per distance.
    async fn personal_bests(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<PersonalBest>>;

    /// Store `best` when the athlete holds none at its distance, or replace
    /// the stored one when `best` is strictly faster. Returns whether the row
    /// was written; a slower or equal time leaves the stored best in place.
    async fn record_personal_best(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        best: &PersonalBest,
    ) -> AppResult<bool>;

    /// Whether the run `activity_id` from `provider` was already scanned.
    async fn is_activity_scanned(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Record that the run `activity_id` from `provider` was scanned at
    /// `scanned_at`. Recording a run twice keeps the first scan.
    async fn record_activity_scan(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
        scanned_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Where the walk of the athlete's history at `provider` stands, or
    /// `None` when it has not started.
    async fn personal_best_seed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<PersonalBestSeed>>;

    /// Store where the walk of the athlete's history at `provider` stands,
    /// replacing what was stored.
    async fn save_personal_best_seed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        seed: &PersonalBestSeed,
    ) -> AppResult<()>;

    /// Up to `limit` athletes holding a `provider` token whose history walk
    /// has not completed, and whose connection is not revoked or waiting on a
    /// reconnect: those never started first, then the least recently
    /// advanced, so passes share the budget among them in turn.
    async fn list_personal_best_seed_candidates(
        &self,
        provider: &str,
        limit: i64,
    ) -> AppResult<Vec<PersonalBestSeedCandidate>>;
}

pub(crate) const LIST_PERSONAL_BESTS_SQL: &str = r"
            SELECT distance, elapsed_seconds, provider, activity_id, achieved_at
            FROM personal_best_efforts
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY elapsed_seconds ASC
            ";

/// The update only runs when the proposed time is strictly faster, so two
/// syncs racing on one distance can never replace a best with a slower one.
/// `excluded` names the row the insert proposed on both engines.
pub(crate) const RECORD_PERSONAL_BEST_SQL: &str = r"
            INSERT INTO personal_best_efforts
                (user_id, tenant_id, distance, elapsed_seconds, provider, activity_id,
                 achieved_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (user_id, tenant_id, distance) DO UPDATE SET
                elapsed_seconds = excluded.elapsed_seconds,
                provider = excluded.provider,
                activity_id = excluded.activity_id,
                achieved_at = excluded.achieved_at,
                updated_at = excluded.updated_at
            WHERE excluded.elapsed_seconds < personal_best_efforts.elapsed_seconds
            ";

pub(crate) const IS_ACTIVITY_SCANNED_SQL: &str = r"
            SELECT 1
            FROM best_effort_scans
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4
            ";

pub(crate) const RECORD_ACTIVITY_SCAN_SQL: &str = r"
            INSERT INTO best_effort_scans (user_id, tenant_id, provider, activity_id, scanned_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, tenant_id, provider, activity_id) DO NOTHING
            ";

pub(crate) const GET_PERSONAL_BEST_SEED_SQL: &str = r"
            SELECT cursor_before, completed_at
            FROM personal_best_seeds
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ";

pub(crate) const SAVE_PERSONAL_BEST_SEED_SQL: &str = r"
            INSERT INTO personal_best_seeds
                (user_id, tenant_id, provider, cursor_before, completed_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (user_id, tenant_id, provider) DO UPDATE SET
                cursor_before = excluded.cursor_before,
                completed_at = excluded.completed_at,
                updated_at = excluded.updated_at
            ";

/// The token, connection and seed tables disagree on the id types on
/// Postgres (`uuid` user and seed tenant, `VARCHAR` token tenant, `TEXT`
/// connection ids), so each join compares the `uuid` side cast down to text.
/// A connection row is optional — a token without one is usable — and the
/// order puts athletes with no seed row first, then the least recently
/// advanced, spelled with a `CASE` because the engines sort NULL differently.
pub(crate) const LIST_PERSONAL_BEST_SEED_CANDIDATES_SQL: &str = r"
            SELECT t.user_id, t.tenant_id
            FROM user_oauth_tokens t
            LEFT JOIN personal_best_seeds s
              ON s.user_id = t.user_id
             AND CAST(s.tenant_id AS TEXT) = t.tenant_id
             AND s.provider = t.provider
            WHERE t.provider = $1
              AND s.completed_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM provider_connections c
                  WHERE c.user_id = CAST(t.user_id AS TEXT)
                    AND c.tenant_id = t.tenant_id
                    AND c.provider = t.provider
                    AND c.status IN ('revoked', 'needs_reauth')
              )
            ORDER BY CASE WHEN s.updated_at IS NULL THEN 0 ELSE 1 END,
                     s.updated_at, CAST(t.user_id AS TEXT), t.tenant_id
            LIMIT $2
            ";

/// Emit the whole [`PersonalBestRepository`] implementation for one backend
/// type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_personal_best_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl PersonalBestRepository for $ty {
            async fn personal_bests(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<PersonalBest>> {
                let rows: Vec<(String, f64, String, String, DateTime<Utc>)> =
                    sqlx::query_as(LIST_PERSONAL_BESTS_SQL)
                        .bind($ids::bind(user_id))
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to read personal bests: {e}"))
                        })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(distance, elapsed_seconds, provider, activity_id, achieved_at)| {
                            PersonalBest {
                                distance,
                                elapsed_seconds,
                                provider,
                                activity_id,
                                achieved_at,
                            }
                        },
                    )
                    .collect())
            }

            async fn record_personal_best(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                best: &PersonalBest,
            ) -> AppResult<bool> {
                let result = sqlx::query(RECORD_PERSONAL_BEST_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(&best.distance)
                    .bind(best.elapsed_seconds)
                    .bind(&best.provider)
                    .bind(&best.activity_id)
                    .bind(best.achieved_at)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a personal best: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn is_activity_scanned(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<bool> {
                let row: Option<(i32,)> = sqlx::query_as(IS_ACTIVITY_SCANNED_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a best-effort scan: {e}"))
                    })?;
                Ok(row.is_some())
            }

            async fn record_activity_scan(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                activity_id: &str,
                scanned_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_ACTIVITY_SCAN_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(provider)
                    .bind(activity_id)
                    .bind(scanned_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a best-effort scan: {e}"))
                    })?;
                Ok(())
            }

            async fn personal_best_seed(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<PersonalBestSeed>> {
                let row: Option<(Option<i64>, Option<DateTime<Utc>>)> =
                    sqlx::query_as(GET_PERSONAL_BEST_SEED_SQL)
                        .bind($ids::bind(user_id))
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind(provider)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to read a personal-best seed: {e}"))
                        })?;
                Ok(row.map(|(cursor_before, completed_at)| PersonalBestSeed {
                    cursor_before,
                    completed_at,
                }))
            }

            async fn save_personal_best_seed(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                seed: &PersonalBestSeed,
            ) -> AppResult<()> {
                sqlx::query(SAVE_PERSONAL_BEST_SEED_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(provider)
                    .bind(seed.cursor_before)
                    .bind(seed.completed_at)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to save a personal-best seed: {e}"))
                    })?;
                Ok(())
            }

            async fn list_personal_best_seed_candidates(
                &self,
                provider: &str,
                limit: i64,
            ) -> AppResult<Vec<PersonalBestSeedCandidate>> {
                let rows = sqlx::query(LIST_PERSONAL_BEST_SEED_CANDIDATES_SQL)
                    .bind(provider)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list personal-best seed candidates: {e}"
                        ))
                    })?;
                rows.iter()
                    .map(|row| {
                        Ok(PersonalBestSeedCandidate {
                            user_id: $ids::read(row, "user_id")?,
                            tenant_id: row.try_get("tenant_id").map_err(|e| {
                                AppError::database(format!(
                                    "Failed to read a seed candidate's tenant: {e}"
                                ))
                            })?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_personal_best_repository;
