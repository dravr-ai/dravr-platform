// ABOUTME: The AiGovernedProvider decorator every authenticated provider is wrapped in
// ABOUTME: Reads pass through the provider-terms filters under a gate; writes and auth pass straight through

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::CivilDate;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::provider_context::{
    ProviderActivitySearch, ProviderActivitySearchPage, ProviderActivityZones, ProviderBestEffort,
    ProviderGear, ProviderPersonalRecord, ProviderWeight,
};
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, LibraryWorkoutRef, PlannedSession, PlannedWorkout,
    ProviderThresholds, SportType, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};

use super::{
    consented_read, filter_activities, filter_one, filter_planned_workouts, first_party_only_read,
    note_raw_rows, policies_apply, withheld_error,
};
use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    TokenRefreshCallback,
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
        // belong to and let its policy, and a grant's window, decide.
        let activity = self.get_activity_with_streams(id).await?;
        let streams = activity.time_series_data().cloned();
        // Each sample is a raw row of a grant's daily budget.
        note_raw_rows(streams.as_ref().map_or(0, |series| series.timestamps.len()));
        Ok(streams)
    }

    async fn read_thresholds(
        &self,
        primary_sport: Option<&SportType>,
    ) -> AppResult<ProviderThresholds> {
        self.untagged_read()?;
        self.inner.read_thresholds(primary_sport).await
    }

    /// Gated as a read without item provenance: what the provider says about
    /// the athlete is its own data, so its terms decide before it is asked.
    async fn read_activity_zones(&self, id: &str) -> AppResult<Option<ProviderActivityZones>> {
        self.untagged_read()?;
        self.inner.read_activity_zones(id).await
    }

    async fn read_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
        self.untagged_read()?;
        self.inner.read_gear().await
    }

    async fn read_personal_records(&self) -> AppResult<Option<Vec<ProviderPersonalRecord>>> {
        self.untagged_read()?;
        self.inner.read_personal_records().await
    }

    async fn read_activity_best_efforts(
        &self,
        id: &str,
    ) -> AppResult<Option<Vec<ProviderBestEffort>>> {
        self.untagged_read()?;
        self.inner.read_activity_best_efforts(id).await
    }

    async fn read_weight(&self) -> AppResult<Option<ProviderWeight>> {
        self.untagged_read()?;
        self.inner.read_weight().await
    }

    /// The rows are activities, so each is judged by its own provenance as a
    /// list read judges it.
    async fn search_activities(
        &self,
        search: &ProviderActivitySearch,
    ) -> AppResult<ProviderActivitySearchPage> {
        let mut page = self.inner.search_activities(search).await?;
        page.activities = filter_activities(self.lookup(), mem::take(&mut page.activities));
        Ok(page)
    }

    async fn list_planned_workouts(
        &self,
        after: CivilDate,
        before: CivilDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        let workouts = self.inner.list_planned_workouts(after, before).await?;
        Ok(filter_planned_workouts(self.lookup(), workouts))
    }

    fn calendar_key(&self, external_id: &str) -> String {
        self.inner.calendar_key(external_id)
    }

    async fn list_calendar_events(
        &self,
        from: CivilDate,
        to: CivilDate,
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

    async fn save_library_workout(
        &self,
        session: &PlannedSession,
        folder: &str,
    ) -> AppResult<LibraryWorkoutRef> {
        self.inner.save_library_workout(session, folder).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
    use pierre_core::errors::{AppError, AppResult, ErrorCode};
    use pierre_core::models::provider_context::ProviderGear;
    use pierre_core::models::{Activity, Athlete, ProviderThresholds, SportType};
    use pierre_core::pagination::{CursorPage, PaginationParams};
    use pierre_core::transport::{Transport, TransportPolicy};

    use super::AiGovernedProvider;
    use crate::ai_scope::{ai_read, serve_over, with_ai_consent};
    use crate::core::{ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig};
    use crate::provider_context::is_unsupported;

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

    /// A Strava provider that reads its thresholds, a read with no item
    /// provenance, and counts the reads it was asked for.
    struct Reading {
        config: ProviderConfig,
        calls: Arc<AtomicUsize>,
    }

    fn thresholds() -> ProviderThresholds {
        ProviderThresholds {
            origin: "strava",
            ftp_watts: None,
            threshold_hr: None,
            max_hr: None,
            threshold_pace_sec_per_km: None,
        }
    }

    #[async_trait]
    impl FitnessProvider for Reading {
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

        async fn read_thresholds(
            &self,
            _primary_sport: Option<&SportType>,
        ) -> AppResult<ProviderThresholds> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(thresholds())
        }

        async fn read_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(Vec::new()))
        }
    }

    fn governed() -> (Box<dyn FitnessProvider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Reading {
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

    /// Strava's terms keep its data off every MCP and agent interface, so an
    /// external caller is refused a read without item provenance before the
    /// provider is asked at all.
    #[tokio::test]
    async fn an_external_caller_is_refused_an_untagged_read_before_any_call() {
        let (provider, calls) = governed();
        let (read, _) =
            serve_over(Transport::McpHttp, ai_read(provider.read_thresholds(None))).await;
        assert_eq!(
            read.expect_err("bug: Strava thresholds reached an MCP caller")
                .code,
            ErrorCode::UnavailableOverTransport,
            "bug: the refusal is the transport's, not the provider's"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "bug: the provider was asked before the gate refused"
        );
    }

    /// Dravr's own surfaces get the provider's untagged read, forwarded once.
    #[tokio::test]
    async fn dravrs_own_surfaces_get_the_providers_untagged_read() {
        let (provider, calls) = governed();
        let (read, _) = serve_over(
            Transport::WebApp,
            with_ai_consent(BTreeSet::new(), ai_read(provider.read_thresholds(None))),
        )
        .await;
        assert_eq!(
            read.expect("bug: the athlete's own surface reads the thresholds"),
            thresholds(),
            "bug: the wrapper must forward to the provider, not answer for it"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "bug: the wrapper asks the provider exactly once"
        );
    }

    /// What a provider says about the athlete is its own data: Strava's terms
    /// keep it off an external caller before the provider is asked.
    #[tokio::test]
    async fn an_external_caller_is_refused_the_providers_context_before_any_call() {
        let (provider, calls) = governed();
        let (gear, _) = serve_over(Transport::McpHttp, ai_read(provider.read_gear())).await;
        assert_eq!(
            gear.expect_err("bug: Strava's gear reached an MCP caller")
                .code,
            ErrorCode::UnavailableOverTransport,
            "bug: the refusal is not the transport's"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "bug: the provider was asked first"
        );
    }

    #[tokio::test]
    async fn dravrs_own_surfaces_get_the_providers_context_and_its_refusals() {
        let (provider, calls) = governed();
        let (gear, _) = serve_over(
            Transport::WebApp,
            with_ai_consent(BTreeSet::new(), ai_read(provider.read_gear())),
        )
        .await;
        assert_eq!(
            gear.expect("bug: the athlete's own surface reads gear"),
            Some(Vec::new()),
            "bug: the wrapper altered the provider's answer"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "bug: the wrapper asks the provider exactly once"
        );
        // A provider that serves no weight says so through the wrapper.
        let (weight, _) = serve_over(
            Transport::WebApp,
            with_ai_consent(BTreeSet::new(), ai_read(provider.read_weight())),
        )
        .await;
        let refusal = weight.expect_err("bug: the default read answered");
        assert!(is_unsupported(&refusal), "{refusal}");
    }
}
