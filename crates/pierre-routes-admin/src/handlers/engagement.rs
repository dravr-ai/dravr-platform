// ABOUTME: GET /api/admin/engagement — activity tiers, agent leaderboard and published count, all in one admin scope
// ABOUTME: A super admin reads the platform or one picked tenant; a plain admin reads their own tenant only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin Engagement screen.
//!
//! Every figure is read in the one scope the scope rule grants the caller, so a
//! tenant's screen never mixes the platform's user counts with its own agents.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::AppResult;

use super::admin_scope::console_read_scope;
use crate::context::AdminApiContext;

/// The agents the leaderboard lists.
const LEADERBOARD_SIZE: usize = 20;

/// What the screen asks to read.
#[derive(Debug, Deserialize)]
pub struct EngagementQuery {
    /// What the caller asks to read; see [`pierre_core::admin::scope`].
    pub tenant_id: Option<String>,
}

/// Users by last activity. The tiers are cumulative: a daily user is also
/// weekly and monthly.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ActivityTiers {
    /// Every user in scope.
    pub total: u32,
    /// Active in the last 24 hours.
    pub daily: u32,
    /// Active in the last 7 days.
    pub weekly: u32,
    /// Active in the last 30 days.
    pub monthly: u32,
    /// Not active in the last 30 days, or never.
    pub dormant: u32,
}

/// One agent on the leaderboard.
#[derive(Debug, Serialize)]
pub struct LeaderboardAgent {
    /// Agent id.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Category slug.
    pub category: String,
    /// Uses by the scope's users, all time.
    pub use_count: u32,
}

/// The Engagement screen's figures.
#[derive(Debug, Serialize)]
pub struct EngagementResponse {
    /// The tenants every figure covers.
    pub scope: AdminScope,
    /// Users by last activity.
    pub users: ActivityTiers,
    /// System agents in scope.
    pub total_agents: usize,
    /// Store listings published in scope.
    pub published_agents: u32,
    /// The most used system agents, most used first, ties by title.
    pub leaderboard: Vec<LeaderboardAgent>,
}

/// Bucket last-activity instants into cumulative tiers as of `now`.
fn activity_tiers(last_active: &[Option<DateTime<Utc>>], now: DateTime<Utc>) -> ActivityTiers {
    let mut tiers = ActivityTiers::default();
    for at in last_active {
        tiers.total += 1;
        let Some(at) = at else {
            tiers.dormant += 1;
            continue;
        };
        let since = now - *at;
        if since <= Duration::days(1) {
            tiers.daily += 1;
        }
        if since <= Duration::days(7) {
            tiers.weekly += 1;
        }
        if since <= Duration::days(30) {
            tiers.monthly += 1;
        } else {
            tiers.dormant += 1;
        }
    }
    tiers
}

/// Handle `GET /api/admin/engagement`.
///
/// # Errors
///
/// Returns an error when the scope rule refuses the caller or a read fails.
pub async fn handle_get_engagement(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<EngagementQuery>,
) -> AppResult<impl IntoResponse> {
    let scope =
        console_read_scope(&context.repos, &admin_token, params.tenant_id.as_deref()).await?;
    let repos = &context.repos;

    let last_active = repos.engagement.last_active_in_scope(scope).await?;
    let agents = match scope {
        AdminScope::AllTenants => repos.agents.list_catalogue_system_agents().await?,
        AdminScope::Tenant { tenant_id } => repos.agents.list_system_agents(tenant_id).await?,
    };
    let uses = repos.engagement.agent_uses_in_scope(scope).await?;
    let published_agents = repos.engagement.published_listings_in_scope(scope).await?;

    let total_agents = agents.len();
    let mut leaderboard: Vec<LeaderboardAgent> = agents
        .into_iter()
        .map(|agent| {
            let id = agent.id.to_string();
            LeaderboardAgent {
                use_count: uses.get(&id).copied().unwrap_or(0),
                id,
                title: agent.title,
                category: agent.category.as_str().to_owned(),
            }
        })
        .collect();
    leaderboard.sort_by(|a, b| {
        b.use_count
            .cmp(&a.use_count)
            .then_with(|| a.title.cmp(&b.title))
    });
    leaderboard.truncate(LEADERBOARD_SIZE);

    Ok((
        StatusCode::OK,
        Json(EngagementResponse {
            scope,
            users: activity_tiers(&last_active, Utc::now()),
            total_agents,
            published_agents,
            leaderboard,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tiers_are_cumulative_and_a_user_never_seen_is_dormant() {
        let now = Utc::now();
        let tiers = activity_tiers(
            &[
                Some(now - Duration::hours(2)),
                Some(now - Duration::days(3)),
                Some(now - Duration::days(20)),
                Some(now - Duration::days(45)),
                None,
            ],
            now,
        );
        assert_eq!(
            tiers,
            ActivityTiers {
                total: 5,
                daily: 1,
                weekly: 2,
                monthly: 3,
                dormant: 2,
            }
        );
    }
}
