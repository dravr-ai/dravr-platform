// ABOUTME: Output format helpers shared by universal protocol tool handlers
// ABOUTME: Provides JSON/TOON envelope shaping for UniversalRequest/UniversalResponse payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use pierre_formatters::{format_output, OutputFormat};
use serde::Serialize;
use serde_json::{json, to_value, Value};
use std::collections::HashMap;
use std::hash::BuildHasher;
use tracing::warn;

/// Extract output format parameter from request
/// Returns `OutputFormat::Json` as default for backwards compatibility
pub fn extract_output_format(request: &UniversalRequest) -> OutputFormat {
    request
        .parameters
        .get("format")
        .and_then(|v| v.as_str())
        .map_or(OutputFormat::Json, OutputFormat::from_str_param)
}

/// Apply format transformation to an existing `UniversalResponse`.
///
/// This is useful for handlers that delegate to internal functions returning `UniversalResponse`.
/// If the response is successful and has a result, formats it according to `output_format`.
pub fn apply_format_to_response(
    mut response: UniversalResponse,
    data_key: &str,
    output_format: OutputFormat,
) -> UniversalResponse {
    // Only apply formatting to successful responses with data
    if !response.success || response.result.is_none() {
        return response;
    }

    // JSON is the default, no transformation needed
    if matches!(output_format, OutputFormat::Json) {
        // Add format metadata
        if let Some(ref mut metadata) = response.metadata {
            metadata.insert("format".to_owned(), Value::String("json".to_owned()));
        }
        return response;
    }

    // Apply TOON formatting - result presence was verified by guard above
    let Some(result_value) = response.result.take() else {
        // Defensive: return unchanged if result is unexpectedly None
        return response;
    };

    match format_output(&result_value, OutputFormat::Toon) {
        Ok(formatted) => {
            let toon_key = format!("{data_key}_toon");
            response.result = Some(json!({
                toon_key: formatted.data,
                "format": "toon"
            }));
            if let Some(ref mut metadata) = response.metadata {
                metadata.insert("format".to_owned(), Value::String("toon".to_owned()));
            }
        }
        Err(e) => {
            // Fall back to JSON on encoding error
            warn!("TOON encoding failed, falling back to JSON: {}", e);
            response.result = Some(json!({
                data_key: result_value,
                "format": "json",
                "format_fallback": true,
                "format_error": e.to_string()
            }));
            if let Some(ref mut metadata) = response.metadata {
                metadata.insert("format".to_owned(), Value::String("json".to_owned()));
                metadata.insert("format_fallback".to_owned(), Value::Bool(true));
            }
        }
    }

    response
}

/// Build a formatted response with format support (JSON or TOON).
///
/// Generic helper for all data-returning handlers.
///
/// # Errors
///
/// Returns `ProtocolError::SerializationError` if:
/// - Data serialization to JSON fails
/// - TOON encoding fails (falls back to JSON with metadata flag)
pub fn build_formatted_response<T, S>(
    data: &T,
    data_key: &str,
    output_format: OutputFormat,
    metadata: HashMap<String, Value, S>,
) -> Result<UniversalResponse, ProtocolError>
where
    T: Serialize,
    S: BuildHasher,
{
    // Convert to standard HashMap for UniversalResponse compatibility
    let mut metadata: HashMap<String, Value> = metadata.into_iter().collect();

    // Add format to metadata
    metadata.insert(
        "format".to_owned(),
        Value::String(output_format.as_str().to_owned()),
    );

    let result_json = match output_format {
        OutputFormat::Toon => {
            // Convert data to JSON value first for TOON encoding
            let data_value = to_value(data).map_err(|e| {
                ProtocolError::SerializationError(format!("Failed to serialize data: {e}"))
            })?;

            match format_output(&data_value, OutputFormat::Toon) {
                Ok(formatted) => {
                    // Use _toon suffix for the data key to indicate TOON format
                    let toon_key = format!("{data_key}_toon");
                    json!({
                        toon_key: formatted.data,
                        "format": "toon"
                    })
                }
                Err(e) => {
                    // Fall back to JSON if TOON serialization fails
                    warn!("TOON serialization failed, falling back to JSON: {}", e);
                    metadata.insert("format".to_owned(), Value::String("json".to_owned()));
                    metadata.insert("format_fallback".to_owned(), Value::Bool(true));
                    metadata.insert("format_error".to_owned(), Value::String(e.to_string()));
                    json!({
                        data_key: to_value(data).map_err(|e| {
                            ProtocolError::SerializationError(format!("Failed to serialize data: {e}"))
                        })?,
                        "format": "json"
                    })
                }
            }
        }
        OutputFormat::Json => {
            json!({
                data_key: to_value(data).map_err(|e| {
                    ProtocolError::SerializationError(format!("Failed to serialize data: {e}"))
                })?,
                "format": "json"
            })
        }
    };

    Ok(UniversalResponse {
        success: true,
        result: Some(result_json),
        error: None,
        metadata: Some(metadata),
    })
}
