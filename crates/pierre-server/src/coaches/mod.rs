// ABOUTME: Coach definition parsing from markdown files
// ABOUTME: Converts Claude Skills-style markdown to structured coach definitions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Definition Parser
//!
//! This module provides parsing functionality for coach markdown files.
//! Coaches are defined in markdown format with YAML frontmatter and structured sections,
//! following a format inspired by Claude Skills.
//!
//! ## File Format
//!
//! ```markdown
//! ---
//! name: marathon-coach
//! title: Marathon Training Coach
//! category: training
//! tags: [running, marathon]
//! ---
//!
//! ## Purpose
//! Coach description here.
//!
//! ## Instructions
//! System prompt for the AI.
//! ```

/// Coach markdown file parser with frontmatter and section extraction
pub mod parser;

pub use parser::{
    is_locale_code, parse_coach_content, parse_coach_file, parse_frontmatter, parse_sections,
    to_markdown, CoachDefinition, CoachFrontmatter, CoachSections, CoachStartup, RelatedCoach,
    RelationType, CANONICAL_LOCALE, SUPPORTED_LOCALES,
};
pub use pierre_core::models::coaches::CoachPrerequisites;
