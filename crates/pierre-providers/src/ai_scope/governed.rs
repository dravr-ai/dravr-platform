// ABOUTME: The AiGovernedProvider decorator every authenticated provider is wrapped in
// ABOUTME: Reads pass through the provider-terms filters under a gate; writes and auth pass straight through

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, Stats, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};

use super::{
    consented_read, filter_activities, filter_one, filter_planned_workouts, first_party_only_read,
    policies_apply, withheld_error,
};
use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
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

    async fn list_planned_workouts(
        &self,
        after: NaiveDate,
        before: NaiveDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        let workouts = self.inner.list_planned_workouts(after, before).await?;
        Ok(filter_planned_workouts(self.lookup(), workouts))
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.untagged_read()?;
        self.inner.list_calendar_events(from, to).await
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
