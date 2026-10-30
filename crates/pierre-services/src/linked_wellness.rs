// ABOUTME: What a human coach reads of a coached athlete's wellness — only what the athlete's link to them filed under their wellness_read consent
// ABOUTME: The newest HRV, resting heart rate, night's sleep and weight that link's platform delivered, under each row's provider terms

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coached athlete's wellness, as their coach reads it
//!
//! A member lets their coach's confirmed link sync their sleep, HRV, resting
//! heart rate and weight through the coach's own platform account by granting
//! the coach `wellness_read` ([`crate::coach_consents`]). That consent is the
//! only thing that lets the coach read any of it, and what it lets the coach
//! read is what the link filed: the rows the link's platform delivered, under
//! the tenant the member confirmed in. A wellness source of the athlete's own
//! (their Garmin, their WHOOP) is never read here, granted or not — the link
//! syncs none of it, and revoking the consent deletes what the link filed.
//!
//! The rows are read through [`stored_health`] narrowed to the link
//! ([`SourceFilter::Link`]): each source's provider terms apply exactly as
//! they do to the athlete's own reads, and the link's rows are never merged
//! with the athlete's other sources. A night is dated by the morning it ended
//! on, in the athlete's zone.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{DelegationStatus, TenantId};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::athlete_clock::athlete_zone;
use crate::coach_consents::member_consents;
use crate::provider_notice::under_ai_consent;
use crate::stored_health::{self, SourceFilter};

/// How far back the newest readings are looked for.
pub const LINKED_WELLNESS_DAYS: i64 = 7;

/// One reading and the day it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading<T> {
    /// The value.
    pub value: T,
    /// The day it describes.
    pub date: NaiveDate,
}

/// The newest wellness a coach's link filed for a coached athlete in the
/// last [`LINKED_WELLNESS_DAYS`] days. A reading the link filed none of in
/// that window is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedWellness {
    /// The backend slug of the link the readings came through.
    pub provider: String,
    /// Overnight HRV, in milliseconds (RMSSD where the platform reports it).
    pub hrv_ms: Option<Reading<f64>>,
    /// Resting heart rate, in beats per minute.
    pub resting_heart_rate: Option<Reading<u32>>,
    /// The night's sleep, in seconds; dated by the day the night ended.
    pub sleep_seconds: Option<Reading<u32>>,
    /// Body weight, in kilograms.
    pub weight_kg: Option<Reading<f64>>,
}

impl LinkedWellness {
    /// Whether the link filed none of the four readings in the window.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.hrv_ms.is_none()
            && self.resting_heart_rate.is_none()
            && self.sleep_seconds.is_none()
            && self.weight_kg.is_none()
    }
}

/// The wellness `coach` reads of `athlete` through the link they hold in
/// `group_id`, or `None` when no confirmed link there names the athlete or
/// the athlete has not granted the coach `wellness_read`.
///
/// The caller has run the coaching gates for this pair
/// ([`coached_athlete_access`](crate::coached_athlete::coached_athlete_access));
/// this decides only what the link lets the coach read. The reads run under
/// the athlete's own AI consents, as every model read of a member's data
/// does: the coach's consents say nothing about the athlete's providers.
///
/// # Errors
///
/// Returns the repository error from the link, sleep, recovery or health
/// reads.
pub async fn linked_wellness(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    coach: Uuid,
    athlete: Uuid,
    group_id: Uuid,
    now: DateTime<Utc>,
) -> AppResult<Option<LinkedWellness>> {
    if !member_consents(repos.groups.as_ref(), group_id, athlete)
        .await?
        .wellness_read
    {
        return Ok(None);
    }
    let link = repos
        .delegated_connections
        .list_live_for_coach_in_group(group_id, coach)
        .await?
        .into_iter()
        .find(|link| link.member_user_id == athlete && link.status == DelegationStatus::Confirmed);
    let Some((link, tenant)) =
        link.and_then(|link| link.member_tenant_id.map(|tenant| (link, tenant)))
    else {
        return Ok(None);
    };
    let filed = Filed {
        athlete,
        tenant,
        provider: link.provider,
        start: now - Duration::days(LINKED_WELLNESS_DAYS),
        end: now,
    };
    under_ai_consent(
        repos,
        tenant.as_uuid(),
        athlete,
        newest_filed(repos, terms, filed),
    )
    .await
    .map(Some)
}

/// Whose rows, under which tenant, filed by which platform, in which span.
struct Filed {
    athlete: Uuid,
    tenant: TenantId,
    provider: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

/// The newest of each reading the link's platform filed in the span.
async fn newest_filed(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    filed: Filed,
) -> AppResult<LinkedWellness> {
    let Filed {
        athlete,
        tenant,
        provider,
        start,
        end: now,
    } = filed;
    let link = SourceFilter::Link(&provider);
    let window = (start, now);

    let mut recovery: Vec<_> =
        stored_health::merged_recovery(repos, terms, athlete, &tenant, window, link)
            .await?
            .records
            .into_iter()
            .map(|row| row.record)
            .collect();
    recovery.sort_by_key(|row| row.date);
    let hrv_ms = recovery.iter().rev().find_map(|row| {
        row.daily_hrv().map(|value| Reading {
            value,
            date: row.date,
        })
    });
    let resting_heart_rate = recovery.iter().rev().find_map(|row| {
        row.resting_heart_rate.map(|value| Reading {
            value,
            date: row.date,
        })
    });

    let zone = athlete_zone(repos, athlete).await;
    let sleep_seconds = stored_health::merged_sleep(repos, terms, athlete, &tenant, window, link)
        .await?
        .records
        .into_iter()
        .map(|night| night.record)
        .filter(|night| !night.is_nap)
        .filter_map(|night| {
            night.total_sleep_seconds.map(|value| Reading {
                value,
                date: night.night_date(&zone),
            })
        })
        .max_by_key(|reading| reading.date);

    let weight_kg = stored_health::merged_body(repos, terms, athlete, &tenant, window, link)
        .await?
        .records
        .into_iter()
        .map(|row| row.record)
        .filter_map(|row| {
            row.weight_kg.map(|value| Reading {
                value,
                date: row.date,
            })
        })
        .max_by_key(|reading| reading.date);

    Ok(LinkedWellness {
        provider,
        hrv_ms,
        resting_heart_rate,
        sleep_seconds,
        weight_kg,
    })
}
