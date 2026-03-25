// ABOUTME: Markdown command definition loader for messaging slash commands
// ABOUTME: Parses YAML frontmatter from command .md files into CommandDefinition
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Markdown frontmatter parser for command definition files
pub mod parser;

pub use parser::load_command_definitions;
