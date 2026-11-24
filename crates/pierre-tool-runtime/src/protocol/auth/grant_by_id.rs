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
//! for ([`error_falls_back`]), or the athlete's session-read cap refused it
//! ([`capped_session_falls_back`], carnet#1661), makes it once more through
//! the grant. A session the read found dead is flagged for its sign-in
//! exactly as a list read flags it (`activity_fetch::flag_refusal`), whether the grant then
//! answers or not.
//!
//! What the grant answers is Strava API data, and the activity it returns
//! names `strava`: a caller that keeps it stores it under the grant's key
//! (`activity_detail_store::answered_key`), so the grant's revocation purges
//! it.
//!
//! Strava only. The session's detail page is `strava.com/activities/{id}`, so
//! its ids are the API's. Garmin's grant is push-only: its ids are summary
//! ids and it has no by-id read at all, so a Garmin session gets no by-id
//! fallback.

use std::fmt;

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::provider_context::{
    ProviderActivitySearch, ProviderActivitySearchPage, ProviderActivityZones, ProviderBestEffort,
    ProviderGear, ProviderPersonalRecord, ProviderWeight,
};
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, ProviderThresholds,
    SportType, TenantId, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    TokenRefreshCallback,
};
use pierre_providers::delegation::CoachRoster;
use pierre_providers::request_budget::session_read_cap;
use tokio::sync::OnceCell;
use tracing::{info, warn};
use uuid::Uuid;

use super::AuthService;
use crate::activity_fetch::flag_refusal;
use crate::activity_fetch::sync_verdict::{capped_session_falls_back, error_falls_back};

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
        started_at: Timestamp,
    ) -> Option<&dyn FitnessProvider> {
        // A capped read of the session asks the grant too (carnet#1661): it
        // was refused before anything was sent, and its refusal was logged.
        let capped = capped_session_falls_back(failed, self.session.name());
        if !(capped || error_falls_back(failed)) {
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
        log_falling_back(capped, *user_id, grant, id, failed);
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

/// Log that a by-id read of `id` the session answered with `failed` is read
/// through `grant`: a warning for a failed read, information for one the
/// athlete's session-read cap refused (`capped`), which the limiter logged
/// once already, on the usage target (carnet#1661).
fn log_falling_back(capped: bool, user_id: Uuid, grant: &str, id: &str, failed: &AppError) {
    if capped {
        info!(
            user_id = %user_id,
            grant = %grant,
            activity_id = %id,
            "by-id read of the session capped; reading it through its grant"
        );
    } else {
        warn!(
            user_id = %user_id,
            grant = %grant,
            activity_id = %id,
            cause = %failed,
            "by-id read of the session failed; reading it through its grant"
        );
    }
}

/// The error a by-id read answers when the grant asked in the session's
/// place failed with `grant_error`: a session-read cap's refusal stands, so
/// a stored copy may still answer the read (carnet#1661); any other session
/// failure gives way to the grant's.
fn unanswered<T>(failed: AppError, grant_error: AppError) -> AppResult<T> {
    if session_read_cap(&failed).is_some() {
        Err(failed)
    } else {
        Err(grant_error)
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

    /// The session's profile; a read its session-read cap refused is asked
    /// of the grant (carnet#1661), any other failure stands.
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let started_at = Timestamp::now();
        let failed = match self.session.get_athlete().await {
            Err(e) if capped_session_falls_back(&e, self.session.name()) => e,
            read => return read,
        };
        match self.grant_for("athlete", &failed, started_at).await {
            Some(grant) => grant.get_athlete().await.or_else(|e| unanswered(failed, e)),
            None => Err(failed),
        }
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
        let started_at = Timestamp::now();
        let failed = match self.session.get_activity(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant
                .get_activity(id)
                .await
                .or_else(|e| unanswered(failed, e)),
            None => Err(failed),
        }
    }

    async fn get_activity_detailed(&self, id: &str) -> AppResult<Activity> {
        let started_at = Timestamp::now();
        let failed = match self.session.get_activity_detailed(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant
                .get_activity_detailed(id)
                .await
                .or_else(|e| unanswered(failed, e)),
            None => Err(failed),
        }
    }

    fn serves_activity_streams(&self) -> bool {
        self.session.serves_activity_streams()
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let started_at = Timestamp::now();
        let failed = match self.session.get_activity_with_streams(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant
                .get_activity_with_streams(id)
                .await
                .or_else(|e| unanswered(failed, e)),
            None => Err(failed),
        }
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        let started_at = Timestamp::now();
        let failed = match self.session.get_activity_streams(id).await {
            Err(e) => e,
            read => return read,
        };
        match self.grant_for(id, &failed, started_at).await {
            Some(grant) => grant
                .get_activity_streams(id)
                .await
                .or_else(|e| unanswered(failed, e)),
            None => Err(failed),
        }
    }

    /// The session's own: what a provider says about the athlete is no by-id
    /// read of an activity, and the grant's "unsupported" would mask the
    /// session's real error.
    async fn read_activity_zones(&self, id: &str) -> AppResult<Option<ProviderActivityZones>> {
        self.session.read_activity_zones(id).await
    }

    async fn read_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
        self.session.read_gear().await
    }

    async fn read_personal_records(&self) -> AppResult<Option<Vec<ProviderPersonalRecord>>> {
        self.session.read_personal_records().await
    }

    async fn read_activity_best_efforts(
        &self,
        id: &str,
    ) -> AppResult<Option<Vec<ProviderBestEffort>>> {
        self.session.read_activity_best_efforts(id).await
    }

    async fn read_weight(&self) -> AppResult<Option<ProviderWeight>> {
        self.session.read_weight().await
    }

    async fn search_activities(
        &self,
        search: &ProviderActivitySearch,
    ) -> AppResult<ProviderActivitySearchPage> {
        self.session.search_activities(search).await
    }

    async fn list_planned_workouts(
        &self,
        after: CivilDate,
        before: CivilDate,
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
        from: CivilDate,
        to: CivilDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.session.list_calendar_events(from, to).await
    }

    async fn calendar_event_detail(&self, event: CalendarEventRef) -> AppResult<CalendarEventRef> {
        self.session.calendar_event_detail(event).await
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

#[cfg(test)]
mod tests {
    use pierre_core::errors::{AppError, ErrorCode};
    use serde_json::json;

    use super::unanswered;

    /// A session-read cap's refusal, as the limiter marks it.
    fn capped() -> AppError {
        let mut error = AppError::new(
            ErrorCode::ExternalRateLimited,
            "Strava has been read as often as Dravr allows for now",
        );
        error.details = Some(Box::new(json!({
            "refused_before_send": true,
            "retry_after_secs": 300,
            "session_read_cap": "asked",
        })));
        error
    }

    #[test]
    fn a_capped_read_the_grant_did_not_answer_keeps_its_refusal() {
        let grant_error = AppError::external_service("strava", "502 from the API");
        let kept: Result<(), AppError> = unanswered(capped(), grant_error.clone());
        let kept = kept.expect_err("an unanswered read is an error");
        assert_eq!(
            kept.retry_after_secs(),
            Some(300),
            "bug: the grant's failure hid the cap's refusal, so no stored copy can stand in"
        );
        let other: Result<(), AppError> = unanswered(
            AppError::new(ErrorCode::ExternalServiceUnavailable, "the browser wedged"),
            grant_error,
        );
        assert_eq!(
            other.expect_err("an unanswered read is an error").code,
            ErrorCode::ExternalServiceError,
            "any other session failure gives way to the grant's, as before"
        );
    }
}
