// ABOUTME: Output format helpers shared by universal protocol tool handlers
// ABOUTME: Provides JSON/TOON envelope shaping for UniversalRequest/UniversalResponse payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::conversions::{apply_format, Formatted};
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use pierre_formatters::OutputFormat;
use serde::Serialize;
use serde_json::{to_value, Value};
use std::collections::HashMap;
use std::hash::BuildHasher;

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
/// on the wire is `Formatted<T>` for a `T` the compiler knows. The untyped
/// predecessor could not: its payload was a `Value` and its TOON key was
/// built from a runtime string, so the key changed per tool and no schema
/// could name it.
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
