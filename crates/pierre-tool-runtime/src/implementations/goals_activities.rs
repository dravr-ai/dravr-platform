// ABOUTME: Reads the activities the goal tools measure, merged into workouts, from the provider or its stored copy
// ABOUTME: A read the athlete's session-read cap refused is answered by the stored copy with its read_limit caveat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity reads behind `suggest_goals`, `track_progress` and
//! `analyze_goal_feasibility`.
//!
//! Each read goes through [`activities_or_stored`], so a read the athlete's
//! session-read cap refused is answered by their Strava grant or the stored
//! copy (carnet#1661), and the caveat a stored copy carries reaches the tool's
//! answer with the activities.

use dravr_cageux::physiological_constants::goal_feasibility::{
    GOAL_SUGGESTION_ACTIVITY_LIMIT, PROGRESS_TRACKING_ACTIVITY_LIMIT,
};
use pierre_config::constants::limits::ACTIVITY_CAPACITY_HINT;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::Activity;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use tracing::debug;
use uuid::Uuid;

use crate::context::ToolExecutionContext;
use crate::implementations::stored_copy::{activities_or_stored, Reader, Served};
use crate::protocol::auth::AuthService;

/// The workouts among a provider's `activities`: each workout's recordings
/// (an auto-split, a watch and a bike computer) merged into one session
/// ([`merge_duplicates`]), so a goal's progress counts and sums workouts.
fn sessions(activities: Vec<Activity>) -> Vec<Activity> {
    merge_duplicates(activities, &DedupConfig::from_env()).0
}

/// Fetch activities for goal suggestions, with the caveat of a stored copy
/// standing in for them; empty if auth/fetch fails.
pub(super) async fn fetch_suggestion_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> Served<Vec<Activity>> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    let mut activities = Served::live(Vec::new());
    if let Ok(provider) = auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        let store = Reader::new(&context.resources, user_uuid, tenant_id.as_deref());
        if let Ok(served) =
            activities_or_stored(store, provider.as_ref(), GOAL_SUGGESTION_ACTIVITY_LIMIT).await
        {
            activities = served.map(sessions);
        }
    }
    activities
}

/// Fetch activities for goal feasibility analysis, with the caveat of a
/// stored copy standing in for them; empty if auth/fetch fails.
pub(super) async fn fetch_feasibility_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> Served<Vec<Activity>> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    let mut activities = Served::live(Vec::with_capacity(ACTIVITY_CAPACITY_HINT));
    if let Ok(provider) = auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        let store = Reader::new(&context.resources, user_uuid, tenant_id.as_deref());
        if let Ok(served) =
            activities_or_stored(store, provider.as_ref(), PROGRESS_TRACKING_ACTIVITY_LIMIT).await
        {
            activities = served.map(sessions);
        }
    }
    activities
}

/// Fetch activities for progress tracking, with the caveat of a stored copy
/// standing in for them; returns error if auth fails.
pub(super) async fn fetch_progress_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> AppResult<Served<Vec<Activity>>> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    match auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        Ok(provider) => {
            debug!("Provider authenticated for progress tracking");
            let store = Reader::new(&context.resources, user_uuid, tenant_id.as_deref());
            let read =
                activities_or_stored(store, provider.as_ref(), PROGRESS_TRACKING_ACTIVITY_LIMIT);
            Ok(read
                .await
                .unwrap_or_else(|_| Served::live(Vec::new()))
                .map(sessions))
        }
        Err(response) => {
            debug!("Authentication failed for progress tracking");
            Err(AppError::external_service(
                "fitness_provider",
                response
                    .error
                    .unwrap_or_else(|| "Authentication failed".to_owned()),
            ))
        }
    }
}
