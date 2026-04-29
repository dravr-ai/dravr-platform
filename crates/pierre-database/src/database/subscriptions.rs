// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: SQLite implementation of SubscriptionsRepository for pluggable BillingProvider-backed billing
// ABOUTME: Upsert keyed on (provider, provider_customer_id); reads by user, tenant, or provider identifier

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::SubscriptionsRepository;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserTier};

#[async_trait]
impl SubscriptionsRepository for Database {
    async fn upsert_subscription(&self, subscription: &Subscription) -> AppResult<Subscription> {
        let metadata_json = subscription
            .metadata
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_owned()));

        sqlx::query(
            r"
            INSERT INTO subscriptions (
                id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                status, plan_tier, current_period_start, current_period_end,
                cancel_at_period_end, canceled_at, trial_end, metadata,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(provider, provider_customer_id) DO UPDATE SET
                provider_subscription_id = excluded.provider_subscription_id,
                status = excluded.status,
                plan_tier = excluded.plan_tier,
                current_period_start = excluded.current_period_start,
                current_period_end = excluded.current_period_end,
                cancel_at_period_end = excluded.cancel_at_period_end,
                canceled_at = excluded.canceled_at,
                trial_end = excluded.trial_end,
                metadata = excluded.metadata,
                updated_at = excluded.updated_at
            ",
        )
        .bind(subscription.id.to_string())
        .bind(subscription.tenant_id.to_string())
        .bind(subscription.user_id.to_string())
        .bind(&subscription.provider)
        .bind(&subscription.provider_customer_id)
        .bind(&subscription.provider_subscription_id)
        .bind(subscription.status.as_str())
        .bind(subscription.plan_tier.as_str())
        .bind(subscription.current_period_start.map(|d| d.to_rfc3339()))
        .bind(subscription.current_period_end.map(|d| d.to_rfc3339()))
        .bind(i64::from(subscription.cancel_at_period_end))
        .bind(subscription.canceled_at.map(|d| d.to_rfc3339()))
        .bind(subscription.trial_end.map(|d| d.to_rfc3339()))
        .bind(metadata_json)
        .bind(subscription.created_at.to_rfc3339())
        .bind(subscription.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert subscription: {e}")))?;

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

    async fn get_subscription_by_user(&self, user_id: Uuid) -> AppResult<Option<Subscription>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, provider, provider_customer_id, provider_subscription_id,
                   status, plan_tier, current_period_start, current_period_end,
                   cancel_at_period_end, canceled_at, trial_end, metadata,
                   created_at, updated_at
            FROM subscriptions
            WHERE user_id = ?1
            ORDER BY updated_at DESC
            LIMIT 1
            ",
        )
        .bind(user_id.to_string())
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
            WHERE tenant_id = ?1
            ORDER BY updated_at DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
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
            WHERE provider = ?1 AND provider_subscription_id = ?2
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
            WHERE provider = ?1 AND provider_customer_id = ?2
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
            WHERE status = ?1
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
            "SELECT COUNT(*) FROM billing_events WHERE provider = ?1 AND event_id = ?2",
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
            "INSERT OR IGNORE INTO billing_events (provider, event_id, event_type) VALUES (?1, ?2, ?3)",
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

fn row_to_subscription(row: &SqliteRow) -> AppResult<Subscription> {
    let id_str: String = row.try_get("id").map_err(|e| map_row_err(&e))?;
    let tenant_id_str: String = row.try_get("tenant_id").map_err(|e| map_row_err(&e))?;
    let user_id_str: String = row.try_get("user_id").map_err(|e| map_row_err(&e))?;
    let provider: String = row.try_get("provider").map_err(|e| map_row_err(&e))?;
    let provider_customer_id: String = row
        .try_get("provider_customer_id")
        .map_err(|e| map_row_err(&e))?;
    let provider_subscription_id: Option<String> = row
        .try_get("provider_subscription_id")
        .map_err(|e| map_row_err(&e))?;
    let status_str: String = row.try_get("status").map_err(|e| map_row_err(&e))?;
    let plan_tier_str: String = row.try_get("plan_tier").map_err(|e| map_row_err(&e))?;
    let current_period_start: Option<String> = row
        .try_get("current_period_start")
        .map_err(|e| map_row_err(&e))?;
    let current_period_end: Option<String> = row
        .try_get("current_period_end")
        .map_err(|e| map_row_err(&e))?;
    let cancel_at_period_end: i64 = row
        .try_get("cancel_at_period_end")
        .map_err(|e| map_row_err(&e))?;
    let canceled_at: Option<String> = row.try_get("canceled_at").map_err(|e| map_row_err(&e))?;
    let trial_end: Option<String> = row.try_get("trial_end").map_err(|e| map_row_err(&e))?;
    let metadata_str: Option<String> = row.try_get("metadata").map_err(|e| map_row_err(&e))?;
    let created_at: String = row.try_get("created_at").map_err(|e| map_row_err(&e))?;
    let updated_at: String = row.try_get("updated_at").map_err(|e| map_row_err(&e))?;

    Ok(Subscription {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::internal(format!("invalid subscription id uuid: {e}")))?,
        tenant_id: TenantId::from_str(&tenant_id_str)
            .map_err(|e| AppError::internal(format!("invalid tenant_id: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("invalid user_id uuid: {e}")))?,
        provider,
        provider_customer_id,
        provider_subscription_id,
        status: SubscriptionStatus::from_str(&status_str)
            .map_err(|e| AppError::internal(format!("invalid status: {e}")))?,
        plan_tier: UserTier::from_str(&plan_tier_str)
            .map_err(|e| AppError::internal(format!("invalid plan_tier: {e}")))?,
        current_period_start: current_period_start.as_deref().map(parse_dt).transpose()?,
        current_period_end: current_period_end.as_deref().map(parse_dt).transpose()?,
        cancel_at_period_end: cancel_at_period_end != 0,
        canceled_at: canceled_at.as_deref().map(parse_dt).transpose()?,
        trial_end: trial_end.as_deref().map(parse_dt).transpose()?,
        metadata: metadata_str
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| AppError::internal(format!("invalid metadata json: {e}")))?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn parse_dt(s: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::internal(format!("invalid timestamp '{s}': {e}")))
}

fn map_row_err(e: &sqlx::Error) -> AppError {
    AppError::database(format!("subscription row decode failed: {e}"))
}
