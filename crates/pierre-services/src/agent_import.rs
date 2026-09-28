// ABOUTME: Business logic for agent markdown import operations
// ABOUTME: Import warnings and definition-to-request conversion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_agent_parser::AgentDefinition;
use pierre_core::models::agents::CreateAgentRequest;

/// Token count threshold that triggers a size warning during import
const TOKEN_COUNT_WARNING_THRESHOLD: u32 = 10_000;

/// Generate warnings for an imported agent definition.
///
/// Checks for common quality issues that don't prevent import
/// but should be brought to the user's attention:
/// - Missing optional sections (`example_inputs`, `example_outputs`, `success_criteria`)
/// - Excessive token count (> 10,000)
/// - No tags defined
#[must_use]
pub fn generate_import_warnings(definition: &AgentDefinition) -> Vec<String> {
    let mut warnings = Vec::new();

    if definition.sections.example_inputs.is_none() {
        warnings.push("Missing optional section: Example Inputs".to_owned());
    }

    if definition.sections.example_outputs.is_none() {
        warnings.push("Missing optional section: Example Outputs".to_owned());
    }

    if definition.sections.success_criteria.is_none() {
        warnings.push("Missing optional section: Success Criteria".to_owned());
    }

    if definition.token_count > TOKEN_COUNT_WARNING_THRESHOLD {
        warnings.push(format!(
            "High token count: {} (recommended < {})",
            definition.token_count, TOKEN_COUNT_WARNING_THRESHOLD
        ));
    }

    if definition.frontmatter.tags.is_empty() {
        warnings.push("No tags defined — tags improve discoverability".to_owned());
    }

    warnings
}

/// Convert an `AgentDefinition` to a `CreateAgentRequest`, mapping all fields.
///
/// Extracts sample prompts from the `example_inputs` section (lines starting with `-`)
/// and maps all frontmatter and section fields to the request structure.
#[must_use]
pub fn definition_to_create_request(definition: &AgentDefinition) -> CreateAgentRequest {
    let sample_prompts = definition
        .sections
        .example_inputs
        .as_ref()
        .map(|inputs| {
            inputs
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix('-')
                        .map(|s| s.trim().trim_matches('"').to_owned())
                })
                .collect()
        })
        .unwrap_or_default();

    CreateAgentRequest {
        title: definition.frontmatter.title.clone(),
        description: Some(definition.sections.purpose.clone()),
        system_prompt: definition.sections.instructions.clone(),
        category: definition.frontmatter.category,
        tags: definition.frontmatter.tags.clone(),
        sample_prompts,
        startup_query: definition.frontmatter.startup.query.clone(),
        data_requirements: definition.frontmatter.startup.data_requirements.clone(),
        purpose: Some(definition.sections.purpose.clone()),
        when_to_use: definition.sections.when_to_use.clone(),
        instructions: Some(definition.sections.instructions.clone()),
        example_inputs: definition.sections.example_inputs.clone(),
        example_outputs: definition.sections.example_outputs.clone(),
        success_criteria: definition.sections.success_criteria.clone(),
        max_tool_iterations: None,
    }
}
