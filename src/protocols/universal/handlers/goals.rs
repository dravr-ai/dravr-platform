// ABOUTME: Goal management handlers for fitness objectives
// ABOUTME: Handle goal setting, tracking, and feasibility analysis

use crate::database_plugins::DatabaseProvider;
use crate::intelligence::goal_engine::GoalEngineTrait;
use crate::protocols::universal::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use std::future::Future;
use std::pin::Pin;

/// Handle `set_goal` tool - set a new fitness goal
#[must_use]
pub fn handle_set_goal(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use crate::utils::uuid::parse_user_id_for_protocol;

        let goal_type = request
            .parameters
            .get("goal_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProtocolError::InvalidRequest("goal_type is required".to_string()))?;

        let target_value = request
            .parameters
            .get("target_value")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| ProtocolError::InvalidRequest("target_value is required".to_string()))?;

        let timeframe = request
            .parameters
            .get("timeframe")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProtocolError::InvalidRequest("timeframe is required".to_string()))?;

        let title = request
            .parameters
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Fitness Goal");

        // Parse user ID
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Save goal to database
        let created_at = chrono::Utc::now();
        let goal_data = serde_json::json!({
            "goal_type": goal_type,
            "target_value": target_value,
            "timeframe": timeframe,
            "title": title,
            "created_at": created_at.to_rfc3339()
        });

        let goal_id = (*executor.resources.database)
            .create_goal(user_uuid, goal_data)
            .await
            .map_err(|e| ProtocolError::InternalError(format!("Database error: {e}")))?;

        Ok(UniversalResponse {
            success: true,
            result: Some(serde_json::json!({
                "goal_id": goal_id,
                "goal_type": goal_type,
                "target_value": target_value,
                "timeframe": timeframe,
                "title": title,
                "created_at": created_at.to_rfc3339(),
                "status": "created"
            })),
            error: None,
            metadata: None,
        })
    })
}

/// Handle `suggest_goals` tool - get AI-suggested fitness goals
#[must_use]
pub fn handle_suggest_goals(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use crate::constants::{limits, user_defaults};
        use crate::utils::uuid::parse_user_id_for_protocol;

        // Parse user ID
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Get recent activities using token-based approach
        let mut activities = Vec::new();
        if let Ok(Some(_token_data)) = executor
            .get_valid_token(
                user_uuid,
                crate::constants::oauth_providers::STRAVA,
                request.tenant_id.as_deref(),
            )
            .await
        {
            // Create provider and get activities (simplified approach)
            if let Ok(provider) =
                crate::providers::create_provider(crate::constants::oauth_providers::STRAVA)
            {
                if let Ok(provider_activities) = provider.get_activities(Some(10), None).await {
                    activities = provider_activities;
                }
            }
        }

        // Use the goal engine from intelligence module
        let goal_engine = crate::intelligence::goal_engine::AdvancedGoalEngine::new();

        // Create a default user profile for the goal engine
        let user_profile = crate::intelligence::UserFitnessProfile {
            user_id: request.user_id.clone(),
            age: Some(i32::try_from(user_defaults::DEFAULT_USER_AGE).unwrap_or(30)),
            gender: None,
            weight: None,
            height: None,
            fitness_level: crate::intelligence::FitnessLevel::Intermediate,
            primary_sports: vec!["general".into()],
            training_history_months: 6,
            preferences: crate::intelligence::UserPreferences {
                preferred_units: "metric".into(),
                training_focus: vec!["endurance".into()],
                injury_history: vec![],
                time_availability: crate::intelligence::TimeAvailability {
                    hours_per_week: 5.0,
                    preferred_days: vec!["Monday".into(), "Wednesday".into(), "Friday".into()],
                    preferred_duration_minutes: Some(
                        i32::try_from(limits::MINUTES_PER_HOUR).unwrap_or(60),
                    ),
                },
            },
        };

        match goal_engine.suggest_goals(&user_profile, &activities).await {
            Ok(suggestions) => Ok(UniversalResponse {
                success: true,
                result: Some(serde_json::json!({
                    "suggested_goals": suggestions.into_iter().map(|g| {
                        serde_json::json!({
                            "goal_type": format!("{:?}", g.goal_type),
                            "target_value": g.suggested_target,
                            "difficulty": format!("{:?}", g.difficulty),
                            "rationale": g.rationale,
                            "estimated_timeline_days": g.estimated_timeline_days,
                            "success_probability": g.success_probability
                        })
                    }).collect::<Vec<_>>(),
                    "activities_analyzed": activities.len()
                })),
                error: None,
                metadata: Some({
                    let mut map = std::collections::HashMap::with_capacity(4);
                    map.insert(
                        "analysis_engine".into(),
                        serde_json::Value::String("smart_goal_engine".into()),
                    );
                    map.insert(
                        "suggestion_algorithm".into(),
                        serde_json::Value::String("adaptive_goal_generation".into()),
                    );
                    map
                }),
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to suggest goals: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `analyze_goal_feasibility` tool - analyze if goal is achievable
#[must_use]
pub fn handle_analyze_goal_feasibility(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    _request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // TODO: Extract from original universal.rs
        Ok(UniversalResponse {
            success: false,
            result: None,
            error: Some("Handler not yet implemented".to_string()),
            metadata: None,
        })
    })
}

/// Handle `track_progress` tool - track progress towards goals
#[must_use]
pub fn handle_track_progress(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    _request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // TODO: Extract from original universal.rs
        Ok(UniversalResponse {
            success: false,
            result: None,
            error: Some("Handler not yet implemented".to_string()),
            metadata: None,
        })
    })
}
