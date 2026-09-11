// ABOUTME: System agents seeding utility for Pierre MCP Server
// ABOUTME: Loads agent definitions from a contremaitre checkout (single source of truth)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent Markdown Seeder
//!
//! This binary loads agent definitions from markdown files and syncs them
//! to the database. The checkout is the whole roster: a catalogue agent whose
//! directory is gone is deleted on the next run, store listing included. Coaches are defined in the dravr-contremaitre repo
//! under `prompts/agents/<category>/<slug>/<locale>.md`, with `en.md` as
//! the canonical source and per-locale siblings (e.g. `fr.md`) layered on
//! top via [`pierre_database::AgentsRepository::apply_translations`].
//!
//! An agent directory may also carry a training package beside its prompt —
//! `flavour.yaml`, `skeleton.yaml`, `workouts/<slug>.toml` — which
//! [`crate::agent_packages`] reads through the periodization kernel and
//! stores as `agent_artefacts` rows, replaced as a set on every run.
//!
//! ## Usage
//!
//! ```bash
//! # Seed agents from a contremaitre checkout (path required)
//! pierre-cli seed agents --agents-dir /tmp/contremaitre/prompts/agents
//!
//! # The flag can also come from PIERRE_AGENTS_DIR; the seed-entrypoint
//! # script clones contremaitre and exports it before invoking the binary.
//! PIERRE_AGENTS_DIR=/tmp/contremaitre/prompts/agents pierre-cli seed agents
//!
//! # Dry run (show what would be done)
//! pierre-cli seed agents --agents-dir <path> --dry-run
//! ```

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use glob::glob;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::seed_models::{
    SeedAgent, SeedAgentAuthor, SeedAgentRelation, SeedAgentTranslation, SeedStoreListing,
};
use pierre_database::RepositoryRegistry;
use tracing::{debug, info, warn};
use uuid::Uuid;

use pierre_agent_parser::{
    is_locale_code, parse_agent_file, AgentDefinition, RelatedAgent, RelationType, CANONICAL_LOCALE,
};

use crate::agent_packages::sync_packages;

/// CLI arguments for the agents seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Path to a `prompts/agents` checkout from the dravr-contremaitre
    /// repository. Agent definitions live in the contremaitre repo as the
    /// single source of truth, laid out as
    /// `<category>/<slug>/<locale>.md`. Set via the flag or
    /// `$PIERRE_AGENTS_DIR`. The Cloud Run seed-entrypoint clones
    /// contremaitre to a temp dir and exports this variable; local
    /// development can point it at a sibling checkout.
    #[arg(long, env = "PIERRE_AGENTS_DIR")]
    pub agents_dir: PathBuf,

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
    packages_synced: u32,
    artefacts_synced: u32,
    errors: Vec<String>,
}

impl SeedStats {
    const fn total_processed(&self) -> u32 {
        self.created + self.updated + self.unchanged
    }
}

/// Parse agent markdown definitions and sync them to the database, publishing system agents to the store.
///
/// # Errors
///
/// Returns an error if agent markdown files cannot be discovered or parsed, or if any
/// repository operation fails while syncing agents, relations, or store listings.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    info!(
        "=== Pierre MCP Server Coach Seeder (dry_run={}) ===",
        args.dry_run
    );

    let discovery = discover_agents(&args.agents_dir)?;
    if discovery.agents.is_empty() {
        warn!("No coach files found in {:?}", args.agents_dir);
        return Ok(());
    }

    let admin = find_admin_user(repos).await?;
    info!(
        "Found {} coach files, using admin {} (tenant: {})",
        discovery.agents.len(),
        admin.email,
        admin.tenant_id
    );

    let stats = run_agent_passes(repos, &discovery, &args.agents_dir, &admin, args.dry_run).await;
    print_summary(&stats, args.dry_run);
    finalize_stats(&stats)
}

/// Execute the agent sync passes (canonical upsert, package artefacts, translation
/// upsert, relations, store publishing, retired-agent pruning) and return the
/// accumulated stats.
async fn run_agent_passes(
    repos: &RepositoryRegistry,
    discovery: &Discovery,
    agents_dir: &Path,
    admin: &AdminUser,
    dry_run: bool,
) -> SeedStats {
    let agents = &discovery.agents;
    let canon: Vec<&AgentDefinition> = agents.iter().map(|c| &c.canonical).collect();
    let (mut stats, slug_to_id) = sync_agents(repos, &canon, admin, dry_run).await;
    let packages = sync_packages(
        repos,
        agents_dir,
        &slug_to_id,
        &admin.tenant_id.to_string(),
        dry_run,
    )
    .await;
    stats.packages_synced = packages.packages_synced;
    stats.artefacts_synced = packages.artefacts_synced;
    stats.errors.extend(packages.errors);
    take_catalogue_ownership(repos, admin, &mut stats, dry_run).await;
    sync_translations(repos, agents, &slug_to_id, &mut stats, dry_run).await;
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

/// Sync all agents to the database (Pass 1)
async fn sync_agents(
    repos: &RepositoryRegistry,
    agents: &[&AgentDefinition],
    admin: &AdminUser,
    dry_run: bool,
) -> (SeedStats, HashMap<String, String>) {
    info!("");
    info!("=== Pass 1: Syncing Coaches ===");
    let mut stats = SeedStats::default();
    let mut slug_to_id: HashMap<String, String> = HashMap::new();

    for agent in agents {
        match upsert_agent(repos, agent, admin, dry_run).await {
            Ok((agent_id, action)) => {
                slug_to_id.insert(agent.frontmatter.name.clone(), agent_id);
                log_upsert_result(&agent.frontmatter.title, &action, &mut stats);
            }
            Err(e) => {
                warn!("  ✗ {} - Error: {}", agent.frontmatter.title, e);
                stats
                    .errors
                    .push(format!("{}: {}", agent.frontmatter.name, e));
            }
        }
    }

    (stats, slug_to_id)
}

/// Claim the tenant's legacy `source = 'seed'` rows for the catalogue.
///
/// Pass 1 only writes a row whose content hash changed, so an agent untouched
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

/// Sync per-locale translations to `agent_translations` (Pass 2).
///
/// Runs AFTER [`sync_agents`] so every `agent_id` referenced here is already
/// present in the `agents` table. Skips agents that failed canonical
/// upsert; never re-parents translations onto a different slug.
async fn sync_translations(
    repos: &RepositoryRegistry,
    agents: &[AgentWithTranslations],
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 2: Syncing Translations ===");
    let mut synced = 0u32;
    for item in agents {
        synced += sync_translations_for_agent(repos, item, slug_to_id, stats, dry_run).await;
    }
    info!("  → {} translation(s) synced", synced);
}

/// Upsert every translation file for one agent; returns the number synced.
async fn sync_translations_for_agent(
    repos: &RepositoryRegistry,
    item: &AgentWithTranslations,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) -> u32 {
    let slug = &item.canonical.frontmatter.name;
    let Some(agent_id) = slug_to_id.get(slug) else {
        return 0;
    };
    let mut synced = 0u32;
    for tr in &item.translations {
        if upsert_single_translation(repos, slug, agent_id, tr, stats, dry_run).await {
            synced += 1;
        }
    }
    synced
}

/// Apply a single [`AgentTranslationFile`] to `agent_translations`.
///
/// Returns `true` when the row was upserted (or would be, under dry-run).
/// Errors accumulate into `stats.errors`; callers use the return value for
/// progress counting only.
async fn upsert_single_translation(
    repos: &RepositoryRegistry,
    slug: &str,
    agent_id: &str,
    tr: &AgentTranslationFile,
    stats: &mut SeedStats,
    dry_run: bool,
) -> bool {
    if dry_run {
        info!("  + [dry-run] {} [{}]", slug, tr.locale);
        return true;
    }
    // description mirrors `## Purpose` per the same convention the
    // canonical seeder uses (see `build_seed_agent`). `purpose` carries
    // the same copy explicitly so a caller reading Agent.purpose still
    // sees the translated text.
    let seed = SeedAgentTranslation {
        agent_id: agent_id.to_owned(),
        locale: tr.locale.clone(),
        title: Some(tr.agent.frontmatter.title.clone()),
        description: Some(tr.agent.sections.purpose.clone()),
        purpose: Some(tr.agent.sections.purpose.clone()),
        instructions: Some(tr.agent.sections.instructions.clone()),
        // Prefer the canonical English content hash for drift tracking when
        // the translation file doesn't declare its own source_sha. Phase 3
        // will add a frontmatter `source_sha:` override path.
        source_sha: Some(tr.source_sha_hint.clone()),
        // A locale file that declares tags renames the chips for that locale;
        // one that declares none leaves the English tags visible.
        tags: (!tr.agent.frontmatter.tags.is_empty()).then(|| tr.agent.frontmatter.tags.clone()),
    };
    match repos.seeder.seed_upsert_agent_translation(&seed).await {
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

/// Sync agent relations to the database (Pass 3)
async fn sync_relations(
    repos: &RepositoryRegistry,
    agents: &[&AgentDefinition],
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    info!("");
    info!("=== Pass 3: Syncing Relations ===");

    for agent in agents {
        process_agent_relations(repos, agent, slug_to_id, stats, dry_run).await;
    }

    log_relations_created(stats.relations_created);
}

/// Process all relations for a single agent
async fn process_agent_relations(
    repos: &RepositoryRegistry,
    agent: &AgentDefinition,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    let Some(agent_id) = slug_to_id.get(&agent.frontmatter.name) else {
        return;
    };

    for relation in &agent.sections.related_agents {
        process_single_relation(
            repos,
            agent_id,
            &agent.frontmatter.name,
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

/// Process a single agent relation
async fn process_single_relation(
    repos: &RepositoryRegistry,
    agent_id: &str,
    agent_name: &str,
    relation: &RelatedAgent,
    slug_to_id: &HashMap<String, String>,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    let Some(related_id) = slug_to_id.get(&relation.slug) else {
        debug!(
            "  Skipping relation {} -> {} (target not found)",
            agent_name, relation.slug
        );
        return;
    };

    if dry_run {
        log_dry_run_relation(agent_name, relation.relation_type, &relation.slug);
        return;
    }

    let relation_created = create_relation(repos, agent_id, related_id, relation.relation_type)
        .await
        .unwrap_or(false);
    if relation_created {
        stats.relations_created += 1;
    }
}

/// Log a relation that would be created in dry run mode
fn log_dry_run_relation(agent_name: &str, relation_type: RelationType, target_slug: &str) {
    info!(
        "  Would create: {} --[{}]--> {}",
        agent_name,
        format!("{relation_type:?}").to_lowercase(),
        target_slug
    );
}

/// Print final summary
fn print_summary(stats: &SeedStats, dry_run: bool) {
    info!("");
    info!("=== Seeding Complete ===");
    log_agent_counts(stats);
    print_errors(&stats.errors);
    log_dry_run_status(dry_run);
}

/// Log the agent processing counts
fn log_agent_counts(stats: &SeedStats) {
    info!(
        "Processed: {} coaches ({} created, {} updated, {} unchanged)",
        stats.total_processed(),
        stats.created,
        stats.updated,
        stats.unchanged
    );
    log_pass_counts(stats);
}

/// Log the counts of the passes that ran after the upsert, each only when
/// it did something.
fn log_pass_counts(stats: &SeedStats) {
    if stats.store_published > 0 {
        info!("Published: {} coaches to store", stats.store_published);
    }
    if stats.packages_synced > 0 {
        info!(
            "Packages: {} coaches carry {} artefact(s)",
            stats.packages_synced, stats.artefacts_synced
        );
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

/// Publish seeded agents to the store (Pass 4)
///
/// System agents are auto-published so they appear in the Discover tab.
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

    for (slug, agent_id) in slug_to_id {
        publish_or_skip(repos, slug, agent_id, admin, stats, dry_run).await;
    }
}

/// Publish one agent or log skip in dry-run mode
async fn publish_or_skip(
    repos: &RepositoryRegistry,
    slug: &str,
    agent_id: &str,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    if dry_run {
        info!("  Would publish: {slug}");
        stats.store_published += 1;
        return;
    }

    let result = publish_single_agent(repos, agent_id, admin).await;
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

/// Delete catalogue-owned agents whose markdown directory is gone (Pass 5).
///
/// The passes above only ever add or rewrite rows, so an agent retired from
/// dravr-contremaitre used to keep its row — and its store listing — in every
/// database forever. This pass diffs the tenant's catalogue-owned rows against
/// the slugs discovered on disk and deletes the rest through the same
/// repository method the admin console uses. The store listing, relations,
/// translations and assignments follow by cascade; an athlete's installed
/// copy keeps working because its `forked_from` pointer is set to NULL rather
/// than deleted.
///
/// Live references go first. A discovered agent that names the retired slug
/// in its `replaces` frontmatter is the successor: the retired agent's
/// conversations, groups and agent pointers are handed to it, so an athlete
/// mid-conversation with a merged agent continues with the agent that
/// absorbed it. Without a successor the conversations are detached and drop
/// to the default prompt; a group still bound to such an agent blocks the
/// delete, the error is counted, and the seed job exits non-zero so an
/// operator picks an agent for it.
///
/// The pass is skipped when any agent file failed to parse: an agent that is
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
    let keep = discovered_slugs(&discovery.agents);
    let successors = successor_ids(&discovery.agents, slug_to_id);
    for (agent_id, slug) in rows
        .iter()
        .filter(|(_, slug)| !keep.contains(slug.as_str()))
    {
        let successor = successors.get(slug.as_str()).copied();
        retire_agent(repos, agent_id, slug, successor, admin, stats, dry_run).await;
    }
    log_pruned(stats.pruned);
}

/// The slugs present in the checkout — the roster every catalogue row must be on.
fn discovered_slugs(discovered: &[AgentWithTranslations]) -> HashSet<&str> {
    discovered
        .iter()
        .map(|c| c.canonical.frontmatter.name.as_str())
        .collect()
}

/// Map each retired slug to the discovered agent whose `replaces` names it.
///
/// Both halves of the successor's identity are carried: the slug-keyed
/// tables cannot be re-pointed from an id, and the id-keyed ones cannot be
/// re-pointed from a slug. A successor that failed its own upsert has no id
/// and is left out, so the agents it would have absorbed fall back to
/// detaching.
fn successor_ids<'a>(
    discovered: &'a [AgentWithTranslations],
    slug_to_id: &'a HashMap<String, String>,
) -> HashMap<&'a str, Successor<'a>> {
    let mut out = HashMap::new();
    for agent in discovered {
        let slug = agent.canonical.frontmatter.name.as_str();
        let Some(id) = slug_to_id.get(slug) else {
            continue;
        };
        for retired in &agent.canonical.frontmatter.replaces {
            out.insert(retired.as_str(), Successor { id, slug });
        }
    }
    out
}

/// The agent a retired slug hands its athletes to, by both of its names.
#[derive(Clone, Copy)]
struct Successor<'a> {
    id: &'a str,
    slug: &'a str,
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
        .seed_list_catalogue_agents(&admin.tenant_id.to_string())
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

/// Hand a retired agent's live references over, then delete it — or log what
/// dry-run would delete.
async fn retire_agent(
    repos: &RepositoryRegistry,
    agent_id: &str,
    slug: &str,
    successor: Option<Successor<'_>>,
    admin: &AdminUser,
    stats: &mut SeedStats,
    dry_run: bool,
) {
    if dry_run {
        info!("  Would delete: {slug} (no longer in the catalogue)");
        stats.pruned += 1;
        return;
    }

    if let Err(e) = hand_over_references(repos, agent_id, slug, successor).await {
        warn!("  ✗ {slug} - Hand-over error: {e}");
        stats.errors.push(format!("{slug}: hand-over failed: {e}"));
        return;
    }
    let result = repos
        .agents
        .delete_system_agent(agent_id, admin.tenant_id)
        .await;
    log_delete_result(slug, result, stats);
}

/// Re-point the retired agent's athlete-side references at the successor, or
/// detach its conversations when there is none.
///
/// Both keys are re-pointed: the tables that name the agent by id, and the
/// playbooks, pending advice and training plans that name it by slug.
async fn hand_over_references(
    repos: &RepositoryRegistry,
    agent_id: &str,
    slug: &str,
    successor: Option<Successor<'_>>,
) -> AppResult<()> {
    if let Some(successor) = successor {
        let moved = repos
            .seeder
            .seed_repoint_agent_references(agent_id, successor.id)
            .await?
            + repos
                .seeder
                .seed_repoint_agent_slug_references(slug, successor.slug)
                .await?;
        if moved > 0 {
            info!("    → {moved} reference(s) handed to the successor");
        }
    } else {
        let detached = repos
            .seeder
            .seed_detach_agent_conversations(agent_id)
            .await?;
        if detached > 0 {
            info!("    → {detached} conversation(s) detached, no successor declared");
        }
    }
    Ok(())
}

/// Log and record the result of one retired-agent deletion
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

/// Log how many retired agents were pruned
fn log_pruned(count: u32) {
    if count > 0 {
        info!("  Pruned {} retired coach(es)", count);
    }
}

/// Publish a single agent to the store, returning true if newly published
async fn publish_single_agent(
    repos: &RepositoryRegistry,
    agent_id: &str,
    admin: &AdminUser,
) -> AppResult<bool> {
    let listing = SeedStoreListing {
        id: Uuid::new_v4().to_string(),
        agent_id: agent_id.to_owned(),
        tenant_id: admin.tenant_id,
        author_id: admin.agent_author_id.clone(),
        created_at: Utc::now(),
    };

    repos
        .seeder
        .seed_insert_store_listing_if_absent(&listing)
        .await
}

/// Discover canonical agent definitions + their non-canonical translations.
///
/// Expected layout: `agents_dir/<category>/<slug>/<locale>.md` where `<locale>`
/// satisfies [`pierre_agent_parser::is_locale_code`] — the predicate this
/// walk actually calls, which reads the platform's one locale list. Each agent directory MUST
/// contain `en.md` (the canonical source); optional `fr.md` / `es.md` / `de.md`
/// / `pt.md` siblings become [`AgentTranslationFile`] rows that layer over the
/// canonical copy at read time via
/// [`pierre_database::AgentsRepository::apply_translations`].
///
/// Directories without `en.md` are skipped with a warning — the seeder never
/// stores an agent whose canonical English content is missing.
fn discover_agents(agents_dir: &Path) -> AppResult<Discovery> {
    let (canonical, translations, parse_failures) = scan_agent_files(agents_dir)?;
    Ok(Discovery {
        agents: pivot_and_sort(canonical, translations),
        parse_failures,
    })
}

/// What the checkout scan found: the agents, plus how many files the parser refused.
///
/// The prune pass reads the count — a refused file is an agent it cannot see,
/// not an agent that left the catalogue.
pub(crate) struct Discovery {
    /// Every agent with a parseable `en.md`, sorted by category then slug.
    pub agents: Vec<AgentWithTranslations>,
    /// Locale files that failed to parse and were skipped with a warning.
    pub parse_failures: usize,
}

/// Snapshot of the filesystem scan: canonical agents keyed by slug, the
/// translation files keyed by the same slug, and how many files failed to parse.
type ScannedAgents = (
    HashMap<String, AgentDefinition>,
    HashMap<String, Vec<AgentTranslationFile>>,
    usize,
);

/// Walk `agents_dir/<category>/<slug>/*.md` and bucket by slug into two maps.
///
/// Filters out non-locale filenames (README.md, stray `*.md` files) up-front
/// so the caller only ever sees valid per-locale entries. Parsing errors are
/// logged and skipped — one malformed file never blocks the whole seed.
fn scan_agent_files(agents_dir: &Path) -> AppResult<ScannedAgents> {
    let pattern = agents_dir.join("*/*/*.md");
    let pattern_str = pattern.to_string_lossy();

    let mut canonical: HashMap<String, AgentDefinition> = HashMap::new();
    let mut translations: HashMap<String, Vec<AgentTranslationFile>> = HashMap::new();
    let mut parse_failures = 0usize;

    for entry in
        glob(&pattern_str).map_err(|e| AppError::internal(format!("Glob pattern error: {e}")))?
    {
        let path = entry.map_err(|e| AppError::internal(format!("Glob error: {e}")))?;
        if !record_agent_path(&path, &mut canonical, &mut translations) {
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
fn record_agent_path(
    path: &Path,
    canonical: &mut HashMap<String, AgentDefinition>,
    translations: &mut HashMap<String, Vec<AgentTranslationFile>>,
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
    let agent = match parse_agent_file(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse {}: {}", path.display(), e);
            return false;
        }
    };
    let slug = agent.frontmatter.name.clone();
    if stem == CANONICAL_LOCALE {
        canonical.insert(slug, agent);
    } else {
        let canonical_sha = AgentDefinition::source_sha_prefix(&agent.content_hash);
        translations
            .entry(slug)
            .or_default()
            .push(AgentTranslationFile {
                locale: stem.to_owned(),
                agent,
                source_sha_hint: canonical_sha,
            });
    }
    true
}

/// Pivot the two scan maps into a sorted `Vec<AgentWithTranslations>`.
///
/// Coaches without an `en.md` are dropped with a warning so we never store
/// orphan translations. Output is sorted by category then slug for
/// deterministic test output and log stability.
fn pivot_and_sort(
    canonical: HashMap<String, AgentDefinition>,
    mut translations: HashMap<String, Vec<AgentTranslationFile>>,
) -> Vec<AgentWithTranslations> {
    let mut out: Vec<AgentWithTranslations> = Vec::with_capacity(canonical.len());
    for (slug, canon) in canonical {
        let trs = translations.remove(&slug).unwrap_or_default();
        out.push(AgentWithTranslations {
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

/// Canonical agent + optional locale translation siblings discovered together.
/// Produced by [`discover_agents`] and consumed by the canonical + translation
/// seeder passes so the two always see a consistent view of what's on disk.
pub(crate) struct AgentWithTranslations {
    /// English source of truth.
    pub canonical: AgentDefinition,
    /// Non-English siblings: one [`AgentTranslationFile`] per `<locale>.md`.
    pub translations: Vec<AgentTranslationFile>,
}

/// A single `<slug>/<locale>.md` translation file (non-canonical).
pub(crate) struct AgentTranslationFile {
    /// BCP-47 short locale code captured from the filename stem.
    pub locale: String,
    /// Parsed `AgentDefinition` view of the translated file — only
    /// `title`/`description`/`purpose`/`instructions` will be overlaid
    /// onto the canonical row; the rest is ignored at upsert time.
    pub agent: AgentDefinition,
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
    /// Agent author profile ID (from `agent_authors` table, used as `store_listings.author_id`)
    agent_author_id: String,
}

/// Find the first admin user and their tenant, ensuring an `agent_authors` row exists
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

    // Ensure an agent_authors row exists for the admin (required by store_listings FK)
    let now = Utc::now();
    let agent_author = SeedAgentAuthor {
        id: Uuid::new_v4().to_string(),
        user_id: user.id,
        tenant_id: tenant_id.to_string(),
        display_name: user.email.clone(),
        created_at: now,
        updated_at: now,
    };
    let agent_author_id = repos.seeder.seed_upsert_agent_author(&agent_author).await?;
    info!("Coach author profile: {agent_author_id}");

    Ok(AdminUser {
        id: user.id,
        email: user.email,
        tenant_id,
        agent_author_id,
    })
}

/// Result of upsert operation
enum UpsertAction {
    Created,
    Updated,
    Unchanged,
}

/// Upsert an agent into the database
async fn upsert_agent(
    repos: &RepositoryRegistry,
    agent: &AgentDefinition,
    admin: &AdminUser,
    dry_run: bool,
) -> AppResult<(String, UpsertAction)> {
    let now = Utc::now();
    let slug = &agent.frontmatter.name;

    // Check if agent exists by slug
    let existing = repos
        .seeder
        .seed_find_agent_by_slug(slug, &admin.tenant_id.to_string())
        .await?;

    let action = if let Some((existing_id, existing_hash)) = existing {
        // Agent exists - check if content changed
        if existing_hash.as_deref() == Some(&agent.content_hash) {
            return Ok((existing_id, UpsertAction::Unchanged));
        }

        if !dry_run {
            let seed_agent = build_seed_agent(&existing_id, agent, admin, now)?;
            repos.seeder.seed_update_agent(&seed_agent).await?;
        }
        (existing_id, UpsertAction::Updated)
    } else {
        // New agent
        let new_id = Uuid::new_v4().to_string();

        if !dry_run {
            let seed_agent = build_seed_agent(&new_id, agent, admin, now)?;
            repos.seeder.seed_insert_agent(&seed_agent).await?;
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

/// Build a `SeedAgent` from a parsed `AgentDefinition` and admin context
fn build_seed_agent(
    id: &str,
    agent: &AgentDefinition,
    admin: &AdminUser,
    now: DateTime<Utc>,
) -> AppResult<SeedAgent> {
    let prerequisites_json = serde_json::to_string(&agent.frontmatter.prerequisites)
        .map_err(|e| AppError::internal(format!("JSON error: {e}")))?;
    let tags_json = serde_json::to_string(&agent.frontmatter.tags)
        .map_err(|e| AppError::internal(format!("JSON error: {e}")))?;
    let sample_prompts_json = parse_sample_prompts(agent.sections.example_inputs.as_ref());

    Ok(SeedAgent {
        id: id.to_owned(),
        user_id: admin.id,
        tenant_id: admin.tenant_id,
        title: agent.frontmatter.title.clone(),
        description: agent.sections.purpose.clone(),
        system_prompt: agent.sections.instructions.clone(),
        category: agent.frontmatter.category.as_str().to_owned(),
        tags_json,
        sample_prompts_json,
        token_count: i64::from(agent.token_count),
        visibility: agent.frontmatter.visibility.as_str().to_owned(),
        slug: agent.frontmatter.name.clone(),
        purpose: Some(agent.sections.purpose.clone()),
        when_to_use: agent.sections.when_to_use.clone(),
        instructions: Some(agent.sections.instructions.clone()),
        example_inputs: agent.sections.example_inputs.clone(),
        example_outputs: agent.sections.example_outputs.clone(),
        success_criteria: agent.sections.success_criteria.clone(),
        prerequisites_json,
        source_file: Some(agent.source_file.clone()),
        content_hash: Some(agent.content_hash.clone()),
        startup_query: agent.frontmatter.startup.query.clone(),
        data_requirements: agent
            .frontmatter
            .startup
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok()),
        visuals: visuals_column(&agent.frontmatter.startup.visuals),
        created_at: now,
        updated_at: now,
    })
}

/// Create a relation between two agents
async fn create_relation(
    repos: &RepositoryRegistry,
    agent_id: &str,
    related_id: &str,
    relation_type: RelationType,
) -> AppResult<bool> {
    let relation_str = match relation_type {
        RelationType::Related => "related",
        RelationType::Alternative => "alternative",
        RelationType::Prerequisite => "prerequisite",
        RelationType::Sequel => "sequel",
    };

    let relation = SeedAgentRelation {
        id: Uuid::new_v4().to_string(),
        agent_id: agent_id.to_owned(),
        related_agent_id: related_id.to_owned(),
        relation_type: relation_str.to_owned(),
        created_at: Utc::now(),
    };

    repos
        .seeder
        .seed_insert_agent_relation_if_absent(&relation)
        .await
}

/// Join an agent's visual grants into the stored column form.
///
/// `None` rather than an empty string when there are no grants, so the column
/// reads as "no grant" instead of "granted nothing" — the two are the same to
/// the runtime, but NULL keeps the seeded rows honest about intent.
fn visuals_column(visuals: &[pierre_agent_parser::VisualKind]) -> Option<String> {
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
