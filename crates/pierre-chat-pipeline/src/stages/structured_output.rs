// ABOUTME: Extracts and schema-validates a structured plan JSON from a builder-coach reply.
// ABOUTME: Valid plans become structured_content (rendered as a card); invalid/absent leaves prose.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::OnceLock;

use serde_json::Value;
use tracing::warn;

/// Schema identifier emitted by the workout-builder coaches. Matches the
/// `output_schema` frontmatter value and the contremaitre schema key.
const STRUCTURED_WORKOUT: &str = "structured-workout";

/// Result of a successful structured-plan extraction.
pub struct StructuredExtraction {
    /// Compact JSON of the validated plan, persisted as `structured_content`
    /// and rendered as a plan card by clients.
    pub structured_content: String,
    /// User-visible text with the plan object removed. Usually empty (the
    /// contract asks the model for JSON only), but any surrounding prose is
    /// preserved here so it still reaches the user alongside the card.
    pub cleaned_text: String,
}

/// Compiled validator for the structured-workout schema. Compiled once on
/// first use from the schema text threaded through the pipeline context.
static WORKOUT_VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();

fn workout_validator(schema_json: &str) -> Option<&'static jsonschema::Validator> {
    WORKOUT_VALIDATOR
        .get_or_init(|| match serde_json::from_str::<Value>(schema_json) {
            Ok(mut schema) => {
                // Drop the `$schema`/`$id` URIs before compiling. Their presence
                // makes the validator resolve the draft meta-schema (and the
                // `$id` base) over the network, which would block the first
                // builder-coach turn on json-schema.org reachability. Stripped,
                // the embedded draft is used and the schema's relative
                // `#/$defs/...` refs still resolve against the document root.
                if let Some(obj) = schema.as_object_mut() {
                    obj.remove("$schema");
                    obj.remove("$id");
                }
                match jsonschema::validator_for(&schema) {
                    Ok(validator) => Some(validator),
                    Err(e) => {
                        warn!(error = %e, "structured-output: schema failed to compile");
                        None
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "structured-output: schema text is not valid JSON");
                None
            }
        })
        .as_ref()
}

/// Extract and schema-validate a structured plan from an assistant reply.
///
/// Returns `Some` only when the coach declares the matching `output_schema`,
/// the channel can render a plan card (`is_messaging == false`), a balanced
/// JSON object is present in the reply, and it validates against the schema.
/// Otherwise returns `None`, leaving the reply as free text — the expected
/// path for a prose refusal, a non-builder coach, or a messaging channel
/// (Telegram/WhatsApp/etc.) that has no card renderer.
pub fn extract_structured_plan(
    output_schema: Option<&str>,
    is_messaging: bool,
    schema_json: &str,
    reply: &str,
) -> Option<StructuredExtraction> {
    // Messaging channels have no plan-card renderer; stripping the JSON would
    // leave an empty reply. The matching prompt directive is also withheld
    // there (prompt_assembly), so the coach emits a plain-prose plan instead.
    if is_messaging {
        return None;
    }
    if output_schema != Some(STRUCTURED_WORKOUT) {
        return None;
    }

    // Strip code fences the model may have wrapped the object in, then locate
    // the first balanced top-level object.
    let unfenced = reply.replace("```json", "").replace("```", "");
    let (object, before, after) = find_json_object(&unfenced)?;

    let plan: Value = match serde_json::from_str(object) {
        Ok(value) => value,
        Err(e) => {
            warn!(error = %e, "structured-output: candidate object is not valid JSON");
            return None;
        }
    };

    let validator = workout_validator(schema_json)?;
    if !validator.is_valid(&plan) {
        if let Some(error) = validator.iter_errors(&plan).next() {
            warn!(
                error = %error,
                path = %error.instance_path(),
                "structured-output: plan failed schema validation; leaving raw reply"
            );
        }
        return None;
    }

    // Compact re-serialization for storage (drops incidental whitespace).
    let structured_content = serde_json::to_string(&plan).ok()?;
    let cleaned_text = format!("{}\n{}", before.trim(), after.trim())
        .trim()
        .to_owned();

    Some(StructuredExtraction {
        structured_content,
        cleaned_text,
    })
}

/// Find the first balanced top-level JSON object in `text`, returning the
/// object slice and the text before and after it. Brace counting ignores
/// braces and quotes that appear inside JSON string literals.
fn find_json_object(text: &str) -> Option<(&str, &str, &str)> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, &byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let end = offset + 1;
                    return Some((&text[start..end], &text[..start], &text[end..]));
                }
            }
            _ => {}
        }
    }
    None
}
