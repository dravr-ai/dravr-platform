// ABOUTME: System coaches seeding utility for Pierre MCP Server
// ABOUTME: Loads coach definitions from markdown files in coaches/ directory
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Markdown Seeder
//!
//! This binary loads coach definitions from markdown files and syncs them to the database.
//! Coaches are defined in `coaches/` directory with YAML frontmatter and structured sections.
//!
//! ## Usage
//!
//! ```bash
//! # Seed coaches from markdown files
//! cargo run --bin seed-coaches
//!
//! # Override database URL
//! cargo run --bin seed-coaches -- --database-url sqlite:./data/users.db
//!
//! # Verbose output
//! cargo run --bin seed-coaches -- -v
//!
//! # Dry run (show what would be done)
//! cargo run --bin seed-coaches -- --dry-run
//! ```

use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::Parser;
use glob::glob;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::SeederRepository;
use pierre_database::seed_models::{
    SeedCoach, SeedCoachAuthor, SeedCoachRelation, SeedStoreListing,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use pierre_mcp_server::coaches::{parse_coach_file, CoachDefinition, RelatedCoach, RelationType};

#[derive(Parser)]
#[command(
    name = "seed-coaches",
    about = "Pierre MCP Server Coach Seeder",
    long_about = "Load coach definitions from markdown files and sync to database"
)]
struct SeedArgs {
    /// Database URL override
    #[arg(long)]
    database_url: Option<String>,

    /// Path to coaches directory
    #[arg(long, default_value = "coaches")]
    coaches_dir: PathBuf,

    /// Dry run - show what would be done without making changes
    #[arg(long)]
    dry_run: bool,

    /// Enable verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,
}

/// Seeding result statistics
#[derive(Default)]
struct SeedStats {
    created: u32,
    updated: u32,
    unchanged: u32,
    relations_created: u32,
    store_published: u32,
    errors: Vec<String>,
}

impl SeedStats {
    const fn total_processed(&self) -> u32 {
        self.created + self.updated + self.unchanged
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let args = SeedArgs::parse();

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("=== Pierre MCP Server Coach Seeder ===");

    if args.dry_run {
        info!("DRY RUN - no changes will be made");
    }

    // Find and parse all coach markdown files
    let coaches = discover_coaches(&args.coaches_dir)?;
    info!("Found {} coach markdown files", coaches.len());

    if coaches.is_empty() {
        warn!("No coach files found in {:?}", args.coaches_dir);
        return Ok(());
    }

    // Load database URL
    let database_url = args
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    // Connect to database
    info!("Connecting to database: {}", database_url);
    let db = Database::init_for_seeding(&database_url).await?;

    // Find admin user
    let admin = find_admin_user(&db).await?;
    info!(
        "Using admin user: {} (tenant: {})",
        admin.email, admin.tenant_id
    );

    // Pass 1: Upsert coaches
    let (mut stats, slug_to_id) = sync_coaches(&db, &coaches, &admin, args.dry_run).await;

    // Pass 2: Create relations
    sync_relations(&db, &coaches, &slug_to_id, &mut stats, args.dry_run).await;

    // Pass 3: Publish system coaches to the store
    publish_to_store(&db, &slug_to_id, &admin, &mut stats, args.dry_run).await;

    // Summary
    print_summary(&stats, args.dry_run);

    if !stats.errors.is_empty() {
        return Err(AppError::config(format!(
            "{} coach(es) failed to seed",
            stats.errors.len()
        )));
    }

    Ok(())
}

/// Sync all coaches to the database (Pass 1)
async fn sync_coaches(
    db: &Database,
    coaches: &[CoachDefinition],
    admin: &AdminUser,
    dry_run: bool,
) -> (SeedStats, HashMap<String, String>) {
    info!("");
    info!("=== Pass 1: Syncing Coaches ===");
    let mut stats = SeedStats::default();
    let mut slug_to_id: HashMap<String, String> = HashMap::new();

    for coach in coaches {
        match upsert_coach(db, coach, admin, dry_run).await {
            Ok((coach_id, action)) => {
                slug_to_id.insert(coach.frontmatter.name.clone(), coach_id);
                log_upsert_result(&coach.frontmatter.title, &action, &mut stats);
            }
            Err(e) => {
                warn!("  ✗ {} - Error: {}", coach.frontmatter.title, e);
                stats
                    .errors
                    .push(format!("{}: {}", coach.frontmatter.name, e));
            }
        }
    }

    (stats, slug_to_id)
}

/// Log the result of an upsert operation and update stats
fn log_upsert_result(title: &str, action: &UpsertAction, stats: &mut SeedStats) {
    match action {
        UpsertAction::Created => {
            info!("  + {} (created)", title);
            stats.created += 1;
        }
        UpsertAction::Updated => {
            info!("  ~ {} (updated)", title);
            stats.updated += 1;
        }
        UpsertAction::Unchanged => {
            debug!("  = {} (unchanged)", title);
            stats.unchanged += 1;
        }
    }
}

/// Sync coach relations to the database (Pass 2)
async fn sync_relations(
    db: &Database,
    coaches: &[CoachDefinition],
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 2: Syncing Relations ===");

    for coach in coaches {
        process_coach_relations(db, coach, slug_to_id, stats, dry_run).await;
    }

    log_relations_created(stats.relations_created);
}

/// Process all relations for a single coach
async fn process_coach_relations(
    db: &Database,
    coach: &CoachDefinition,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    let Some(coach_id) = slug_to_id.get(&coach.frontmatter.name) else {
        return;
    };

    for relation in &coach.sections.related_coaches {
        process_single_relation(
            db,
            coach_id,
            &coach.frontmatter.name,
            relation,
            slug_to_id,
            stats,
            dry_run,
        )
        .await;
    }
}

/// Log how many relations were created
fn log_relations_created(count: u32) {
    if count > 0 {
        info!("  Created {} relations", count);
    }
}

/// Process a single coach relation
async fn process_single_relation(
    db: &Database,
    coach_id: &str,
    coach_name: &str,
    relation: &RelatedCoach,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    let Some(related_id) = slug_to_id.get(&relation.slug) else {
        debug!(
            "  Skipping relation {} -> {} (target not found)",
            coach_name, relation.slug
        );
        return;
    };

    if dry_run {
        log_dry_run_relation(coach_name, relation.relation_type, &relation.slug);
        return;
    }

    let relation_created = create_relation(db, coach_id, related_id, relation.relation_type)
        .await
        .unwrap_or(false);
    if relation_created {
        stats.relations_created += 1;
    }
}

/// Log a relation that would be created in dry run mode
fn log_dry_run_relation(coach_name: &str, relation_type: RelationType, target_slug: &str) {
    info!(
        "  Would create: {} --[{}]--> {}",
        coach_name,
        format!("{relation_type:?}").to_lowercase(),
        target_slug
    );
}

/// Print final summary
fn print_summary(stats: &SeedStats, dry_run: bool) {
    info!("");
    info!("=== Seeding Complete ===");
    log_coach_counts(stats);
    print_errors(&stats.errors);
    log_dry_run_status(dry_run);
}

/// Log the coach processing counts
fn log_coach_counts(stats: &SeedStats) {
    info!(
        "Processed: {} coaches ({} created, {} updated, {} unchanged)",
        stats.total_processed(),
        stats.created,
        stats.updated,
        stats.unchanged
    );
    if stats.store_published > 0 {
        info!("Published: {} coaches to store", stats.store_published);
    }
}

/// Print error list if any errors occurred
fn print_errors(errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    warn!("Errors: {}", errors.len());
    for error in errors {
        warn!("  - {}", error);
    }
}

/// Log dry run completion status
fn log_dry_run_status(dry_run: bool) {
    if dry_run {
        info!("DRY RUN complete - no changes were made");
    }
}

/// Publish seeded coaches to the store (Pass 3)
///
/// System coaches are auto-published so they appear in the Discover tab.
/// Uses INSERT OR IGNORE to be idempotent on re-runs.
async fn publish_to_store(
    db: &Database,
    slug_to_id: &HashMap<String, String>,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 3: Publishing to Store ===");

    for (slug, coach_id) in slug_to_id {
        publish_or_skip(db, slug, coach_id, admin, stats, dry_run).await;
    }
}

/// Publish one coach or log skip in dry-run mode
async fn publish_or_skip(
    db: &Database,
    slug: &str,
    coach_id: &str,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    if dry_run {
        info!("  Would publish: {slug}");
        stats.store_published += 1;
        return;
    }

    let result = publish_single_coach(db, coach_id, admin).await;
    log_publish_result(slug, result, stats);
}

/// Log and record the result of a store publish attempt
fn log_publish_result(slug: &str, result: AppResult<bool>, stats: &mut SeedStats) {
    match result {
        Ok(true) => {
            info!("  + {slug} (published)");
            stats.store_published += 1;
        }
        Ok(false) => {
            debug!("  = {slug} (already published)");
        }
        Err(e) => {
            warn!("  ✗ {slug} - Store publish error: {e}");
            stats
                .errors
                .push(format!("{slug}: store publish failed: {e}"));
        }
    }
}

/// Publish a single coach to the store, returning true if newly published
async fn publish_single_coach(db: &Database, coach_id: &str, admin: &AdminUser) -> AppResult<bool> {
    let listing = SeedStoreListing {
        id: Uuid::new_v4().to_string(),
        coach_id: coach_id.to_owned(),
        tenant_id: admin.tenant_id,
        author_id: admin.coach_author_id.clone(),
        created_at: Utc::now(),
    };

    db.seed_insert_store_listing_if_absent(&listing).await
}

/// Discover and parse all coach markdown files
fn discover_coaches(coaches_dir: &Path) -> AppResult<Vec<CoachDefinition>> {
    let pattern = coaches_dir.join("**/*.md");
    let pattern_str = pattern.to_string_lossy();

    let mut coaches = Vec::new();

    for entry in
        glob(&pattern_str).map_err(|e| AppError::internal(format!("Glob pattern error: {e}")))?
    {
        let path = entry.map_err(|e| AppError::internal(format!("Glob error: {e}")))?;

        // Skip README files
        if path.file_name().is_some_and(|n| n == "README.md") {
            continue;
        }

        match parse_coach_file(&path) {
            Ok(coach) => {
                debug!("Parsed: {} ({})", coach.frontmatter.name, path.display());
                coaches.push(coach);
            }
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
            }
        }
    }

    // Sort by category then name for consistent ordering
    coaches.sort_by(|a, b| {
        let cat_cmp = a
            .frontmatter
            .category
            .as_str()
            .cmp(b.frontmatter.category.as_str());
        if cat_cmp == Ordering::Equal {
            a.frontmatter.name.cmp(&b.frontmatter.name)
        } else {
            cat_cmp
        }
    });

    Ok(coaches)
}

/// Admin user info needed for seeding
struct AdminUser {
    id: Uuid,
    email: String,
    tenant_id: TenantId,
    /// Coach author profile ID (from `coach_authors` table, used as `store_listings.author_id`)
    coach_author_id: String,
}

/// Find the first admin user and their tenant, ensuring a `coach_authors` row exists
async fn find_admin_user(db: &Database) -> AppResult<AdminUser> {
    let user = db.seed_get_admin_user().await?.ok_or_else(|| {
        AppError::config(
            "No admin user found. Run 'cargo run --bin pierre-cli -- user create' first.",
        )
    })?;

    let tenant_id_str = db.seed_get_user_tenant(user.id).await?.ok_or_else(|| {
        AppError::config("Admin user has no tenant_id. Please assign a tenant first.")
    })?;

    let tenant_id = tenant_id_str
        .parse::<TenantId>()
        .map_err(|e| AppError::internal(format!("Failed to parse tenant_id: {e}")))?;

    // Ensure a coach_authors row exists for the admin (required by store_listings FK)
    let now = Utc::now();
    let coach_author = SeedCoachAuthor {
        id: Uuid::new_v4().to_string(),
        user_id: user.id,
        tenant_id: tenant_id.to_string(),
        display_name: user.email.clone(),
        created_at: now,
        updated_at: now,
    };
    let coach_author_id = db.seed_upsert_coach_author(&coach_author).await?;
    info!("Coach author profile: {coach_author_id}");

    Ok(AdminUser {
        id: user.id,
        email: user.email,
        tenant_id,
        coach_author_id,
    })
}

/// Result of upsert operation
enum UpsertAction {
    Created,
    Updated,
    Unchanged,
}

/// Upsert a coach into the database
async fn upsert_coach(
    db: &Database,
    coach: &CoachDefinition,
    admin: &AdminUser,
    dry_run: bool,
) -> AppResult<(String, UpsertAction)> {
    let now = Utc::now();
    let slug = &coach.frontmatter.name;

    // Check if coach exists by slug
    let existing = db
        .seed_find_coach_by_slug(slug, &admin.tenant_id.to_string())
        .await?;

    let action = if let Some((existing_id, existing_hash)) = existing {
        // Coach exists - check if content changed
        if existing_hash.as_deref() == Some(&coach.content_hash) {
            return Ok((existing_id, UpsertAction::Unchanged));
        }

        if !dry_run {
            let seed_coach = build_seed_coach(&existing_id, coach, admin, now)?;
            db.seed_update_coach(&seed_coach).await?;
        }
        (existing_id, UpsertAction::Updated)
    } else {
        // New coach
        let new_id = Uuid::new_v4().to_string();

        if !dry_run {
            let seed_coach = build_seed_coach(&new_id, coach, admin, now)?;
            db.seed_insert_coach(&seed_coach).await?;
        }
        (new_id, UpsertAction::Created)
    };

    Ok(action)
}

/// Convert example inputs bullet list to JSON array
fn parse_sample_prompts(example_inputs: Option<&String>) -> String {
    example_inputs.map_or_else(
        || "[]".to_owned(),
        |inputs| {
            let prompts: Vec<&str> = inputs
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix('-')
                        .map(|rest| rest.trim().trim_matches('"'))
                })
                .collect();
            serde_json::to_string(&prompts).unwrap_or_else(|_| "[]".to_owned())
        },
    )
}

/// Build a `SeedCoach` from a parsed `CoachDefinition` and admin context
fn build_seed_coach(
    id: &str,
    coach: &CoachDefinition,
    admin: &AdminUser,
    now: DateTime<Utc>,
) -> AppResult<SeedCoach> {
    let prerequisites_json = serde_json::to_string(&coach.frontmatter.prerequisites)
        .map_err(|e| AppError::internal(format!("JSON error: {e}")))?;
    let tags_json = serde_json::to_string(&coach.frontmatter.tags)
        .map_err(|e| AppError::internal(format!("JSON error: {e}")))?;
    let sample_prompts_json = parse_sample_prompts(coach.sections.example_inputs.as_ref());

    Ok(SeedCoach {
        id: id.to_owned(),
        user_id: admin.id,
        tenant_id: admin.tenant_id,
        title: coach.frontmatter.title.clone(),
        description: coach.sections.purpose.clone(),
        system_prompt: coach.sections.instructions.clone(),
        category: coach.frontmatter.category.as_str().to_owned(),
        tags_json,
        sample_prompts_json,
        token_count: i64::from(coach.token_count),
        visibility: coach.frontmatter.visibility.as_str().to_owned(),
        slug: coach.frontmatter.name.clone(),
        purpose: Some(coach.sections.purpose.clone()),
        when_to_use: coach.sections.when_to_use.clone(),
        instructions: Some(coach.sections.instructions.clone()),
        example_inputs: coach.sections.example_inputs.clone(),
        example_outputs: coach.sections.example_outputs.clone(),
        success_criteria: coach.sections.success_criteria.clone(),
        prerequisites_json,
        source_file: Some(coach.source_file.clone()),
        content_hash: Some(coach.content_hash.clone()),
        startup_query: coach.frontmatter.startup.query.clone(),
        created_at: now,
        updated_at: now,
    })
}

/// Create a relation between two coaches
async fn create_relation(
    db: &Database,
    coach_id: &str,
    related_id: &str,
    relation_type: RelationType,
) -> AppResult<bool> {
    let relation_str = match relation_type {
        RelationType::Related => "related",
        RelationType::Alternative => "alternative",
        RelationType::Prerequisite => "prerequisite",
        RelationType::Sequel => "sequel",
    };

    let relation = SeedCoachRelation {
        id: Uuid::new_v4().to_string(),
        coach_id: coach_id.to_owned(),
        related_coach_id: related_id.to_owned(),
        relation_type: relation_str.to_owned(),
        created_at: Utc::now(),
    };

    db.seed_insert_coach_relation_if_absent(&relation).await
}
