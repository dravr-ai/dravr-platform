// ABOUTME: Recipe and agent export/import business logic extracted from route handlers
// ABOUTME: Handles markdown conversion, filename generation, and diff computation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_agent_parser::{AgentDefinition, AgentFrontmatter, AgentSections, AgentStartup};
use pierre_core::models::agents::{Agent, AgentPrerequisites};

/// Convert a Agent database model to `AgentDefinition` for export
///
/// Transforms the stored agent data into the markdown-exportable format
/// used for agent file interchange.
#[must_use]
pub fn agent_to_definition(agent: &Agent) -> AgentDefinition {
    let name = agent
        .title
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>();

    AgentDefinition {
        frontmatter: AgentFrontmatter {
            name,
            title: agent.title.clone(),
            category: agent.category,
            tags: agent.tags.clone(),
            prerequisites: AgentPrerequisites::default(),
            visibility: agent.visibility,
            startup: AgentStartup::default(),
            replaces: vec![],
        },
        sections: AgentSections {
            purpose: agent.description.clone().unwrap_or_default(),
            when_to_use: None,
            instructions: agent.system_prompt.clone(),
            example_inputs: if agent.sample_prompts.is_empty() {
                None
            } else {
                Some(
                    agent
                        .sample_prompts
                        .iter()
                        .map(|p| format!("- {p}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            },
            example_outputs: None,
            success_criteria: None,
            related_agents: Vec::new(),
        },
        source_file: format!("exported/{}.md", agent.id),
        content_hash: String::new(),
        token_count: agent.token_count,
    }
}

/// Generate a safe filename from agent title for markdown export
///
/// Converts to lowercase, replaces spaces with hyphens, and strips
/// non-alphanumeric characters (except hyphens).
#[must_use]
pub fn generate_agent_filename(title: &str) -> String {
    let safe_name: String = title
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();

    format!("{safe_name}.md")
}

/// A field-level change between two agent version snapshots
#[derive(Debug)]
pub struct FieldChange {
    /// Name of the changed field
    pub field: String,
    /// Previous value (None if field was added)
    pub old_value: Option<serde_json::Value>,
    /// New value (None if field was removed)
    pub new_value: Option<serde_json::Value>,
}

/// Compute field-level differences between two JSON agent version snapshots
///
/// Compares specific agent fields (title, description, `system_prompt`, category,
/// tags, `sample_prompts`, visibility) and returns a list of changes.
#[must_use]
pub fn compute_version_diff(from: &serde_json::Value, to: &serde_json::Value) -> Vec<FieldChange> {
    let mut changes = Vec::new();

    let fields = [
        "title",
        "description",
        "system_prompt",
        "category",
        "tags",
        "sample_prompts",
        "visibility",
    ];

    for field in fields {
        let old_val = from.get(field);
        let new_val = to.get(field);

        match (old_val, new_val) {
            (Some(old), Some(new)) if old != new => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: Some(old.clone()),
                    new_value: Some(new.clone()),
                });
            }
            (None, Some(new)) => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: None,
                    new_value: Some(new.clone()),
                });
            }
            (Some(old), None) => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: Some(old.clone()),
                    new_value: None,
                });
            }
            _ => {}
        }
    }

    changes
}
