// ABOUTME: `pierre-cli canary` commands — the credential-login canary the scheduled Cloud Run job runs
// ABOUTME: Reads the canary account from the environment, prints one JSON verdict line, fails the job on a stopped login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli canary sciotte-login --provider garmin`.
//!
//! Runs in the `*-sciotte-login-canary-job` Cloud Run job on a schedule. The
//! account comes from `SCIOTTE_CANARY_EMAIL` / `SCIOTTE_CANARY_PASSWORD`
//! (Secret Manager), never from arguments, and the service from the same
//! `DRAVR_SCIOTTE_REMOTE_URL` / `DRAVR_SCIOTTE_AUDIENCE` the API reads. A login
//! the provider stopped logs at ERROR and fails the job, which is what the
//! `dravr-mcp-server-job-failures` alert counts.

use std::env;

use clap::Subcommand;
use dravr_sciotte::client::SciotteClient;
use pierre_cli::canary::{run_login_canary, CanaryVerdict};
use pierre_core::errors::{AppError, AppResult};
use pierre_middleware::redaction::mask_email;
use tracing::{error, info};

const EMAIL_ENV: &str = "SCIOTTE_CANARY_EMAIL";
const PASSWORD_ENV: &str = "SCIOTTE_CANARY_PASSWORD";

/// `pierre-cli canary <check>`.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum CanaryCommand {
    /// Sign the canary account in through the deployed sciotte service and
    /// report whether the provider let the login through.
    SciotteLogin {
        /// The provider to sign in to, as the sciotte service names it.
        #[arg(long, default_value = "garmin")]
        provider: String,
    },
}

/// Run a canary command.
///
/// # Errors
///
/// Returns an error when the canary account or the service is not
/// configured, when the provider stopped the login, or when the service could
/// not be asked — each fails the job.
pub async fn dispatch(action: CanaryCommand) -> AppResult<()> {
    match action {
        CanaryCommand::SciotteLogin { provider } => sciotte_login(&provider).await,
    }
}

async fn sciotte_login(provider: &str) -> AppResult<()> {
    let email = required_env(EMAIL_ENV)?;
    let password = required_env(PASSWORD_ENV)?;
    let client = SciotteClient::require_from_env()
        .map_err(|e| AppError::config(format!("sciotte service is not configured: {e}")))?;

    info!(provider, account = %mask_email(&email), "Starting the credential-login canary");
    let verdict = run_login_canary(&client, provider, &email, &password).await;
    println!("{}", verdict.to_json(provider));
    report(provider, verdict)
}

/// Log the verdict and turn a stopped or unasked login into the job's failure.
fn report(provider: &str, verdict: CanaryVerdict) -> AppResult<()> {
    match verdict {
        CanaryVerdict::Pass { step } => {
            info!(provider, step, "Credential-login canary passed");
            Ok(())
        }
        CanaryVerdict::Fail { reason, detail } => {
            error!(
                provider,
                reason = reason.as_str(),
                detail,
                "Credential-login canary: the provider stopped the login"
            );
            Err(AppError::external_service(
                provider,
                format!("canary login stopped by the provider ({})", reason.as_str()),
            ))
        }
        CanaryVerdict::Infra { detail } => {
            error!(
                provider,
                detail, "Credential-login canary could not reach the sciotte service"
            );
            Err(AppError::external_service("sciotte", detail))
        }
    }
}

fn required_env(name: &str) -> AppResult<String> {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| AppError::config(format!("{name} is not set")))
}
