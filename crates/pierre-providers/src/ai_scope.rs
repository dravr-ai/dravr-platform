// ABOUTME: When a provider read is for a model — the AI-read scope, its withheld tally, and the unfiltered escape
// ABOUTME: Typed AI-policy filters and the AiGovernedProvider decorator every authenticated provider is wrapped in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a model may see of the athlete's provider data (carnet#723).
//!
//! A read is **for a model** when it runs inside [`ai_read`] — the tool
//! executor scopes every tool body in it, and a prompt builder scopes its own
//! reads with [`for_model`]. Inside that scope the provider policies
//! ([`crate::provider_ai_terms`]) apply; outside it (the athlete's web and
//! mobile routes, webhooks, sync sweeps) nothing is filtered, so the athlete
//! always sees their own data.
//!
//! [`unfiltered`] lifts the policy inside an AI read, for exactly two kinds of
//! work: writing the activity cache (which must hold the athlete's full data)
//! and drawing a chart the athlete sees ([`for_display`]). A writer fetches
//! under it, writes, and then filters what it hands back.
//!
//! Live reads are governed in one place: every provider a tool obtains is an
//! [`AiGovernedProvider`], which filters each read it serves inside an AI read.

use std::cell::RefCell;
use std::future::Future;
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::ai_policy::{filter_items, AiPolicyLookup, Withheld};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, Stats, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
};

tokio::task_local! {
    /// Present while a read is for a model; tallies what the policies held back.
    static AI_READ: RefCell<Withheld>;
    /// Set while the policies are lifted inside an AI read (cache writes, display).
    static UNFILTERED: bool;
}

/// Run `fut` as a read for a model, returning what the policies withheld.
///
/// The tool executor wraps every tool body in this; the tally becomes the
/// neutral note on the tool's result.
pub async fn ai_read<F: Future>(fut: F) -> (F::Output, Withheld) {
    AI_READ
        .scope(RefCell::new(Withheld::default()), async {
            let output = fut.await;
            let withheld = AI_READ.with(RefCell::take);
            (output, withheld)
        })
        .await
}

/// Run a prompt builder's reads as reads for a model. The tally is dropped:
/// a prompt has no tool result to annotate.
pub async fn for_model<F: Future>(fut: F) -> F::Output {
    ai_read(fut).await.0
}

/// Lift the policies inside an AI read, for a cache write: the cache holds the
/// athlete's full data, and the writer filters what it returns.
pub fn unfiltered<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    UNFILTERED.scope(true, fut)
}

/// Lift the policies for output the athlete sees and the model does not — a
/// map drawn from an activity's streams. Anything a model reads must not run
/// under this.
pub fn for_display<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    unfiltered(fut)
}

/// Whether the policies are lifted here ([`unfiltered`] or [`for_display`]).
#[must_use]
pub fn lifted() -> bool {
    UNFILTERED.try_with(|flag| *flag).unwrap_or(false)
}

/// Whether a read made now is for a model and the policies apply.
#[must_use]
pub fn policies_apply() -> bool {
    AI_READ.try_with(|_| ()).is_ok() && !lifted()
}

/// Add `withheld` to the current AI read's tally — for a read that tallied in a
/// scope of its own. Nothing happens outside an AI read.
pub fn record_withheld(withheld: Withheld) {
    if !withheld.is_empty() {
        let _ = AI_READ.try_with(|tally| tally.borrow_mut().merge(withheld));
    }
}

fn filter_typed<T, F>(
    lookup: &dyn AiPolicyLookup,
    items: Vec<T>,
    origin: F,
    required: &[&str],
) -> Vec<T>
where
    T: Serialize + DeserializeOwned,
    F: Fn(&T) -> (String, Option<String>),
{
    if !policies_apply() {
        return items;
    }
    let (kept, withheld) = filter_items(lookup, items, origin, required);
    record_withheld(withheld);
    kept
}

/// Activities as a model may see them, inside an AI read; unchanged outside.
#[must_use]
pub fn filter_activities(lookup: &dyn AiPolicyLookup, activities: Vec<Activity>) -> Vec<Activity> {
    filter_typed(
        lookup,
        activities,
        |a| (a.provider().to_owned(), a.source().map(str::to_owned)),
        &["name"],
    )
}

/// Planned workouts as a model may see them, inside an AI read.
#[must_use]
pub fn filter_planned_workouts(
    lookup: &dyn AiPolicyLookup,
    workouts: Vec<PlannedWorkout>,
) -> Vec<PlannedWorkout> {
    filter_typed(
        lookup,
        workouts,
        |w| (w.provider().to_owned(), w.source().map(str::to_owned)),
        &["title"],
    )
}

/// One activity as a model may see it: `None` when its policy drops it.
fn filter_one(lookup: &dyn AiPolicyLookup, activity: Activity) -> Option<Activity> {
    filter_activities(lookup, vec![activity]).pop()
}

fn withheld_error(id: &str) -> AppError {
    AppError::not_found(format!(
        "activity {id} is withheld from AI by the terms of the service it came from"
    ))
}

/// A provider whose reads, inside an AI read, return only what each
/// provider's AI policy lets a model see. Writes and auth pass straight
/// through, and outside an AI read so does everything.
pub struct AiGovernedProvider {
    inner: Box<dyn FitnessProvider>,
    policies: Arc<dyn AiPolicyLookup>,
}

impl AiGovernedProvider {
    /// Govern `inner`'s reads by the policies `policies` declares (the provider registry).
    #[must_use]
    pub fn wrap(
        inner: Box<dyn FitnessProvider>,
        policies: Arc<dyn AiPolicyLookup>,
    ) -> Box<dyn FitnessProvider> {
        Box::new(Self { inner, policies })
    }

    fn lookup(&self) -> &dyn AiPolicyLookup {
        self.policies.as_ref()
    }

    fn governed_one(&self, id: &str, activity: Activity) -> AppResult<Activity> {
        filter_one(self.lookup(), activity).ok_or_else(|| withheld_error(id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_ai_terms::NOLIO;
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{ActivityBuilder, SportType};

    struct NolioOnly;

    impl AiPolicyLookup for NolioOnly {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "nolio").then_some(&NOLIO)
        }
    }

    fn activity(id: &str, source: &str) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 6, 0, 0).unwrap();
        ActivityBuilder::new(id, "Hills", SportType::Ride, start, 3600, "nolio")
            .average_heart_rate(150)
            .source(source)
            .build()
    }

    /// A Nolio account relaying one Garmin, one Strava and one Zepp session.
    struct Relay {
        config: ProviderConfig,
    }

    impl Relay {
        fn governed() -> Box<dyn FitnessProvider> {
            let relay = Self {
                config: ProviderConfig {
                    name: "nolio".to_owned(),
                    auth_url: String::new(),
                    token_url: String::new(),
                    api_base_url: String::new(),
                    revoke_url: None,
                    default_scopes: Vec::new(),
                },
            };
            AiGovernedProvider::wrap(Box::new(relay), Arc::new(NolioOnly))
        }

        fn all() -> Vec<Activity> {
            vec![
                activity("g", "garmin"),
                activity("s", "strava"),
                activity("z", "zepp"),
            ]
        }
    }

    #[async_trait]
    impl FitnessProvider for Relay {
        fn name(&self) -> &'static str {
            "nolio"
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
            Ok(Self::all())
        }

        async fn get_activities_cursor(
            &self,
            _params: &PaginationParams,
        ) -> AppResult<CursorPage<Activity>> {
            Ok(CursorPage::new(Self::all(), None, None, false))
        }

        async fn get_activity(&self, id: &str) -> AppResult<Activity> {
            Self::all()
                .into_iter()
                .find(|a| a.id() == id)
                .ok_or_else(|| AppError::not_found(id.to_owned()))
        }

        async fn get_stats(&self) -> AppResult<Stats> {
            Err(AppError::internal("not used by these tests"))
        }
    }

    fn ids(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(Activity::id).collect()
    }

    #[tokio::test]
    async fn an_ai_read_sees_what_the_policy_permits() {
        let provider = Relay::governed();
        let (activities, withheld) = ai_read(provider.get_activities(None, None)).await;
        let activities = activities.unwrap();

        assert_eq!(ids(&activities), vec!["g", "s"], "zepp is denied");
        assert_eq!(activities[0].average_heart_rate(), Some(150));
        assert_eq!(
            activities[1].average_heart_rate(),
            None,
            "strava: existence only"
        );
        assert_eq!((withheld.dropped, withheld.reduced), (1, 1));

        let (page, _) =
            ai_read(provider.get_activities_cursor(&PaginationParams::forward(None, 10))).await;
        let page = page.unwrap();
        assert_eq!(ids(&page.items), vec!["g", "s"]);
        assert_eq!(page.count, 2, "the count follows the filter");
    }

    #[tokio::test]
    async fn a_denied_single_read_is_refused_and_streams_follow_their_activity() {
        let provider = Relay::governed();
        let (denied, _) = ai_read(provider.get_activity("z")).await;
        assert!(denied.is_err());
        let (garmin, _) = ai_read(provider.get_activity_detailed("g")).await;
        assert_eq!(garmin.unwrap().average_heart_rate(), Some(150));
        let (streams, _) = ai_read(provider.get_activity_streams("z")).await;
        assert!(
            streams.is_err(),
            "a denied activity's streams are refused too"
        );
    }

    #[tokio::test]
    async fn outside_an_ai_read_and_under_unfiltered_everything_passes() {
        let provider = Relay::governed();
        let athlete_view = provider.get_activities(None, None).await.unwrap();
        assert_eq!(ids(&athlete_view), vec!["g", "s", "z"]);

        let (cache_write, withheld) =
            ai_read(unfiltered(provider.get_activities(None, None))).await;
        assert_eq!(ids(&cache_write.unwrap()), vec!["g", "s", "z"]);
        assert!(withheld.is_empty());
    }
}
