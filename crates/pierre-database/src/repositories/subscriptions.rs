// ABOUTME: SubscriptionsRepository trait plus the one shared implementation both backends emit, for BillingProvider-backed billing
// ABOUTME: Upsert keyed on (provider, provider_customer_id); reads by user, tenant, or provider identifier; billing_events dedupe
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserId, UserTier};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::uuid_column::UuidColumn;

/// Provider-agnostic subscription persistence.
///
/// Webhook handlers upsert by `(provider, provider_customer_id)`;
/// read paths query by user, tenant, or `(provider, *)` identifier.
#[async_trait]
pub trait SubscriptionsRepository: Send + Sync {
    /// Insert a new subscription row or update the existing row that
    /// shares its `(provider, provider_customer_id)` key. Returns the
    /// freshly written row.
    async fn upsert_subscription(&self, subscription: &Subscription) -> AppResult<Subscription>;
    /// Look up the most recently updated subscription for a user.
    async fn get_subscription_by_user(&self, user_id: Uuid) -> AppResult<Option<Subscription>>;
    /// Look up the most recently updated subscription for a tenant.
    async fn get_subscription_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side subscription identifier.
    async fn get_subscription_by_provider_subscription_id(
        &self,
        provider: &str,
        provider_subscription_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side customer identifier.
    async fn get_subscription_by_provider_customer_id(
        &self,
        provider: &str,
        provider_customer_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// List every subscription with the given lifecycle status.
    /// Used by admin filter views and the dunning sweep.
    async fn list_subscriptions_by_status(
        &self,
        status: SubscriptionStatus,
    ) -> AppResult<Vec<Subscription>>;

    /// Returns true when the given `(provider, event_id)` has already
    /// been processed. The webhook handler skips dispatch on `true`,
    /// making the entire pipeline safe against provider retries.
    async fn is_billing_event_processed(&self, provider: &str, event_id: &str) -> AppResult<bool>;
    /// Mark a `(provider, event_id)` as processed. Call this AFTER the
    /// event dispatch path succeeds — never before — so a partial failure
    /// allows the webhook to retry with the full body.
    async fn mark_billing_event_processed(
        &self,
        provider: &str,
        event_id: &str,
        event_type: &str,
    ) -> AppResult<()>;
}

/// The column list every read projects, in the order [`subscription_from_row`]
/// names them.
macro_rules! subscription_columns {
    () => {
        "id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id, \
         status, plan_tier, current_period_start, current_period_end, \
         cancel_at_period_end, canceled_at, trial_end, metadata, \
         created_at, updated_at"
    };
}

/// Insert a subscription, or refresh the row sharing its
/// `(provider, provider_customer_id)` key.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres. `id` binds as [`UuidColumn`], `tenant_id` as [`TenantId`] and
/// `user_id` as [`UserId`], each of which encodes as hyphenated text on
/// `SQLite` and as a native `uuid` on Postgres. Timestamps bind as
/// `DateTime<Utc>`: RFC3339 text on `SQLite`, `TIMESTAMPTZ` on Postgres.
/// `cancel_at_period_end` binds as `bool`, which `SQLite` stores as 0/1 in
/// its `INTEGER` column. `metadata` binds as an optional [`Value`]: the
/// stored JSON is an opaque blob whose only reader is
/// [`subscription_from_row`], which hands it back as a [`Value`].
pub(crate) const UPSERT_SUBSCRIPTION_SQL: &str = concat!(
    "
            INSERT INTO subscriptions (",
    subscription_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (provider, provider_customer_id) DO UPDATE SET
                provider_subscription_id = EXCLUDED.provider_subscription_id,
                status = EXCLUDED.status,
                plan_tier = EXCLUDED.plan_tier,
                current_period_start = EXCLUDED.current_period_start,
                current_period_end = EXCLUDED.current_period_end,
                cancel_at_period_end = EXCLUDED.cancel_at_period_end,
                canceled_at = EXCLUDED.canceled_at,
                trial_end = EXCLUDED.trial_end,
                metadata = EXCLUDED.metadata,
                updated_at = EXCLUDED.updated_at
            "
);

/// The most recently updated subscription of one user.
///
/// Scoped by the globally-unique user UUID: a subscription belongs to exactly
/// one user in one tenant, so this lookup cannot cross tenants. No `tenant_id`
/// predicate is threaded because the billing callers only carry an optional
/// active tenant and the user-id key already makes the result single-tenant.
pub(crate) const GET_SUBSCRIPTION_BY_USER_SQL: &str = concat!(
    "
            SELECT ",
    subscription_columns!(),
    "
            FROM subscriptions
            WHERE user_id = $1
            ORDER BY updated_at DESC
            LIMIT 1
            "
);

/// The most recently updated subscription of one tenant.
pub(crate) const GET_SUBSCRIPTION_BY_TENANT_SQL: &str = concat!(
    "
            SELECT ",
    subscription_columns!(),
    "
            FROM subscriptions
            WHERE tenant_id = $1
            ORDER BY updated_at DESC
            LIMIT 1
            "
);

/// The subscription a provider knows by its subscription identifier.
pub(crate) const GET_SUBSCRIPTION_BY_PROVIDER_SUBSCRIPTION_ID_SQL: &str = concat!(
    "
            SELECT ",
    subscription_columns!(),
    "
            FROM subscriptions
            WHERE provider = $1 AND provider_subscription_id = $2
            "
);

/// The subscription a provider knows by its customer identifier.
pub(crate) const GET_SUBSCRIPTION_BY_PROVIDER_CUSTOMER_ID_SQL: &str = concat!(
    "
            SELECT ",
    subscription_columns!(),
    "
            FROM subscriptions
            WHERE provider = $1 AND provider_customer_id = $2
            "
);

/// Every subscription in one lifecycle status, most recently updated first.
pub(crate) const LIST_SUBSCRIPTIONS_BY_STATUS_SQL: &str = concat!(
    "
            SELECT ",
    subscription_columns!(),
    "
            FROM subscriptions
            WHERE status = $1
            ORDER BY updated_at DESC
            "
);

/// How many times a provider event has been recorded (0 or 1).
pub(crate) const COUNT_BILLING_EVENT_SQL: &str =
    "SELECT COUNT(*) FROM billing_events WHERE provider = $1 AND event_id = $2";

/// Record a provider event once; a retry of the same event is a no-op.
pub(crate) const MARK_BILLING_EVENT_SQL: &str =
    "INSERT INTO billing_events (provider, event_id, event_type) VALUES ($1, $2, $3) \
     ON CONFLICT (provider, event_id) DO NOTHING";

/// Rebuild a [`Subscription`] from one row of [`subscription_columns`].
///
/// `try_get` rather than `Row::get` so a corrupt row surfaces as a
/// recoverable error rather than a panic; an enum value the application no
/// longer knows is rejected rather than defaulted.
///
/// # Errors
/// Returns a database error when a column cannot be decoded and an internal
/// error when a stored status or tier is not one the application knows.
pub(crate) fn subscription_from_row<R>(row: &R) -> AppResult<Subscription>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Value: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UuidColumn: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UserId: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let status_str: String = row.try_get("status").map_err(|e| map_row_err(&e))?;
    let plan_tier_str: String = row.try_get("plan_tier").map_err(|e| map_row_err(&e))?;

    Ok(Subscription {
        id: row
            .try_get::<UuidColumn, _>("id")
            .map_err(|e| map_row_err(&e))?
            .into(),
        tenant_id: row
            .try_get::<TenantId, _>("tenant_id")
            .map_err(|e| map_row_err(&e))?,
        user_id: row
            .try_get::<UserId, _>("user_id")
            .map_err(|e| map_row_err(&e))?
            .as_uuid(),
        provider: row.try_get("provider").map_err(|e| map_row_err(&e))?,
        provider_customer_id: row
            .try_get("provider_customer_id")
            .map_err(|e| map_row_err(&e))?,
        provider_subscription_id: row
            .try_get("provider_subscription_id")
            .map_err(|e| map_row_err(&e))?,
        status: SubscriptionStatus::from_str(&status_str)
            .map_err(|e| AppError::internal(format!("invalid status: {e}")))?,
        plan_tier: UserTier::from_str(&plan_tier_str)
            .map_err(|e| AppError::internal(format!("invalid plan_tier: {e}")))?,
        current_period_start: row
            .try_get::<Option<DateTime<Utc>>, _>("current_period_start")
            .map_err(|e| map_row_err(&e))?,
        current_period_end: row
            .try_get::<Option<DateTime<Utc>>, _>("current_period_end")
            .map_err(|e| map_row_err(&e))?,
        cancel_at_period_end: row
            .try_get::<bool, _>("cancel_at_period_end")
            .map_err(|e| map_row_err(&e))?,
        canceled_at: row
            .try_get::<Option<DateTime<Utc>>, _>("canceled_at")
            .map_err(|e| map_row_err(&e))?,
        trial_end: row
            .try_get::<Option<DateTime<Utc>>, _>("trial_end")
            .map_err(|e| map_row_err(&e))?,
        metadata: row
            .try_get::<Option<Value>, _>("metadata")
            .map_err(|e| map_row_err(&e))?,
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(|e| map_row_err(&e))?,
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .map_err(|e| map_row_err(&e))?,
    })
}

fn map_row_err(e: &sqlx::Error) -> AppError {
    AppError::database(format!("subscription row decode failed: {e}"))
}

/// Emit the whole [`SubscriptionsRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
macro_rules! impl_subscriptions_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl SubscriptionsRepository for $ty {
            async fn upsert_subscription(
                &self,
                subscription: &Subscription,
            ) -> AppResult<Subscription> {
                sqlx::query(UPSERT_SUBSCRIPTION_SQL)
                    .bind(UuidColumn(subscription.id))
                    .bind(subscription.tenant_id)
                    .bind(UserId::from_uuid(subscription.user_id))
                    .bind(&subscription.provider)
                    .bind(&subscription.provider_customer_id)
                    .bind(&subscription.provider_subscription_id)
                    .bind(subscription.status.as_str())
                    .bind(subscription.plan_tier.as_str())
                    .bind(subscription.current_period_start)
                    .bind(subscription.current_period_end)
                    .bind(subscription.cancel_at_period_end)
                    .bind(subscription.canceled_at)
                    .bind(subscription.trial_end)
                    .bind(&subscription.metadata)
                    .bind(subscription.created_at)
                    .bind(subscription.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert subscription: {e}"))
                    })?;

                // Re-read to capture any DB-side defaults / coercions.
                self.get_subscription_by_provider_customer_id(
                    &subscription.provider,
                    &subscription.provider_customer_id,
                )
                .await?
                .ok_or_else(|| {
                    AppError::internal("upsert_subscription: row missing after insert".to_owned())
                })
            }

            async fn get_subscription_by_user(
                &self,
                user_id: Uuid,
            ) -> AppResult<Option<Subscription>> {
                let row = sqlx::query(GET_SUBSCRIPTION_BY_USER_SQL)
                    .bind(UserId::from_uuid(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch subscription by user: {e}"))
                    })?;
                row.as_ref().map(subscription_from_row).transpose()
            }

            async fn get_subscription_by_tenant(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Option<Subscription>> {
                let row = sqlx::query(GET_SUBSCRIPTION_BY_TENANT_SQL)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch subscription by tenant: {e}"))
                    })?;
                row.as_ref().map(subscription_from_row).transpose()
            }

            async fn get_subscription_by_provider_subscription_id(
                &self,
                provider: &str,
                provider_subscription_id: &str,
            ) -> AppResult<Option<Subscription>> {
                let row = sqlx::query(GET_SUBSCRIPTION_BY_PROVIDER_SUBSCRIPTION_ID_SQL)
                    .bind(provider)
                    .bind(provider_subscription_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to fetch subscription by provider_subscription_id: {e}"
                        ))
                    })?;
                row.as_ref().map(subscription_from_row).transpose()
            }

            async fn get_subscription_by_provider_customer_id(
                &self,
                provider: &str,
                provider_customer_id: &str,
            ) -> AppResult<Option<Subscription>> {
                let row = sqlx::query(GET_SUBSCRIPTION_BY_PROVIDER_CUSTOMER_ID_SQL)
                    .bind(provider)
                    .bind(provider_customer_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to fetch subscription by provider_customer_id: {e}"
                        ))
                    })?;
                row.as_ref().map(subscription_from_row).transpose()
            }

            async fn list_subscriptions_by_status(
                &self,
                status: SubscriptionStatus,
            ) -> AppResult<Vec<Subscription>> {
                let rows = sqlx::query(LIST_SUBSCRIPTIONS_BY_STATUS_SQL)
                    .bind(status.as_str())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list subscriptions by status: {e}"))
                    })?;
                rows.iter().map(subscription_from_row).collect()
            }

            async fn is_billing_event_processed(
                &self,
                provider: &str,
                event_id: &str,
            ) -> AppResult<bool> {
                let count: i64 = sqlx::query_scalar(COUNT_BILLING_EVENT_SQL)
                    .bind(provider)
                    .bind(event_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query billing_events: {e}"))
                    })?;
                Ok(count > 0)
            }

            async fn mark_billing_event_processed(
                &self,
                provider: &str,
                event_id: &str,
                event_type: &str,
            ) -> AppResult<()> {
                sqlx::query(MARK_BILLING_EVENT_SQL)
                    .bind(provider)
                    .bind(event_id)
                    .bind(event_type)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark billing event: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_subscriptions_repository;
