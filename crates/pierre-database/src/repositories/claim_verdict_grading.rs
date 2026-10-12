// ABOUTME: The blind-grading pool of claim verdicts (carnet#854) — the consent and provenance rules, statements and decoders
// ABOUTME: Read by ClaimVerdictRepository::grading_pool on both backends through impl_claim_verdict_repository!
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::claims::{GradingCandidate, GradingPool, GradingWithheld};

use super::claim_verdicts::{verdict_columns, verdict_from_row};

/// Why verdict `v`, joined to its athlete's `users` row `u`, is kept away from
/// blind graders, or `NULL` when nothing keeps it. `$1` is the tenant. The
/// first rule a verdict fails names it:
///
/// - `first_party_only` — the reply it judged was built from data whose
///   provider keeps it on Dravr's own surfaces (carnet#769), and a grader
///   outside the conversation is not one of them. Most connected providers
///   declare so, so this rule withholds nearly every verdict on a reply that
///   read athlete data; its count is returned so the grade can say what share
///   of the window the pool leaves out.
/// - `no_analytics_consent` — analytics is opt-in (`users.analytics_consent`
///   defaults off), so an athlete who never opted in, or whose row is gone, is
///   withheld.
/// - `coach_sharing_withheld` — coach sharing is opt-out (joining a group
///   grants it, ADR-002), so only a live membership in one of this tenant's
///   groups that turned it off withholds the athlete.
///
/// `users` is read by user id for the athlete's own consent and locale, as
/// every other users join here is; the verdict row carries the tenant scope.
macro_rules! grading_withheld_reason {
    () => {
        "CASE
                WHEN v.first_party_only = TRUE THEN 'first_party_only'
                WHEN u.id IS NULL OR u.analytics_consent = FALSE THEN 'no_analytics_consent'
                WHEN EXISTS (
                    SELECT 1 FROM coaching_group_members gm
                    WHERE gm.tenant_id = $1
                      AND CAST(gm.user_id AS TEXT) = v.user_id
                      AND gm.left_at IS NULL
                      AND gm.coach_sharing_consent = FALSE
                ) THEN 'coach_sharing_withheld'
             END"
    };
}

/// The tenant's verdicts since `$2`, each with its athlete's locale and the
/// reason it is withheld from graders. Both grading statements read this one
/// window, so the counts and the pool cannot disagree about a rule.
macro_rules! grading_window_sql {
    () => {
        concat!(
            "
            SELECT v.*, u.locale AS athlete_locale, ",
            grading_withheld_reason!(),
            " AS withheld
            FROM claim_verdicts v
            LEFT JOIN users u ON CAST(u.id AS TEXT) = v.user_id
            WHERE v.tenant_id = $1 AND v.created_at >= $2"
        )
    };
}

/// The verdicts nothing withholds, newest first, at most `$3`.
pub(crate) const GRADING_POOL_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    ", athlete_locale
            FROM (",
    grading_window_sql!(),
    "
            ) pool
            WHERE withheld IS NULL
            ORDER BY created_at DESC
            LIMIT $3
            "
);

/// The window counted by withholding reason; the `NULL` reason is the
/// eligible count.
pub(crate) const GRADING_WITHHELD_SQL: &str = concat!(
    "
            SELECT withheld, COUNT(*) AS c
            FROM (",
    grading_window_sql!(),
    "
            ) pool
            GROUP BY withheld
            "
);

/// Decode one grading-pool row: the verdict and its athlete's locale.
pub(crate) fn grading_candidate_from_row<R>(row: &R) -> AppResult<GradingCandidate>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(GradingCandidate {
        verdict: verdict_from_row(row)?,
        locale: row
            .try_get("athlete_locale")
            .map_err(|e| AppError::database(format!("claim_verdicts athlete_locale: {e}")))?,
    })
}

/// Decode one `(withheld, count)` row of the grading counts.
pub(crate) fn grading_count_from_row<R>(row: &R) -> AppResult<(Option<String>, i64)>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts grading {name}: {e}"))
    };
    Ok((
        row.try_get("withheld").map_err(|e| col("withheld", e))?,
        row.try_get("c").map_err(|e| col("c", e))?,
    ))
}

/// Fold the grading counts into `(in_window, withheld, eligible)`, rejecting
/// a reason the statement cannot produce rather than dropping its rows.
pub(crate) fn fold_grading_counts(
    rows: &[(Option<String>, i64)],
) -> AppResult<(i64, GradingWithheld, i64)> {
    let mut withheld = GradingWithheld::default();
    let mut eligible = 0;
    for (reason, count) in rows {
        match reason.as_deref() {
            None => eligible += count,
            Some("first_party_only") => withheld.first_party_only += count,
            Some("no_analytics_consent") => withheld.no_analytics_consent += count,
            Some("coach_sharing_withheld") => withheld.coach_sharing_withheld += count,
            Some(other) => {
                return Err(AppError::internal(format!(
                    "Unknown grading withholding reason: {other}"
                )))
            }
        }
    }
    let in_window = eligible
        + withheld.first_party_only
        + withheld.no_analytics_consent
        + withheld.coach_sharing_withheld;
    Ok((in_window, withheld, eligible))
}

/// The window a grading pool is read over: the tenant as both statements bind
/// it, and the clamped length and lower bound.
pub(crate) struct GradingWindow {
    /// The tenant, as the `TEXT` both schemas store it in.
    pub tenant: String,
    /// Days scanned, clamped to `1..=365`.
    pub days: i64,
    /// Window lower bound (inclusive).
    pub start: DateTime<Utc>,
}

impl GradingWindow {
    /// The last `days` of `tenant`, clamped to `1..=365`.
    pub(crate) fn new(tenant: TenantId, days: i64) -> Self {
        let days = days.clamp(1, 365);
        Self {
            tenant: tenant.to_string(),
            days,
            start: Utc::now() - Duration::days(days),
        }
    }

    /// The pool from the two statements' rows: every count folded, every
    /// candidate decoded.
    pub(crate) fn pool<R>(&self, counts: &[R], candidates: &[R]) -> AppResult<GradingPool>
    where
        R: sqlx::Row,
        for<'a> &'a str: sqlx::ColumnIndex<R>,
        String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        f32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
        bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    {
        let counts = counts
            .iter()
            .map(grading_count_from_row)
            .collect::<AppResult<Vec<_>>>()?;
        let (in_window, withheld, eligible) = fold_grading_counts(&counts)?;
        Ok(GradingPool {
            window_start: self.start,
            window_days: self.days,
            in_window,
            withheld,
            eligible,
            candidates: candidates
                .iter()
                .map(grading_candidate_from_row)
                .collect::<AppResult<Vec<_>>>()?,
        })
    }
}

/// The error a failed grading-pool read surfaces as.
pub(crate) fn grading_read_failed(e: &sqlx::Error) -> AppError {
    AppError::database(format!("Failed to read the grading pool: {e}"))
}
