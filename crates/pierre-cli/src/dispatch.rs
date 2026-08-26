// ABOUTME: Dispatch for the pierre-cli subcommands that speak HTTP, not DATABASE_URL
// ABOUTME: Taken before the KeyManager/DB bootstrap so they work against any environment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The remote half of the CLI.
//!
//! Every command routed from here carries a `--server` / `--token` pair and a
//! cached device login, and holds no repository handle — which is what lets one
//! binary serve a laptop and a deployed environment alike. `main` takes these
//! arms before it bootstraps the KeyManager, because a command that bootstraps
//! can only ever read the local database.

use pierre_core::errors::{AppError, AppResult};

use crate::commands;
use crate::{AuthCommand, Result, StravaPoolCommand, UserCommand};

/// Dispatch the remote user commands, which never touch the local database.
///
/// # Errors
///
/// Returns the command's own error; an unreachable arm would mean the caller
/// routed a database-backed variant here, which the match at the call site
/// prevents.
pub(crate) async fn dispatch_remote_user(action: UserCommand) -> AppResult<()> {
    match action {
        UserCommand::Get {
            selector,
            status,
            tier,
            limit,
            all,
            format,
            server,
            token,
        } => {
            let client = commands::auth::admin_client(server, token)?;
            if let Some(sel) = selector {
                let id = commands::user_admin::resolve_user_id(&client, &sel).await?;
                commands::user_admin::get_user(&client, &id).await
            } else {
                let args = commands::user_admin::GetUsersArgs {
                    status,
                    tier,
                    limit,
                    all,
                    format: commands::user_admin::OutputFormat::parse(&format)?,
                };
                commands::user_admin::get_users(&client, &args).await
            }
        }
        UserCommand::Set {
            selector,
            tier,
            note,
            server,
            token,
        } => {
            let client = commands::auth::admin_client(server, token)?;
            let id = commands::user_admin::resolve_user_id(&client, &selector).await?;
            commands::user_admin::set_user_tier(&client, &id, &tier, note.as_deref()).await
        }
        UserCommand::Allow {
            email,
            note,
            server,
            token,
        } => {
            let client = commands::auth::admin_client(server, token)?;
            commands::user_admin::allow_email(&client, &email, note.as_deref()).await
        }
        UserCommand::Disallow {
            email,
            server,
            token,
        } => {
            let client = commands::auth::admin_client(server, token)?;
            commands::user_admin::disallow_email(&client, &email).await
        }
        UserCommand::ListAllowed {
            format,
            server,
            token,
        } => {
            let client = commands::auth::admin_client(server, token)?;
            let format = commands::user_admin::OutputFormat::parse(&format)?;
            commands::user_admin::list_allowed(&client, format).await
        }
        _ => Err(AppError::internal(
            "dispatch_remote_user received a database-backed user command",
        )),
    }
}

pub(crate) async fn dispatch_auth(action: AuthCommand) -> Result<()> {
    match action {
        AuthCommand::Login { server, no_browser } => {
            commands::auth::login(server, no_browser).await
        }
        AuthCommand::Logout => commands::auth::logout(),
        AuthCommand::Status => commands::auth::status(),
        AuthCommand::Approve {
            user_code,
            server,
            token,
        } => commands::auth::resolve(user_code, server, token, false).await,
        AuthCommand::Deny {
            user_code,
            server,
            token,
        } => commands::auth::resolve(user_code, server, token, true).await,
    }
}

pub(crate) async fn dispatch_strava_pool(action: StravaPoolCommand) -> Result<()> {
    match action {
        StravaPoolCommand::Add {
            client_id,
            client_secret,
            seat_cap,
            label,
        } => commands::strava_pool::add(client_id, client_secret, seat_cap, label).await,
        StravaPoolCommand::List => commands::strava_pool::list().await,
        StravaPoolCommand::Enable { client_id } => {
            commands::strava_pool::set_enabled(client_id, true).await
        }
        StravaPoolCommand::Disable { client_id } => {
            commands::strava_pool::set_enabled(client_id, false).await
        }
        StravaPoolCommand::Delete { client_id } => commands::strava_pool::delete(client_id).await,
    }
}
