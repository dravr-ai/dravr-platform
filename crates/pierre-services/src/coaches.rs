// ABOUTME: Coach business logic extracted from route handlers
// ABOUTME: Prerequisites checking, bulk assignment, rejection formatting, and provider display
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::hash::BuildHasher;

use pierre_config::coach_recommendations::CoachRecommendationConfig;
use pierre_core::models::coaches::CoachPrerequisites;
use pierre_core::models::SportProfile;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::database::repositories::CoachesRepository;
use pierre_database::database::repositories::TenantRepository;
use uuid::Uuid;

/// A missing prerequisite for a coach, protocol-agnostic
#[derive(Debug)]
pub struct MissingPrerequisite {
    /// Type of prerequisite (provider, `activity_count`, `activity_type`)
    pub prerequisite_type: String,
    /// The specific requirement (e.g., "strava", "50 activities", "Run")
    pub requirement: String,
    /// Human-readable message explaining what's missing
    pub message: String,
}

/// Result of a prerequisites check
#[derive(Debug)]
pub struct PrerequisiteCheckResult {
    /// Whether all prerequisites are met
    pub met: bool,
    /// List of missing prerequisites (empty if all met)
    pub missing: Vec<MissingPrerequisite>,
}

/// Check if prerequisites are met given user's connected providers
///
/// Validates that the user has connected all required fitness providers
/// specified in the coach's prerequisite configuration.
#[must_use]
pub fn check_prerequisites<S: BuildHasher>(
    prerequisites: &CoachPrerequisites,
    user_providers: &HashSet<String, S>,
) -> PrerequisiteCheckResult {
    let mut missing = Vec::new();

    for provider in &prerequisites.providers {
        let provider_lower = provider.to_lowercase();
        if !user_providers.contains(&provider_lower) {
            missing.push(MissingPrerequisite {
                prerequisite_type: "provider".to_owned(),
                requirement: provider.clone(),
                message: format!(
                    "Connect {} to unlock this coach",
                    capitalize_provider(provider)
                ),
            });
        }
    }

    let met = missing.is_empty();
    PrerequisiteCheckResult { met, missing }
}

/// Outcome of scoring one coach against a user's profile.
#[derive(Debug, Clone, Copy)]
pub struct CoachRecommendation {
    /// Whether this coach is *eligible* for the "Recommended for you" set
    /// (providers connected and either a sport match or a cold-start starter).
    /// The caller still caps the surfaced set to `max_recommended` by score.
    pub eligible: bool,
    /// Relevance score in `0.0..=1.0` (higher is a better match). Used to rank
    /// the recommended set; ineligible coaches keep their score for
    /// transparency but stay in the full catalog.
    pub match_score: f32,
}

/// Score a single coach against the user's connected providers and recent
/// sport mix, using the env-tunable [`CoachRecommendationConfig`].
///
/// A coach is only ever eligible when its provider prerequisites are met.
/// Beyond that:
/// - **Sport-specific** coaches (non-empty `activity_types`) score by how much
///   the user's active sports overlap the coach's required types.
/// - **Sport-agnostic** coaches (empty `activity_types`) help everyone, so once
///   their providers are connected they get `sport_agnostic_base_score`.
/// - With **no activity profile yet** (cold start), only the broadly-useful,
///   provider-free starter coaches are eligible — everything sport- or
///   provider-gated waits until we can see what the user actually does.
#[must_use]
pub fn score_coach<S: BuildHasher>(
    prerequisites: &CoachPrerequisites,
    profile: Option<&SportProfile>,
    user_providers: &HashSet<String, S>,
    config: &CoachRecommendationConfig,
) -> CoachRecommendation {
    // Provider gate: a coach that needs a fitness provider requires the user to
    // have at least one connected. The prerequisite names a specific provider
    // (e.g. `strava`) but it really expresses "needs activity data" — a Garmin
    // runner is just as valid for a running coach as a Strava one. So we gate on
    // "has any provider connected", not the exact name; the activity_types
    // overlap below is what actually establishes sport relevance. A coach with
    // no provider prerequisite (sport-agnostic) is never gated here.
    let needs_provider = !prerequisites.providers.is_empty();
    if needs_provider && user_providers.is_empty() {
        return CoachRecommendation {
            eligible: false,
            match_score: 0.0,
        };
    }

    let sport_agnostic = prerequisites.activity_types.is_empty();

    match profile {
        Some(profile) if profile.total_activities > 0 => {
            if sport_agnostic {
                CoachRecommendation {
                    eligible: true,
                    match_score: config.sport_agnostic_base_score,
                }
            } else {
                let overlap = profile.activity_type_overlap(
                    &prerequisites.activity_types,
                    config.min_activities_for_sport,
                    config.min_share_for_sport,
                );
                let min_met = profile.total_activities >= prerequisites.min_activities;
                let match_score = if min_met {
                    overlap
                } else {
                    overlap * config.below_min_activities_penalty
                };
                CoachRecommendation {
                    eligible: overlap > 0.0,
                    match_score,
                }
            }
        }
        _ => {
            // Cold start: recommend only the broadly-useful coaches that need
            // neither a provider nor a specific sport.
            let starter = sport_agnostic && prerequisites.providers.is_empty();
            CoachRecommendation {
                eligible: starter,
                match_score: if starter {
                    config.sport_agnostic_base_score
                } else {
                    0.0
                },
            }
        }
    }
}

/// Format a rejection reason by combining the base reason with optional notes
///
/// Returns just the reason if notes are empty or absent,
/// otherwise combines them as "reason: notes".
#[must_use]
pub fn format_rejection_reason(reason: &str, notes: Option<&str>) -> String {
    match notes {
        Some(n) if !n.trim().is_empty() => format!("{reason}: {}", n.trim()),
        _ => reason.to_owned(),
    }
}

/// Result of a bulk coach assignment operation
#[derive(Debug)]
pub struct BulkAssignmentResult {
    /// Number of users successfully assigned/unassigned
    pub affected_count: usize,
    /// Total number of users requested
    pub total_requested: usize,
}

/// Assign a coach to multiple users after verifying tenant membership
///
/// Each target user is validated to belong to the specified tenant before
/// the assignment is made. Fails on the first tenant membership violation.
///
/// # Errors
///
/// Returns error if any user ID is invalid, any user doesn't belong to the tenant,
/// or any database operation fails
pub async fn bulk_assign_coach<DB: TenantRepository + ?Sized>(
    manager: &dyn CoachesRepository,
    database: &DB,
    coach_id: &str,
    tenant_id: TenantId,
    admin_user_id: Uuid,
    user_ids: &[String],
) -> AppResult<BulkAssignmentResult> {
    let mut assigned_count = 0;

    for user_id_str in user_ids {
        let user_id = Uuid::parse_str(user_id_str)
            .map_err(|_| AppError::invalid_input(format!("Invalid user ID: {user_id_str}")))?;

        verify_tenant_membership(database, user_id, tenant_id).await?;

        if manager
            .assign_coach(coach_id, user_id, admin_user_id)
            .await?
        {
            assigned_count += 1;
        }
    }

    Ok(BulkAssignmentResult {
        affected_count: assigned_count,
        total_requested: user_ids.len(),
    })
}

/// Unassign a coach from multiple users after verifying tenant membership
///
/// Each target user is validated to belong to the specified tenant before
/// the unassignment. Fails on the first tenant membership violation.
///
/// # Errors
///
/// Returns error if any user ID is invalid, any user doesn't belong to the tenant,
/// or any database operation fails
pub async fn bulk_unassign_coach<DB: TenantRepository + ?Sized>(
    manager: &dyn CoachesRepository,
    database: &DB,
    coach_id: &str,
    tenant_id: TenantId,
    user_ids: &[String],
) -> AppResult<BulkAssignmentResult> {
    let mut removed_count = 0;

    for user_id_str in user_ids {
        let user_id = Uuid::parse_str(user_id_str)
            .map_err(|_| AppError::invalid_input(format!("Invalid user ID: {user_id_str}")))?;

        verify_tenant_membership(database, user_id, tenant_id).await?;

        if manager.unassign_coach(coach_id, user_id).await? {
            removed_count += 1;
        }
    }

    Ok(BulkAssignmentResult {
        affected_count: removed_count,
        total_requested: user_ids.len(),
    })
}

/// Verify that a user belongs to a specific tenant
///
/// # Errors
///
/// Returns error if the user is not a member of the specified tenant
async fn verify_tenant_membership<DB: TenantRepository + ?Sized>(
    database: &DB,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<()> {
    let user_tenants = database.list_for_user(user_id).await.map_err(|e| {
        AppError::database(format!(
            "Failed to verify tenant membership for user {user_id}: {e}"
        ))
    })?;

    if !user_tenants.iter().any(|t| t.id == tenant_id) {
        return Err(AppError::auth_invalid(format!(
            "User {user_id} does not belong to this tenant"
        )));
    }

    Ok(())
}

/// Capitalize provider name for user-friendly display
#[must_use]
pub fn capitalize_provider(provider: &str) -> String {
    let provider_lower = provider.to_lowercase();
    match provider_lower.as_str() {
        "strava" => "Strava".to_owned(),
        "garmin" => "Garmin".to_owned(),
        "fitbit" => "Fitbit".to_owned(),
        "terra" => "Terra".to_owned(),
        _ => {
            let mut chars = provider.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}
