// ABOUTME: The AiGovernedProvider decorator every authenticated provider is wrapped in
// ABOUTME: Reads pass through the provider-terms filters under a gate; writes and auth pass straight through

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, ProviderThresholds,
    SportType, Stats, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};

use super::{
    consented_read, filter_activities, filter_one, filter_planned_workouts, first_party_only_read,
    policies_apply, withheld_error,
};
use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    SegmentEffortHit, SegmentEffortWindow, TokenRefreshCallback,
};

/// A provider whose reads return only what each provider's terms let the
/// reader see.
///
/// Under a gate, the reader is a model (AI policy) or a caller over an
/// external transport (transport policy). Writes and auth pass straight
/// through, and where no gate applies so does everything.
pub struct AiGovernedProvider {
    inner: Box<dyn FitnessProvider>,
    policies: Arc<dyn ProviderTerms>,
}

impl fmt::Debug for AiGovernedProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiGovernedProvider").finish_non_exhaustive()
    }
}

impl AiGovernedProvider {
    /// Govern `inner`'s reads by the policies `policies` declares (the provider registry).
    #[must_use]
    pub fn wrap(
        inner: Box<dyn FitnessProvider>,
        policies: Arc<dyn ProviderTerms>,
    ) -> Box<dyn FitnessProvider> {
        Box::new(Self { inner, policies })
    }

    fn lookup(&self) -> &dyn ProviderTerms {
        self.policies.as_ref()
    }

    fn governed_one(&self, id: &str, activity: Activity) -> AppResult<Activity> {
        first_party_only_read(self.lookup(), activity.provider(), activity.source())?;
        filter_one(self.lookup(), activity).ok_or_else(|| withheld_error(id))
    }

    fn untagged_read(&self) -> AppResult<()> {
        first_party_only_read(self.lookup(), self.inner.name(), None)?;
        consented_read(self.inner.name())
    }
}

#[async_trait]
impl FitnessProvider for AiGovernedProvider {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn config(&self) -> &ProviderConfig {
        self.inner.config()
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        self.inner.set_credentials(credentials).await
    }

    async fn is_authenticated(&self) -> bool {
        self.inner.is_authenticated().await
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        self.inner.refresh_token_if_needed().await
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        self.inner.set_token_refresh_callback(callback);
    }

    fn bind_owner_stores(&self, stores: OwnerStores) {
        self.inner.bind_owner_stores(stores);
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        self.untagged_read()?;
        self.inner.get_athlete().await
    }

    async fn get_activities(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<Activity>> {
        let activities = self.inner.get_activities(limit, offset).await?;
        Ok(filter_activities(self.lookup(), activities))
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let activities = self.inner.get_activities_with_params(params).await?;
        Ok(filter_activities(self.lookup(), activities))
    }

    fn head_complete(&self) -> bool {
        self.inner.head_complete()
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let mut page = self.inner.get_activities_cursor(params).await?;
        page.items = filter_activities(self.lookup(), mem::take(&mut page.items));
        page.count = page.items.len();
        Ok(page)
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let activity = self.inner.get_activity(id).await?;
        self.governed_one(id, activity)
    }

    async fn get_activity_detailed(&self, id: &str) -> AppResult<Activity> {
        let activity = self.inner.get_activity_detailed(id).await?;
        self.governed_one(id, activity)
    }

    fn serves_activity_streams(&self) -> bool {
        self.inner.serves_activity_streams()
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let activity = self.inner.get_activity_with_streams(id).await?;
        self.governed_one(id, activity)
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        if !policies_apply() {
            return self.inner.get_activity_streams(id).await;
        }
        // Streams carry no provenance of their own: read the activity they
        // belong to and let its policy decide.
        let activity = self.get_activity_with_streams(id).await?;
        Ok(activity.time_series_data().cloned())
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        self.untagged_read()?;
        self.inner.get_stats().await
    }

    async fn read_thresholds(
        &self,
        primary_sport: Option<&SportType>,
    ) -> AppResult<ProviderThresholds> {
        self.untagged_read()?;
        self.inner.read_thresholds(primary_sport).await
    }

    /// Gated as a read without item provenance: the provider's terms decide
    /// before it is asked, so Strava's efforts never reach an external caller.
    async fn segment_effort_history(
        &self,
        segment_id: &str,
        window: SegmentEffortWindow,
    ) -> AppResult<Vec<SegmentEffortHit>> {
        self.untagged_read()?;
        self.inner.segment_effort_history(segment_id, window).await
    }

    async fn list_planned_workouts(
        &self,
        after: NaiveDate,
        before: NaiveDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        let workouts = self.inner.list_planned_workouts(after, before).await?;
        Ok(filter_planned_workouts(self.lookup(), workouts))
    }

    fn calendar_key(&self, external_id: &str) -> String {
        self.inner.calendar_key(external_id)
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.untagged_read()?;
        self.inner.list_calendar_events(from, to).await
    }

    async fn calendar_event_detail(&self, event: CalendarEventRef) -> AppResult<CalendarEventRef> {
        self.untagged_read()?;
        self.inner.calendar_event_detail(event).await
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        self.inner.push_planned_session(session).await
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        self.inner
            .update_planned_session(provider_event_id, session)
            .await
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        self.inner.delete_planned_sessions(provider_event_ids).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone, Utc};
    use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
    use pierre_core::errors::{AppError, AppResult, ErrorCode};
    use pierre_core::models::{Activity, Athlete, SegmentEffort, Stats};
    use pierre_core::pagination::{CursorPage, PaginationParams};
    use pierre_core::transport::{Transport, TransportPolicy};

    use super::AiGovernedProvider;
    use crate::ai_scope::{ai_read, serve_over, with_ai_consent};
    use crate::core::{
        ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, SegmentEffortHit,
        SegmentEffortWindow,
    };

    /// Terms that keep Strava's data on Dravr's own surfaces, as its
    /// descriptor declares.
    struct StravaFirstParty;

    impl ProviderTerms for StravaFirstParty {
        fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
            None
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "strava").then_some(TransportPolicy::FirstPartyOnly)
        }
    }

    /// A Strava provider that lists one effort on any segment and counts the
    /// listings it was asked for.
    struct Listing {
        config: ProviderConfig,
        calls: Arc<AtomicUsize>,
    }

    fn hit() -> SegmentEffortHit {
        SegmentEffortHit {
            effort: SegmentEffort {
                id: "9001".to_owned(),
                segment_id: Some("611413".to_owned()),
                name: "Camillien-Houde".to_owned(),
                elapsed_time: 412,
                moving_time: None,
                start_date: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
                distance: 1_820.5,
                average_heart_rate: None,
                max_heart_rate: None,
                average_cadence: None,
                average_watts: None,
                kom_rank: None,
                pr_rank: Some(1),
                climb_category: None,
                average_grade: None,
                elevation_gain: None,
            },
            activity_id: "16101".to_owned(),
            activity_type: None,
        }
    }

    #[async_trait]
    impl FitnessProvider for Listing {
        fn name(&self) -> &'static str {
            "strava"
        }

        fn config(&self) -> &ProviderConfig {
            &self.config
        }

        async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
            Ok(())
        }

        async fn is_authenticated(&self) -> bool {
            true
        }

        async fn refresh_token_if_needed(&self) -> AppResult<()> {
            Ok(())
        }

        async fn get_athlete(&self) -> AppResult<Athlete> {
            Err(AppError::internal("not used by these tests"))
        }

        async fn get_activities_with_params(
            &self,
            _params: &ActivityQueryParams,
        ) -> AppResult<Vec<Activity>> {
            Ok(Vec::new())
        }

        async fn get_activities_cursor(
            &self,
            _params: &PaginationParams,
        ) -> AppResult<CursorPage<Activity>> {
            Ok(CursorPage::new(Vec::new(), None, None, false))
        }

        async fn get_activity(&self, id: &str) -> AppResult<Activity> {
            Err(AppError::not_found(id.to_owned()))
        }

        async fn get_stats(&self) -> AppResult<Stats> {
            Err(AppError::internal("not used by these tests"))
        }

        async fn segment_effort_history(
            &self,
            _segment_id: &str,
            _window: SegmentEffortWindow,
        ) -> AppResult<Vec<SegmentEffortHit>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![hit()])
        }
    }

    fn governed() -> (Box<dyn FitnessProvider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Listing {
            config: ProviderConfig {
                name: "strava".to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: Vec::new(),
            },
            calls: Arc::clone(&calls),
        };
        (
            AiGovernedProvider::wrap(Box::new(inner), Arc::new(StravaFirstParty)),
            calls,
        )
    }

    fn window() -> SegmentEffortWindow {
        SegmentEffortWindow {
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            to: NaiveDate::from_ymd_opt(2026, 11, 13).unwrap(),
        }
    }

    /// Strava's terms keep its data off every MCP and agent interface, so an
    /// external caller is refused before the provider is asked at all.
    #[tokio::test]
    async fn an_external_caller_is_refused_a_segments_efforts_before_any_call() {
        let (provider, calls) = governed();
        let (listed, _) = serve_over(
            Transport::McpHttp,
            ai_read(provider.segment_effort_history("611413", window())),
        )
        .await;
        assert_eq!(
            listed
                .expect_err("bug: Strava efforts reached an MCP caller")
                .code,
            ErrorCode::UnavailableOverTransport
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "bug: the provider was asked before the gate refused"
        );
    }

    #[tokio::test]
    async fn dravrs_own_surfaces_get_the_providers_listing() {
        let (provider, calls) = governed();
        let (listed, _) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                BTreeSet::new(),
                ai_read(provider.segment_effort_history("611413", window())),
            ),
        )
        .await;
        assert_eq!(
            listed.expect("bug: the athlete's own surface reads the listing"),
            vec![hit()],
            "bug: the wrapper must forward to the provider, not answer for it"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
