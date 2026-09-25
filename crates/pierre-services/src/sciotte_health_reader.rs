// ABOUTME: Health sync's daily-summary reads, made on the dedicated dravr-sciotte scraper service
// ABOUTME: Implements dravr-enforme's DailySummaryReader over RemoteSciotteClient (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The daily-summary reader the health sync's scrape-backed providers use.
//!
//! dravr-enforme's Garmin and COROS adapters read one day's summary at a
//! time through a [`DailySummaryReader`]. Here that read runs on the
//! dedicated scraper service, exactly as the platform's activity reads do
//! (ADR-021): the platform-held session is imported first, which re-hydrates
//! a service that scaled to zero, then the day is scraped over HTTP. No
//! browser runs in the API pod.
//!
//! The service's answers map onto the scraper errors the adapters branch on:
//! a dead session is [`ScraperError::SessionExpired`] (the sync reports expired
//! credentials instead of an empty day), and a load-shed is
//! [`ScraperError::Busy`] carrying the service's own wait.

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_enforme::providers::sciotte_reader::{
    AuthSession, DailySummary, DailySummaryReader, ScraperError, ScraperResult,
};
use pierre_providers::sciotte_remote::{shed_retry_after_secs, RemoteSciotteClient};

/// Reads daily summaries on the dravr-sciotte scraper service.
///
/// The service is the one `DRAVR_SCIOTTE_REMOTE_URL` names. The client is
/// built per read, as every other remote sciotte read builds it; it is a thin
/// handle over a pooled HTTP client and a cached identity token.
#[derive(Debug, Clone, Copy, Default)]
pub struct SciotteServiceReader;

#[async_trait]
impl DailySummaryReader for SciotteServiceReader {
    async fn daily_summary(
        &self,
        provider: &str,
        session: &AuthSession,
        date: NaiveDate,
    ) -> ScraperResult<DailySummary> {
        let remote = RemoteSciotteClient::require_from_env().map_err(scraper_error)?;
        let session_id = remote
            .import_session(session, provider)
            .await
            .map_err(scraper_error)?;
        remote
            .get_daily_summary(&session_id, date)
            .await
            .map_err(scraper_error)
    }
}

/// The scraper error a failed remote read is, by what the service answered.
#[must_use]
pub fn scraper_error(error: AppError) -> ScraperError {
    if matches!(error.code, ErrorCode::ProviderAuthRequired) {
        return ScraperError::SessionExpired {
            reason: error.message,
        };
    }
    if let Some(retry_after_secs) = shed_retry_after_secs(&error) {
        return ScraperError::Busy {
            reason: error.message,
            retry_after_secs,
        };
    }
    ScraperError::Internal {
        reason: error.message,
    }
}
