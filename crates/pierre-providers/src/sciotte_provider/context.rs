// ABOUTME: The scraped Strava's own zones, gear, records, best efforts, weight and paged search, read live through sciotte
// ABOUTME: Provider-reported and never computed; an answer the provider has none of stays absent (None), never an empty default

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider context reads of the scraped Strava session (carnet#1622,
//! sciotte 2.3).
//!
//! Only the Strava target serves them: Garmin, `TrainingPeaks` and COROS share
//! the provider type and answer the `unsupported` refusal. Each read goes
//! through [`SciotteProvider::read_through`] like every other, so a rotated
//! session is adopted and a dead one re-tagged with its backend.
//!
//! What the provider has none of stays absent: a `404` is `Ok(None)` (the one
//! status that says the thing is not there), a weight the profile shows none
//! of is `None`, and zones the provider reported for neither family are
//! `None`. A `429` or a `5xx` at the provider is a retryable error, never an
//! absence (see `sciotte_error::context_read`).

use dravr_sciotte::client::{ClientError, StatusCode};
use dravr_sciotte::context::{
    is_activity_id, ActivityZones as SciotteZones, AthleteWeight, BestEffort as SciotteEffort,
    GearKind as SciotteGearKind, PersonalRecord as SciottePersonalRecord, StravaGear,
};
use dravr_sciotte::models::{ActivityParams, ActivitySearch, AuthSession, DetailPass};
use pierre_core::models::provider_context::{
    ProviderActivitySearch, ProviderActivitySearchPage, ProviderActivityZones, ProviderBestEffort,
    ProviderGear, ProviderGearKind, ProviderHeartRateZone, ProviderPaceZone,
    ProviderPersonalRecord, ProviderWeight,
};

use super::{convert_activity, SciotteProvider, SciotteTarget};
use crate::errors::{AppError, AppResult};
use crate::provider_context::unsupported;

/// A read's answer, with the provider's `404` read as absence.
fn absent_on_404<T>(answer: Result<T, ClientError>) -> Result<Option<T>, ClientError> {
    match answer {
        Ok(value) => Ok(Some(value)),
        Err(ClientError::Unexpected { status, .. }) if status == StatusCode::NOT_FOUND => Ok(None),
        Err(other) => Err(other),
    }
}

impl SciotteProvider {
    /// The session a context read goes through, after the target check: only
    /// the Strava target serves these reads.
    ///
    /// A delegated provider reads a coach's session on behalf of one of the
    /// coach's athletes: the session's own weight, gear and records are the
    /// coach's, so no context read is answered from it.
    async fn context_session(&self, what: &str) -> AppResult<AuthSession> {
        if self.subject.is_some() {
            return Err(AppError::invalid_input(format!(
                "{what} cannot be read through a delegated session: it would answer with the coach's own figures"
            )));
        }
        if !matches!(
            SciotteTarget::from_backend_name(self.provider_name),
            SciotteTarget::Strava
        ) {
            return Err(unsupported(self.provider_name, what));
        }
        self.session_snapshot().await
    }

    /// An activity id the provider's URLs and bodies can carry.
    fn context_activity_id(id: &str) -> AppResult<()> {
        if is_activity_id(id) {
            Ok(())
        } else {
            Err(AppError::invalid_input(
                "An activity id is made of digits".to_owned(),
            ))
        }
    }

    /// Strava's heart rate and pace zones of activity `id`.
    pub(super) async fn strava_activity_zones(
        &self,
        id: &str,
    ) -> AppResult<Option<ProviderActivityZones>> {
        let session = self.context_session("activity zones").await?;
        Self::context_activity_id(id)?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let zones = self
            .read_through(remote, &session, || async move {
                absent_on_404(remote.get_activity_zones(sid, id).await)
            })
            .await?;
        Ok(zones.and_then(convert_zones))
    }

    /// The shoes and bikes of the signed-in Strava account.
    pub(super) async fn strava_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
        let session = self.context_session("the athlete's gear").await?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let gear = self
            .read_through(remote, &session, || async move {
                absent_on_404(remote.get_gear(sid).await)
            })
            .await?;
        Ok(gear.map(|items| items.into_iter().map(convert_gear).collect()))
    }

    /// The all-time records of the signed-in Strava account.
    pub(super) async fn strava_personal_records(
        &self,
    ) -> AppResult<Option<Vec<ProviderPersonalRecord>>> {
        let session = self.context_session("personal records").await?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let records = self
            .read_through(remote, &session, || async move {
                absent_on_404(remote.get_personal_records(sid).await)
            })
            .await?;
        Ok(records.map(|items| items.into_iter().map(convert_record).collect()))
    }

    /// The best efforts Strava found in activity `id`.
    pub(super) async fn strava_best_efforts(
        &self,
        id: &str,
    ) -> AppResult<Option<Vec<ProviderBestEffort>>> {
        let session = self.context_session("activity best efforts").await?;
        Self::context_activity_id(id)?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let efforts = self
            .read_through(remote, &session, || async move {
                absent_on_404(remote.get_best_efforts(sid, id).await)
            })
            .await?;
        Ok(efforts.map(|items| items.into_iter().map(convert_effort).collect()))
    }

    /// The weight on the signed-in Strava account's profile, `None` when it
    /// shows none this build can read.
    pub(super) async fn strava_weight(&self) -> AppResult<Option<ProviderWeight>> {
        let session = self.context_session("the athlete's weight").await?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let weight = self
            .read_through(remote, &session, || async move {
                absent_on_404(remote.get_athlete_weight(sid).await)
            })
            .await?;
        Ok(weight
            .and_then(|AthleteWeight { weight_kg }| weight_kg)
            .map(|weight_kg| ProviderWeight { weight_kg }))
    }

    /// A page of Strava's own filtered activity list.
    ///
    /// Reads the list, not the training log: it does not touch the head
    /// completeness of the provider's ordinary list reads, which belongs to
    /// those.
    pub(super) async fn strava_search_activities(
        &self,
        search: &ProviderActivitySearch,
    ) -> AppResult<ProviderActivitySearchPage> {
        let session = self.context_session("a filtered activity search").await?;
        let remote = self.remote_for(&session)?;
        let sid = session.session_id.as_str();
        let wanted = ActivitySearch {
            page: search.page,
            tag: None,
            commute_only: false,
        };
        let params = ActivityParams {
            limit: search
                .limit
                .map(|limit| u32::try_from(limit).unwrap_or(u32::MAX)),
            after: None,
            before: None,
            sport_type: None,
            athlete: None,
            detail: DetailPass::Skip,
        };
        let list = self
            .read_through(remote, &session, || {
                remote.search_activities(sid, &wanted, &params)
            })
            .await?;
        Ok(ProviderActivitySearchPage {
            activities: list
                .activities
                .iter()
                .map(|activity| convert_activity(activity, SciotteTarget::Strava))
                .collect(),
            complete: list.head_complete,
        })
    }
}

/// Strava's zones as the platform's, `None` when it reported neither family.
fn convert_zones(zones: SciotteZones) -> Option<ProviderActivityZones> {
    if zones.heart_rate.is_none() && zones.pace.is_none() {
        return None;
    }
    Some(ProviderActivityZones {
        heart_rate: zones.heart_rate.map(|buckets| {
            buckets
                .into_iter()
                .map(|b| ProviderHeartRateZone {
                    zone: b.zone,
                    label: b.label,
                    min_bpm: b.min_bpm,
                    max_bpm: b.max_bpm,
                    seconds: b.seconds,
                })
                .collect()
        }),
        pace: zones.pace.map(|buckets| {
            buckets
                .into_iter()
                .map(|b| ProviderPaceZone {
                    zone: b.zone,
                    label: b.label,
                    min_mps: b.min_mps,
                    max_mps: b.max_mps,
                    seconds: b.seconds,
                })
                .collect()
        }),
    })
}

fn convert_gear(gear: StravaGear) -> ProviderGear {
    ProviderGear {
        id: gear.id,
        kind: match gear.kind {
            SciotteGearKind::Shoe => ProviderGearKind::Shoe,
            SciotteGearKind::Bike => ProviderGearKind::Bike,
        },
        name: gear.name,
        brand: gear.brand,
        model: gear.model,
        total_distance_m: gear.total_distance_m,
        primary: gear.primary,
        retired: gear.retired,
    }
}

fn convert_record(record: SciottePersonalRecord) -> ProviderPersonalRecord {
    ProviderPersonalRecord {
        sport: record.sport,
        label: record.label,
        distance_m: record.distance_m,
        elapsed_seconds: record.elapsed_seconds,
        value_m: record.value_m,
        activity_id: record.activity_id,
    }
}

fn convert_effort(effort: SciotteEffort) -> ProviderBestEffort {
    ProviderBestEffort {
        label: effort.label,
        distance_m: effort.distance_m,
        elapsed_seconds: effort.elapsed_seconds,
        start_index: effort.start_index,
        end_index: effort.end_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{FitnessProvider, ProviderConfig};
    use pierre_core::errors::ErrorCode;

    #[tokio::test]
    async fn a_delegated_session_never_answers_a_context_read() {
        let config = ProviderConfig {
            name: "sciotte".to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: Vec::new(),
        };
        let provider = SciotteProvider {
            subject: Some("12345".parse().expect("an athlete id")),
            ..SciotteProvider::new(config, SciotteTarget::Strava, None)
        };
        let error = provider
            .read_weight()
            .await
            .expect_err("a coach session must not answer the coach's own weight");
        assert_eq!(
            error.code,
            ErrorCode::InvalidInput,
            "bug: a delegated context read is not refused as misuse: {error}"
        );
        assert!(error.message.contains("delegated"), "{error}");
        assert!(
            provider.read_gear().await.is_err(),
            "bug: a coach session answered the coach's own gear"
        );
        assert!(
            provider.read_personal_records().await.is_err(),
            "bug: a coach session answered the coach's own records"
        );
    }
}
