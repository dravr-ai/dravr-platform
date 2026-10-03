// ABOUTME: When a provider read is for a model or served over an external transport — the scopes, tally and escapes
// ABOUTME: Typed provider-terms filters and the AiGovernedProvider decorator every authenticated provider is wrapped in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a model may see of the athlete's provider data (carnet#723).
//!
//! A read is **for a model** when it runs inside [`ai_read`] — the tool
//! executor scopes every tool body in it, and a prompt builder scopes its own
//! reads with [`for_model`]. Inside that scope the provider policies
//! ([`crate::provider_terms`]) apply; outside it (the athlete's web and
//! mobile routes, webhooks, sync sweeps) nothing is filtered, so the athlete
//! always sees their own data.
//!
//! Each entry point also declares the [`Transport`] it serves ([`serve_over`],
//! carnet#724). On an external call, a provider whose terms keep its data
//! first-party ([`TransportPolicy::FirstPartyOnly`](pierre_core::transport::TransportPolicy))
//! has its items dropped whole, whether a model reads them or not. Inside an AI
//! read, a call that declared no transport is external: an entry point that
//! forgets to declare withholds data instead of leaking it. Outside one, only a
//! declared external transport gates anything, so the athlete's own routes,
//! webhooks and sweeps stay unfiltered.
//!
//! [`unfiltered`] lifts every policy, for writing the activity cache (which
//! must hold the athlete's full data); a writer fetches under it, writes, and
//! then filters what it hands back. [`for_display`] lifts only the AI rules,
//! for a chart the athlete sees: a chart served over an external transport is
//! still gated.
//!
//! Live reads are governed in one place: every provider a tool obtains is an
//! [`AiGovernedProvider`], which filters each read it serves under a gate.

use std::cell::RefCell;
use std::future::Future;
use std::mem;
use std::sync::Arc;

use tokio::task::LocalKey;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::ai_policy::{filter_items, first_party_only, Exposure, ProviderTerms, Withheld};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, PlannedSession, PlannedWorkout, Stats, TimeSeriesData,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::transport::Transport;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
};

tokio::task_local! {
    /// Set while a read is for a model.
    static FOR_MODEL: bool;
    /// Tallies what the policies held back, for the innermost scope that asked.
    static TALLY: RefCell<Withheld>;
    /// Set while every policy is lifted (cache writes).
    static UNFILTERED: bool;
    /// Set while the AI rules are lifted for output the athlete sees.
    static DISPLAY: bool;
    /// The transport the current call serves, declared at its entry point.
    static TRANSPORT: Transport;
}

/// Run `fut` as a read for a model, returning what the policies withheld.
///
/// The tool executor wraps every tool body in this; the tally becomes the
/// neutral note on the tool's result.
pub async fn ai_read<F: Future>(fut: F) -> (F::Output, Withheld) {
    FOR_MODEL.scope(true, tallied(fut)).await
}

/// Run `fut`, returning what the policies held back from its reads.
///
/// Its reads are not made reads for a model: whatever gates already apply
/// here still do, and no other. The caller decides what the tally means, and
/// passes it on with [`record_withheld`].
pub async fn tallied<F: Future>(fut: F) -> (F::Output, Withheld) {
    TALLY
        .scope(RefCell::new(Withheld::default()), async {
            let output = fut.await;
            let withheld = TALLY.with(RefCell::take);
            (output, withheld)
        })
        .await
}

/// Run a prompt builder's reads as reads for a model. The tally is dropped:
/// a prompt has no tool result to annotate.
pub async fn for_model<F: Future>(fut: F) -> F::Output {
    ai_read(fut).await.0
}

/// Lift every policy, for a cache write: the cache holds the athlete's full
/// data, and the writer filters what it returns.
pub fn unfiltered<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    UNFILTERED.scope(true, fut)
}

/// Lift the AI rules for output the athlete sees and the model does not.
///
/// For a map drawn from an activity's streams. Anything a model reads must not
/// run under this. The transport gate stays: a chart served over an external
/// transport never carries a first-party-only provider's data.
pub fn for_display<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    DISPLAY.scope(true, fut)
}

/// Run `fut` as a call served over `transport`.
///
/// A declaration only narrows: inside a call already served over an external
/// transport, the work stays external whatever it declares. The enclosing
/// declaration is read when the future runs, not when it is built, so a
/// future built outside a scope and awaited inside it is still narrowed.
pub async fn serve_over<F: Future>(transport: Transport, fut: F) -> F::Output {
    let effective = declared_transport().map_or(transport, |outer| outer.narrowed_by(transport));
    TRANSPORT.scope(effective, fut).await
}

/// The transport declared for the current call, if any.
#[must_use]
pub fn declared_transport() -> Option<Transport> {
    TRANSPORT.try_with(|transport| *transport).ok()
}

/// The transport a call bound to `bound` serves when it runs here: the
/// enclosing declaration narrowed by `bound`, or whichever of the two exists.
#[must_use]
pub fn effective_transport(bound: Option<Transport>) -> Option<Transport> {
    match (declared_transport(), bound) {
        (Some(outer), Some(inner)) => Some(outer.narrowed_by(inner)),
        (outer, inner) => outer.or(inner),
    }
}

fn flag(key: &'static LocalKey<bool>) -> bool {
    key.try_with(|set| *set).unwrap_or(false)
}

/// The gates a read made now crosses, or `None` when none applies.
#[must_use]
pub fn exposure() -> Option<Exposure> {
    let for_model = flag(&FOR_MODEL);
    gates(for_model, declared_transport())
}

/// The gates a tool result crosses on its way to the caller, for an executor
/// that served `transport` (`None`: undeclared, so external). Read after the
/// tool body, outside the scope it ran in.
#[must_use]
pub fn result_exposure(transport: Option<Transport>) -> Option<Exposure> {
    gates(true, transport)
}

fn gates(for_model: bool, transport: Option<Transport>) -> Option<Exposure> {
    if flag(&UNFILTERED) {
        return None;
    }
    let external = transport.map_or(for_model, |t| !t.is_first_party());
    let exposure = Exposure {
        to_model: for_model && !flag(&DISPLAY),
        external,
    };
    exposure.any().then_some(exposure)
}

/// Whether a read made now crosses any gate.
#[must_use]
pub fn policies_apply() -> bool {
    exposure().is_some()
}

/// Add `withheld` to the enclosing tally — for a read that tallied in a scope
/// of its own. Nothing happens where no tally is kept.
pub fn record_withheld(withheld: Withheld) {
    if !withheld.is_empty() {
        let _ = TALLY.try_with(|tally| tally.borrow_mut().merge(withheld));
    }
}

fn filter_typed<T, F>(
    lookup: &dyn ProviderTerms,
    items: Vec<T>,
    origin: F,
    required: &[&str],
) -> Vec<T>
where
    T: Serialize + DeserializeOwned,
    F: Fn(&T) -> (String, Option<String>),
{
    let Some(exposure) = exposure() else {
        return items;
    };
    let (kept, withheld) = filter_items(lookup, items, origin, required, exposure);
    record_withheld(withheld);
    kept
}

/// The items this call may serve to a reader that is not a model.
///
/// Over an external transport, those whose terms keep them first-party are
/// dropped; otherwise every item is kept as it is. `origin` names each item's
/// provider and upstream source.
///
/// For a route that reads stored rows outside every filter point: dropped
/// before any merge, a withheld copy of a workout can never fill the fields of
/// a served one (carnet#724).
pub fn retain_served_here<T, F>(lookup: &dyn ProviderTerms, items: Vec<T>, origin: F) -> Vec<T>
where
    F: Fn(&T) -> (&str, Option<&str>),
{
    if !exposure().is_some_and(|gate| gate.external) {
        return items;
    }
    items
        .into_iter()
        .filter(|item| {
            let (provider, source) = origin(item);
            !first_party_only(lookup, provider, source)
        })
        .collect()
}

/// Activities as a model, or a caller over an external transport, may see
/// them; unchanged when no gate applies.
#[must_use]
pub fn filter_activities(lookup: &dyn ProviderTerms, activities: Vec<Activity>) -> Vec<Activity> {
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
    lookup: &dyn ProviderTerms,
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
fn filter_one(lookup: &dyn ProviderTerms, activity: Activity) -> Option<Activity> {
    filter_activities(lookup, vec![activity]).pop()
}

fn withheld_error(id: &str) -> AppError {
    AppError::not_found(format!(
        "activity {id} is withheld by the terms of the service it came from"
    ))
}

/// Refuse a read of `provider`'s data (upstream `source`) its terms keep off
/// this transport.
///
/// With its own code, never an auth or reconnect one: the connection is fine.
/// Covers the reads whose result carries no item provenance (profile, stats,
/// calendar) and single-item reads, and a tool that serves such a read from a
/// cache before any provider is asked.
///
/// # Errors
///
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode::UnavailableOverTransport)
/// when the call is external and the data's terms keep it first-party.
pub fn first_party_only_read(
    lookup: &dyn ProviderTerms,
    provider: &str,
    source: Option<&str>,
) -> AppResult<()> {
    let off_interface =
        exposure().is_some_and(|gate| gate.external) && first_party_only(lookup, provider, source);
    if off_interface {
        return Err(AppError::unavailable_over_transport(
            "this data is not available over this interface",
        ));
    }
    Ok(())
}

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
        first_party_only_read(self.lookup(), self.inner.name(), None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_terms::{NOLIO, NOLIO_TRANSPORT};
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_core::transport::TransportPolicy;

    struct NolioOnly;

    impl ProviderTerms for NolioOnly {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "nolio").then_some(&NOLIO)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "nolio").then_some(NOLIO_TRANSPORT)
        }
    }

    /// A read made from Dravr's web app.
    fn first_party<F: Future>(fut: F) -> impl Future<Output = F::Output> {
        serve_over(Transport::WebApp, fut)
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
        let (activities, withheld) =
            first_party(ai_read(provider.get_activities(None, None))).await;
        let activities = activities.unwrap();

        assert_eq!(ids(&activities), vec!["g", "s"], "zepp is denied");
        assert_eq!(activities[0].average_heart_rate(), Some(150));
        assert_eq!(
            activities[1].average_heart_rate(),
            None,
            "strava: existence only"
        );
        assert_eq!((withheld.dropped, withheld.reduced), (1, 1));

        let (page, _) = first_party(ai_read(
            provider.get_activities_cursor(&PaginationParams::forward(None, 10)),
        ))
        .await;
        let page = page.unwrap();
        assert_eq!(ids(&page.items), vec!["g", "s"]);
        assert_eq!(page.count, 2, "the count follows the filter");
    }

    #[tokio::test]
    async fn a_denied_single_read_is_refused_and_streams_follow_their_activity() {
        let provider = Relay::governed();
        let (denied, _) = first_party(ai_read(provider.get_activity("z"))).await;
        assert!(denied.is_err());
        let (garmin, _) = first_party(ai_read(provider.get_activity_detailed("g"))).await;
        assert_eq!(garmin.unwrap().average_heart_rate(), Some(150));
        let (streams, _) = first_party(ai_read(provider.get_activity_streams("z"))).await;
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

        let external_cache_write = serve_over(
            Transport::McpHttp,
            unfiltered(provider.get_activities(None, None)),
        )
        .await
        .unwrap();
        assert_eq!(
            ids(&external_cache_write),
            vec!["g", "s", "z"],
            "a cache write keeps every row whichever transport triggered it"
        );
    }

    #[tokio::test]
    async fn an_external_call_gets_none_of_a_first_party_only_relay() {
        let provider = Relay::governed();
        let (activities, withheld) = serve_over(
            Transport::McpHttp,
            ai_read(provider.get_activities(None, None)),
        )
        .await;
        assert!(activities.unwrap().is_empty());
        assert_eq!(withheld.off_interface, 3);
        assert!(!withheld.from_model(), "nothing reached the AI rules");

        let (one, _) = serve_over(Transport::A2a, ai_read(provider.get_activity("g"))).await;
        assert_eq!(
            one.expect_err("a single read is refused too").code,
            ErrorCode::UnavailableOverTransport
        );
    }

    #[tokio::test]
    async fn an_undeclared_ai_read_is_external_and_an_undeclared_route_is_not() {
        let provider = Relay::governed();
        let (activities, withheld) = ai_read(provider.get_activities(None, None)).await;
        assert!(
            activities.unwrap().is_empty(),
            "no declaration inside an AI read means external"
        );
        assert_eq!(withheld.off_interface, 3);

        let route_read = provider.get_activities(None, None).await.unwrap();
        assert_eq!(
            ids(&route_read),
            vec!["g", "s", "z"],
            "the athlete's own routes declare nothing and see everything"
        );

        let api_key_route = serve_over(Transport::ApiKey, provider.get_activities(None, None))
            .await
            .unwrap();
        assert!(
            api_key_route.is_empty(),
            "a route declared external is gated outside an AI read"
        );
    }

    #[tokio::test]
    async fn a_chart_lifts_the_ai_rules_but_never_the_gate() {
        let provider = Relay::governed();
        let (chart, _) =
            first_party(for_display(ai_read(provider.get_activities(None, None)))).await;
        let chart = chart.unwrap();
        assert_eq!(ids(&chart), vec!["g", "s", "z"]);
        assert_eq!(
            chart[1].average_heart_rate(),
            Some(150),
            "the athlete's chart keeps every value"
        );

        let (external_chart, withheld) = serve_over(
            Transport::McpHttp,
            for_display(ai_read(provider.get_activities(None, None))),
        )
        .await;
        assert!(external_chart.unwrap().is_empty());
        assert_eq!(withheld.off_interface, 3);
    }

    #[tokio::test]
    async fn a_first_party_declaration_inside_an_external_call_stays_external() {
        let provider = Relay::governed();
        let nested = serve_over(
            Transport::McpHttp,
            first_party(ai_read(provider.get_activities(None, None))),
        )
        .await;
        assert!(nested.0.unwrap().is_empty());
        assert_eq!(
            serve_over(Transport::A2a, first_party(async { declared_transport() })).await,
            Some(Transport::A2a)
        );
    }

    #[tokio::test]
    async fn reads_without_item_provenance_are_refused_with_their_own_code() {
        let provider = Relay::governed();
        let (stats, _) = serve_over(Transport::McpHttp, ai_read(provider.get_stats())).await;
        let error = stats.expect_err("an external call reads no first-party-only stats");
        assert_eq!(error.code, ErrorCode::UnavailableOverTransport);

        let (profile, _) = serve_over(Transport::ApiKey, ai_read(provider.get_athlete())).await;
        assert_eq!(
            profile.expect_err("nor its profile").code,
            ErrorCode::UnavailableOverTransport
        );

        let (first_party_stats, _) = first_party(ai_read(provider.get_stats())).await;
        assert_eq!(
            first_party_stats
                .expect_err("the relay itself serves no stats")
                .code,
            ErrorCode::InternalError,
            "a first-party read reaches the provider"
        );
    }
}
