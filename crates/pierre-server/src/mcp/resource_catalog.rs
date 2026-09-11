// ABOUTME: MCP resources/list + resources/read backed by the global agent marketplace catalog
// ABOUTME: Exposes only published (public marketplace) agents as dravr://agents/{id} text/markdown resources
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_core::models::agents::Agent;
use pierre_database::database::store_listings::AgentWithListing;
use serde_json::{json, Value};
use std::fmt::Write;

/// URI scheme prefix for agent catalog resources.
const AGENT_URI_PREFIX: &str = "dravr://coaches/";

/// MIME type advertised for agent markdown resources.
const AGENT_MIME_TYPE: &str = "text/markdown";

/// Maximum number of marketplace agents surfaced in a single `resources/list`.
const AGENT_LIST_LIMIT: u32 = 100;

/// Build the `resources/list` payload from the published marketplace agents.
///
/// Only globally published agents are exposed — there is no tenant or auth
/// threading. Each agent becomes a `dravr://agents/{id}` resource.
#[must_use]
pub fn list_resources(published: &[AgentWithListing]) -> Value {
    let resources: Vec<Value> = published
        .iter()
        .map(|item| {
            let agent = &item.agent;
            json!({
                "uri": agent_uri(agent),
                "name": agent.title,
                "description": agent.description.clone().unwrap_or_default(),
                "mimeType": AGENT_MIME_TYPE,
            })
        })
        .collect();

    json!({ "resources": resources })
}

/// Limit applied when fetching published agents for `resources/list`.
#[must_use]
pub const fn list_limit() -> u32 {
    AGENT_LIST_LIMIT
}

/// Extract the agent id from a `dravr://agents/{id}` URI.
///
/// Returns `None` when the URI does not use the agent scheme.
#[must_use]
pub fn agent_id_from_uri(uri: &str) -> Option<&str> {
    uri.strip_prefix(AGENT_URI_PREFIX)
        .filter(|id| !id.is_empty())
}

/// Build the `resources/read` payload for a single published agent.
///
/// The agent markdown is reconstructed from the agent's structured sections so
/// MCP clients receive the same document an operator authored.
///
/// # Errors
///
/// Returns an error only if the JSON payload cannot be constructed; the caller
/// is responsible for handling the not-found case before invoking this.
pub fn read_resource(agent: &Agent) -> AppResult<Value> {
    Ok(json!({
        "contents": [
            {
                "uri": agent_uri(agent),
                "mimeType": AGENT_MIME_TYPE,
                "text": render_agent_markdown(agent),
            }
        ]
    }))
}

/// Build the canonical resource URI for an agent.
fn agent_uri(agent: &Agent) -> String {
    format!("{AGENT_URI_PREFIX}{}", agent.id)
}

/// Reconstruct an agent markdown document from its structured fields.
///
/// Mirrors the authored agent format: YAML-style frontmatter followed by the
/// section headings the catalog populates. Empty optional sections are
/// omitted so the document only carries content the agent actually provides.
fn render_agent_markdown(agent: &Agent) -> String {
    let mut out = String::new();

    out.push_str("---\n");
    let _ = writeln!(out, "title: {}", agent.title);
    let _ = writeln!(out, "category: {}", agent.category.as_str());
    if !agent.tags.is_empty() {
        let _ = writeln!(out, "tags: [{}]", agent.tags.join(", "));
    }
    out.push_str("---\n\n");

    let _ = writeln!(out, "# {}\n", agent.title);

    if let Some(description) = non_empty(agent.description.as_deref()) {
        out.push_str(description);
        out.push_str("\n\n");
    }

    push_section(&mut out, "Purpose", agent.purpose.as_deref());
    push_section(&mut out, "When to Use", agent.when_to_use.as_deref());
    push_section(&mut out, "Instructions", agent.instructions.as_deref());
    push_section(&mut out, "Example Inputs", agent.example_inputs.as_deref());
    push_section(
        &mut out,
        "Example Outputs",
        agent.example_outputs.as_deref(),
    );
    push_section(
        &mut out,
        "Success Criteria",
        agent.success_criteria.as_deref(),
    );

    if !agent.sample_prompts.is_empty() {
        out.push_str("## Sample Prompts\n\n");
        for prompt in &agent.sample_prompts {
            let _ = writeln!(out, "- {prompt}");
        }
        out.push('\n');
    }

    // Structured user agents carry their full body in `instructions`; agents
    // without extracted sections fall back to the raw system prompt so the
    // document is never empty.
    if agent.purpose.is_none() && agent.instructions.is_none() {
        out.push_str("## Instructions\n\n");
        out.push_str(agent.system_prompt.trim());
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
