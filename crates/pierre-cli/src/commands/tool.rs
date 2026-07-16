// ABOUTME: Per-user tool allow/deny commands for pierre-cli
// ABOUTME: Force-enable/disable individual MCP tools for one user (user_tool_overrides)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops;
use uuid::Uuid;

type Result<T> = AppResult<T>;

/// Force-enable a specific MCP tool for a user (overrides plan + tenant gating).
pub async fn enable(
    repos: &RepositoryRegistry,
    email: String,
    tool: String,
    reason: Option<String>,
) -> Result<()> {
    set(repos, email, tool, true, reason).await
}

/// Force-disable a specific MCP tool for a user.
pub async fn disable(
    repos: &RepositoryRegistry,
    email: String,
    tool: String,
    reason: Option<String>,
) -> Result<()> {
    set(repos, email, tool, false, reason).await
}

async fn set(
    repos: &RepositoryRegistry,
    email: String,
    tool: String,
    is_enabled: bool,
    reason: Option<String>,
) -> Result<()> {
    let user_id = lookup_user(repos, &email).await?;
    let reason = reason.unwrap_or_else(|| "set via pierre-cli".to_owned());
    admin_ops::set_user_tool_override(repos, user_id, &tool, is_enabled, None, Some(reason))
        .await?;
    let verb = if is_enabled { "enabled" } else { "disabled" };
    println!(
        "Success: tool '{tool}' {verb} for {email} (per-user override; effective immediately)"
    );
    Ok(())
}

/// Remove a per-user tool override so the tool reverts to plan/tenant/default.
pub async fn reset(repos: &RepositoryRegistry, email: String, tool: String) -> Result<()> {
    let user_id = lookup_user(repos, &email).await?;
    let removed = admin_ops::remove_user_tool_override(repos, user_id, &tool).await?;
    if removed {
        println!("Success: cleared per-user override for '{tool}' on {email}");
    } else {
        println!("No per-user override existed for '{tool}' on {email} (nothing to clear)");
    }
    Ok(())
}

/// List a user's explicit per-user tool overrides.
pub async fn list(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user_id = lookup_user(repos, &email).await?;
    let overrides = admin_ops::list_user_tool_overrides(repos, user_id).await?;

    if overrides.is_empty() {
        println!("No per-user tool overrides for {email}");
        return Ok(());
    }

    println!(
        "Per-user tool overrides for {email} ({} total):",
        overrides.len()
    );
    println!("{:<32}  {:<9}  REASON", "TOOL", "STATE");
    for o in overrides {
        let state = if o.is_enabled { "enabled" } else { "disabled" };
        println!(
            "{:<32}  {:<9}  {}",
            o.tool_name,
            state,
            o.reason.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

async fn lookup_user(repos: &RepositoryRegistry, email: &str) -> Result<Uuid> {
    repos
        .users
        .get_by_email(email)
        .await?
        .map(|u| u.id)
        .ok_or_else(|| AppError::not_found(format!("User with email {email} not found")))
}
