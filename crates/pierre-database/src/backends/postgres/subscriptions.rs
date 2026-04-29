// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: PostgreSQL implementation of SubscriptionsRepository for pluggable BillingProvider-backed billing
// ABOUTME: Mirrors the SQLite impl shape but uses native PG types (UUID, TIMESTAMPTZ, JSONB)

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::SubscriptionsRepository;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserTier};

#[async_trait]
impl SubscriptionsRepository for PostgresDatabase {
    async fn upsert_subscription(&self, subscription: &Subscription) -> AppResult<Subscription> {
        sqlx::query(
            r"
            INSERT INTO subscriptions (
                id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                status, plan_tier, current_period_start, current_period_end,
                cancel_at_period_end, canceled_at, trial_end, metadata,
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
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
            ",
        )
        .bind(subscription.id)
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.user_id)
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
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert subscription: {e}")))?;

        self.get_subscription_by_provider_customer_id(
            &subscription.provider,
            &subscription.provider_customer_id,
        )
        .await?
        .ok_or_else(|| {
            AppError::internal("upsert_subscription: row missing after insert".to_owned())
        })
    }

    async fn get_subscription_by_user(&self, user_id: Uuid) -> AppResult<Option<Subscription>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE user_id = $1
            ORDER BY updated_at DESC
            LIMIT 1
            ",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch subscription by user: {e}")))?;

        row.as_ref().map(row_to_subscription).transpose()
    }

    async fn get_subscription_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Option<Subscription>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE tenant_id = $1
            ORDER BY updated_at DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch subscription by tenant: {e}")))?;

        row.as_ref().map(row_to_subscription).transpose()
    }

    async fn get_subscription_by_provider_subscription_id(
        &self,
        provider: &str,
        provider_subscription_id: &str,
    ) -> AppResult<Option<Subscription>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE provider = $1 AND provider_subscription_id = $2
            ",
        )
        .bind(provider)
        .bind(provider_subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to fetch subscription by provider_subscription_id: {e}"
            ))
        })?;

        row.as_ref().map(row_to_subscription).transpose()
    }

    async fn get_subscription_by_provider_customer_id(
        &self,
        provider: &str,
        provider_customer_id: &str,
    ) -> AppResult<Option<Subscription>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE provider = $1 AND provider_customer_id = $2
            ",
        )
        .bind(provider)
        .bind(provider_customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to fetch subscription by provider_customer_id: {e}"
            ))
        })?;

        row.as_ref().map(row_to_subscription).transpose()
    }

    async fn list_subscriptions_by_status(
        &self,
        status: SubscriptionStatus,
    ) -> AppResult<Vec<Subscription>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE status = $1
            ORDER BY updated_at DESC
            ",
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list subscriptions by status: {e}")))?;

        rows.iter().map(row_to_subscription).collect()
    }

    async fn is_billing_event_processed(&self, provider: &str, event_id: &str) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM billing_events WHERE provider = $1 AND event_id = $2",
        )
        .bind(provider)
        .bind(event_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query billing_events: {e}")))?;
        Ok(count > 0)
    }

    async fn mark_billing_event_processed(
        &self,
        provider: &str,
        event_id: &str,
        event_type: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO billing_events (provider, event_id, event_type) VALUES ($1, $2, $3) ON CONFLICT (provider, event_id) DO NOTHING",
        )
        .bind(provider)
        .bind(event_id)
        .bind(event_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark billing event: {e}")))?;
        Ok(())
    }
}

fn row_to_subscription(row: &PgRow) -> AppResult<Subscription> {
    let status_str: String = row.try_get("status").map_err(|e| map_row_err(&e))?;
    let plan_tier_str: String = row.try_get("plan_tier").map_err(|e| map_row_err(&e))?;

    Ok(Subscription {
        id: row.try_get("id").map_err(|e| map_row_err(&e))?,
        tenant_id: TenantId::from(
            row.try_get::<Uuid, _>("tenant_id")
                .map_err(|e| map_row_err(&e))?,
        ),
        user_id: row.try_get("user_id").map_err(|e| map_row_err(&e))?,
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
            .try_get::<Option<serde_json::Value>, _>("metadata")
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
