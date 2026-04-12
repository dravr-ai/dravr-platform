// ABOUTME: Insight sample seeder and validator for Pierre MCP Server
// ABOUTME: Loads insight samples from markdown files and validates against LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Insight Sample Seeder
//!
//! This binary loads insight sample definitions from markdown files and optionally
//! validates them against the LLM-powered insight validation service.
//!
//! ## Usage
//!
//! ```bash
//! # List all insight samples (no validation)
//! pierre-cli seed insight-samples
//!
//! # Validate samples against LLM
//! pierre-cli seed insight-samples --validate
//!
//! # Validate with specific tier
//! pierre-cli seed insight-samples --validate --tier professional
//!
//! # Show tags and content preview
//! pierre-cli seed insight-samples --detailed
//!
//! # Dry run (show what would be done)
//! pierre-cli seed insight-samples --dry-run
//! ```

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use glob::glob;
use pierre_core::errors::{AppError, AppResult};
use tracing::{debug, warn};

use crate::insight_samples::{parse_insight_sample_file, InsightSampleDefinition};
use crate::models::UserTier;

/// CLI arguments for the insight samples validator.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Path to `insight_samples` directory
    #[arg(long, default_value = "insight_samples")]
    pub samples_dir: PathBuf,

    /// Run LLM validation on samples (requires API key)
    #[arg(long)]
    pub validate: bool,

    /// User tier for validation (starter, professional, enterprise)
    #[arg(long, default_value = "starter")]
    pub tier: String,

    /// Dry run - show what would be done without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Show tags and content preview for each sample
    #[arg(long)]
    pub detailed: bool,

    /// Only show samples in a specific category (valid, invalid, improvable)
    #[arg(long)]
    pub category: Option<String>,
}

/// Seeding result statistics
#[derive(Default)]
struct SeedStats {
    valid_samples: u32,
    invalid_samples: u32,
    improvable_samples: u32,
    validation_passed: u32,
    validation_failed: u32,
    errors: Vec<String>,
}

impl SeedStats {
    const fn total_samples(&self) -> u32 {
        self.valid_samples + self.invalid_samples + self.improvable_samples
    }
}

/// Parse insight sample markdown files and optionally validate them against the LLM service.
///
/// # Errors
///
/// Returns an error if the tier string is invalid or if sample discovery fails.
pub fn run(args: &SeedArgs) -> AppResult<()> {
    println!("=== Pierre MCP Server Insight Sample Seeder ===");
    if args.dry_run {
        println!("DRY RUN - no validation will be performed");
    }

    let user_tier = parse_user_tier(&args.tier)?;
    println!("Using tier: {user_tier:?}");

    let samples = discover_samples(&args.samples_dir, args.category.as_deref())?;
    println!("Found {} insight sample files", samples.len());
    if samples.is_empty() {
        eprintln!(
            "Warning: No insight sample files found in {}",
            args.samples_dir.display()
        );
        return Ok(());
    }

    let stats = display_samples(&samples, args.detailed);
    if args.validate && !args.dry_run {
        run_validation_mode(&samples, &user_tier);
    }
    print_summary(&stats, args.dry_run);
    Ok(())
}

fn parse_user_tier(tier: &str) -> AppResult<UserTier> {
    match tier.to_lowercase().as_str() {
        "starter" => Ok(UserTier::Starter),
        "professional" => Ok(UserTier::Professional),
        "enterprise" => Ok(UserTier::Enterprise),
        other => Err(AppError::config(format!(
            "Invalid tier '{other}'. Use: starter, professional, or enterprise"
        ))),
    }
}

/// Run validation mode and display expected results
fn run_validation_mode(samples: &[InsightSampleDefinition], user_tier: &UserTier) {
    println!();
    println!("=== Validation Mode ===");
    println!("Note: LLM validation requires GEMINI_API_KEY environment variable");
    println!("Preview mode: Displaying expected validation results per tier configuration");

    for sample in samples {
        let expected = sample.frontmatter.tier_behavior.verdict_for_tier(user_tier);
        println!(
            "  {} - Expected: {} ({})",
            sample.frontmatter.name, expected, sample.frontmatter.expected_verdict
        );
    }
}

/// Discover and parse all insight sample markdown files
fn discover_samples(
    samples_dir: &Path,
    category_filter: Option<&str>,
) -> AppResult<Vec<InsightSampleDefinition>> {
    let pattern = samples_dir.join("**/*.md");
    let pattern_str = pattern.to_string_lossy();

    let mut samples = Vec::new();

    let entries =
        glob(&pattern_str).map_err(|e| AppError::config(format!("Invalid glob pattern: {e}")))?;
    for entry in entries {
        let path = entry.map_err(|e| AppError::config(format!("Glob iteration error: {e}")))?;

        // Skip README files
        if path.file_name().is_some_and(|n| n == "README.md") {
            continue;
        }

        // Apply category filter if specified
        if let Some(category) = category_filter {
            let path_str = path.to_string_lossy();
            if !path_str.contains(&format!("/{category}/")) {
                continue;
            }
        }

        match parse_insight_sample_file(&path) {
            Ok(sample) => {
                debug!("Parsed: {} ({})", sample.frontmatter.name, path.display());
                samples.push(sample);
            }
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
            }
        }
    }

    // Sort by expected_verdict (valid, invalid, improvable) then by name
    samples.sort_by(|a, b| {
        let verdict_cmp = verdict_order(&a.frontmatter.expected_verdict)
            .cmp(&verdict_order(&b.frontmatter.expected_verdict));
        if verdict_cmp == Ordering::Equal {
            a.frontmatter.name.cmp(&b.frontmatter.name)
        } else {
            verdict_cmp
        }
    });

    Ok(samples)
}

/// Get ordering value for verdict categories
fn verdict_order(verdict: &str) -> u8 {
    match verdict {
        "valid" => 0,
        "improved" => 1,
        "rejected" => 2,
        _ => 3,
    }
}

/// Update stats counter based on expected verdict
fn count_verdict(stats: &mut SeedStats, verdict: &str) {
    match verdict {
        "valid" => stats.valid_samples += 1,
        "rejected" => stats.invalid_samples += 1,
        "improved" => stats.improvable_samples += 1,
        _ => {}
    }
}

/// Display a single sample's information
fn display_sample_info(sample: &InsightSampleDefinition, verbose: bool) {
    let sport = sample
        .frontmatter
        .sport_type
        .as_deref()
        .unwrap_or("general");

    println!(
        "  {} [{}/{}]",
        sample.frontmatter.name, sample.frontmatter.insight_type, sport
    );

    if verbose {
        let tags = sample.frontmatter.tags.join(", ");
        if !tags.is_empty() {
            println!("    Tags: {tags}");
        }
        println!(
            "    Content preview: {}...",
            truncate(&sample.sections.content, 60)
        );
    }
}

/// Display all samples and gather statistics
fn display_samples(samples: &[InsightSampleDefinition], verbose: bool) -> SeedStats {
    let mut stats = SeedStats::default();
    let mut current_verdict = String::new();

    for sample in samples {
        // Print section header when verdict changes
        if sample.frontmatter.expected_verdict != current_verdict {
            current_verdict.clone_from(&sample.frontmatter.expected_verdict);
            println!();
            println!("=== {} Samples ===", current_verdict.to_uppercase());
        }

        count_verdict(&mut stats, &sample.frontmatter.expected_verdict);
        display_sample_info(sample, verbose);
    }

    stats
}

/// Truncate string to max length with ellipsis
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Print sample count summary
fn print_sample_counts(stats: &SeedStats) {
    println!();
    println!("=== Summary ===");
    println!("Total samples: {}", stats.total_samples());
    println!("  - Valid samples: {}", stats.valid_samples);
    println!("  - Invalid samples: {}", stats.invalid_samples);
    println!("  - Improvable samples: {}", stats.improvable_samples);
}

/// Print validation results if any validation was performed
fn print_validation_results(stats: &SeedStats) {
    if stats.validation_passed > 0 || stats.validation_failed > 0 {
        println!();
        println!("Validation results:");
        println!("  - Passed: {}", stats.validation_passed);
        println!("  - Failed: {}", stats.validation_failed);
    }
}

/// Print any errors that occurred during processing
fn print_errors(stats: &SeedStats) {
    if !stats.errors.is_empty() {
        eprintln!();
        eprintln!("Errors: {}", stats.errors.len());
        for error in &stats.errors {
            eprintln!("  - {error}");
        }
    }
}

/// Print final summary
fn print_summary(stats: &SeedStats, dry_run: bool) {
    print_sample_counts(stats);
    print_validation_results(stats);
    print_errors(stats);

    if dry_run {
        println!();
        println!("DRY RUN complete - no validation was performed");
    }
}
