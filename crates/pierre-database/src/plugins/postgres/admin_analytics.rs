// ABOUTME: PostgreSQL implementation of admin analytics queries for the dashboard
// ABOUTME: Provides tenant-scoped aggregate queries using PostgreSQL date functions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::AdminAnalyticsRepository;
use super::PostgresDatabase;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::admin_analytics::{
    CategoryCountRow, DailyActiveUsersRow, DailyConversationRow, EngagementTiers, GroupStatsRow,
    LlmCostAggregateRow, ProviderCountRow, TopCoachRow,
};
use pierre_core::models::TenantId;
use sqlx::Row;

#[async_trait]
impl AdminAnalyticsRepository for PostgresDatabase {
    async fn count_active_users(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(DISTINCT u.id)
            FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id
            WHERE tu.tenant_id = $1 AND u.last_active >= $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count active users: {e}")))?;

        Ok(count)
    }

    async fn count_users_for_tenant(&self, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM tenant_users
            WHERE tenant_id = $1 AND is_active = 1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count tenant users: {e}")))?;

        Ok(count)
    }

    async fn count_conversations_since(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM chat_conversations
            WHERE tenant_id = $1 AND created_at >= $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count conversations: {e}")))?;

        Ok(count)
    }

    async fn count_messages_since(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM chat_messages cm
            INNER JOIN chat_conversations cc ON cm.conversation_id = cc.id
            WHERE cc.tenant_id = $1 AND cm.created_at >= $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count messages: {e}")))?;

        Ok(count)
    }

    async fn get_llm_usage_for_cost(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<LlmCostAggregateRow>> {
        let rows = sqlx::query(
            r"
            SELECT provider, model,
                   SUM(prompt_tokens) as prompt_tokens,
                   SUM(completion_tokens) as completion_tokens
            FROM llm_usage
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY provider, model
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get LLM usage for cost: {e}")))?;

        let results = rows
            .iter()
            .map(|row| LlmCostAggregateRow {
                provider: row.get("provider"),
                model: row.get("model"),
                prompt_tokens: row.get("prompt_tokens"),
                completion_tokens: row.get("completion_tokens"),
            })
            .collect();

        Ok(results)
    }

    async fn count_users_with_coach_assignment(&self, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(DISTINCT ca.user_id)
            FROM coach_assignments ca
            INNER JOIN tenant_users tu ON ca.user_id = tu.user_id
            WHERE tu.tenant_id = $1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count users with coach assignment: {e}"))
        })?;

        Ok(count)
    }

    async fn count_connected_providers(&self, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM provider_connections
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count connected providers: {e}")))?;

        Ok(count)
    }

    async fn count_active_groups(&self, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM coaching_groups
            WHERE tenant_id = $1 AND is_active = 1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count active groups: {e}")))?;

        Ok(count)
    }

    async fn get_daily_conversation_series(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<DailyConversationRow>> {
        let rows = sqlx::query(
            r"
            SELECT SUBSTRING(created_at FROM 1 FOR 10) as date, COUNT(*) as count
            FROM chat_conversations
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY SUBSTRING(created_at FROM 1 FOR 10)
            ORDER BY date ASC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get daily conversation series: {e}")))?;

        let results = rows
            .iter()
            .map(|row| DailyConversationRow {
                date: row.get("date"),
                count: row.get("count"),
            })
            .collect();

        Ok(results)
    }

    async fn get_peak_conversation_hour(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<Option<i64>> {
        // PG: extract hour from ISO 8601 text using SUBSTRING
        let row = sqlx::query(
            r"
            SELECT CAST(SUBSTRING(created_at FROM 12 FOR 2) AS INTEGER) as hour, COUNT(*) as cnt
            FROM chat_conversations
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY hour
            ORDER BY cnt DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get peak conversation hour: {e}")))?;

        Ok(row.map(|r| r.get("hour")))
    }

    async fn get_avg_messages_per_conversation(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<f64> {
        let row = sqlx::query(
            r"
            SELECT
                COALESCE(AVG(msg_count), 0.0) as avg_msgs
            FROM (
                SELECT cc.id, COUNT(cm.id) as msg_count
                FROM chat_conversations cc
                LEFT JOIN chat_messages cm ON cm.conversation_id = cc.id
                WHERE cc.tenant_id = $1 AND cc.created_at >= $2
                GROUP BY cc.id
            ) sub
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get avg messages per conversation: {e}"))
        })?;

        let avg: f64 = row.get("avg_msgs");
        Ok(avg)
    }

    async fn get_top_coaches(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<TopCoachRow>> {
        let rows = sqlx::query(
            r"
            SELECT c.title, c.category, COUNT(ca.id) as assignment_count
            FROM coaches c
            INNER JOIN coach_assignments ca ON c.id = ca.coach_id
            WHERE c.tenant_id = $1
            GROUP BY c.id, c.title, c.category
            ORDER BY assignment_count DESC
            LIMIT $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get top coaches: {e}")))?;

        let results = rows
            .iter()
            .map(|row| TopCoachRow {
                title: row.get("title"),
                category: row.get("category"),
                assignment_count: row.get("assignment_count"),
            })
            .collect();

        Ok(results)
    }

    async fn get_coach_category_distribution(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CategoryCountRow>> {
        let rows = sqlx::query(
            r"
            SELECT category, COUNT(*) as count
            FROM coaches
            WHERE tenant_id = $1
            GROUP BY category
            ORDER BY count DESC
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get coach category distribution: {e}"))
        })?;

        let results = rows
            .iter()
            .map(|row| CategoryCountRow {
                category: row.get("category"),
                count: row.get("count"),
            })
            .collect();

        Ok(results)
    }

    async fn get_system_vs_user_coach_counts(&self, tenant_id: TenantId) -> AppResult<(i64, i64)> {
        let row = sqlx::query(
            r"
            SELECT
                COALESCE(SUM(CASE WHEN is_system = 1 THEN 1 ELSE 0 END), 0) as system_count,
                COALESCE(SUM(CASE WHEN is_system = 0 THEN 1 ELSE 0 END), 0) as user_count
            FROM coaches
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get system vs user coach counts: {e}"))
        })?;

        let system_count: i64 = row.get("system_count");
        let user_count: i64 = row.get("user_count");
        Ok((system_count, user_count))
    }

    async fn count_store_installations(&self, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COALESCE(SUM(install_count), 0) FROM store_listings
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count store installations: {e}")))?;

        Ok(count)
    }

    async fn get_daily_active_users_series(
        &self,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<DailyActiveUsersRow>> {
        let rows = sqlx::query(
            r"
            SELECT SUBSTRING(ju.timestamp FROM 1 FOR 10) as date, COUNT(DISTINCT ju.user_id) as count
            FROM jwt_usage ju
            INNER JOIN tenant_users tu ON ju.user_id = tu.user_id
            WHERE tu.tenant_id = $1 AND ju.timestamp >= $2
            GROUP BY SUBSTRING(ju.timestamp FROM 1 FOR 10)
            ORDER BY date ASC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get daily active users series: {e}"))
        })?;

        let results = rows
            .iter()
            .map(|row| DailyActiveUsersRow {
                date: row.get("date"),
                count: row.get("count"),
            })
            .collect();

        Ok(results)
    }

    async fn get_engagement_tiers(&self, tenant_id: TenantId) -> AppResult<EngagementTiers> {
        // PG: use NOW() - INTERVAL for date arithmetic
        let row = sqlx::query(
            r"
            SELECT
                COALESCE(SUM(CASE WHEN u.last_active >= (NOW() - INTERVAL '1 day')::text THEN 1 ELSE 0 END), 0) as daily_active,
                COALESCE(SUM(CASE WHEN u.last_active >= (NOW() - INTERVAL '7 days')::text THEN 1 ELSE 0 END), 0) as weekly_active,
                COALESCE(SUM(CASE WHEN u.last_active >= (NOW() - INTERVAL '30 days')::text THEN 1 ELSE 0 END), 0) as monthly_active,
                COALESCE(SUM(CASE WHEN u.last_active < (NOW() - INTERVAL '30 days')::text THEN 1 ELSE 0 END), 0) as dormant
            FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id
            WHERE tu.tenant_id = $1 AND tu.is_active = 1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get engagement tiers: {e}")))?;

        Ok(EngagementTiers {
            daily_active: row.get("daily_active"),
            weekly_active: row.get("weekly_active"),
            monthly_active: row.get("monthly_active"),
            dormant: row.get("dormant"),
        })
    }

    async fn get_provider_distribution(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ProviderCountRow>> {
        let rows = sqlx::query(
            r"
            SELECT provider, COUNT(*) as count
            FROM provider_connections
            WHERE tenant_id = $1
            GROUP BY provider
            ORDER BY count DESC
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get provider distribution: {e}")))?;

        let results = rows
            .iter()
            .map(|row| ProviderCountRow {
                provider: row.get("provider"),
                count: row.get("count"),
            })
            .collect();

        Ok(results)
    }

    async fn get_group_stats(&self, tenant_id: TenantId) -> AppResult<GroupStatsRow> {
        let row = sqlx::query(
            r"
            SELECT
                COUNT(*) as total_groups,
                COALESCE(SUM(CASE WHEN is_active = 1 THEN 1 ELSE 0 END), 0) as active_groups,
                COALESCE((
                    SELECT COUNT(*) FROM coaching_group_members cgm
                    INNER JOIN coaching_groups cg ON cgm.group_id = cg.id
                    WHERE cg.tenant_id = $1 AND cgm.left_at IS NULL
                ), 0) as total_members
            FROM coaching_groups
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get group stats: {e}")))?;

        Ok(GroupStatsRow {
            total_groups: row.get("total_groups"),
            active_groups: row.get("active_groups"),
            total_members: row.get("total_members"),
        })
    }
}
