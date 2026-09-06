// ABOUTME: Output format helpers shared by universal protocol tool handlers
// ABOUTME: Provides JSON/TOON envelope shaping for UniversalRequest/UniversalResponse payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::conversions::{apply_format, Formatted};
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

/// Put a typed payload on a response, honouring the caller's `format` choice.
///
/// The format logic itself is [`apply_format`], the single TOON primitive the
/// tool crate has; this adds only what a `UniversalResponse` carries and a
/// `ToolResult` does not — the `format` stamp in its metadata, which predates
/// the envelope and is what an operator reads in a trace.
///
/// A handler that calls this can declare an output schema, because the shape
/// on the wire is now `Formatted<T>` for a `T` the compiler knows. The
/// untyped [`apply_format_to_response`] cannot: its payload is a `Value` and
/// its TOON key is built from a runtime string.
///
/// # Errors
///
/// Returns [`ProtocolError::InternalError`] if `payload` does not serialize.
/// For a type deriving `Serialize` over owned data this cannot happen; it is
/// an error rather than a panic because the alternative is taking the server
/// down over one malformed reply.
pub fn apply_format_typed<T: Serialize>(
    mut response: UniversalResponse,
    payload: T,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    let formatted = apply_format(payload, output_format);
    let stamp = match formatted {
        Formatted::Toon { .. } => "toon",
        Formatted::Json(_) | Formatted::Fallback { .. } => "json",
    };
    let fell_back = matches!(formatted, Formatted::Fallback { .. });

    response.result = Some(
        to_value(formatted)
            .map_err(|e| ProtocolError::InternalError(format!("result did not serialize: {e}")))?,
    );
    if let Some(ref mut metadata) = response.metadata {
        metadata.insert("format".to_owned(), Value::String(stamp.to_owned()));
        if fell_back {
            metadata.insert("format_fallback".to_owned(), Value::Bool(true));
        }
    }
    Ok(response)
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

/// Put a typed payload on a fresh response, honouring the caller's format.
///
/// The same primitive [`apply_format`] as everywhere else, plus the `format`
/// stamp `UniversalResponse` carries in its metadata and a `ToolResult` does
/// not. Takes no `data_key`: the envelope's keys are fixed, because a
/// property name that changes per tool cannot be stated in a schema, and a
/// handler calling this is one that declares an `outputSchema`.
///
/// # Errors
///
/// Returns [`ProtocolError::InternalError`] if `data` does not serialize.
pub fn formatted_response<T, S>(
    data: &T,
    output_format: OutputFormat,
    metadata: HashMap<String, Value, S>,
) -> Result<UniversalResponse, ProtocolError>
where
    T: Serialize,
    S: BuildHasher,
{
    let response = UniversalResponse {
        success: true,
        result: None,
        error: None,
        metadata: Some(metadata.into_iter().collect()),
    };
    apply_format_typed(response, data, output_format)
}
