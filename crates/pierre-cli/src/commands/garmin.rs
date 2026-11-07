// ABOUTME: pierre-cli garmin subcommands — retire the usr/pwd sessions a Garmin grant left serving nothing
// ABOUTME: And count the athletes still on usr/pwd alone against the seats left on Garmin's OAuth client

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin operator commands (carnet#1386).
//!
//! `garmin retire-history-sessions` is the one-off for athletes who already
//! held a Garmin grant beside a usr/pwd session before sign-ins became a
//! one-time history import: the session serves nothing, and retiring it
//! drops the session, its token row and its connection row, purging nothing.
//! Each athlete's cached activities are counted by key before and after, so
//! the operator sees that no row went with the session.
//!
//! `garmin migration-report` counts the athletes of a tenant still connected
//! by usr/pwd alone, with no Garmin grant (carnet#1454), beside the seats
//! left on the server's Garmin client: how many of them could move to
//! Garmin's official connection today. It reads, and changes nothing.

use clap::Subcommand;
use dravr_sciotte::client::SciotteClient;
use pierre_auth::config::oauth::OAuthConfig;
use pierre_auth::garmin_seats::garmin_seat_summary;
use pierre_core::constants::oauth_providers::{GARMIN, SCIOTTE_GARMIN};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_services::history_import;
use tracing::warn;

/// `pierre-cli garmin <subcommand>` dispatch.
#[derive(Subcommand)]
pub(crate) enum GarminCommand {
    /// Retire every usr/pwd Garmin session that stands beside a Garmin grant
    /// in a tenant, keeping every row it copied.
    ///
    /// Drops each session from the scraper service (when
    /// `DRAVR_SCIOTTE_REMOTE_URL` is set; otherwise it idles out there),
    /// then deletes its token and connection rows. Never purges data. A
    /// session whose grant withholds the health export is kept: it is still
    /// the athlete's only wellness source.
    RetireHistorySessions {
        /// Tenant whose athletes are retired (UUID string).
        #[arg(long)]
        tenant_id: String,

        /// List the athletes and their cached activities by key, retiring
        /// nothing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Count the athletes of a tenant connected to Garmin by usr/pwd alone,
    /// with no Garmin grant, beside the seats left on the server's Garmin
    /// client. Reads only.
    MigrationReport {
        /// Tenant whose athletes are counted (UUID string).
        #[arg(long)]
        tenant_id: String,
    },
}

/// Dispatch a `pierre-cli garmin ...` subcommand.
///
/// # Errors
///
/// Returns an error when the tenant id is malformed, or when the finder, a
/// count or a retirement fails.
pub(crate) async fn dispatch(command: GarminCommand, repos: &RepositoryRegistry) -> AppResult<()> {
    match command {
        GarminCommand::RetireHistorySessions { tenant_id, dry_run } => {
            let tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            retire_history_sessions(repos, tenant, dry_run).await
        }
        GarminCommand::MigrationReport { tenant_id } => {
            let tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            migration_report(repos, &OAuthConfig::from_env(), tenant).await
        }
    }
}

/// Print how many athletes of `tenant` hold a usr/pwd Garmin session with no
/// Garmin grant beside it, and the seats left on the server's Garmin client
/// under `oauth`.
///
/// A seat count that fails, or a server with no Garmin client, is printed as
/// such: the athlete count stands on its own.
async fn migration_report(
    repos: &RepositoryRegistry,
    oauth: &OAuthConfig,
    tenant: TenantId,
) -> AppResult<()> {
    let athletes = repos
        .provider_connections
        .list_mirror_only_users(tenant, SCIOTTE_GARMIN, GARMIN)
        .await?;
    println!(
        "{} athlete(s) in tenant {tenant} hold a {SCIOTTE_GARMIN} session with no {GARMIN} grant",
        athletes.len()
    );
    for user in &athletes {
        println!("  {user}");
    }
    match garmin_seat_summary(repos.provider_connections.as_ref(), oauth).await {
        Ok(seats) => println!(
            "Garmin OAuth seats: {} left of {} ({} used, across every tenant)",
            seats.left(),
            seats.total,
            seats.used
        ),
        Err(e) => println!("Garmin OAuth seats could not be counted: {e}"),
    }
    Ok(())
}

/// List, and unless `dry_run` retire, the sessions beside a Garmin grant in
/// `tenant`, printing each athlete's cached activities by key before and
/// after.
async fn retire_history_sessions(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    dry_run: bool,
) -> AppResult<()> {
    let remote = if dry_run {
        None
    } else {
        SciotteClient::require_from_env()
            .inspect_err(|e| {
                warn!(error = %e, "No scraper service configured; each session idles out there instead");
            })
            .ok()
    };
    let reports =
        history_import::retire_history_sessions(repos, remote.as_ref(), tenant, dry_run).await?;
    println!(
        "{} athlete(s) in tenant {tenant} hold a {SCIOTTE_GARMIN} session beside a {GARMIN} grant",
        reports.len()
    );
    for report in &reports {
        let user = report.user_id;
        println!("  {user} before: {}", counts(&report.before));
        if report.kept_for_wellness {
            println!("  {user} kept: the grant withholds the health export");
        }
        if let Some(after) = &report.after {
            println!("  {user} after:  {}", counts(after));
        }
    }
    if dry_run {
        println!("(dry run — run without --dry-run to retire)");
    }
    Ok(())
}

/// Cached activities by key, as `key=count` pairs.
fn counts(counts: &[(String, i64)]) -> String {
    if counts.is_empty() {
        return "no cached activities".to_owned();
    }
    counts
        .iter()
        .map(|(key, count)| format!("{key}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}
