// ABOUTME: PostgreSQL seed writes for the social graph — settings, friends, insights, reactions
// ABOUTME: Free functions over the pool so the SeederRepository impl stays inside its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Social-graph seeding.
//!
//! Split out of the `SeederRepository` impl purely for file size: a single
//! trait impl cannot span modules, so the bodies move here and the trait
//! methods delegate. Each takes the pool the impl would have used.

use pierre_core::errors::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::seed_models::{
    SeedAdaptedInsight, SeedFriendConnection, SeedInsightReaction, SeedSharedInsight,
    SeedSocialSettings,
};

pub(super) async fn seed_reset_social_data(pool: &PgPool) -> AppResult<()> {
    // Delete in FK order: dependent tables first
    sqlx::query("DELETE FROM adapted_insights")
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reset adapted_insights: {e}")))?;

    sqlx::query("DELETE FROM insight_reactions")
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reset insight_reactions: {e}")))?;

    sqlx::query("DELETE FROM shared_insights")
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reset shared_insights: {e}")))?;

    sqlx::query("DELETE FROM friend_connections")
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reset friend_connections: {e}")))?;

    sqlx::query("DELETE FROM user_social_settings")
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reset user_social_settings: {e}")))?;

    Ok(())
}

pub(super) async fn seed_upsert_social_settings(
    pool: &PgPool,
    settings: &SeedSocialSettings,
) -> AppResult<bool> {
    let existing = sqlx::query("SELECT user_id FROM user_social_settings WHERE user_id = $1")
        .bind(settings.user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to check social settings existence: {e}"))
        })?;

    if existing.is_some() {
        return Ok(false);
    }

    sqlx::query(
        "INSERT INTO user_social_settings \
         (user_id, discoverable, default_visibility, share_activity_types, \
          notify_friend_requests, notify_insight_reactions, notify_adapted_insights, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, $4, true, true, true, $5, $6)",
    )
    .bind(settings.user_id)
    .bind(settings.discoverable)
    .bind(&settings.default_visibility)
    .bind(&settings.share_activity_types)
    .bind(settings.created_at)
    .bind(settings.updated_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to insert social settings: {e}")))?;

    Ok(true)
}

pub(super) async fn seed_insert_friend_connection_if_absent(
    pool: &PgPool,
    conn: &SeedFriendConnection,
) -> AppResult<bool> {
    let existing = sqlx::query(
        "SELECT id FROM friend_connections \
         WHERE (initiator_id = $1 AND receiver_id = $2) \
            OR (initiator_id = $2 AND receiver_id = $1)",
    )
    .bind(conn.initiator_id)
    .bind(conn.receiver_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check friend connection existence: {e}")))?;

    if existing.is_some() {
        return Ok(false);
    }

    // friend_connections.id is UUID (native)
    sqlx::query(
        "INSERT INTO friend_connections \
         (id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(conn.id)
    .bind(conn.initiator_id)
    .bind(conn.receiver_id)
    .bind(&conn.status)
    .bind(conn.created_at)
    .bind(conn.updated_at)
    .bind(conn.accepted_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to insert friend connection: {e}")))?;

    Ok(true)
}

pub(super) async fn seed_insert_shared_insight(
    pool: &PgPool,
    insight: &SeedSharedInsight,
) -> AppResult<()> {
    // shared_insights.id is TEXT, user_id is UUID
    sqlx::query(
        "INSERT INTO shared_insights \
         (id, user_id, visibility, insight_type, sport_type, content, title, \
          training_phase, reaction_count, adapt_count, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 0, $9, $10)",
    )
    .bind(insight.id.to_string())
    .bind(insight.user_id)
    .bind(&insight.visibility)
    .bind(&insight.insight_type)
    .bind(&insight.sport_type)
    .bind(&insight.content)
    .bind(&insight.title)
    .bind(&insight.training_phase)
    .bind(insight.created_at)
    .bind(insight.updated_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to insert shared insight: {e}")))?;

    Ok(())
}

pub(super) async fn seed_get_shared_insight_ids(pool: &PgPool) -> AppResult<Vec<Uuid>> {
    // shared_insights.id is TEXT, parse to Uuid
    let rows = sqlx::query("SELECT id FROM shared_insights")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get shared insight IDs: {e}")))?;

    rows.into_iter()
        .map(|r| {
            let id_str: String = r.get("id");
            id_str.parse::<Uuid>().map_err(|e| {
                AppError::database(format!("Failed to parse shared insight ID as UUID: {e}"))
            })
        })
        .collect()
}

pub(super) async fn seed_insert_reaction_if_absent(
    pool: &PgPool,
    reaction: &SeedInsightReaction,
) -> AppResult<bool> {
    // insight_reactions.insight_id is TEXT, user_id is UUID
    let existing =
        sqlx::query("SELECT id FROM insight_reactions WHERE insight_id = $1 AND user_id = $2")
            .bind(reaction.insight_id.to_string())
            .bind(reaction.user_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check reaction existence: {e}")))?;

    if existing.is_some() {
        return Ok(false);
    }

    sqlx::query(
        "INSERT INTO insight_reactions \
         (id, insight_id, user_id, reaction_type, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(reaction.id.to_string())
    .bind(reaction.insight_id.to_string())
    .bind(reaction.user_id)
    .bind(&reaction.reaction_type)
    .bind(reaction.created_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to insert insight reaction: {e}")))?;

    Ok(true)
}

pub(super) async fn seed_get_shared_insights_with_authors(
    pool: &PgPool,
) -> AppResult<Vec<(Uuid, Uuid)>> {
    // shared_insights.id is TEXT, user_id is UUID
    let rows = sqlx::query("SELECT id, user_id FROM shared_insights")
        .fetch_all(pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get shared insights with authors: {e}"))
        })?;

    rows.into_iter()
        .map(|r| {
            let id_str: String = r.get("id");
            let insight_id = id_str.parse::<Uuid>().map_err(|e| {
                AppError::database(format!("Failed to parse insight ID as UUID: {e}"))
            })?;
            let author_id: Uuid = r.get("user_id");
            Ok((insight_id, author_id))
        })
        .collect()
}

pub(super) async fn seed_insert_adapted_insight_if_absent(
    pool: &PgPool,
    adapted: &SeedAdaptedInsight,
) -> AppResult<bool> {
    // adapted_insights.id is TEXT, source_insight_id is TEXT, user_id is UUID
    let existing = sqlx::query(
        "SELECT id FROM adapted_insights WHERE source_insight_id = $1 AND user_id = $2",
    )
    .bind(adapted.source_insight_id.to_string())
    .bind(adapted.user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check adapted insight existence: {e}")))?;

    if existing.is_some() {
        return Ok(false);
    }

    sqlx::query(
        "INSERT INTO adapted_insights \
         (id, source_insight_id, user_id, adapted_content, adaptation_context, \
          was_helpful, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(adapted.id.to_string())
    .bind(adapted.source_insight_id.to_string())
    .bind(adapted.user_id)
    .bind(&adapted.adapted_content)
    .bind(&adapted.adaptation_context)
    .bind(adapted.was_helpful)
    .bind(adapted.created_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to insert adapted insight: {e}")))?;

    Ok(true)
}

pub(super) async fn seed_get_shared_insights_not_by_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> AppResult<Vec<Uuid>> {
    // shared_insights.id is TEXT, user_id is UUID
    let rows = sqlx::query(
        "SELECT id FROM shared_insights WHERE user_id != $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get shared insights not by user: {e}")))?;

    rows.into_iter()
        .map(|r| {
            let id_str: String = r.get("id");
            id_str
                .parse::<Uuid>()
                .map_err(|e| AppError::database(format!("Failed to parse insight ID as UUID: {e}")))
        })
        .collect()
}
