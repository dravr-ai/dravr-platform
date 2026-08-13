// ABOUTME: Parser for command markdown files with YAML frontmatter
// ABOUTME: Extracts command definitions from .md files following the coach parser pattern
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use pierre_messaging::commands::{CommandDefinition, CommandRole};
use serde::Deserialize;
use tracing::{debug, info, warn};

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
    /// Argument signature shown after the command in `/help` — literal
    /// alternatives as `yes|no`, optional groups as `[week|today]`,
    /// free values as lowercase placeholders like `area/city`.
    ///
    /// Absent for commands that take no arguments. Angle brackets are
    /// deliberately not part of the convention: `/help` renders as plain
    /// text, and Slack turns `<...>` into a link.
    #[serde(default)]
    arguments: Option<String>,
}

/// A loaded command catalog: the command definitions plus the argument
/// signatures `/help` renders beside them.
///
/// `CommandDefinition` is canonical in dravr-canot and carries no argument
/// field, so the signatures travel next to it in a name-keyed index built
/// from the very same markdown files — one catalog, one parse.
pub struct CommandCatalog {
    /// Every command definition parsed from the catalog directory.
    pub definitions: Vec<CommandDefinition>,
    /// Command name → argument signature, for commands that take arguments.
    pub arg_specs: HashMap<String, String>,
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

/// One command as parsed from its markdown file: the canonical definition plus
/// the argument signature [`CommandDefinition`] has no field for.
struct ParsedCommand {
    /// The definition as dravr-canot models it.
    definition: CommandDefinition,
    /// Argument signature, absent for commands taking no arguments.
    arguments: Option<String>,
}

/// Parse a single command markdown file into a [`ParsedCommand`].
fn parse_command_file(content: &str) -> Option<ParsedCommand> {
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

    let arguments = fm
        .arguments
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty());

    Some(ParsedCommand {
        definition: CommandDefinition {
            name: fm.name,
            command: fm.command,
            aliases: fm.aliases,
            description: fm.description,
            domain: fm.domain,
            // Who a command admits is decided by its handler's
            // `CommandHandler::is_available`, next to the check `execute`
            // enforces. These two fields are dravr-canot's schema and no code
            // in either crate reads them, so the catalog no longer declares
            // them and they carry the "nothing declared" values.
            required_role: CommandRole::Any,
            requires_group: false,
            response_template,
        },
        arguments,
    })
}

/// Load the whole command catalog from a directory tree.
///
/// Recursively scans for `.md` files, parses YAML frontmatter, and returns
/// the [`CommandDefinition`]s together with the argument signatures `/help`
/// renders next to them.
///
/// Skips files that fail to parse (logged at WARN level).
#[must_use]
pub fn load_command_catalog(commands_dir: &Path) -> CommandCatalog {
    let mut catalog = CommandCatalog {
        definitions: Vec::new(),
        arg_specs: HashMap::new(),
    };

    if !commands_dir.exists() {
        warn!(
            path = %commands_dir.display(),
            "Commands directory not found, starting with empty command registry"
        );
        return catalog;
    }

    load_recursive(commands_dir, &mut catalog);

    info!(
        count = catalog.definitions.len(),
        with_arguments = catalog.arg_specs.len(),
        "Loaded command definitions from {}",
        commands_dir.display()
    );

    catalog
}

// Cognitive complexity split: recursive loader delegates file processing
fn load_recursive(dir: &Path, catalog: &mut CommandCatalog) {
    let Ok(entries) = fs::read_dir(dir) else {
        warn!(path = %dir.display(), "Failed to read commands directory");
        return;
    };

    for entry in entries.flatten() {
        process_entry(&entry.path(), catalog);
    }
}

fn process_entry(path: &Path, catalog: &mut CommandCatalog) {
    if path.is_dir() {
        load_recursive(path, catalog);
        return;
    }

    if path.extension().is_none_or(|ext| ext != "md") {
        return;
    }

    match fs::read_to_string(path) {
        Ok(content) => add_to_catalog(path, &content, catalog),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read command file");
        }
    }
}

// Cognitive complexity split: file processing delegates catalog insertion
fn add_to_catalog(path: &Path, content: &str, catalog: &mut CommandCatalog) {
    let Some(parsed) = parse_command_file(content) else {
        warn!(path = %path.display(), "Failed to parse command file");
        return;
    };
    let def = parsed.definition;

    debug!(name = %def.name, command = %def.command, "Loaded command");
    if let Some(args) = parsed.arguments {
        catalog.arg_specs.insert(def.name.clone(), args);
    }
    catalog.definitions.push(def);
}
