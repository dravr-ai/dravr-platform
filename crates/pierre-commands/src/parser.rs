// ABOUTME: Parser for command markdown files with YAML frontmatter
// ABOUTME: Extracts command definitions from .md files following the coach parser pattern
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fs;
use std::path::Path;

use pierre_messaging::commands::{CommandDefinition, CommandRole};
use serde::Deserialize;
use tracing::{info, warn};

/// YAML frontmatter parsed from command markdown file
#[derive(Debug, Clone, Deserialize)]
struct CommandFrontmatter {
    /// Unique command identifier (e.g., "group-status")
    name: String,
    /// The command string (e.g., "/group status")
    command: String,
    /// Alternative command strings
    #[serde(default)]
    aliases: Vec<String>,
    /// One-line description for help and Telegram bot menu
    description: String,
    /// Domain grouping (e.g., "general", "group", "coach")
    domain: String,
    /// Required role: "any", "member", "admin", "owner"
    #[serde(default = "default_role")]
    required_role: String,
    /// Whether the command requires an active group context
    #[serde(default)]
    requires_group: bool,
}

fn default_role() -> String {
    "any".to_owned()
}

/// Extract the `## Response Template` section from markdown body
fn extract_response_template(body: &str) -> String {
    let mut in_section = false;
    let mut template = String::new();

    for line in body.lines() {
        if line.starts_with("## Response Template") {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            break;
        }
        if in_section {
            template.push_str(line);
            template.push('\n');
        }
    }

    template.trim().to_owned()
}

/// Parse a single command markdown file into a `CommandDefinition`
fn parse_command_file(content: &str) -> Option<CommandDefinition> {
    // Split frontmatter from body
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }

    let after_first = &content[3..];
    let end_idx = after_first.find("---")?;
    let frontmatter_str = &after_first[..end_idx];
    let body = &after_first[end_idx + 3..];

    let fm: CommandFrontmatter = serde_yaml::from_str(frontmatter_str).ok()?;
    let response_template = extract_response_template(body);

    Some(CommandDefinition {
        name: fm.name,
        command: fm.command,
        aliases: fm.aliases,
        description: fm.description,
        domain: fm.domain,
        required_role: CommandRole::from_str_value(&fm.required_role),
        requires_group: fm.requires_group,
        response_template,
    })
}

/// Load all command definitions from a directory tree.
///
/// Recursively scans for `.md` files, parses YAML frontmatter,
/// and returns a list of `CommandDefinition` objects.
///
/// Skips files that fail to parse (logged at WARN level).
#[must_use]
pub fn load_command_definitions(commands_dir: &Path) -> Vec<CommandDefinition> {
    let mut definitions = Vec::new();

    if !commands_dir.exists() {
        warn!(
            path = %commands_dir.display(),
            "Commands directory not found, starting with empty command registry"
        );
        return definitions;
    }

    load_recursive(commands_dir, &mut definitions);

    info!(
        count = definitions.len(),
        "Loaded command definitions from {}",
        commands_dir.display()
    );

    definitions
}

// Cognitive complexity split: recursive loader delegates file processing
fn load_recursive(dir: &Path, definitions: &mut Vec<CommandDefinition>) {
    let Ok(entries) = fs::read_dir(dir) else {
        warn!(path = %dir.display(), "Failed to read commands directory");
        return;
    };

    for entry in entries.flatten() {
        process_entry(&entry.path(), definitions);
    }
}

fn process_entry(path: &Path, definitions: &mut Vec<CommandDefinition>) {
    if path.is_dir() {
        load_recursive(path, definitions);
        return;
    }

    if path.extension().is_none_or(|ext| ext != "md") {
        return;
    }

    match fs::read_to_string(path) {
        Ok(content) => {
            if let Some(def) = parse_command_file(&content) {
                info!(name = %def.name, command = %def.command, "Loaded command");
                definitions.push(def);
            } else {
                warn!(path = %path.display(), "Failed to parse command file");
            }
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read command file");
        }
    }
}
