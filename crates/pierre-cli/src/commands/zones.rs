// ABOUTME: pierre-cli zones subcommands — re-cut stored profile zones with cageux's one zone model
// ABOUTME: Wraps pierre-services' zone backfill in clap: dry by default, --apply writes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Zone operator commands.
//!
//! `zones backfill` re-cuts the heart-rate and power zones stored on a
//! tenant's physiological profiles from each profile's own inputs, so rows
//! written by the platform's former zone tables read as cageux's one zone
//! model without waiting for the athlete's next save.

use clap::Subcommand;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_services::zone_backfill::run_zone_backfill;

/// `pierre-cli zones <subcommand>` dispatch.
#[derive(Subcommand)]
pub enum ZonesCommand {
    /// Re-cut the zones stored on every physiological profile in a tenant.
    ///
    /// Each profile's heart-rate zones are cut again from its resting and
    /// maximum heart rate and `VO2max`, its power zones from its FTP — the
    /// same cut `set_physiology` makes. A profile already holding them is
    /// left alone, so a second run writes nothing. Reports and changes
    /// nothing unless `--apply` is given.
    Backfill {
        /// Tenant whose profiles are re-cut (UUID string).
        #[arg(long)]
        tenant_id: String,

        /// Write the re-cut zones. Without it the run only reports them.
        #[arg(long)]
        apply: bool,
    },
}

/// Dispatch a `pierre-cli zones ...` subcommand.
///
/// # Errors
///
/// Returns an error when the tenant id is malformed or a profile cannot be
/// read or written.
pub async fn dispatch(command: ZonesCommand, repos: &RepositoryRegistry) -> AppResult<()> {
    match command {
        ZonesCommand::Backfill { tenant_id, apply } => {
            let tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            let stats =
                run_zone_backfill(repos.user_physiological_profile.as_ref(), tenant, !apply)
                    .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&stats).map_err(|e| AppError::internal(format!(
                    "Failed to serialize zone backfill stats: {e}"
                )))?
            );
            if !apply && stats.rewritten > 0 {
                println!("(dry run — pass --apply to write)");
            }
        }
    }
    Ok(())
}
