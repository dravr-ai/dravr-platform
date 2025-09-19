// ABOUTME: Strava API handlers with clean authentication and error handling
// ABOUTME: Single responsibility handlers that delegate auth to AuthService

use crate::constants::oauth_providers;
use crate::intelligence::physiological_constants::api_limits::{
    DEFAULT_ACTIVITY_LIMIT, MAX_ACTIVITY_LIMIT,
};
use crate::protocols::universal::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::utils::{json_responses::activity_not_found_error, uuid::parse_user_id_for_protocol};
use std::future::Future;
use std::pin::Pin;

/// Handle `get_activities` tool - retrieve user's fitness activities
#[must_use]
pub fn handle_get_activities(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract and validate parameters
        let requested_limit = request
            .parameters
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10)
            .try_into()
            .unwrap_or(DEFAULT_ACTIVITY_LIMIT);

        let limit = requested_limit.min(MAX_ACTIVITY_LIMIT);

        // Get valid Strava token (with automatic refresh if needed)
        match executor
            .get_valid_token(
                user_uuid,
                oauth_providers::STRAVA,
                request.tenant_id.as_deref(),
            )
            .await
        {
            Ok(Some(token_data)) => {
                // Create Strava provider with token
                match crate::providers::create_provider(oauth_providers::STRAVA) {
                    Ok(mut provider) => {
                        // Set credentials using the token data
                        let credentials = crate::providers::OAuth2Credentials {
                            client_id: std::env::var("STRAVA_CLIENT_ID").unwrap_or_default(),
                            client_secret: std::env::var("STRAVA_CLIENT_SECRET")
                                .unwrap_or_default(),
                            access_token: Some(token_data.access_token),
                            refresh_token: Some(token_data.refresh_token),
                            expires_at: Some(token_data.expires_at),
                            scopes: crate::constants::oauth::STRAVA_DEFAULT_SCOPES
                                .split(',')
                                .map(str::to_string)
                                .collect(),
                        };

                        if let Err(e) = provider.set_credentials(credentials).await {
                            return Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to set provider credentials: {e}")),
                                metadata: None,
                            });
                        }

                        // Get activities from provider
                        match provider.get_activities(Some(limit), None).await {
                            Ok(activities) => Ok(UniversalResponse {
                                success: true,
                                result: Some(serde_json::to_value(&activities).map_err(|e| {
                                    ProtocolError::SerializationError(format!(
                                        "Failed to serialize activities: {e}"
                                    ))
                                })?),
                                error: None,
                                metadata: Some({
                                    let mut map = std::collections::HashMap::new();
                                    map.insert(
                                        "total_activities".to_string(),
                                        serde_json::Value::Number(activities.len().into()),
                                    );
                                    map.insert(
                                        "limit_applied".to_string(),
                                        serde_json::Value::Number(limit.into()),
                                    );
                                    map.insert(
                                        "requested_limit".to_string(),
                                        serde_json::Value::Number(requested_limit.into()),
                                    );
                                    map
                                }),
                            }),
                            Err(e) => Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to fetch activities: {e}")),
                                metadata: None,
                            }),
                        }
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to create provider: {e}")),
                        metadata: None,
                    }),
                }
            }
            Ok(None) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(
                    "No valid Strava token found. Please connect your Strava account.".to_string(),
                ),
                metadata: None,
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `get_athlete` tool - retrieve user's athlete profile
#[must_use]
pub fn handle_get_athlete(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Get valid Strava token (with automatic refresh if needed)
        match executor
            .get_valid_token(
                user_uuid,
                oauth_providers::STRAVA,
                request.tenant_id.as_deref(),
            )
            .await
        {
            Ok(Some(token_data)) => {
                // Create Strava provider with token
                match crate::providers::create_provider(oauth_providers::STRAVA) {
                    Ok(mut provider) => {
                        // Set credentials using the token data
                        let credentials = crate::providers::OAuth2Credentials {
                            client_id: std::env::var("STRAVA_CLIENT_ID").unwrap_or_default(),
                            client_secret: std::env::var("STRAVA_CLIENT_SECRET")
                                .unwrap_or_default(),
                            access_token: Some(token_data.access_token),
                            refresh_token: Some(token_data.refresh_token),
                            expires_at: Some(token_data.expires_at),
                            scopes: crate::constants::oauth::STRAVA_DEFAULT_SCOPES
                                .split(',')
                                .map(str::to_string)
                                .collect(),
                        };

                        if let Err(e) = provider.set_credentials(credentials).await {
                            return Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to set provider credentials: {e}")),
                                metadata: None,
                            });
                        }

                        // Get athlete profile from provider
                        match provider.get_athlete().await {
                            Ok(athlete) => Ok(UniversalResponse {
                                success: true,
                                result: Some(serde_json::to_value(athlete).map_err(|e| {
                                    ProtocolError::SerializationError(format!(
                                        "Failed to serialize athlete: {e}"
                                    ))
                                })?),
                                error: None,
                                metadata: None,
                            }),
                            Err(e) => Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to fetch athlete profile: {e}")),
                                metadata: None,
                            }),
                        }
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to create provider: {e}")),
                        metadata: None,
                    }),
                }
            }
            Ok(None) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(
                    "No valid Strava token found. Please connect your Strava account.".to_string(),
                ),
                metadata: None,
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `get_stats` tool - retrieve user's performance statistics
#[must_use]
pub fn handle_get_stats(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Get valid Strava token (with automatic refresh if needed)
        match executor
            .get_valid_token(
                user_uuid,
                oauth_providers::STRAVA,
                request.tenant_id.as_deref(),
            )
            .await
        {
            Ok(Some(token_data)) => {
                // Create Strava provider with token
                match crate::providers::create_provider(oauth_providers::STRAVA) {
                    Ok(mut provider) => {
                        // Set credentials using the token data
                        let credentials = crate::providers::OAuth2Credentials {
                            client_id: std::env::var("STRAVA_CLIENT_ID").unwrap_or_default(),
                            client_secret: std::env::var("STRAVA_CLIENT_SECRET")
                                .unwrap_or_default(),
                            access_token: Some(token_data.access_token),
                            refresh_token: Some(token_data.refresh_token),
                            expires_at: Some(token_data.expires_at),
                            scopes: crate::constants::oauth::STRAVA_DEFAULT_SCOPES
                                .split(',')
                                .map(str::to_string)
                                .collect(),
                        };

                        if let Err(e) = provider.set_credentials(credentials).await {
                            return Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to set provider credentials: {e}")),
                                metadata: None,
                            });
                        }

                        // Get stats from provider
                        match provider.get_stats().await {
                            Ok(stats) => Ok(UniversalResponse {
                                success: true,
                                result: Some(serde_json::to_value(stats).map_err(|e| {
                                    ProtocolError::SerializationError(format!(
                                        "Failed to serialize stats: {e}"
                                    ))
                                })?),
                                error: None,
                                metadata: None,
                            }),
                            Err(e) => Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to fetch stats: {e}")),
                                metadata: None,
                            }),
                        }
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to create provider: {e}")),
                        metadata: None,
                    }),
                }
            }
            Ok(None) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(
                    "No valid Strava token found. Please connect your Strava account.".to_string(),
                ),
                metadata: None,
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `analyze_activity` tool - analyze a specific activity with intelligence
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn handle_analyze_activity(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract activity ID from parameters
        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::InvalidRequest("activity_id parameter required".to_string())
            })?;

        // Get valid Strava token (with automatic refresh if needed)
        match executor
            .get_valid_token(
                user_uuid,
                oauth_providers::STRAVA,
                request.tenant_id.as_deref(),
            )
            .await
        {
            Ok(Some(token_data)) => {
                // Create Strava provider with token
                match crate::providers::create_provider(oauth_providers::STRAVA) {
                    Ok(mut provider) => {
                        // Set credentials using the token data
                        let credentials = crate::providers::OAuth2Credentials {
                            client_id: std::env::var("STRAVA_CLIENT_ID").unwrap_or_default(),
                            client_secret: std::env::var("STRAVA_CLIENT_SECRET")
                                .unwrap_or_default(),
                            access_token: Some(token_data.access_token),
                            refresh_token: Some(token_data.refresh_token),
                            expires_at: Some(token_data.expires_at),
                            scopes: crate::constants::oauth::STRAVA_DEFAULT_SCOPES
                                .split(',')
                                .map(str::to_string)
                                .collect(),
                        };

                        if let Err(e) = provider.set_credentials(credentials).await {
                            return Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!("Failed to set provider credentials: {e}")),
                                metadata: None,
                            });
                        }

                        // Get all activities to find the specific one
                        match provider
                            .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                            .await
                        {
                            Ok(activities) => {
                                // Find the specific activity
                                if let Some(_activity) =
                                    activities.iter().find(|a| a.id == activity_id)
                                {
                                    // Use real activity intelligence from original implementation
                                    let analysis = executor
                                        .get_real_activity_intelligence(&request)
                                        .map_err(|e| {
                                            ProtocolError::InternalError(format!(
                                                "Analysis failed: {e}"
                                            ))
                                        })?;

                                    Ok(UniversalResponse {
                                        success: true,
                                        result: Some(serde_json::to_value(analysis).map_err(
                                            |e| {
                                                ProtocolError::SerializationError(format!(
                                                    "Failed to serialize analysis: {e}"
                                                ))
                                            },
                                        )?),
                                        error: None,
                                        metadata: Some({
                                            let mut map = std::collections::HashMap::new();
                                            map.insert(
                                                "activity_id".to_string(),
                                                serde_json::Value::String(activity_id.to_string()),
                                            );
                                            map.insert(
                                                "analysis_type".to_string(),
                                                serde_json::Value::String(
                                                    "comprehensive".to_string(),
                                                ),
                                            );
                                            map
                                        }),
                                    })
                                } else {
                                    Ok(UniversalResponse {
                                        success: false,
                                        result: Some(activity_not_found_error(
                                            activity_id,
                                            Some("Strava"),
                                        )),
                                        error: Some("Activity not found".to_string()),
                                        metadata: None,
                                    })
                                }
                            }
                            Err(e) => Ok(UniversalResponse {
                                success: false,
                                result: None,
                                error: Some(format!(
                                    "Failed to fetch activities for analysis: {e}"
                                )),
                                metadata: None,
                            }),
                        }
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to create provider: {e}")),
                        metadata: None,
                    }),
                }
            }
            Ok(None) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(
                    "No valid Strava token found. Please connect your Strava account.".to_string(),
                ),
                metadata: None,
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            }),
        }
    })
}
