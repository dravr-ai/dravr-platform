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
//! first-party ([`TransportPolicy::FirstPartyOnly`](pierre_core::transport::TransportPolicy::FirstPartyOnly))
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
//!
//! # Provenance (carnet#769)
//!
//! Content derived from the athlete's data — a reply, a fact, a plan — is
//! stamped with the [`TransportPolicy`] of what it was built from. A turn (or
//! any derivation) runs under [`tracking`], which scopes a [`Provenance`]: a
//! positive accumulator every filter point marks when it serves an item whose
//! terms keep it first-party, and every stored-content reader marks when it
//! serves a row already so stamped ([`admit_derived`]). The writer then stamps
//! its row with [`Provenance::policy`]. The accumulator is shared through an
//! `Arc`, so an executor built inside the turn carries it onto the task its
//! calls run on (the Copilot loop's loopback listener) and marks the turn's
//! own flag from there.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pierre_core::ai_policy::{
    filter_items, first_party_only, Exposure, ProviderTerms, WithConsent, Withheld,
};
use pierre_core::constants::oauth_providers::{
    garmin_device, source_attribution, GARMIN_ATTRIBUTION,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, PlannedWorkout};
use pierre_core::transport::{Transport, TransportPolicy};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::task::LocalKey;

/// The athlete's withdrawable consent to AI use of each provider's data.
mod consent;
/// The decorator that applies the provider-terms filters to every read.
mod governed;
/// Provider-terms filtering of stored health records.
mod stored_health;

pub use consent::{ai_consent_withheld, consented_read, with_ai_consent};
pub use governed::AiGovernedProvider;
pub use stored_health::{filter_stored_health, StoredHealthRecord};

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
    /// What the current derivation has served, for the row it will write.
    static PROVENANCE: Provenance;
}

/// Whether a turn, or any other derivation, has served data whose terms keep
/// it first-party (carnet#769).
///
/// A positive accumulator: it starts clear and only ever gets set. Cloning
/// shares the flag — the `Arc` is what lets an executor built inside a turn
/// mark the turn's own flag from the task its calls run on.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// Set once anything first-party-only was served. Shared between the turn
    /// and every executor and nested derivation it hands a clone to.
    served_first_party_only: Arc<AtomicBool>,
    /// The attributions of what was served (`"Garmin"` for Garmin
    /// device-sourced data): what a reply derived from it must say it drew on
    /// (carnet#521). Shared the same way as the flag.
    attributions: Arc<Mutex<BTreeSet<&'static str>>>,
}

impl Provenance {
    /// A clear accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that first-party-only data was served.
    pub fn mark(&self) {
        self.served_first_party_only.store(true, Ordering::Release);
    }

    /// Record that data carrying `attribution` was served.
    pub fn attribute(&self, attribution: &'static str) {
        if let Ok(mut attributions) = self.attributions.lock() {
            attributions.insert(attribution);
        }
    }

    /// The attributions of everything served so far.
    #[must_use]
    pub fn attributions(&self) -> BTreeSet<&'static str> {
        self.attributions
            .lock()
            .map(|attributions| attributions.clone())
            .unwrap_or_default()
    }

    /// The policy a row derived from what was served so far is stamped with.
    #[must_use]
    pub fn policy(&self) -> TransportPolicy {
        TransportPolicy::from_first_party_only(self.served_first_party_only.load(Ordering::Acquire))
    }
}

/// Run `fut` accumulating what it serves into `provenance`.
///
/// Whatever the enclosing derivation was accumulating is marked too when
/// `fut` serves first-party-only data: content derived inside derived content
/// taints both.
pub async fn tracking<F: Future>(provenance: Provenance, fut: F) -> F::Output {
    let enclosing = current_provenance();
    let output = PROVENANCE.scope(provenance.clone(), fut).await;
    if let Some(enclosing) = enclosing {
        if provenance.policy().is_first_party_only() {
            enclosing.mark();
        }
        for attribution in provenance.attributions() {
            enclosing.attribute(attribution);
        }
    }
    output
}

/// Run `fut` under a fresh accumulator, returning the policy what it derived
/// must be stamped with.
pub async fn derived<F: Future>(fut: F) -> (F::Output, TransportPolicy) {
    let provenance = Provenance::new();
    let output = tracking(provenance.clone(), fut).await;
    (output, provenance.policy())
}

/// The accumulator of the derivation running here, if any.
#[must_use]
pub fn current_provenance() -> Option<Provenance> {
    PROVENANCE.try_with(Clone::clone).ok()
}

/// The policy content derived here so far must be stamped with:
/// [`TransportPolicy::AnyTransport`] outside any tracked derivation.
#[must_use]
pub fn derived_policy() -> TransportPolicy {
    current_provenance().map_or(TransportPolicy::AnyTransport, |p| p.policy())
}

/// Mark the derivation running here as having served first-party-only data.
///
/// Nothing is marked under [`unfiltered`]: a cache write holds the athlete's
/// full data for later reads and serves no one, and its caller filters — and
/// marks — what it hands back.
pub fn mark_first_party_only_served() {
    if flag(&UNFILTERED) {
        return;
    }
    let _ = PROVENANCE.try_with(Provenance::mark);
}

/// Mark the running derivation when an item from `provider` (upstream
/// `source`) is first-party-only.
fn note_served(lookup: &dyn ProviderTerms, provider: &str, source: Option<&str>) {
    if PROVENANCE.try_with(|_| ()).is_ok() && first_party_only(lookup, provider, source) {
        mark_first_party_only_served();
    }
    // What it was recorded by, for the attribution its derivation owes
    // (carnet#521).
    note_attribution(source_attribution(source));
}

/// Record `attribution` on the running derivation.
///
/// For a reader that knows what served data owes beyond its item's own
/// `source` (an athlete's wellness rows, by the device their activities
/// name). Not under `unfiltered`, for the reason marking is not.
pub fn note_attribution(attribution: Option<&'static str>) {
    if let Some(attribution) = attribution {
        if !flag(&UNFILTERED) {
            let _ = PROVENANCE.try_with(|provenance| provenance.attribute(attribution));
        }
    }
}

/// Whether stored content stamped `policy` may be served here, marking the
/// running derivation when it is first-party-only content that is served.
///
/// Over an external transport, content stamped
/// [`TransportPolicy::FirstPartyOnly`] is withheld; everywhere else it is
/// served, and whatever is derived from it inherits the stamp.
#[must_use]
pub fn admit_derived(policy: TransportPolicy) -> bool {
    if !policy.is_first_party_only() {
        return true;
    }
    if exposure().is_some_and(|gate| gate.external) {
        return false;
    }
    mark_first_party_only_served();
    true
}

/// The strictest stamp a stored row may carry and still be read here.
///
/// [`TransportPolicy::AnyTransport`] over an external transport (only
/// unstamped rows), [`TransportPolicy::FirstPartyOnly`] everywhere else.
///
/// A list read binds it into its SQL so its `LIMIT` counts only rows the
/// caller may read; the reader still passes the rows through
/// [`retain_admitted`], which stamps a first-party derivation built on them.
#[must_use]
pub fn readable_policy() -> TransportPolicy {
    if serving_external() {
        TransportPolicy::AnyTransport
    } else {
        TransportPolicy::FirstPartyOnly
    }
}

/// Keep only the stored items whose stamp [`admit_derived`] serves here.
pub fn retain_admitted<T>(items: &mut Vec<T>, policy: impl Fn(&T) -> TransportPolicy) {
    items.retain(|item| admit_derived(policy(item)));
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

/// Whether what is read now is served over an external transport.
#[must_use]
pub fn serving_external() -> bool {
    exposure().is_some_and(|gate| gate.external)
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
    let kept = match exposure() {
        None => items,
        Some(exposure) => {
            // The athlete's AI consents ride on the provider terms (carnet#726).
            let consent = ai_consent_withheld();
            let terms = WithConsent {
                terms: lookup,
                withheld: &consent,
            };
            let (kept, withheld) = filter_items(&terms, items, &origin, required, exposure);
            record_withheld(withheld);
            kept
        }
    };
    for item in &kept {
        let (provider, source) = origin(item);
        note_served(lookup, &provider, source.as_deref());
    }
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
    let external = exposure().is_some_and(|gate| gate.external);
    items
        .into_iter()
        .filter(|item| {
            let (provider, source) = origin(item);
            if !first_party_only(lookup, provider, source) {
                return true;
            }
            if external {
                return false;
            }
            mark_first_party_only_served();
            true
        })
        .collect()
}

/// Activities as a model, or a caller over an external transport, may see
/// them; unchanged when no gate applies.
#[must_use]
pub fn filter_activities(lookup: &dyn ProviderTerms, activities: Vec<Activity>) -> Vec<Activity> {
    let kept = filter_typed(
        lookup,
        activities,
        |a| (a.provider().to_owned(), a.source().map(str::to_owned)),
        &["name"],
    );
    // A Garmin device's recording owes the attribution whichever service
    // relayed it; its `source` names only the relay (carnet#521).
    for activity in &kept {
        if garmin_device(activity.device_name()) {
            note_attribution(Some(GARMIN_ATTRIBUTION));
        }
    }
    kept
}

/// Planned workouts as a model, or a caller over an external transport, may
/// see them; unchanged when no gate applies.
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

/// One activity as the gates in force let its reader see it: `None` when a
/// policy drops it.
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
    if !first_party_only(lookup, provider, source) {
        return Ok(());
    }
    if exposure().is_some_and(|gate| gate.external) {
        return Err(AppError::unavailable_over_transport(
            "this data is not available over this interface",
        ));
    }
    // Served: whatever is derived from it inherits the stamp (carnet#769).
    mark_first_party_only_served();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig};
    use crate::provider_terms::{NOLIO, NOLIO_TRANSPORT};
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_core::models::{Athlete, Stats};
    use pierre_core::pagination::{CursorPage, PaginationParams};
    use pierre_core::transport::TransportPolicy;

    #[cfg(all(feature = "provider-strava", feature = "provider-sciotte"))]
    use crate::registry::ProviderRegistry;

    struct NolioOnly;

    impl ProviderTerms for NolioOnly {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "nolio").then_some(&NOLIO)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "nolio").then_some(NOLIO_TRANSPORT)
        }
    }

    /// A read made from Dravr's web app, for an athlete who consented to AI
    /// use of every provider's data (carnet#726).
    pub(super) fn first_party<F: Future>(fut: F) -> impl Future<Output = F::Output> {
        serve_over(Transport::WebApp, with_ai_consent(BTreeSet::new(), fut))
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

    #[tokio::test]
    async fn serving_a_first_party_only_item_stamps_the_derivation_and_withholding_it_does_not() {
        let provider = Relay::governed();
        let (_, served) = derived(first_party(ai_read(provider.get_activities(None, None)))).await;
        assert_eq!(served, TransportPolicy::FirstPartyOnly);

        let (_, external) = derived(serve_over(
            Transport::McpHttp,
            ai_read(provider.get_activities(None, None)),
        ))
        .await;
        assert_eq!(
            external,
            TransportPolicy::AnyTransport,
            "nothing first-party-only reached the caller"
        );

        let (_, route) = derived(provider.get_activities(None, None)).await;
        assert_eq!(
            route,
            TransportPolicy::FirstPartyOnly,
            "an unfiltered first-party read still taints what is derived from it"
        );

        let (_, cache_write) = derived(unfiltered(provider.get_activities(None, None))).await;
        assert_eq!(
            cache_write,
            TransportPolicy::AnyTransport,
            "a cache write serves no one"
        );

        let (_, stats) = derived(first_party(ai_read(provider.get_stats()))).await;
        assert_eq!(
            stats,
            TransportPolicy::FirstPartyOnly,
            "a read with no item provenance is the relay's"
        );
    }

    #[tokio::test]
    async fn stamped_content_is_withheld_externally_and_taints_what_is_built_from_it() {
        let (admitted, policy) = derived(first_party(async {
            admit_derived(TransportPolicy::FirstPartyOnly)
        }))
        .await;
        assert!(admitted);
        assert_eq!(policy, TransportPolicy::FirstPartyOnly);

        let (admitted, policy) = derived(serve_over(Transport::ApiKey, async {
            admit_derived(TransportPolicy::FirstPartyOnly)
        }))
        .await;
        assert!(!admitted, "an external caller never reads stamped content");
        assert_eq!(policy, TransportPolicy::AnyTransport);

        let (admitted, policy) = derived(serve_over(Transport::ApiKey, async {
            admit_derived(TransportPolicy::AnyTransport)
        }))
        .await;
        assert!(admitted, "unstamped content is served everywhere");
        assert_eq!(policy, TransportPolicy::AnyTransport);

        let mut rows = vec![
            ("kept", TransportPolicy::AnyTransport),
            ("stamped", TransportPolicy::FirstPartyOnly),
        ];
        serve_over(Transport::A2a, async {
            retain_admitted(&mut rows, |row| row.1);
        })
        .await;
        assert_eq!(rows, vec![("kept", TransportPolicy::AnyTransport)]);

        assert_eq!(
            serve_over(Transport::McpHttp, async { readable_policy() }).await,
            TransportPolicy::AnyTransport,
            "an external list reads unstamped rows only"
        );
        assert_eq!(
            first_party(async { readable_policy() }).await,
            TransportPolicy::FirstPartyOnly
        );
        assert_eq!(
            readable_policy(),
            TransportPolicy::FirstPartyOnly,
            "a job outside any declaration reads every row"
        );
    }

    #[tokio::test]
    async fn a_nested_derivation_taints_its_parent_and_a_clone_marks_from_another_task() {
        let (inner, outer) =
            derived(async { derived(async { mark_first_party_only_served() }).await.1 }).await;
        assert_eq!(inner, TransportPolicy::FirstPartyOnly);
        assert_eq!(outer, TransportPolicy::FirstPartyOnly);

        let provenance = Provenance::new();
        let carried = tracking(provenance.clone(), async {
            current_provenance().expect("scoped")
        })
        .await;
        tokio::spawn(async move { carried.mark() }).await.unwrap();
        assert_eq!(provenance.policy(), TransportPolicy::FirstPartyOnly);
        assert_eq!(
            derived_policy(),
            TransportPolicy::AnyTransport,
            "outside any derivation nothing is stamped"
        );
    }

    #[tokio::test]
    async fn a_derivation_notes_the_garmin_attribution_of_what_it_served() {
        let provider = Relay::governed();
        let outer = Provenance::new();
        tracking(outer.clone(), async {
            let inner = Provenance::new();
            let _ = tracking(
                inner.clone(),
                first_party(ai_read(provider.get_activities(None, None))),
            )
            .await;
            assert_eq!(inner.attributions(), BTreeSet::from(["Garmin"]));
        })
        .await;
        assert_eq!(
            outer.attributions(),
            BTreeSet::from(["Garmin"]),
            "a nested derivation's attributions reach its parent"
        );

        let cache_write = Provenance::new();
        let _ = tracking(
            cache_write.clone(),
            unfiltered(provider.get_activities(None, None)),
        )
        .await;
        assert!(
            cache_write.attributions().is_empty(),
            "a cache write serves no one"
        );
    }

    /// The same Strava session read through the API and through the sciotte
    /// Strava backend, which stamps it `sciotte` with `source = "strava"`.
    #[cfg(all(feature = "provider-strava", feature = "provider-sciotte"))]
    fn strava_sessions() -> Vec<Activity> {
        let start = Utc.with_ymd_and_hms(2026, 10, 6, 6, 0, 0).unwrap();
        vec![
            ActivityBuilder::new("api", "Tempo", SportType::Run, start, 3600, "strava").build(),
            ActivityBuilder::new("scraped", "Tempo", SportType::Run, start, 3600, "sciotte")
                .source("strava")
                .build(),
        ]
    }

    /// Strava API Policy §5.16 and §2.3 (carnet#765), through the registry
    /// the server builds: no MCP, A2A or API-key caller gets Strava data,
    /// whichever backend read it; Dravr's own surfaces get all of it.
    #[cfg(all(feature = "provider-strava", feature = "provider-sciotte"))]
    #[tokio::test]
    async fn strava_data_direct_or_scraped_is_served_only_to_dravrs_own_surfaces() {
        let registry = ProviderRegistry::new();
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            let served = serve_over(transport, async {
                filter_activities(&registry, strava_sessions())
            })
            .await;
            assert!(served.is_empty(), "{transport:?}: an API read");

            let (for_model, withheld) = serve_over(
                transport,
                with_ai_consent(
                    BTreeSet::new(),
                    ai_read(async { filter_activities(&registry, strava_sessions()) }),
                ),
            )
            .await;
            assert!(for_model.is_empty(), "{transport:?}: an AI read");
            assert_eq!(withheld.off_interface, 2, "{transport:?}");

            for backend in ["strava", "sciotte"] {
                let untagged = serve_over(transport, async {
                    first_party_only_read(&registry, backend, None)
                })
                .await;
                assert_eq!(
                    untagged.expect_err("an unstamped read is refused").code,
                    ErrorCode::UnavailableOverTransport,
                    "{transport:?}: {backend}"
                );
            }
        }

        for transport in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
            Transport::PlatformJob,
        ] {
            let served = serve_over(transport, async {
                filter_activities(&registry, strava_sessions())
            })
            .await;
            assert_eq!(ids(&served), vec!["api", "scraped"], "{transport:?}");

            let (for_model, withheld) = serve_over(
                transport,
                with_ai_consent(
                    BTreeSet::new(),
                    ai_read(async { filter_activities(&registry, strava_sessions()) }),
                ),
            )
            .await;
            assert_eq!(ids(&for_model), vec!["api", "scraped"], "{transport:?}");
            assert!(withheld.is_empty(), "{transport:?}");

            for backend in ["strava", "sciotte"] {
                let untagged = serve_over(transport, async {
                    first_party_only_read(&registry, backend, None)
                })
                .await;
                assert!(untagged.is_ok(), "{transport:?}: {backend}");
            }
        }
    }
}
