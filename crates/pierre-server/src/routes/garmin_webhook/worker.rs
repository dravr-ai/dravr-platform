// ABOUTME: Works the Garmin push inbox — runs each stored batch through the ingest, retries with backoff, sweeps leftovers
// ABOUTME: Also the server side of the ingest: Garmin deregistration, one-shot activity-file downloads, landed notices

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Garmin push inbox worker.
//!
//! A stored batch is run once by the instance that received it, under the
//! lease the route wrote with it. A run that fails, or that has records
//! which must wait (an activity file whose summary has not arrived, a spent
//! download budget), hands the row back with an exponential backoff; the
//! sweep ([`process_garmin_inbox`], every [`SWEEP_INTERVAL`]) claims rows
//! whose lease lapsed, whether handed back or left by an instance that died.
//! After [`MAX_ATTEMPTS`] runs a row is marked `failed`; the backoff spreads
//! those runs over about four hours, inside the 24 hours an activity file's
//! URL lives. Processed rows lose their payload at once and are deleted
//! after [`PROCESSED_RETENTION`].
//!
//! Nothing here asks Garmin for data: the Starter tier allows no pull. The
//! only calls are the one-shot activity-file download a push offers and,
//! through the disconnect chokepoint, nothing at all (a deregistered athlete
//! has no grant left to revoke).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::http_client::{api_client, SharedHttpError};
use pierre_database::repositories::{
    GarminPushInboxRepository, GarminPushInboxRow, InboxClaim, JobLease, PushedDataRepository,
};
use pierre_providers::http_client::read_capped;
use pierre_providers::request_budget::GARMIN_ACTIVITY_FILES;
use pierre_services::garmin_push::{
    ingest_batch, parse_envelope, ActivityFile, BatchOutcome, GarminOwner, GarminPushHost, Retry,
    SummaryType,
};
use pierre_services::oauth_flow::OAuthService;
use pierre_services::periodic::spawn_periodic;
use pierre_services::provider_revocation::{clear_backend, DisconnectReason};
use pierre_tool_runtime::protocol::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use reqwest::{StatusCode, Url};
use secrecy::ExposeSecret;
use tracing::{error, info, warn};

use crate::mcp::resources::ServerContext;
use crate::routes::webhooks::{notify_owner, report_sync, stamp_last_sync};
use crate::services::activity_upload::MAX_UPLOAD_BYTES;

/// How long one run holds a row before the sweep may take it.
const GARMIN_PUSH_LEASE: Duration = Duration::from_mins(10);

/// How often the sweep looks for rows to run and rows to delete.
const SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// Runs a row is offered before it is marked `failed`.
const MAX_ATTEMPTS: i64 = 8;

/// The wait before the second run; each later run waits twice the last.
const FIRST_BACKOFF: Duration = Duration::from_mins(1);

/// Rows one sweep claims.
const SWEEP_BATCH: i64 = 20;

/// How long a processed row's bookkeeping is kept.
const PROCESSED_RETENTION: Duration = Duration::from_hours(24);

/// How long a downloaded activity file is kept waiting for its samples to
/// land: past every retry of its batch (about four hours) with a margin.
const KEPT_FILE_RETENTION: Duration = Duration::from_hours(24);

/// [`GARMIN_PUSH_LEASE`] in milliseconds.
pub(super) fn lease_ms() -> i64 {
    duration_ms(GARMIN_PUSH_LEASE)
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// The wait before the run after `attempts` runs.
fn backoff_ms(attempts: i64) -> i64 {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
    duration_ms(FIRST_BACKOFF).saturating_mul(1_i64 << doublings)
}

/// Run one leased row through the ingest and settle it: `done`, handed back
/// with a backoff, or `failed` after its last attempt. Never fails: every
/// outcome is written to the row or logged.
pub(super) async fn process_push(
    resources: &Arc<ServerContext>,
    row: GarminPushInboxRow,
    lease: JobLease,
) {
    let result = run(resources, &row).await;
    let settled = settle(
        resources.common.repos.garmin_push_inbox.as_ref(),
        &row,
        &lease,
        result,
    )
    .await;
    match settled {
        Ok(true) => {}
        Ok(false) => warn!(
            inbox_id = %row.id,
            "Garmin push: another run took the row while this one ran; its outcome stands"
        ),
        Err(e) => error!(
            inbox_id = %row.id,
            error = %e,
            "Garmin push outcome could not be recorded; the sweep runs the row again"
        ),
    }
}

/// Write a run's outcome to its row.
async fn settle(
    inbox: &dyn GarminPushInboxRepository,
    row: &GarminPushInboxRow,
    lease: &JobLease,
    result: AppResult<BatchOutcome>,
) -> AppResult<bool> {
    let now_ms = Utc::now().timestamp_millis();
    match result {
        Ok(outcome) if outcome.retry.is_none() => {
            info!(
                inbox_id = %row.id,
                summary_type = %row.summary_type,
                stored = outcome.stored,
                skipped = outcome.skipped,
                unowned = outcome.unowned,
                unreadable = outcome.unreadable,
                "Garmin push processed"
            );
            inbox.finish_push(lease, now_ms, None).await
        }
        Ok(BatchOutcome {
            retry: Some(Retry::Wait(why)),
            ..
        }) if row.attempts >= MAX_ATTEMPTS => {
            warn!(
                inbox_id = %row.id,
                summary_type = %row.summary_type,
                reason = %why,
                "Garmin push settled with records still waiting on a push that never came"
            );
            inbox.finish_push(lease, now_ms, Some(&why)).await
        }
        Ok(outcome) => {
            let why = outcome
                .retry
                .as_ref()
                .map(Retry::reason)
                .unwrap_or_default()
                .to_owned();
            hand_back(inbox, lease, row, &why, now_ms).await
        }
        Err(e) => hand_back(inbox, lease, row, &e.to_string(), now_ms).await,
    }
}

/// Hand a row back for a later run, or mark it `failed` after its last.
async fn hand_back(
    inbox: &dyn GarminPushInboxRepository,
    lease: &JobLease,
    row: &GarminPushInboxRow,
    why: &str,
    now_ms: i64,
) -> AppResult<bool> {
    let attempts = row.attempts;
    if attempts >= MAX_ATTEMPTS {
        error!(
            inbox_id = %row.id,
            summary_type = %row.summary_type,
            attempts,
            reason = %why,
            "Garmin push failed for the last time; its records are not stored"
        );
        return inbox.fail_push(lease, why, now_ms).await;
    }
    let retry_at = now_ms.saturating_add(backoff_ms(attempts));
    warn!(
        inbox_id = %row.id,
        summary_type = %row.summary_type,
        attempts,
        reason = %why,
        "Garmin push handed back for another run"
    );
    inbox.retry_push(lease, why, retry_at).await
}

/// Parse a stored row and ingest it.
async fn run(resources: &Arc<ServerContext>, row: &GarminPushInboxRow) -> AppResult<BatchOutcome> {
    let (kind, records) = parse_envelope(row.payload.as_bytes())
        .map_err(|e| AppError::internal(format!("stored Garmin push no longer parses: {e:?}")))?;
    let received_at =
        DateTime::<Utc>::from_timestamp_millis(row.received_at_ms).unwrap_or_else(Utc::now);
    let host = ServerHost {
        resources: Arc::clone(resources),
    };
    ingest_batch(&resources.common.repos, &host, kind, records, received_at).await
}

/// One sweep: give up on exhausted rows, run the rows whose lease lapsed,
/// delete processed rows past their retention. Returns how many rows ran.
///
/// # Errors
///
/// Returns the inbox's error when the claim fails; the other steps log
/// their failure and the sweep goes on.
pub async fn process_garmin_inbox(resources: &Arc<ServerContext>) -> AppResult<usize> {
    let inbox = Arc::clone(&resources.common.repos.garmin_push_inbox);
    let now_ms = Utc::now().timestamp_millis();
    give_up_exhausted(inbox.as_ref(), now_ms).await;
    let rows = inbox
        .claim_due_pushes(InboxClaim {
            now_ms,
            lease_ms: lease_ms(),
            max_attempts: MAX_ATTEMPTS,
            limit: SWEEP_BATCH,
        })
        .await?;
    let ran = rows.len();
    for row in rows {
        // The claim wrote a token on every row it returned.
        let Some(token) = row.lease_token.clone() else {
            continue;
        };
        let lease = JobLease {
            id: row.id.clone(),
            token,
        };
        process_push(resources, row, lease).await;
    }
    delete_processed(inbox.as_ref(), now_ms).await;
    drop_stale_files(resources.common.repos.pushed_data.as_ref(), Utc::now()).await;
    Ok(ran)
}

/// Mark `failed` the rows past the attempt cap whose last run died.
async fn give_up_exhausted(inbox: &dyn GarminPushInboxRepository, now_ms: i64) {
    match inbox.give_up_exhausted_pushes(now_ms, MAX_ATTEMPTS).await {
        Ok(0) => {}
        Ok(given_up) => error!(
            given_up,
            "Garmin pushes whose last run died are marked failed; their records are not stored"
        ),
        Err(e) => warn!(error = %e, "Garmin inbox: exhausted rows could not be given up"),
    }
}

/// Delete the rows processed before the retention window.
async fn delete_processed(inbox: &dyn GarminPushInboxRepository, now_ms: i64) {
    let cutoff = now_ms.saturating_sub(duration_ms(PROCESSED_RETENTION));
    if let Err(e) = inbox.delete_processed_pushes(cutoff).await {
        warn!(error = %e, "Garmin inbox: processed rows could not be deleted");
    }
}

/// Delete the activity files kept longer than [`KEPT_FILE_RETENTION`]: their
/// batch has run its last attempt by then, so nothing will decode them.
async fn drop_stale_files(pushed: &dyn PushedDataRepository, now: DateTime<Utc>) {
    let Ok(retention) = chrono::Duration::from_std(KEPT_FILE_RETENTION) else {
        return;
    };
    match pushed.drop_files_fetched_before(now - retention).await {
        Ok(0) => {}
        Ok(dropped) => warn!(
            dropped,
            "Garmin activity files kept past their batch's last run are deleted unstored"
        ),
        Err(e) => warn!(error = %e, "Garmin inbox: stale kept files could not be deleted"),
    }
}

/// Start the inbox sweep on the shared periodic-worker ledger.
pub fn start_garmin_push_worker(resources: &Arc<ServerContext>) {
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let resources = Arc::clone(resources);
    spawn_periodic("garmin push inbox", SWEEP_INTERVAL, ledger, move || {
        let resources = Arc::clone(&resources);
        async move {
            let ran = process_garmin_inbox(&resources).await?;
            if ran > 0 {
                info!(ran, "Garmin inbox sweep ran stored pushes");
            }
            Ok(())
        }
    });
}

/// An activity-file download error, without the URL reqwest names in it:
/// the callback URL carries the one-shot download token, which must reach
/// neither a log line nor the inbox row's `last_error`.
fn file_error(what: &str, e: impl Into<SharedHttpError>) -> AppError {
    let why = match e.into() {
        SharedHttpError::Reqwest(e) => e.without_url().to_string(),
        // A middleware's own error is not shown: it may quote the request.
        SharedHttpError::Middleware(_) => "the HTTP middleware refused the request".to_owned(),
    };
    AppError::new(
        ErrorCode::ExternalServiceUnavailable,
        format!("Garmin activity file {what}: {why}"),
    )
}

/// The server side of the ingest.
struct ServerHost {
    resources: Arc<ServerContext>,
}

impl ServerHost {
    /// The callback URL, when it points at the Garmin API this server is
    /// configured for: the owner's bearer token is sent to it, so a URL a
    /// push names on any other origin is refused.
    fn trusted_callback(&self, callback_url: &str) -> AppResult<Url> {
        let base = Url::parse(&self.resources.common.config.garmin_api_config().base_url)
            .map_err(|e| AppError::internal(format!("Garmin API base URL does not parse: {e}")))?;
        let url = Url::parse(callback_url)
            .map_err(|_| AppError::invalid_input("Garmin activity file URL does not parse"))?;
        if url.origin() != base.origin() {
            return Err(AppError::invalid_input(
                "Garmin activity file URL is not on the Garmin API's origin",
            ));
        }
        Ok(url)
    }
}

#[async_trait]
impl GarminPushHost for ServerHost {
    async fn deregister(&self, owner: GarminOwner) -> AppResult<()> {
        // Only the Garmin API connection is withdrawn: the athlete removed
        // Dravr's API access in Garmin Connect, which says nothing about the
        // usr/pwd connection the platform's pair-wide disconnect would end too.
        let service = OAuthService::new(
            self.resources.data(),
            Arc::clone(&self.resources.common.config),
        );
        let data = self.resources.data();
        clear_backend(
            &service,
            &data,
            owner.user_id,
            owner.tenant_id,
            GARMIN,
            DisconnectReason::ProviderRevoked,
        )
        .await?;
        info!(
            target: "notify",
            event = "provider.disconnected",
            provider = GARMIN,
            user_id = %owner.user_id,
            tenant_id = %owner.tenant_id,
            reason = DisconnectReason::ProviderRevoked.as_str(),
            "Garmin deregistered the athlete; their Garmin API connection and its data are deleted"
        );
        Ok(())
    }

    async fn fetch_activity_file(
        &self,
        owner: GarminOwner,
        callback_url: &str,
    ) -> AppResult<ActivityFile> {
        let url = self.trusted_callback(callback_url)?;
        let client_id = self
            .resources
            .common
            .config
            .oauth
            .garmin
            .client_id
            .clone()
            .unwrap_or_default();
        if let Some(budget) = self
            .resources
            .data()
            .provider_registry()
            .request_budget(&client_id, None)
        {
            if let Err(e) = budget.admit(GARMIN_ACTIVITY_FILES).await {
                if e.code == ErrorCode::ExternalRateLimited {
                    let wait = e.retry_after_secs().unwrap_or(60);
                    return Ok(ActivityFile::RateLimited(Duration::from_secs(wait)));
                }
                return Err(e);
            }
        }
        let resources: Arc<ServerContext> = Arc::clone(&self.resources);
        let runtime: Arc<dyn ToolRuntime> = resources;
        let tenant = owner.tenant_id.to_string();
        let token = AuthService::new(runtime)
            .get_valid_token(owner.user_id, GARMIN, Some(&tenant))
            .await
            .map_err(|e| AppError::new(ErrorCode::ExternalAuthFailed, e.to_string()))?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::ExternalAuthFailed,
                    "no Garmin token to fetch the activity file with",
                )
            })?;
        let response = api_client()
            .get(url)
            .bearer_auth(token.access_token.expose_secret())
            .send()
            .await
            .map_err(|e| file_error("request failed", e))?;
        match response.status() {
            StatusCode::GONE | StatusCode::NOT_FOUND => Ok(ActivityFile::Gone),
            StatusCode::TOO_MANY_REQUESTS => Ok(ActivityFile::RateLimited(Duration::from_secs(60))),
            status if status.is_success() => Ok(read_capped(response, MAX_UPLOAD_BYTES)
                .await
                .map_err(|e| file_error("body could not be read", e))?
                .map_or(ActivityFile::TooLarge, ActivityFile::Bytes)),
            status => Err(AppError::new(
                ErrorCode::ExternalServiceUnavailable,
                format!("Garmin activity file answered {status}"),
            )),
        }
    }

    async fn landed(&self, owner: GarminOwner, kind: SummaryType, stored: usize) {
        if matches!(
            kind,
            SummaryType::Deregistrations | SummaryType::UserPermissionsChange
        ) {
            return;
        }
        let tenant = owner.tenant_id.to_string();
        stamp_last_sync(&self.resources, owner.user_id, &tenant, GARMIN).await;
        report_sync(&self.resources, owner.user_id, &tenant, GARMIN, true).await;
        notify_owner(
            &self.resources,
            owner.user_id,
            GARMIN,
            format!("Garmin {}: {stored} new records", kind.key()),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_file_download_never_names_its_tokened_url() {
        // Port 9 on loopback refuses at once; the error reqwest returns
        // names the URL, token and all, until it is stripped.
        let failure = api_client()
            .get("http://127.0.0.1:9/wellness-api/rest/activityFile?id=1&token=one-shot-secret")
            .send()
            .await
            .expect_err("nothing listens on the discard port");
        assert!(
            failure.to_string().contains("one-shot-secret"),
            "the raw reqwest error names the URL, so the strip below is tested"
        );
        let error = file_error("request failed", failure);
        assert!(
            !error.to_string().contains("one-shot-secret"),
            "the download token never reaches the error"
        );
        assert!(
            !error.to_string().contains("activityFile"),
            "the callback URL never reaches the error"
        );
    }

    #[test]
    fn the_backoff_doubles_and_the_runs_fit_inside_a_file_urls_day() {
        assert_eq!(
            backoff_ms(1),
            60_000,
            "the second run waits the first backoff"
        );
        assert_eq!(
            backoff_ms(2),
            120_000,
            "each later run waits twice the last"
        );
        assert_eq!(
            backoff_ms(3),
            240_000,
            "each later run waits twice the last"
        );
        let total: i64 = (1..MAX_ATTEMPTS).map(backoff_ms).sum();
        assert!(
            total < duration_ms(Duration::from_hours(24)),
            "every retry lands inside the 24 hours a callback URL lives"
        );
        assert!(
            total < duration_ms(KEPT_FILE_RETENTION),
            "a kept file outlives every retry of its batch"
        );
    }
}
