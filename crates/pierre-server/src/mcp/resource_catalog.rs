// ABOUTME: MCP resources/list + resources/read backed by the global coach marketplace catalog
// ABOUTME: Exposes only published (public marketplace) coaches as dravr://coaches/{id} text/markdown resources
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_core::models::coaches::Coach;
use pierre_database::database::store_listings::CoachWithListing;
use serde_json::{json, Value};
use std::fmt::Write;

/// URI scheme prefix for coach catalog resources.
const COACH_URI_PREFIX: &str = "dravr://coaches/";

/// MIME type advertised for coach markdown resources.
const COACH_MIME_TYPE: &str = "text/markdown";

/// Maximum number of marketplace coaches surfaced in a single `resources/list`.
const COACH_LIST_LIMIT: u32 = 100;

/// Build the `resources/list` payload from the published marketplace coaches.
///
/// Only globally published coaches are exposed — there is no tenant or auth
/// threading. Each coach becomes a `dravr://coaches/{id}` resource.
#[must_use]
pub fn list_resources(published: &[CoachWithListing]) -> Value {
    let resources: Vec<Value> = published
        .iter()
        .map(|item| {
            let coach = &item.coach;
            json!({
                "uri": coach_uri(coach),
                "name": coach.title,
                "description": coach.description.clone().unwrap_or_default(),
                "mimeType": COACH_MIME_TYPE,
            })
        })
        .collect();

    json!({ "resources": resources })
}

/// Limit applied when fetching published coaches for `resources/list`.
#[must_use]
pub const fn list_limit() -> u32 {
    COACH_LIST_LIMIT
}

/// Extract the coach id from a `dravr://coaches/{id}` URI.
///
/// Returns `None` when the URI does not use the coach scheme.
#[must_use]
pub fn coach_id_from_uri(uri: &str) -> Option<&str> {
    uri.strip_prefix(COACH_URI_PREFIX)
        .filter(|id| !id.is_empty())
}

/// Build the `resources/read` payload for a single published coach.
///
/// The coach markdown is reconstructed from the coach's structured sections so
/// MCP clients receive the same document an operator authored.
///
/// # Errors
///
/// Returns an error only if the JSON payload cannot be constructed; the caller
/// is responsible for handling the not-found case before invoking this.
pub fn read_resource(coach: &Coach) -> AppResult<Value> {
    Ok(json!({
        "contents": [
            {
                "uri": coach_uri(coach),
                "mimeType": COACH_MIME_TYPE,
                "text": render_coach_markdown(coach),
            }
        ]
    }))
}

/// Build the canonical resource URI for a coach.
fn coach_uri(coach: &Coach) -> String {
    format!("{COACH_URI_PREFIX}{}", coach.id)
}

/// Reconstruct a coach markdown document from its structured fields.
///
/// Mirrors the authored coach format: YAML-style frontmatter followed by the
/// section headings the catalog populates. Empty optional sections are
/// omitted so the document only carries content the coach actually provides.
fn render_coach_markdown(coach: &Coach) -> String {
    let mut out = String::new();

    out.push_str("---\n");
    let _ = writeln!(out, "title: {}", coach.title);
    let _ = writeln!(out, "category: {}", coach.category.as_str());
    if !coach.tags.is_empty() {
        let _ = writeln!(out, "tags: [{}]", coach.tags.join(", "));
    }
    out.push_str("---\n\n");

    let _ = writeln!(out, "# {}\n", coach.title);

    if let Some(description) = non_empty(coach.description.as_deref()) {
        out.push_str(description);
        out.push_str("\n\n");
    }

    push_section(&mut out, "Purpose", coach.purpose.as_deref());
    push_section(&mut out, "When to Use", coach.when_to_use.as_deref());
    push_section(&mut out, "Instructions", coach.instructions.as_deref());
    push_section(&mut out, "Example Inputs", coach.example_inputs.as_deref());
    push_section(
        &mut out,
        "Example Outputs",
        coach.example_outputs.as_deref(),
    );
    push_section(
        &mut out,
        "Success Criteria",
        coach.success_criteria.as_deref(),
    );

    if !coach.sample_prompts.is_empty() {
        out.push_str("## Sample Prompts\n\n");
        for prompt in &coach.sample_prompts {
            let _ = writeln!(out, "- {prompt}");
        }
        out.push('\n');
    }

    // Structured user coaches carry their full body in `instructions`; coaches
    // without extracted sections fall back to the raw system prompt so the
    // document is never empty.
    if coach.purpose.is_none() && coach.instructions.is_none() {
        out.push_str("## Instructions\n\n");
        out.push_str(coach.system_prompt.trim());
        out.push('\n');
    }

    out.trim_end().to_owned()
}

/// Append a markdown section when the body is present and non-empty.
fn push_section(out: &mut String, heading: &str, body: Option<&str>) {
    if let Some(content) = non_empty(body) {
        let _ = write!(out, "## {heading}\n\n{content}\n\n");
    }
}

/// Return the trimmed string only when it carries content.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
