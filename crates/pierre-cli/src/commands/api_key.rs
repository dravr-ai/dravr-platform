// ABOUTME: API key commands for pierre-cli — a user's keys, and the tools one key called over a window
// ABOUTME: Reads the same repositories GET /api/keys and GET /api/keys/{id}/usage read, against DATABASE_URL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::RepositoryRegistry;

use super::tool::lookup_user;

type Result<T> = AppResult<T>;

/// List every API key a user owns, revoked ones included and marked.
pub(crate) async fn list(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user_id = lookup_user(repos, &email).await?;
    let keys = repos.api_keys.get_for_user(user_id).await?;

    if keys.is_empty() {
        println!("No API keys for {email}");
        return Ok(());
    }

    println!("API keys for {email} ({} total):", keys.len());
    println!(
        "{:<38}  {:<24}  {:<16}  {:<7}  LAST USED",
        "ID", "NAME", "PREFIX", "STATE"
    );
    for key in keys {
        let state = if key.is_active { "active" } else { "revoked" };
        let last_used = key.last_used_at.map_or_else(
            || "never".to_owned(),
            |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
        );
        println!(
            "{:<38}  {:<24}  {:<16}  {:<7}  {last_used}",
            key.id, key.name, key.key_prefix, state
        );
    }
    Ok(())
}

/// Print one key's request totals and the tools it called over the last `days`, most-called first.
///
/// The key must belong to `email`: the same ownership the usage route
/// enforces, so a mistyped key id names no one else's traffic.
pub(crate) async fn usage(
    repos: &RepositoryRegistry,
    email: String,
    key_id: String,
    days: u32,
) -> Result<()> {
    let user_id = lookup_user(repos, &email).await?;
    let keys = repos.api_keys.get_for_user(user_id).await?;
    let key = keys
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| AppError::not_found(format!("API key {key_id} for {email}")))?;

    if days == 0 {
        return Err(AppError::invalid_input("--days must be at least 1"));
    }
    let end = Utc::now();
    let start = Duration::try_days(i64::from(days))
        .and_then(|window| end.checked_sub_signed(window))
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "--days {days} reaches past the earliest representable date"
            ))
        })?;
    let stats = repos.usage.get_api_key_stats(&key.id, start, end).await?;

    println!(
        "{} ({}) — last {days} days: {} requests, {} succeeded, {} failed",
        key.name,
        key.key_prefix,
        stats.total_requests,
        stats.successful_requests,
        stats.failed_requests
    );
    if stats.tools.is_empty() {
        println!("No tool calls in the window");
        return Ok(());
    }
    println!(
        "{:<40}  {:>8}  {:>8}  {:>9}",
        "TOOL", "CALLS", "SUCCESS", "AVG MS"
    );
    for tool in stats.tools {
        println!(
            "{:<40}  {:>8}  {:>7.1}%  {:>9.0}",
            tool.tool_name, tool.request_count, tool.success_rate, tool.average_response_time
        );
    }
    Ok(())
}
