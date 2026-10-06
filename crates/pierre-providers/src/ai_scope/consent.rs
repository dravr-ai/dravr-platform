// ABOUTME: The athlete's consent to AI use of each provider's data, as the AI-read scope applies it
// ABOUTME: A provider without it is denied whole to every model read, recorded or relayed; undeclared withholds (carnet#726)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Withdrawable AI consent (carnet#726).
//!
//! A provider whose notice is a consent to AI use (WHOOP's owner
//! authorization) can be withdrawn by the athlete at any time. Each entry
//! point that reads for a model on an athlete's behalf declares which
//! providers lack that consent ([`with_ai_consent`]); the parent module's
//! filters then read the provider terms through
//! [`WithConsent`](pierre_core::ai_policy::WithConsent), so such a provider's
//! data is denied whole to the model. The athlete still sees it, and the
//! connection stays live. The parent module re-exports the public surface.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use pierre_core::ai_policy::Withheld;
use pierre_core::constants::oauth_providers::ai_consent_backends;
use pierre_core::errors::{AppError, AppResult};

use super::{exposure, record_withheld};

tokio::task_local! {
    /// The providers whose data the athlete has not consented to hand to AI,
    /// declared at the entry point that knows the athlete.
    static AI_CONSENT_WITHHELD: Arc<BTreeSet<String>>;
}

/// Run `fut` with `withheld` as the providers lacking the athlete's AI consent.
///
/// Those are the providers whose data the athlete has not consented to hand
/// to AI, or withdrew that consent for. Inside a read for a model, a provider
/// in it is denied whole, recorded or relayed; outside one it changes nothing.
/// A read for a model where nothing was declared withholds every provider
/// whose notice is a consent to AI use ([`ai_consent_backends`]): an entry
/// point that forgets withholds the data instead of handing it over.
pub async fn with_ai_consent<F: Future>(withheld: BTreeSet<String>, fut: F) -> F::Output {
    let withheld = withheld
        .into_iter()
        .map(|provider| provider.to_ascii_lowercase())
        .collect();
    AI_CONSENT_WITHHELD.scope(Arc::new(withheld), fut).await
}

/// The providers whose data the athlete has not consented to hand to AI.
///
/// As the current call declared them ([`with_ai_consent`]), or every provider
/// whose notice is a consent to AI use when it declared none.
#[must_use]
pub fn ai_consent_withheld() -> Arc<BTreeSet<String>> {
    AI_CONSENT_WITHHELD
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::new(ai_consent_backends().map(str::to_ascii_lowercase).collect()))
}

/// Refuse a model's untagged read of a provider lacking the athlete's AI consent.
///
/// For the reads that carry no item provenance (profile, stats, calendar),
/// when the athlete has not consented to AI use of `provider`'s data. The
/// refusal is tallied like a dropped item, so the result carries the neutral
/// withheld note rather than a bare failure.
///
/// # Errors
///
/// [`ErrorCode::ResourceNotFound`](pierre_core::errors::ErrorCode::ResourceNotFound)
/// when a model reads and `provider`'s AI consent is withheld.
pub fn consented_read(provider: &str) -> AppResult<()> {
    let for_model = exposure().is_some_and(|gate| gate.to_model);
    if !for_model || !ai_consent_withheld().contains(&provider.to_ascii_lowercase()) {
        return Ok(());
    }
    let mut withheld = Withheld {
        dropped: 1,
        ..Withheld::default()
    };
    withheld.sources.insert(provider.to_ascii_lowercase());
    record_withheld(withheld);
    Err(AppError::not_found(format!(
        "{provider} data is withheld from AI: the athlete has not consented to its AI use"
    )))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_core::transport::{Transport, TransportPolicy};

    use super::*;
    use crate::ai_scope::{ai_read, filter_activities, serve_over, tallied};
    use crate::provider_terms::WHOOP;

    /// WHOOP's declared terms, and no others.
    struct WhoopTerms;

    impl ProviderTerms for WhoopTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "whoop").then_some(&WHOOP)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "whoop").then_some(TransportPolicy::AnyTransport)
        }
    }

    fn workout(id: &str, provider: &str) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 6, 0, 0).unwrap();
        ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3600, provider).build()
    }

    fn week() -> Vec<Activity> {
        vec![workout("w", "whoop"), workout("g", "garmin")]
    }

    fn ids(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(Activity::id).collect()
    }

    #[tokio::test]
    async fn a_withheld_ai_consent_denies_the_provider_and_the_athlete_still_sees_it() {
        let withdrawn = BTreeSet::from(["WHOOP".to_owned()]);
        let (kept, withheld) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                withdrawn,
                ai_read(async { filter_activities(&WhoopTerms, week()) }),
            ),
        )
        .await;
        assert_eq!(ids(&kept), vec!["g"], "no WHOOP workout reaches the model");
        assert!(withheld.sources.contains("whoop"), "{:?}", withheld.sources);

        let consented = serve_over(
            Transport::WebApp,
            with_ai_consent(
                BTreeSet::new(),
                ai_read(async { filter_activities(&WhoopTerms, week()) }),
            ),
        )
        .await
        .0;
        assert_eq!(ids(&consented), vec!["w", "g"]);

        let route = filter_activities(&WhoopTerms, week());
        assert_eq!(
            route.len(),
            2,
            "the athlete's own screens keep every workout"
        );
    }

    #[tokio::test]
    async fn an_ai_read_that_declared_no_consent_withholds_every_ai_consent_provider() {
        assert!(ai_consent_backends().any(|backend| backend == "whoop"));
        let (kept, _) = serve_over(
            Transport::WebApp,
            ai_read(async { filter_activities(&WhoopTerms, week()) }),
        )
        .await;
        assert_eq!(
            ids(&kept),
            vec!["g"],
            "a forgotten declaration withholds, never hands over"
        );
    }

    #[tokio::test]
    async fn an_untagged_model_read_of_a_provider_without_consent_is_refused_and_tallied() {
        let withheld = || BTreeSet::from(["whoop".to_owned()]);
        let (refused, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(withheld(), ai_read(async { consented_read("whoop") })),
        )
        .await;
        assert_eq!(
            refused.expect_err("no stats for a model").code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(tally.dropped, 1);

        let (athlete, _) = tallied(with_ai_consent(withheld(), async {
            consented_read("whoop")
        }))
        .await;
        assert!(athlete.is_ok(), "outside a model read the athlete reads it");
    }
}
