// ABOUTME: The live reads of a provider's own zones, best efforts, records, weight and gear, bounded and never fatal
// ABOUTME: Each block is the provider's, labelled as such; a read that is absent, unsupported, refused or late is left out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider context on a tool's answer (carnet#1622).
//!
//! Strava keeps figures Dravr does not compute: the time an activity spent in
//! each of its heart rate and pace zones, the fastest stretches in it, the
//! athlete's all-time records, their weight and their shoes and bikes. A tool
//! that analyses an activity or reads the athlete's profile carries them
//! beside its own answer, as the provider's:
//!
//! - **Live, never stored.** Every read happens on the tool call, and the
//!   result is neither cached nor written anywhere.
//! - **Never fatal.** Each read is bounded by [`CONTEXT_READ_BUDGET`] and runs
//!   beside the tool's own read, so a slow provider costs the answer neither
//!   its time budget nor its content. A read that finds nothing, a provider
//!   that serves no such figure, a caller the provider's terms refuse, and a
//!   refusal that is not worth a retry all leave the block out without a word.
//!   A retryable failure (a rate limit, a provider that did not answer, a
//!   read that ran out of time) is named in the block's `not_read`.
//! - **Counted per athlete.** Through a signed-in session each read is a page
//!   of the athlete's own session, admitted against that athlete's
//!   context-read windows ([`ContextLimits::budget`]): 200 a day, 30 in 15
//!   minutes. A read the windows refuse is a rate limit like any other.
//! - **Never a Dravr input.** The block is rendered for the model and read
//!   nowhere else: no zone calculation, physiology anchor or gear ledger
//!   takes it.

use std::future::Future;
use std::time::Duration;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::provider_context::{
    ProviderActivityContext, ProviderActivityZones, ProviderAthleteContext, ProviderGear,
    ProviderGearKind, ProviderPersonalRecord, ProviderWeight, PROVIDER_FIGURES_NOTE,
};
use pierre_providers::backend_resolver::{brand_name, user_facing_name};
use pierre_providers::core::FitnessProvider;
use pierre_providers::provider_context::is_unsupported;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{self, RequestBudget};
use tokio::time;
use tracing::{debug, warn};

/// The longest one context read may take.
///
/// Shorter than the refresh budget the tool's own read runs under
/// (`RefreshConfig::wait_for_refresh_timeout`): the context reads run beside
/// it, so even a provider that never answers cannot turn a good answer into a
/// late one.
pub(crate) const CONTEXT_READ_BUDGET: Duration = Duration::from_secs(8);

/// What bounds every context read of one answer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextLimits<'budget> {
    /// The longest one read may take, its admission included.
    pub(crate) time: Duration,
    /// The windows each read is admitted against first: the athlete's
    /// context-read windows when the provider is a signed-in session the
    /// limiter counts ([`ProviderRegistry::session_context_budget`]), else
    /// none.
    pub(crate) budget: Option<&'budget RequestBudget>,
}

/// How one context read came out.
#[derive(Debug, PartialEq)]
enum Read<T> {
    /// The provider reported a figure.
    Got(T),
    /// There is nothing to show: the provider holds none, serves none, or the
    /// read was refused for a reason a retry does not change.
    Nothing,
    /// The read failed in a way that may pass: a rate limit, a provider that
    /// did not answer, a read that ran out of time.
    Retryable,
}

/// Classify one bounded read of `provider`'s, admitted against
/// `limits.budget` first. An empty list is nothing to show, not a figure.
async fn read<T, F>(what: &str, provider: &str, limits: ContextLimits<'_>, call: F) -> Read<T>
where
    F: Future<Output = AppResult<Option<T>>>,
    T: HasContent,
{
    let admitted = async {
        request_budget::admit(limits.budget, provider).await?;
        call.await
    };
    match time::timeout(limits.time, admitted).await {
        Err(_) => {
            warn!(what, "a provider context read ran out of time");
            Read::Retryable
        }
        Ok(Ok(Some(value))) if value.has_content() => Read::Got(value),
        Ok(Ok(_)) => Read::Nothing,
        Ok(Err(e)) => classify_failure(what, &e),
    }
}

/// How a failed read counts: only a refusal that may pass is worth a retry.
fn classify_failure<T>(what: &str, error: &AppError) -> Read<T> {
    if is_unsupported(error) {
        return Read::Nothing;
    }
    if matches!(
        error.code,
        ErrorCode::ExternalRateLimited | ErrorCode::ExternalServiceUnavailable
    ) {
        warn!(what, error = %error, "a provider context read failed; a retry may answer");
        return Read::Retryable;
    }
    // Absent from this caller's reach (the terms, a bad id, a signed out
    // session the tool's own read reports): left out quietly.
    debug!(what, error = %error, "a provider context read is left out");
    Read::Nothing
}

/// Whether a read answer holds anything worth showing.
trait HasContent {
    fn has_content(&self) -> bool;
}

impl<T> HasContent for Vec<T> {
    fn has_content(&self) -> bool {
        !self.is_empty()
    }
}

impl HasContent for ProviderActivityZones {
    fn has_content(&self) -> bool {
        self.heart_rate
            .as_ref()
            .is_some_and(|zones| !zones.is_empty())
            || self.pace.as_ref().is_some_and(|zones| !zones.is_empty())
    }
}

impl HasContent for ProviderWeight {
    fn has_content(&self) -> bool {
        self.weight_kg.is_finite() && self.weight_kg > 0.0
    }
}

/// The provider as the athlete knows it (`Strava`), from the registry; the
/// user-facing slug when no descriptor names it. Never the backend's name.
pub(crate) fn source_label(registry: &ProviderRegistry, provider_name: &str) -> String {
    let slug = user_facing_name(provider_name);
    brand_name(registry, slug).unwrap_or(slug).to_owned()
}

/// Name `what` in `not_read` when its read failed in a way a retry may
/// answer; a figure read, or nothing to show, adds nothing.
fn note_retryable<T>(read: &Read<T>, what: &str, not_read: &mut Vec<String>) {
    if matches!(read, Read::Retryable) {
        not_read.push(what.to_owned());
    }
}

fn got<T>(read: Read<T>) -> Option<T> {
    match read {
        Read::Got(value) => Some(value),
        Read::Nothing | Read::Retryable => None,
    }
}

/// The provider's zones and best efforts for one of its activities, read
/// live and together. `None` when neither was read (a block with nothing in
/// it is left out).
///
/// `activity_id` must be the provider's own id for the activity: the caller
/// reads this only from the provider that owns the id, never from the copy an
/// upload stands in with.
pub(crate) async fn read_activity_context(
    provider: &dyn FitnessProvider,
    source: &str,
    activity_id: &str,
    limits: ContextLimits<'_>,
) -> Option<ProviderActivityContext> {
    let name = provider.name();
    let (zones, efforts) = tokio::join!(
        read(
            "activity zones",
            name,
            limits,
            provider.read_activity_zones(activity_id)
        ),
        read(
            "best efforts",
            name,
            limits,
            provider.read_activity_best_efforts(activity_id)
        ),
    );
    let mut not_read = Vec::new();
    note_retryable(&zones, "zones", &mut not_read);
    note_retryable(&efforts, "best efforts", &mut not_read);
    let zones = got(zones);
    let best_efforts = got(efforts);
    if zones.is_none() && best_efforts.is_none() {
        return None;
    }
    let (heart_rate_zones, pace_zones) = zones.map_or((None, None), |zones| {
        (
            zones.heart_rate.filter(|rows| !rows.is_empty()),
            zones.pace.filter(|rows| !rows.is_empty()),
        )
    });
    Some(ProviderActivityContext {
        source: source.to_owned(),
        note: PROVIDER_FIGURES_NOTE.to_owned(),
        heart_rate_zones,
        pace_zones,
        best_efforts,
        not_read,
    })
}

/// What the athlete's own Dravr gear ledger holds, as far as the provider's
/// shoe fallback needs to know. The ledger has no bike kind, so the
/// provider's bikes never depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedgerShoes {
    /// The athlete declared at least one pair in Dravr. Dravr's shoes win and
    /// the provider's are never shown.
    Declared,
    /// The athlete declared no shoes: the provider's stand in, read-only.
    Empty,
    /// The ledger could not be read. The provider's shoes are not shown: they
    /// would stand in for shoes the athlete may have declared.
    Unreadable,
}

/// The provider's gear as shown: bikes always (Dravr keeps none), shoes only
/// while the athlete declared none in Dravr.
fn shown_gear(gear: Vec<ProviderGear>, ledger: LedgerShoes) -> Option<Vec<ProviderGear>> {
    let shown: Vec<ProviderGear> = gear
        .into_iter()
        .filter(|g| g.kind == ProviderGearKind::Bike || ledger == LedgerShoes::Empty)
        .collect();
    (!shown.is_empty()).then_some(shown)
}

/// The provider's records, weight and gear (every bike; shoes while the
/// athlete declared none in Dravr), read live and together. `None` when
/// nothing was read.
pub(crate) async fn read_athlete_context(
    provider: &dyn FitnessProvider,
    source: &str,
    ledger: LedgerShoes,
    limits: ContextLimits<'_>,
) -> Option<ProviderAthleteContext> {
    let name = provider.name();
    let (weight, records, gear) = tokio::join!(
        read("weight", name, limits, provider.read_weight()),
        read::<Vec<ProviderPersonalRecord>, _>(
            "personal records",
            name,
            limits,
            provider.read_personal_records()
        ),
        read::<Vec<ProviderGear>, _>("gear", name, limits, provider.read_gear()),
    );
    let mut not_read = Vec::new();
    note_retryable(&weight, "weight", &mut not_read);
    note_retryable(&records, "personal records", &mut not_read);
    note_retryable(&gear, "gear", &mut not_read);
    let weight_kg = got(weight).map(|w| w.weight_kg);
    let personal_records = got(records);
    let gear = got(gear).and_then(|gear| shown_gear(gear, ledger));
    if weight_kg.is_none() && personal_records.is_none() && gear.is_none() {
        return None;
    }
    Some(ProviderAthleteContext {
        source: source.to_owned(),
        note: PROVIDER_FIGURES_NOTE.to_owned(),
        weight_kg,
        personal_records,
        gear,
        not_read,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pierre_core::models::provider_context::{ProviderBestEffort, ProviderHeartRateZone};
    use pierre_core::models::{Activity, Athlete};
    use pierre_core::pagination::{CursorPage, PaginationParams};
    use pierre_providers::core::{
        ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
    };
    use pierre_providers::provider_context::unsupported;
    use pierre_providers::registry::global_registry;
    use std::time::Instant;
    use tokio::time;

    /// What each context read of the stub answers with.
    #[derive(Clone)]
    enum Answer<T: Clone + Send + Sync> {
        Value(Option<T>),
        Unsupported,
        RateLimited,
        Refused,
        Slow,
    }

    impl<T: Clone + Send + Sync> Answer<T> {
        async fn play(&self) -> AppResult<Option<T>> {
            match self {
                Self::Value(v) => Ok(v.clone()),
                Self::Unsupported => Err(unsupported("sciotte_garmin", "zones")),
                Self::RateLimited => {
                    Err(AppError::new(ErrorCode::ExternalRateLimited, "slow down"))
                }
                Self::Refused => Err(AppError::new(
                    ErrorCode::UnavailableOverTransport,
                    "not over this transport",
                )),
                Self::Slow => {
                    time::sleep(Duration::from_secs(3600)).await;
                    Ok(None)
                }
            }
        }
    }

    struct Stub {
        config: ProviderConfig,
        zones: Answer<ProviderActivityZones>,
        efforts: Answer<Vec<ProviderBestEffort>>,
        weight: Answer<ProviderWeight>,
        records: Answer<Vec<ProviderPersonalRecord>>,
        gear: Answer<Vec<ProviderGear>>,
    }

    impl Stub {
        fn new() -> Self {
            Self {
                config: ProviderConfig {
                    name: "sciotte".to_owned(),
                    auth_url: String::new(),
                    token_url: String::new(),
                    api_base_url: String::new(),
                    revoke_url: None,
                    default_scopes: Vec::new(),
                },
                zones: Answer::Value(Some(hr_zones())),
                efforts: Answer::Value(Some(vec![effort()])),
                weight: Answer::Value(Some(ProviderWeight { weight_kg: 71.4 })),
                records: Answer::Value(Some(vec![record()])),
                gear: Answer::Value(Some(vec![shoe()])),
            }
        }
    }

    fn hr_zones() -> ProviderActivityZones {
        ProviderActivityZones {
            heart_rate: Some(vec![ProviderHeartRateZone {
                zone: 2,
                label: "Endurance".to_owned(),
                min_bpm: 120,
                max_bpm: Some(150),
                seconds: 1800.0,
            }]),
            pace: None,
        }
    }

    fn effort() -> ProviderBestEffort {
        ProviderBestEffort {
            label: "1K".to_owned(),
            distance_m: Some(1000.0),
            elapsed_seconds: 238,
            start_index: 10,
            end_index: 250,
        }
    }

    fn record() -> ProviderPersonalRecord {
        ProviderPersonalRecord {
            sport: "Run".to_owned(),
            label: "5K".to_owned(),
            distance_m: Some(5000.0),
            elapsed_seconds: Some(1212),
            value_m: None,
            activity_id: "9001".to_owned(),
        }
    }

    fn shoe() -> ProviderGear {
        ProviderGear {
            id: "g1".to_owned(),
            kind: ProviderGearKind::Shoe,
            name: "Pegasus".to_owned(),
            brand: None,
            model: None,
            total_distance_m: Some(412_000.0),
            primary: true,
            retired: false,
        }
    }

    #[async_trait]
    impl FitnessProvider for Stub {
        fn name(&self) -> &'static str {
            "sciotte"
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
            Err(AppError::internal("not read here"))
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
            Err(AppError::internal("not read here"))
        }
        async fn get_activity(&self, id: &str) -> AppResult<Activity> {
            Err(AppError::not_found(id.to_owned()))
        }
        async fn read_activity_zones(&self, _id: &str) -> AppResult<Option<ProviderActivityZones>> {
            self.zones.play().await
        }
        async fn read_activity_best_efforts(
            &self,
            _id: &str,
        ) -> AppResult<Option<Vec<ProviderBestEffort>>> {
            self.efforts.play().await
        }
        async fn read_weight(&self) -> AppResult<Option<ProviderWeight>> {
            self.weight.play().await
        }
        async fn read_personal_records(&self) -> AppResult<Option<Vec<ProviderPersonalRecord>>> {
            self.records.play().await
        }
        async fn read_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
            self.gear.play().await
        }
    }

    const FAST: ContextLimits<'static> = ContextLimits {
        time: Duration::from_millis(100),
        budget: None,
    };

    #[tokio::test]
    async fn an_activity_carries_the_providers_zones_and_best_efforts_labelled() {
        let stub = Stub::new();
        let block = read_activity_context(&stub, "Strava", "9001", FAST)
            .await
            .expect("both reads answered");
        assert_eq!(
            block.source, "Strava",
            "bug: the block is not labelled with its provider"
        );
        assert_eq!(
            block.note, PROVIDER_FIGURES_NOTE,
            "bug: the block lost its quoting note"
        );
        assert_eq!(
            block.heart_rate_zones.expect("hr zones").len(),
            1,
            "bug: the provider's zones were altered"
        );
        assert!(
            block.pace_zones.is_none(),
            "a family not reported is absent"
        );
        assert_eq!(
            block.best_efforts.expect("efforts")[0].label,
            "1K",
            "bug: the provider's best effort was altered"
        );
        assert_eq!(
            block.not_read,
            Vec::<String>::new(),
            "bug: a read that answered was named as not read"
        );
    }

    #[tokio::test]
    async fn a_read_that_is_absent_unsupported_or_refused_leaves_the_block_out() {
        for answer in [
            Answer::Value(None),
            Answer::Value(Some(ProviderActivityZones::default())),
            Answer::Unsupported,
            Answer::Refused,
        ] {
            let mut stub = Stub::new();
            stub.zones = answer;
            stub.efforts = Answer::Value(Some(Vec::new()));
            assert!(
                read_activity_context(&stub, "Strava", "9001", FAST)
                    .await
                    .is_none(),
                "bug: a block with nothing in it was shown"
            );
        }
    }

    #[tokio::test]
    async fn one_read_failing_keeps_the_other_and_names_what_may_pass() {
        let mut stub = Stub::new();
        stub.zones = Answer::RateLimited;
        let block = read_activity_context(&stub, "Strava", "9001", FAST)
            .await
            .expect("the best efforts answered");
        assert!(
            block.heart_rate_zones.is_none(),
            "bug: a rate limited read produced zones"
        );
        assert!(
            block.best_efforts.is_some(),
            "bug: one failed read dropped the other"
        );
        assert_eq!(
            block.not_read,
            vec!["zones".to_owned()],
            "bug: the retryable read was not named"
        );
    }

    #[tokio::test]
    async fn a_slow_read_is_cut_at_the_budget_and_the_other_still_answers() {
        let mut stub = Stub::new();
        stub.efforts = Answer::Slow;
        let started = Instant::now();
        let block = read_activity_context(&stub, "Strava", "9001", FAST)
            .await
            .expect("the zones answered");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bug: waited for a slow read"
        );
        assert!(
            block.best_efforts.is_none(),
            "bug: a read cut at the budget produced efforts"
        );
        assert_eq!(
            block.not_read,
            vec!["best efforts".to_owned()],
            "bug: the late read was not named"
        );
    }

    #[tokio::test]
    async fn the_athlete_carries_weight_records_and_gear_when_none_is_declared() {
        let stub = Stub::new();
        let block = read_athlete_context(&stub, "Strava", LedgerShoes::Empty, FAST)
            .await
            .expect("answered");
        assert_eq!(
            block.weight_kg,
            Some(71.4),
            "bug: the provider's weight was altered"
        );
        assert_eq!(
            block.personal_records.expect("records")[0].label,
            "5K",
            "bug: the provider's record was altered"
        );
        assert_eq!(
            block.gear.expect("gear")[0].name,
            "Pegasus",
            "bug: the provider's shoes were not shown with no Dravr shoes declared"
        );
    }

    #[tokio::test]
    async fn declared_or_unreadable_dravr_shoes_hide_the_providers_but_never_its_bikes() {
        for ledger in [LedgerShoes::Declared, LedgerShoes::Unreadable] {
            let mut stub = Stub::new();
            let mut bike = shoe();
            bike.id = "b1".to_owned();
            bike.kind = ProviderGearKind::Bike;
            stub.gear = Answer::Value(Some(vec![shoe(), bike]));
            let block = read_athlete_context(&stub, "Strava", ledger, FAST)
                .await
                .expect("weight, records and the bike answered");
            let gear = block.gear.expect("the bike is shown");
            assert_eq!(gear.len(), 1, "bug: Strava's shoes over {ledger:?}");
            assert_eq!(
                gear[0].kind,
                ProviderGearKind::Bike,
                "bug: the provider's bike was hidden over {ledger:?}"
            );
            assert_eq!(
                block.weight_kg,
                Some(71.4),
                "bug: the gear rule touched the weight"
            );
        }
    }

    #[tokio::test]
    async fn declared_shoes_and_no_bike_leave_no_gear_block() {
        let stub = Stub::new();
        let block = read_athlete_context(&stub, "Strava", LedgerShoes::Declared, FAST)
            .await
            .expect("weight and records answered");
        assert!(
            block.gear.is_none(),
            "bug: Strava's shoes over declared ones"
        );
    }

    #[tokio::test]
    async fn an_athlete_with_nothing_to_show_has_no_block() {
        let mut stub = Stub::new();
        stub.weight = Answer::Unsupported;
        stub.records = Answer::Value(Some(Vec::new()));
        stub.gear = Answer::Value(Some(Vec::new()));
        assert_eq!(
            read_athlete_context(&stub, "Strava", LedgerShoes::Empty, FAST).await,
            None,
            "bug: a block with nothing in it was shown"
        );
    }

    #[test]
    fn the_source_is_the_brand_never_the_backend() {
        let registry = global_registry();
        assert_eq!(
            source_label(&registry, "sciotte"),
            "Strava",
            "bug: the backend slug labels the provider's figures"
        );
        assert_eq!(
            source_label(&registry, "strava"),
            "Strava",
            "bug: the API slug labels the provider's figures"
        );
    }

    #[test]
    fn the_provider_context_types_serialise_without_absent_fields() {
        let block = ProviderActivityContext {
            source: "Strava".to_owned(),
            note: PROVIDER_FIGURES_NOTE.to_owned(),
            heart_rate_zones: None,
            pace_zones: None,
            best_efforts: Some(vec![effort()]),
            not_read: Vec::new(),
        };
        let json = serde_json::to_value(&block).expect("serialises");
        assert!(
            json.get("heart_rate_zones").is_none(),
            "bug: an absent family serialised"
        );
        assert!(
            json.get("not_read").is_none(),
            "bug: an empty not_read serialised"
        );
        assert!(
            json.get("best_efforts").is_some(),
            "bug: a read figure was dropped"
        );
    }
}
