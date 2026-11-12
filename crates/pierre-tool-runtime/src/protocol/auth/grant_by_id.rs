// ABOUTME: Retries a failed by-id read of the athlete's Strava session once through their Strava grant
// ABOUTME: Both backends key an activity by Strava's own id, so the grant reads the same activity (carnet#1494)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The by-id half of the one fallback (carnet#1494).
//!
//! A list read that the session cannot answer falls back to the grant in
//! `activity_fetch::fallback`. A read of one activity by its id (its detail,
//! its streams, the gear the detail carries) is a different call on the same
//! provider handle, so it falls back here: [`GrantByIdFallback`] wraps the
//! session and, when one of those reads fails the way a list read falls back
//! for ([`error_falls_back`]), makes it once more through the grant. A
//! session the read found dead is flagged for its sign-in exactly as a list
//! read flags it (`activity_fetch::flag_refusal`), whether the grant then
//! answers or not.
//!
//! Strava only. The session's detail page is `strava.com/activities/{id}`, so
//! its ids are the API's. Garmin's grant is push-only: its ids are summary
//! ids and it has no by-id read at all, so a Garmin session gets no by-id
//! fallback.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, ProviderThresholds,
    SportType, Stats, TenantId, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    TokenRefreshCallback,
};
use pierre_providers::delegation::CoachRoster;
use tokio::sync::OnceCell;
use tracing::warn;
use uuid::Uuid;

use super::AuthService;
use crate::activity_fetch::flag_refusal;
use crate::activity_fetch::sync_verdict::error_falls_back;

/// The grant a [`GrantByIdFallback`] reads through, authenticated only when
/// a by-id read of the session first fails.
struct GrantSource {
    auth: AuthService,
    grant: String,
    user_id: Uuid,
    tenant: TenantId,
    tenant_id: String,
}

/// The athlete's Strava session, whose by-id reads fall back to their Strava
/// grant once; every other call is the session's own.
pub(super) struct GrantByIdFallback {
    session: Box<dyn FitnessProvider>,
    source: GrantSource,
    grant: OnceCell<Box<dyn FitnessProvider>>,
}

impl fmt::Debug for GrantByIdFallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantByIdFallback")
            .field("grant", &self.source.grant)
            .finish_non_exhaustive()
    }
}

impl GrantByIdFallback {
    /// Wrap `session` so its by-id reads fall back to `grant`, which `auth`
    /// authenticates for `user_id` in `tenant` when first needed.
    pub(super) fn wrap(
        session: Box<dyn FitnessProvider>,
        auth: AuthService,
        grant: &str,
        user_id: Uuid,
        tenant: TenantId,
    ) -> Box<dyn FitnessProvider> {
        Box::new(Self {
            session,
            source: GrantSource {
                auth,
                grant: grant.to_owned(),
                user_id,
                tenant,
                tenant_id: tenant.to_string(),
            },
            grant: OnceCell::new(),
        })
    }

    /// The grant to read activity `id` through after the session's read of it,
    /// begun at `started_at`, failed with `failed`, or `None` when that
    /// failure does not fall back or the grant cannot be authenticated: the
    /// session's failure then stands.
    ///
    /// A failure that falls back and is the session's own refusal for
    /// authentication flags the session first (`flag_refusal`), as the list
    /// read's fallback does: the grant answering this one read never hides
    /// that the athlete's sign-in is dead.
    async fn grant_for(
        &self,
        id: &str,
        failed: &AppError,
        started_at: DateTime<Utc>,
    ) -> Option<&dyn FitnessProvider> {
        if !error_falls_back(failed) {
            return None;
        }
        let GrantSource {
            auth,
            grant,
            user_id,
            tenant,
            tenant_id,
        } = &self.source;
        flag_refusal(
            auth.runtime(),
            *user_id,
            *tenant,
            self.session.name(),
            failed,
            started_at,
        )
        .await;
        warn!(
            user_id = %user_id,
            grant = %grant,
            activity_id = %id,
            cause = %failed,
            "by-id read of the session failed; reading it through its grant"
        );
        let authenticated = self
            .grant
            .get_or_try_init(|| async {
                auth.authenticate_backend(grant, grant, *user_id, Some(tenant_id))
                    .await
                    .map_err(|response| response.error.unwrap_or_default())
            })
            .await;
        match authenticated {
            Ok(provider) => Some(provider.as_ref()),
            Err(cause) => {
                warn!(
                    user_id = %user_id,
                    grant = %grant,
                    cause = %cause,
                    "the grant behind the session could not be authenticated; the session's failure stands"
                );
                None
            }
        }
    }
}

#[async_trait]
impl FitnessProvider for GrantByIdFallback {
    fn name(&self) -> &'static str {
        self.session.name()
    }

    fn config(&self) -> &ProviderConfig {
        self.session.config()
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        self.session.set_credentials(credentials).await
    }

    async fn is_authenticated(&self) -> bool {
        self.session.is_authenticated().await
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        self.session.refresh_token_if_needed().await
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        self.session.set_token_refresh_callback(callback);
    }

    fn bind_owner_stores(&self, stores: OwnerStores) {
        self.session.bind_owner_stores(stores);
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        self.session.get_athlete().await
    }

    async fn get_activities(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<Activity>> {
        self.session.get_activities(limit, offset).await
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.session.get_activities_with_params(params).await
    }

    fn head_complete(&self) -> bool {
        self.session.head_complete()
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        self.session.get_activities_cursor(params).await
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let started_at = Utc::now();
        let failed = match self.session.get_activity(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant.get_activity(id).await,
            None => Err(failed),
        }
    }

    async fn get_activity_detailed(&self, id: &str) -> AppResult<Activity> {
        let started_at = Utc::now();
        let failed = match self.session.get_activity_detailed(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant.get_activity_detailed(id).await,
            None => Err(failed),
        }
    }

    fn serves_activity_streams(&self) -> bool {
        self.session.serves_activity_streams()
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let started_at = Utc::now();
        let failed = match self.session.get_activity_with_streams(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant.get_activity_with_streams(id).await,
            None => Err(failed),
        }
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        let started_at = Utc::now();
        let failed = match self.session.get_activity_streams(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant.get_activity_streams(id).await,
            None => Err(failed),
        }
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        self.session.get_stats().await
    }

    async fn list_planned_workouts(
        &self,
        after: NaiveDate,
        before: NaiveDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        self.session.list_planned_workouts(after, before).await
    }

    async fn read_coach_roster(&self) -> AppResult<CoachRoster> {
        self.session.read_coach_roster().await
    }

    async fn read_thresholds(
        &self,
        primary_sport: Option<&SportType>,
    ) -> AppResult<ProviderThresholds> {
        self.session.read_thresholds(primary_sport).await
    }

    fn calendar_key(&self, external_id: &str) -> String {
        self.session.calendar_key(external_id)
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.session.list_calendar_events(from, to).await
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        self.session.push_planned_session(session).await
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        self.session
            .update_planned_session(provider_event_id, session)
            .await
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        self.session
            .delete_planned_sessions(provider_event_ids)
            .await
    }
}
