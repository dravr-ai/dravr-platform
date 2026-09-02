// ABOUTME: System coaches seeding utility for Pierre MCP Server
// ABOUTME: Loads coach definitions from a contremaitre checkout (single source of truth)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Markdown Seeder
//!
//! This binary loads coach definitions from markdown files and syncs them
//! to the database. The checkout is the whole roster: a catalogue coach whose
//! directory is gone is deleted on the next run, store listing included. Coaches are defined in the dravr-contremaitre repo
//! under `prompts/coaches/<category>/<slug>/<locale>.md`, with `en.md` as
//! the canonical source and per-locale siblings (e.g. `fr.md`) layered on
//! top via [`pierre_database::CoachesRepository::apply_translations`].
//!
//! ## Usage
//!
//! ```bash
//! # Seed coaches from a contremaitre checkout (path required)
//! pierre-cli seed coaches --coaches-dir /tmp/contremaitre/prompts/coaches
//!
//! # The flag can also come from PIERRE_COACHES_DIR; the seed-entrypoint
//! # script clones contremaitre and exports it before invoking the binary.
//! PIERRE_COACHES_DIR=/tmp/contremaitre/prompts/coaches pierre-cli seed coaches
//!
//! # Dry run (show what would be done)
//! pierre-cli seed coaches --coaches-dir <path> --dry-run
//! ```

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use glob::glob;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::seed_models::{
    SeedCoach, SeedCoachAuthor, SeedCoachRelation, SeedCoachTranslation, SeedStoreListing,
};
use pierre_database::RepositoryRegistry;
use tracing::{debug, info, warn};
use uuid::Uuid;

use pierre_coach_parser::{
    is_locale_code, parse_coach_file, CoachDefinition, RelatedCoach, RelationType, CANONICAL_LOCALE,
};

/// CLI arguments for the coaches seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Path to a `prompts/coaches` checkout from the dravr-contremaitre
    /// repository. Coach definitions live in the contremaitre repo as the
    /// single source of truth, laid out as
    /// `<category>/<slug>/<locale>.md`. Set via the flag or
    /// `$PIERRE_COACHES_DIR`. The Cloud Run seed-entrypoint clones
    /// contremaitre to a temp dir and exports this variable; local
    /// development can point it at a sibling checkout.
    #[arg(long, env = "PIERRE_COACHES_DIR")]
    pub coaches_dir: PathBuf,

    /// Dry run - show what would be done without making changes
    #[arg(long)]
    pub dry_run: bool,
}

/// Seeding result statistics
#[derive(Default)]
struct SeedStats {
    created: u32,
    updated: u32,
    unchanged: u32,
    relations_created: u32,
    store_published: u32,
    pruned: u32,
    errors: Vec<String>,
}

impl SeedStats {
    const fn total_processed(&self) -> u32 {
        self.created + self.updated + self.unchanged
    }
}

/// Parse coach markdown definitions and sync them to the database, publishing system coaches to the store.
///
/// # Errors
///
/// Returns an error if coach markdown files cannot be discovered or parsed, or if any
/// repository operation fails while syncing coaches, relations, or store listings.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    info!(
        "=== Pierre MCP Server Coach Seeder (dry_run={}) ===",
        args.dry_run
    );

    let discovery = discover_coaches(&args.coaches_dir)?;
    if discovery.coaches.is_empty() {
        warn!("No coach files found in {:?}", args.coaches_dir);
        return Ok(());
    }

    let admin = find_admin_user(repos).await?;
    info!(
        "Found {} coach files, using admin {} (tenant: {})",
        discovery.coaches.len(),
        admin.email,
        admin.tenant_id
    );

    let stats = run_coach_passes(repos, &discovery, &admin, args.dry_run).await;
    print_summary(&stats, args.dry_run);
    finalize_stats(&stats)
}

/// Execute the five coach sync passes (canonical upsert, translation upsert, relations,
/// store publishing, retired-coach pruning) and return the accumulated stats.
async fn run_coach_passes(
    repos: &RepositoryRegistry,
    discovery: &Discovery,
    admin: &AdminUser,
    dry_run: bool,
) -> SeedStats {
    let coaches = &discovery.coaches;
    let canon: Vec<&CoachDefinition> = coaches.iter().map(|c| &c.canonical).collect();
    let (mut stats, slug_to_id) = sync_coaches(repos, &canon, admin, dry_run).await;
    take_catalogue_ownership(repos, admin, &mut stats, dry_run).await;
    sync_translations(repos, coaches, &slug_to_id, &mut stats, dry_run).await;
    sync_relations(repos, &canon, &slug_to_id, &mut stats, dry_run).await;
    publish_to_store(repos, &slug_to_id, admin, &mut stats, dry_run).await;
    prune_retired(repos, discovery, &slug_to_id, admin, &mut stats, dry_run).await;
    stats
}

fn finalize_stats(stats: &SeedStats) -> AppResult<()> {
    if stats.errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::config(format!(
            "{} coach(es) failed to seed",
            stats.errors.len()
        )))
    }
}

/// Sync all coaches to the database (Pass 1)
async fn sync_coaches(
    repos: &RepositoryRegistry,
    coaches: &[&CoachDefinition],
    admin: &AdminUser,
    dry_run: bool,
) -> (SeedStats, HashMap<String, String>) {
    info!("");
    info!("=== Pass 1: Syncing Coaches ===");
    let mut stats = SeedStats::default();
    let mut slug_to_id: HashMap<String, String> = HashMap::new();

    for coach in coaches {
        match upsert_coach(repos, coach, admin, dry_run).await {
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

/// Claim the tenant's legacy `source = 'seed'` rows for the catalogue.
///
/// Pass 1 only writes a row whose content hash changed, so a coach untouched
/// since the source-column migration kept its transitional `'seed'` stamp and
/// the daily drift gate warned about it every morning. A catalogue file for
/// the slug is what makes the catalogue authoritative, not an edit. Rows the
/// checkout no longer carries are stamped too; the prune pass deletes them
/// in the same run.
async fn take_catalogue_ownership(
    repos: &RepositoryRegistry,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    if dry_run {
        debug!("  [dry-run] legacy 'seed' rows would be claimed for the catalogue");
        return;
    }
    match repos
        .seeder
        .seed_take_catalogue_ownership(&admin.tenant_id.to_string())
        .await
    {
        Ok(0) => {}
        Ok(claimed) => info!("  ~ {claimed} legacy row(s) now owned by the catalogue"),
        Err(e) => {
            warn!("  ✗ Could not take catalogue ownership: {e}");
            stats.errors.push(format!("ownership: {e}"));
        }
    }
}

/// Sync per-locale translations to `coach_translations` (Pass 2).
///
/// Runs AFTER [`sync_coaches`] so every `coach_id` referenced here is already
/// present in the `coaches` table. Skips coaches that failed canonical
/// upsert; never re-parents translations onto a different slug.
async fn sync_translations(
    repos: &RepositoryRegistry,
    coaches: &[CoachWithTranslations],
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 2: Syncing Translations ===");
    let mut synced = 0u32;
    for item in coaches {
        synced += sync_translations_for_coach(repos, item, slug_to_id, stats, dry_run).await;
    }
    info!("  → {} translation(s) synced", synced);
}

/// Upsert every translation file for one coach; returns the number synced.
async fn sync_translations_for_coach(
    repos: &RepositoryRegistry,
    item: &CoachWithTranslations,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) -> u32 {
    let slug = &item.canonical.frontmatter.name;
    let Some(coach_id) = slug_to_id.get(slug) else {
        return 0;
    };
    let mut synced = 0u32;
    for tr in &item.translations {
        if upsert_single_translation(repos, slug, coach_id, tr, stats, dry_run).await {
            synced += 1;
        }
    }
    synced
}

/// Apply a single [`CoachTranslationFile`] to `coach_translations`.
///
/// Returns `true` when the row was upserted (or would be, under dry-run).
/// Errors accumulate into `stats.errors`; callers use the return value for
/// progress counting only.
async fn upsert_single_translation(
    repos: &RepositoryRegistry,
    slug: &str,
    coach_id: &str,
    tr: &CoachTranslationFile,
    stats: &mut SeedStats,
    dry_run: bool,
) -> bool {
    if dry_run {
        info!("  + [dry-run] {} [{}]", slug, tr.locale);
        return true;
    }
    // description mirrors `## Purpose` per the same convention the
    // canonical seeder uses (see `build_seed_coach`). `purpose` carries
    // the same copy explicitly so a caller reading Coach.purpose still
    // sees the translated text.
    let seed = SeedCoachTranslation {
        coach_id: coach_id.to_owned(),
        locale: tr.locale.clone(),
        title: Some(tr.coach.frontmatter.title.clone()),
        description: Some(tr.coach.sections.purpose.clone()),
        purpose: Some(tr.coach.sections.purpose.clone()),
        instructions: Some(tr.coach.sections.instructions.clone()),
        // Prefer the canonical English content hash for drift tracking when
        // the translation file doesn't declare its own source_sha. Phase 3
        // will add a frontmatter `source_sha:` override path.
        source_sha: Some(tr.source_sha_hint.clone()),
        // A locale file that declares tags renames the chips for that locale;
        // one that declares none leaves the English tags visible.
        tags: (!tr.coach.frontmatter.tags.is_empty()).then(|| tr.coach.frontmatter.tags.clone()),
    };
    match repos.seeder.seed_upsert_coach_translation(&seed).await {
        Ok(()) => {
            debug!("  + {} [{}] (upserted)", slug, tr.locale);
            true
        }
        Err(e) => {
            warn!("  ✗ {} [{}] - Error: {}", slug, tr.locale, e);
            stats.errors.push(format!("{slug}/{}: {}", tr.locale, e));
            false
        }
    }
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

/// Sync coach relations to the database (Pass 3)
async fn sync_relations(
    repos: &RepositoryRegistry,
    coaches: &[&CoachDefinition],
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 3: Syncing Relations ===");

    for coach in coaches {
        process_coach_relations(repos, coach, slug_to_id, stats, dry_run).await;
    }

    log_relations_created(stats.relations_created);
}

/// Process all relations for a single coach
async fn process_coach_relations(
    repos: &RepositoryRegistry,
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
            repos,
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
    repos: &RepositoryRegistry,
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

    let relation_created = create_relation(repos, coach_id, related_id, relation.relation_type)
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
    if stats.pruned > 0 {
        info!("Pruned: {} retired coaches", stats.pruned);
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

/// Publish seeded coaches to the store (Pass 4)
///
/// System coaches are auto-published so they appear in the Discover tab.
/// Uses INSERT OR IGNORE to be idempotent on re-runs.
async fn publish_to_store(
    repos: &RepositoryRegistry,
    slug_to_id: &HashMap<String, String>,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 4: Publishing to Store ===");

    for (slug, coach_id) in slug_to_id {
        publish_or_skip(repos, slug, coach_id, admin, stats, dry_run).await;
    }
}

/// Publish one coach or log skip in dry-run mode
async fn publish_or_skip(
    repos: &RepositoryRegistry,
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

    let result = publish_single_coach(repos, coach_id, admin).await;
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

/// Delete catalogue-owned coaches whose markdown directory is gone (Pass 5).
///
/// The passes above only ever add or rewrite rows, so a coach retired from
/// dravr-contremaitre used to keep its row — and its store listing — in every
/// database forever. This pass diffs the tenant's catalogue-owned rows against
/// the slugs discovered on disk and deletes the rest through the same
/// repository method the admin console uses. The store listing, relations,
/// translations and assignments follow by cascade; an athlete's installed
/// copy keeps working because its `forked_from` pointer is set to NULL rather
/// than deleted.
///
/// Live references go first. A discovered coach that names the retired slug
/// in its `replaces` frontmatter is the successor: the retired coach's
/// conversations, groups and coach pointers are handed to it, so an athlete
/// mid-conversation with a merged coach continues with the coach that
/// absorbed it. Without a successor the conversations are detached and drop
/// to the default prompt; a group still bound to such a coach blocks the
/// delete, the error is counted, and the seed job exits non-zero so an
/// operator picks a coach for it.
///
/// The pass is skipped when any coach file failed to parse: a coach that is
/// still in the checkout would otherwise read as retired.
async fn prune_retired(
    repos: &RepositoryRegistry,
    discovery: &Discovery,
    slug_to_id: &HashMap<String, String>,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 5: Pruning Retired Coaches ===");

    if discovery.parse_failures > 0 {
        warn!(
            "  Skipping the prune pass: {} coach file(s) failed to parse, so a coach still in the checkout could read as retired",
            discovery.parse_failures
        );
        return;
    }
    let Some(rows) = list_catalogue_rows(repos, admin, stats).await else {
        return;
    };
    let keep = discovered_slugs(&discovery.coaches);
    let successors = successor_ids(&discovery.coaches, slug_to_id);
    for (coach_id, slug) in rows
        .iter()
        .filter(|(_, slug)| !keep.contains(slug.as_str()))
    {
        let successor = successors.get(slug.as_str()).map(|id| id.as_str());
        retire_coach(repos, coach_id, slug, successor, admin, stats, dry_run).await;
    }
    log_pruned(stats.pruned);
}

/// The slugs present in the checkout — the roster every catalogue row must be on.
fn discovered_slugs(discovered: &[CoachWithTranslations]) -> HashSet<&str> {
    discovered
        .iter()
        .map(|c| c.canonical.frontmatter.name.as_str())
        .collect()
}

/// Map each retired slug to the id of the discovered coach whose `replaces` names it.
///
/// A successor that failed its own upsert has no id and is left out, so the
/// coaches it would have absorbed fall back to detaching.
fn successor_ids<'a>(
    discovered: &'a [CoachWithTranslations],
    slug_to_id: &'a HashMap<String, String>,
) -> HashMap<&'a str, &'a String> {
    let mut out = HashMap::new();
    for coach in discovered {
        let Some(id) = slug_to_id.get(&coach.canonical.frontmatter.name) else {
            continue;
        };
        for retired in &coach.canonical.frontmatter.replaces {
            out.insert(retired.as_str(), id);
        }
    }
    out
}

/// Catalogue-owned `(id, slug)` rows in the admin tenant, or `None` once the
/// listing error has been recorded — a prune that cannot see the roster must
/// not guess at it.
async fn list_catalogue_rows(
    repos: &RepositoryRegistry,
    admin: &AdminUser,
    stats: &mut SeedStats,
) -> Option<Vec<(String, String)>> {
    match repos
        .seeder
        .seed_list_catalogue_coaches(&admin.tenant_id.to_string())
        .await
    {
        Ok(rows) => Some(rows),
        Err(e) => {
            warn!("  ✗ Could not list catalogue coaches: {e}");
            stats.errors.push(format!("prune: {e}"));
            None
        }
    }
}

/// Hand a retired coach's live references over, then delete it — or log what
/// dry-run would delete.
async fn retire_coach(
    repos: &RepositoryRegistry,
    coach_id: &str,
    slug: &str,
    successor_id: Option<&str>,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    if dry_run {
        info!("  Would delete: {slug} (no longer in the catalogue)");
        stats.pruned += 1;
        return;
    }

    if let Err(e) = hand_over_references(repos, coach_id, successor_id).await {
        warn!("  ✗ {slug} - Hand-over error: {e}");
        stats.errors.push(format!("{slug}: hand-over failed: {e}"));
        return;
    }
    let result = repos
        .coaches
        .delete_system_coach(coach_id, admin.tenant_id)
        .await;
    log_delete_result(slug, result, stats);
}

/// Re-point the retired coach's conversations, groups and coach pointers at
/// the successor, or detach its conversations when there is none.
async fn hand_over_references(
    repos: &RepositoryRegistry,
    coach_id: &str,
    successor_id: Option<&str>,
) -> AppResult<()> {
    if let Some(successor) = successor_id {
        let moved = repos
            .seeder
            .seed_repoint_coach_references(coach_id, successor)
            .await?;
        if moved > 0 {
            info!("    → {moved} reference(s) handed to the successor");
        }
    } else {
        let detached = repos
            .seeder
            .seed_detach_coach_conversations(coach_id)
            .await?;
        if detached > 0 {
            info!("    → {detached} conversation(s) detached, no successor declared");
        }
    }
    Ok(())
}

/// Log and record the result of one retired-coach deletion
fn log_delete_result(slug: &str, result: AppResult<bool>, stats: &mut SeedStats) {
    match result {
        Ok(true) => {
            info!("  - {slug} (deleted, no longer in the catalogue)");
            stats.pruned += 1;
        }
        Ok(false) => {
            debug!("  = {slug} (already gone)");
        }
        Err(e) => {
            warn!("  ✗ {slug} - Delete error: {e}");
            stats.errors.push(format!("{slug}: delete failed: {e}"));
        }
    }
}

/// Log how many retired coaches were pruned
fn log_pruned(count: u32) {
    if count > 0 {
        info!("  Pruned {} retired coach(es)", count);
    }
}

/// Publish a single coach to the store, returning true if newly published
async fn publish_single_coach(
    repos: &RepositoryRegistry,
    coach_id: &str,
    admin: &AdminUser,
) -> AppResult<bool> {
    let listing = SeedStoreListing {
        id: Uuid::new_v4().to_string(),
        coach_id: coach_id.to_owned(),
        tenant_id: admin.tenant_id,
        author_id: admin.coach_author_id.clone(),
        created_at: Utc::now(),
    };

    repos
        .seeder
        .seed_insert_store_listing_if_absent(&listing)
        .await
}

/// Discover canonical coach definitions + their non-canonical translations.
///
/// Expected layout: `coaches_dir/<category>/<slug>/<locale>.md` where `<locale>`
/// is one of [`pierre_coach_parser::SUPPORTED_LOCALES`]. Each coach directory MUST
/// contain `en.md` (the canonical source); optional `fr.md` / `es.md` / `de.md`
/// / `pt.md` siblings become [`CoachTranslationFile`] rows that layer over the
/// canonical copy at read time via
/// [`pierre_database::CoachesRepository::apply_translations`].
///
/// Directories without `en.md` are skipped with a warning — the seeder never
/// stores a coach whose canonical English content is missing.
fn discover_coaches(coaches_dir: &Path) -> AppResult<Discovery> {
    let (canonical, translations, parse_failures) = scan_coach_files(coaches_dir)?;
    Ok(Discovery {
        coaches: pivot_and_sort(canonical, translations),
        parse_failures,
    })
}

/// What the checkout scan found: the coaches, plus how many files the parser refused.
///
/// The prune pass reads the count — a refused file is a coach it cannot see,
/// not a coach that left the catalogue.
pub(crate) struct Discovery {
    /// Every coach with a parseable `en.md`, sorted by category then slug.
    pub coaches: Vec<CoachWithTranslations>,
    /// Locale files that failed to parse and were skipped with a warning.
    pub parse_failures: usize,
}

/// Snapshot of the filesystem scan: canonical coaches keyed by slug, the
/// translation files keyed by the same slug, and how many files failed to parse.
type ScannedCoaches = (
    HashMap<String, CoachDefinition>,
    HashMap<String, Vec<CoachTranslationFile>>,
    usize,
);

/// Walk `coaches_dir/<category>/<slug>/*.md` and bucket by slug into two maps.
///
/// Filters out non-locale filenames (README.md, stray `*.md` files) up-front
/// so the caller only ever sees valid per-locale entries. Parsing errors are
/// logged and skipped — one malformed file never blocks the whole seed.
fn scan_coach_files(coaches_dir: &Path) -> AppResult<ScannedCoaches> {
    let pattern = coaches_dir.join("*/*/*.md");
    let pattern_str = pattern.to_string_lossy();

    let mut canonical: HashMap<String, CoachDefinition> = HashMap::new();
    let mut translations: HashMap<String, Vec<CoachTranslationFile>> = HashMap::new();
    let mut parse_failures = 0usize;

    for entry in
        glob(&pattern_str).map_err(|e| AppError::internal(format!("Glob pattern error: {e}")))?
    {
        let path = entry.map_err(|e| AppError::internal(format!("Glob error: {e}")))?;
        if !record_coach_path(&path, &mut canonical, &mut translations) {
            parse_failures += 1;
        }
    }
    Ok((canonical, translations, parse_failures))
}

/// Parse `path` and route it into either the canonical or translations map.
///
/// Non-locale filenames are logged at debug and ignored. Parse errors get a
/// warning but never bubble up so one broken file can't block the seeder;
/// they return `false` so the caller can count them.
fn record_coach_path(
    path: &Path,
    canonical: &mut HashMap<String, CoachDefinition>,
    translations: &mut HashMap<String, Vec<CoachTranslationFile>>,
) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !is_locale_code(stem) {
        debug!(
            "Skipping non-locale coach file (expected en/fr/es/de/pt): {}",
            path.display()
        );
        return true;
    }
    let coach = match parse_coach_file(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse {}: {}", path.display(), e);
            return false;
        }
    };
    let slug = coach.frontmatter.name.clone();
    if stem == CANONICAL_LOCALE {
        canonical.insert(slug, coach);
    } else {
        let canonical_sha = CoachDefinition::source_sha_prefix(&coach.content_hash);
        translations
            .entry(slug)
            .or_default()
            .push(CoachTranslationFile {
                locale: stem.to_owned(),
                coach,
                source_sha_hint: canonical_sha,
            });
    }
    true
}

/// Pivot the two scan maps into a sorted `Vec<CoachWithTranslations>`.
///
/// Coaches without an `en.md` are dropped with a warning so we never store
/// orphan translations. Output is sorted by category then slug for
/// deterministic test output and log stability.
fn pivot_and_sort(
    canonical: HashMap<String, CoachDefinition>,
    mut translations: HashMap<String, Vec<CoachTranslationFile>>,
) -> Vec<CoachWithTranslations> {
    let mut out: Vec<CoachWithTranslations> = Vec::with_capacity(canonical.len());
    for (slug, canon) in canonical {
        let trs = translations.remove(&slug).unwrap_or_default();
        out.push(CoachWithTranslations {
            canonical: canon,
            translations: trs,
        });
    }
    for (slug, _) in translations {
        warn!(
            "Orphan translations for slug '{}' (no en.md canonical source); skipping",
            slug
        );
    }

    out.sort_by(|a, b| {
        let cat_cmp = a
            .canonical
            .frontmatter
            .category
            .as_str()
            .cmp(b.canonical.frontmatter.category.as_str());
        if cat_cmp == Ordering::Equal {
            a.canonical
                .frontmatter
                .name
                .cmp(&b.canonical.frontmatter.name)
        } else {
            cat_cmp
        }
    });
    out
}

/// Canonical coach + optional locale translation siblings discovered together.
/// Produced by [`discover_coaches`] and consumed by the canonical + translation
/// seeder passes so the two always see a consistent view of what's on disk.
pub(crate) struct CoachWithTranslations {
    /// English source of truth.
    pub canonical: CoachDefinition,
    /// Non-English siblings: one [`CoachTranslationFile`] per `<locale>.md`.
    pub translations: Vec<CoachTranslationFile>,
}

/// A single `<slug>/<locale>.md` translation file (non-canonical).
pub(crate) struct CoachTranslationFile {
    /// BCP-47 short locale code captured from the filename stem.
    pub locale: String,
    /// Parsed `CoachDefinition` view of the translated file — only
    /// `title`/`description`/`purpose`/`instructions` will be overlaid
    /// onto the canonical row; the rest is ignored at upsert time.
    pub coach: CoachDefinition,
    /// First 16 hex chars of `sha256(<this-file-content>)`, used as the
    /// `source_sha` placeholder when the translation file does not ship an
    /// explicit `source_sha:` frontmatter field. Callers should prefer the
    /// translation's declared `source_sha` when present.
    pub source_sha_hint: String,
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
async fn find_admin_user(repos: &RepositoryRegistry) -> AppResult<AdminUser> {
    let user = repos.seeder.seed_get_admin_user().await?.ok_or_else(|| {
        AppError::config(
            "No admin user found. Run 'cargo run --bin pierre-cli -- user create' first.",
        )
    })?;

    let tenant_id_str = repos
        .seeder
        .seed_get_user_tenant(user.id)
        .await?
        .ok_or_else(|| {
            AppError::config("Admin user has no tenant_id. Please assign a tenant first.")
        })?;

    let tenant_id = TenantId::parse_str(&tenant_id_str)
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
    let coach_author_id = repos.seeder.seed_upsert_coach_author(&coach_author).await?;
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
    repos: &RepositoryRegistry,
    coach: &CoachDefinition,
    admin: &AdminUser,
    dry_run: bool,
) -> AppResult<(String, UpsertAction)> {
    let now = Utc::now();
    let slug = &coach.frontmatter.name;

    // Check if coach exists by slug
    let existing = repos
        .seeder
        .seed_find_coach_by_slug(slug, &admin.tenant_id.to_string())
        .await?;

    let action = if let Some((existing_id, existing_hash)) = existing {
        // Coach exists - check if content changed
        if existing_hash.as_deref() == Some(&coach.content_hash) {
            return Ok((existing_id, UpsertAction::Unchanged));
        }

        if !dry_run {
            let seed_coach = build_seed_coach(&existing_id, coach, admin, now)?;
            repos.seeder.seed_update_coach(&seed_coach).await?;
        }
        (existing_id, UpsertAction::Updated)
    } else {
        // New coach
        let new_id = Uuid::new_v4().to_string();

        if !dry_run {
            let seed_coach = build_seed_coach(&new_id, coach, admin, now)?;
            repos.seeder.seed_insert_coach(&seed_coach).await?;
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
        data_requirements: coach
            .frontmatter
            .startup
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok()),
        output_schema: coach.frontmatter.startup.output_schema.clone(),
        visuals: visuals_column(&coach.frontmatter.startup.visuals),
        created_at: now,
        updated_at: now,
    })
}

/// Create a relation between two coaches
async fn create_relation(
    repos: &RepositoryRegistry,
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

    repos
        .seeder
        .seed_insert_coach_relation_if_absent(&relation)
        .await
}

/// Join a coach's visual grants into the stored column form.
///
/// `None` rather than an empty string when there are no grants, so the column
/// reads as "no grant" instead of "granted nothing" — the two are the same to
/// the runtime, but NULL keeps the seeded rows honest about intent.
fn visuals_column(visuals: &[pierre_coach_parser::VisualKind]) -> Option<String> {
    if visuals.is_empty() {
        return None;
    }
    Some(
        visuals
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(","),
    )
}
