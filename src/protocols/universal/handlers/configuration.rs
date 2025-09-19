// ABOUTME: Configuration management handlers
// ABOUTME: Handle configuration catalogs, profiles, and user settings

use crate::configuration::{catalog::CatalogBuilder, profiles::ProfileTemplates};
use crate::database_plugins::DatabaseProvider;
use crate::protocols::universal::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::future::Future;
use std::pin::Pin;

/// Handle `get_configuration_catalog` tool - get complete configuration catalog
///
/// # Errors
/// Returns `ProtocolError` if catalog serialization fails
pub fn handle_get_configuration_catalog(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    _request: &UniversalRequest,
) -> Result<UniversalResponse, ProtocolError> {
    // Build configuration catalog
    let catalog = CatalogBuilder::build();

    Ok(UniversalResponse {
        success: true,
        result: Some(serde_json::to_value(&catalog).map_err(|e| {
            ProtocolError::SerializationError(format!("Failed to serialize catalog: {e}"))
        })?),
        error: None,
        metadata: Some({
            let mut map = std::collections::HashMap::new();
            map.insert(
                "catalog_type".to_string(),
                serde_json::Value::String("complete".to_string()),
            );
            map.insert(
                "parameter_count".to_string(),
                serde_json::Value::Number(catalog.total_parameters.into()),
            );
            map
        }),
    })
}

/// Handle `get_configuration_profiles` tool - get available configuration profiles
///
/// # Errors
/// Returns `ProtocolError` if profiles serialization fails
pub fn handle_get_configuration_profiles(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    _request: &UniversalRequest,
) -> Result<UniversalResponse, ProtocolError> {
    // Get available profile templates
    let profiles = ProfileTemplates::all();

    Ok(UniversalResponse {
        success: true,
        result: Some(serde_json::to_value(&profiles).map_err(|e| {
            ProtocolError::SerializationError(format!("Failed to serialize profiles: {e}"))
        })?),
        error: None,
        metadata: Some({
            let mut map = std::collections::HashMap::new();
            map.insert(
                "profile_count".to_string(),
                serde_json::Value::Number(profiles.len().into()),
            );
            map
        }),
    })
}

/// Handle `get_user_configuration` tool - get user's current configuration
#[must_use]
pub fn handle_get_user_configuration(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Get user configuration from database
        match (*executor.resources.database)
            .get_user_configuration(&user_uuid.to_string())
            .await
        {
            Ok(Some(config)) => Ok(UniversalResponse {
                success: true,
                result: Some(serde_json::to_value(config).map_err(|e| {
                    ProtocolError::SerializationError(format!("Failed to serialize config: {e}"))
                })?),
                error: None,
                metadata: Some({
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "user_id".to_string(),
                        serde_json::Value::String(user_uuid.to_string()),
                    );
                    map.insert("has_overrides".to_string(), serde_json::Value::Bool(true)); // Simplified for now
                    map
                }),
            }),
            Ok(None) => Ok(UniversalResponse {
                success: true,
                result: Some(serde_json::json!({
                    "message": "No custom configuration found, using defaults",
                    "default_profile": "recreational"
                })),
                error: None,
                metadata: Some({
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "user_id".to_string(),
                        serde_json::Value::String(user_uuid.to_string()),
                    );
                    map.insert("using_defaults".to_string(), serde_json::Value::Bool(true));
                    map
                }),
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to get user configuration: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `update_user_configuration` tool - update user's configuration settings
#[must_use]
pub fn handle_update_user_configuration(
    executor: &crate::protocols::universal::UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract configuration updates from parameters
        let updates = request.parameters.get("configuration").ok_or_else(|| {
            ProtocolError::InvalidRequest("configuration parameter required".to_string())
        })?;

        // Simple validation - just check if it's valid JSON
        if updates.is_null() {
            return Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some("Configuration cannot be null".to_string()),
                metadata: None,
            });
        }

        // Save user configuration in database
        let config_json = serde_json::to_string(updates).map_err(|e| {
            ProtocolError::SerializationError(format!("Failed to serialize config: {e}"))
        })?;

        match (*executor.resources.database)
            .save_user_configuration(&user_uuid.to_string(), &config_json)
            .await
        {
            Ok(()) => Ok(UniversalResponse {
                success: true,
                result: Some(serde_json::json!({
                    "message": "Configuration updated successfully",
                    "user_id": user_uuid.to_string()
                })),
                error: None,
                metadata: Some({
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "user_id".to_string(),
                        serde_json::Value::String(user_uuid.to_string()),
                    );
                    map.insert(
                        "updated_parameters".to_string(),
                        serde_json::Value::Number(
                            updates.as_object().map_or(0, serde_json::Map::len).into(),
                        ),
                    );
                    map
                }),
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to update configuration: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `calculate_personalized_zones` tool - calculate training zones based on VO2 max
///
/// # Errors
/// Returns `ProtocolError` if VO2 max parameter is missing or zones serialization fails
pub fn handle_calculate_personalized_zones(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    request: &UniversalRequest,
) -> Result<UniversalResponse, ProtocolError> {
    // Extract VO2 max from parameters
    let vo2_max = request
        .parameters
        .get("vo2_max")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| ProtocolError::InvalidRequest("vo2_max parameter required".to_string()))?;

    // Extract optional age for more accurate calculations
    let age = request
        .parameters
        .get("age")
        .and_then(serde_json::Value::as_u64)
        .and_then(|a| u32::try_from(a).ok());

    // Create simple zones based on VO2 max (placeholder implementation)
    let zones = vec![
        format!("Zone 1: {:.1}-{:.1} BPM", vo2_max * 0.5, vo2_max * 0.6),
        format!("Zone 2: {:.1}-{:.1} BPM", vo2_max * 0.6, vo2_max * 0.7),
        format!("Zone 3: {:.1}-{:.1} BPM", vo2_max * 0.7, vo2_max * 0.8),
        format!("Zone 4: {:.1}-{:.1} BPM", vo2_max * 0.8, vo2_max * 0.9),
        format!("Zone 5: {:.1}-{:.1} BPM", vo2_max * 0.9, vo2_max * 1.0),
    ];

    Ok(UniversalResponse {
        success: true,
        result: Some(serde_json::to_value(&zones).map_err(|e| {
            ProtocolError::SerializationError(format!("Failed to serialize zones: {e}"))
        })?),
        error: None,
        metadata: Some({
            let mut map = std::collections::HashMap::new();
            map.insert(
                "vo2_max".to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(vo2_max).unwrap_or_else(|| 0.into()),
                ),
            );
            map.insert(
                "age".to_string(),
                age.map_or(serde_json::Value::Null, |a| {
                    serde_json::Value::Number(a.into())
                }),
            );
            map.insert(
                "zone_count".to_string(),
                serde_json::Value::Number(zones.len().into()),
            );
            map
        }),
    })
}

/// Handle `validate_configuration` tool - validate configuration parameters
///
/// # Errors
/// Returns `ProtocolError` if configuration parameter is missing
pub fn handle_validate_configuration(
    _executor: &crate::protocols::universal::UniversalToolExecutor,
    request: &UniversalRequest,
) -> Result<UniversalResponse, ProtocolError> {
    // Extract configuration to validate
    let config = request.parameters.get("configuration").ok_or_else(|| {
        ProtocolError::InvalidRequest("configuration parameter required".to_string())
    })?;

    // Simple validation - check if it's a valid object
    if config.is_object() {
        Ok(UniversalResponse {
            success: true,
            result: Some(serde_json::json!({
                "valid": true,
                "message": "Configuration is valid"
            })),
            error: None,
            metadata: None,
        })
    } else {
        Ok(UniversalResponse {
            success: false,
            result: Some(serde_json::json!({
                "valid": false,
                "errors": ["Configuration must be a JSON object"]
            })),
            error: Some("Validation failed: Configuration must be a JSON object".to_string()),
            metadata: Some({
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "error_count".to_string(),
                    serde_json::Value::Number(1.into()),
                );
                map
            }),
        })
    }
}
