// ABOUTME: What each provider's terms allow for its data — the declared AI SourcePolicy, TransportPolicy and cache TTL
// ABOUTME: WHOOP keeps scores out of prompts; Strava and WHOOP stay first-party; Nolio caches 7 days

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Declared provider terms.
//!
//! An AI policy for each provider whose terms restrict AI use, a transport
//! policy for each whose terms keep its data inside Dravr's own surfaces, and
//! a cache TTL for each whose terms cap how long a copy may be held.
//!
//! A descriptor returns them from
//! [`ProviderDescriptor::ai_policy`](crate::spi::ProviderDescriptor::ai_policy),
//! [`ProviderDescriptor::transport_policy`](crate::spi::ProviderDescriptor::transport_policy)
//! and
//! [`ProviderDescriptor::cache_ttl`](crate::spi::ProviderDescriptor::cache_ttl);
//! a provider absent here has no restriction. Adding a provider's terms is a
//! policy here and one descriptor line — no tool changes.
//!
//! Strava's June 2026 terms bar AI use, but direct Strava data stays allowed
//! by product decision (2026-10-02) until that question is settled on its own.
//! Those terms also keep Strava data off every MCP and agent interface, so
//! Strava — through its API or the sciotte scraper — is served only to Dravr's
//! own surfaces ([`STRAVA_TRANSPORT`]). WHOOP's descriptor keeps its data
//! first-party too ([`WHOOP_TRANSPORT`], carnet#766). Garmin, COROS and
//! `TrainingPeaks` keep their data first-party; intervals.icu serves its own
//! over every transport and settles the Garmin-recorded data it relays by its
//! own terms (carnet#767; read: dravr-vault `Work Log/2026-10/Provider
//! Transport Terms — Garmin, COROS, TrainingPeaks, intervals.icu
//! (2026-10-07).md`). No shipped provider's descriptor declares a cache TTL:
//! enforcing Strava's 7-day cache cap is a product decision
//! not yet taken, and WHOOP's tier-1–2 measurements are kept under the athlete's
//! owner authorization (carnet#539).

use std::time::Duration;

use pierre_core::ai_policy::{AiUse, SourcePolicy};
use pierre_core::transport::TransportPolicy;

/// WHOOP's own scores never reach a prompt; its measurements do.
///
/// Recovery, strain, sleep performance and the other scores WHOOP computes
/// are WHOOP's (API Terms effective 2026-10-06, §4; carnet#539). The
/// athlete's measurements (heart rate, HRV, durations, temperatures) stay
/// available. Health sync already drops the scores at ingestion
/// (`whoop_terms`); this is the check at the model.
pub const WHOOP: SourcePolicy = SourcePolicy {
    direct: AiUse::Redact(&[
        "recovery_score",
        "readiness_score",
        "sleep_score",
        "stress_score",
        "body_battery",
        "daily_strain",
        "strain",
    ]),
    by_source: &[],
    other_sources: AiUse::Allow,
};

/// Strava API Policy (effective 2026-06-01), §5.16: Strava data stays inside
/// Dravr's own surfaces.
///
/// No operating "any MCP Server, agent-mediated interface, or analogous
/// mechanism that exposes … Strava Data, or any subset thereof", nor any
/// layer that re-exposes it to third parties, under whatever protocol name.
/// §2.3 lets Strava data be displayed only to the athlete it belongs to, and
/// §5.10 says the athlete's consent does not lift either rule, so the
/// athlete's own API key is no exception. Both the `strava` API provider and
/// the sciotte Strava backend declare it: the scraper reads the same data,
/// under the stricter §5.5 ban on scraping besides (carnet#765).
///
/// Read: dravr-vault `Work Log/2026-08/Strava June 2026 API Terms — Legal
/// Read and Action Plan (2026-08-16).md`, Part 2.
#[cfg(any(feature = "provider-strava", feature = "provider-sciotte"))]
pub const STRAVA_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// WHOOP API Terms (effective 2026-10-06): WHOOP data stays inside Dravr's
/// own surfaces.
///
/// §3 bars redistributing or syndicating access to WHOOP's API materials and
/// sublicensing them for use by a third party; §4 bars exposing WHOOP Data
/// "to other users or to third parties without explicit opt-in consent from
/// the user that provided the WHOOP Data", and distributing it to a third
/// party. Dravr records no such opt-in for an external client, so an MCP,
/// A2A or API-key caller gets no WHOOP item — live reads and the stored
/// sleep, recovery and body records health sync persisted alike
/// (carnet#766). Display in Dravr's own web, mobile and messaging surfaces
/// stays allowed.
///
/// Read: dravr-vault `Compliance/WHOOP API Terms of Use — archived text
/// (effective 2026-10-06).md`, lines 37-39 and 59-62.
pub const WHOOP_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// What a Nolio connector item keeps when only its existence may be used:
/// identity, sport, date and duration — never names, notes or values.
const NOLIO_EXISTENCE: &[&str] = &[
    "id",
    "provider",
    "source",
    "sport_type",
    "start_date",
    "date",
    "duration_seconds",
    "provider_workout_id",
];

/// Nolio API terms v1.0, Annex: connector data keeps its upstream `source`,
/// and the Annex restricts it by that source.
/// - `garmin`: allowed (attribution is a separate duty).
/// - `strava`, `whoop`: never used with AI; existence only.
/// - `oura`: arrives without values; existence only.
/// - `zepp`, `huawei`: detail allowed for display, never submitted to AI.
///
/// Read: dravr-vault `Work Log/2026-10/Nolio API Terms — Legal Read and
/// Integration Assessment (2026-10-01).md`, Part 3 E1.
// LIMITATION(registre#657): `NOLIO` is returned by no descriptor — no Nolio provider exists, so
// only tests exercise this policy.
pub const NOLIO: SourcePolicy = SourcePolicy {
    direct: AiUse::Allow,
    by_source: &[
        ("garmin", AiUse::Allow),
        ("strava", AiUse::ExistenceOnly(NOLIO_EXISTENCE)),
        ("whoop", AiUse::ExistenceOnly(NOLIO_EXISTENCE)),
        ("oura", AiUse::ExistenceOnly(NOLIO_EXISTENCE)),
        ("zepp", AiUse::Deny),
        ("huawei", AiUse::Deny),
    ],
    other_sources: AiUse::Allow,
};

/// Nolio API terms v1.0, §6.9: Nolio data stays inside Dravr's own surfaces.
///
/// No making it available to third parties — through Dravr's own API,
/// outbound notifications, an MCP server, a feed or an export — "including in
/// derived or aggregated form", without Nolio's prior written agreement.
/// Display in Dravr's own web, mobile and messaging surfaces is allowed.
///
/// Read: the same terms read as [`NOLIO`], Part 3 E7.
// LIMITATION(registre#657): `NOLIO_TRANSPORT` is returned by no descriptor — no Nolio provider
// exists, so only tests exercise this policy.
pub const NOLIO_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// Nolio API terms v1.0, §7.1: a transient copy of Nolio data, any cache
/// included, is held at most seven days.
///
/// Read: the same terms read as [`NOLIO`], Part 3 E4.
// LIMITATION(registre#657): `NOLIO_CACHE_TTL` is returned by no descriptor — no Nolio provider
// exists, so only tests exercise this cap.
pub const NOLIO_CACHE_TTL: Duration = Duration::from_hours(7 * 24);

/// Garmin Connect Developer Program Agreement (Garmin International, form
/// FRM-0952 Rev. B): Garmin data stays inside Dravr's own surfaces.
///
/// §4.1(b) licenses data from the API "solely for internal business
/// purposes" and only "to format and display such data" in the licensee's
/// applications; §5.2(o) forbids making the API's functionality available to
/// any third party, including by "developing a Licensee application
/// programming interface to provide End User Data to third-party websites,
/// software applications, platforms, services, or products that have not been
/// approved by Garmin in writing" — what an MCP server, A2A or an API key
/// is. Binds `garmin` and the `sciotte_garmin` read of Garmin Connect alike
/// (§5.2(j) bars scraping outright).
pub const GARMIN_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// COROS API and Data Sharing Agreement (template, 2026-04-03): COROS data
/// stays inside Dravr's own surfaces.
///
/// §5.1(c) licenses displaying COROS Data "to the applicable End User within
/// the Company Application"; §8.14 forbids transferring it "to any third
/// party except as expressly allowed by this Agreement", and §12 allows
/// disclosure only to bound service providers, never to "unrelated third
/// parties" (§12.2). Binds `coros`, the `sciotte_coros` Training Hub read,
/// and the health rows synced from it.
///
/// Read: dravr-vault `Work Log/2026-09/COROS API Agreement — Legal Read and
/// Sync Assessment (2026-09-22).md`; the text is under `Compliance/COROS/`.
pub const COROS_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// `TrainingPeaks` API Terms: `TrainingPeaks` data stays inside Dravr's own
/// surfaces.
///
/// The summary commits a client to "not share data without consent" and to
/// "not scrape" (dravr-vault `TP Terms and conditions.md:16`); the Terms
/// forbid exposing a user's Content "to other users or to third parties
/// without explicit opt-in consent from that user" (line 168) and conveying
/// or distributing it to any third party (lines 135, 179). Binds the
/// `sciotte_trainingpeaks` read, the coach roster read through it, and the
/// planned workouts stamped `trainingpeaks`.
pub const TRAININGPEAKS_TRANSPORT: TransportPolicy = TransportPolicy::FirstPartyOnly;

/// intervals.icu API Terms and Conditions (effective 2025-10-23,
/// <https://forum.intervals.icu/t/114087>): its data is served over every
/// transport.
///
/// The licence is "for any lawful purpose, including commercial use", and
/// "you may integrate, modify, distribute, and sublicense outputs derived
/// from the API without restriction or attribution", save the Garmin
/// attribution [`INTERVALS_ICU_TRANSPORT_BY_SOURCE`] answers.
pub const INTERVALS_ICU_TRANSPORT: TransportPolicy = TransportPolicy::AnyTransport;

/// The relayed sources intervals.icu's terms settle by name.
///
/// Its terms address Garmin-sourced data explicitly — "if your application
/// displays information derived from Garmin-sourced data, you must display
/// attribution to Garmin" — and otherwise license distribution without
/// restriction. Dravr's contract for a Garmin-recorded activity intervals.icu
/// relays is therefore intervals.icu's; Garmin's developer agreement binds
/// intervals.icu, its licensee, not Dravr. The attribution is owed
/// separately (carnet#521). A source the terms do not name keeps its own
/// provider's policy.
pub const INTERVALS_ICU_TRANSPORT_BY_SOURCE: &[(&str, TransportPolicy)] =
    &[("garmin", TransportPolicy::AnyTransport)];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProviderRegistry;
    use pierre_core::ai_policy::{filter_json, Exposure, ProviderTerms};
    use serde_json::{json, Value};

    /// A read for a model on one of Dravr's own surfaces.
    const MODEL: Exposure = Exposure {
        to_model: true,
        external: false,
    };

    /// The registry's resolution, with Nolio registered beside WHOOP and Strava.
    struct Registered;

    impl ProviderTerms for Registered {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "nolio" => Some(&NOLIO),
                "whoop" => Some(&WHOOP),
                "garmin" | "strava" => Some(&SourcePolicy::ALLOW_ALL),
                _ => None,
            }
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            match provider {
                "nolio" => Some(NOLIO_TRANSPORT),
                "whoop" => Some(WHOOP_TRANSPORT),
                "garmin" => Some(GARMIN_TRANSPORT),
                "strava" => Some(TransportPolicy::AnyTransport),
                _ => None,
            }
        }
    }

    fn activity(id: &str, source: &str, extra: &Value) -> Value {
        let mut item = json!({
            "id": id, "provider": "nolio", "source": source, "name": "Session",
            "sport_type": "Ride", "start_date": "2026-09-30T06:00:00Z",
            "duration_seconds": 3600, "average_heart_rate": 148, "description": "felt good"
        });
        if let (Some(item), Some(extra)) = (item.as_object_mut(), extra.as_object()) {
            item.extend(extra.clone());
        }
        item
    }

    #[test]
    fn the_nolio_annex_reaches_the_model_payload() {
        let none = json!({});
        let mut payload = json!({ "activities": [
            activity("g", "garmin", &none),
            activity("s", "strava", &none),
            activity("w", "whoop", &json!({"strain": 14.2, "recovery_score": 61})),
            activity("z", "zepp", &none),
            activity("h", "huawei", &none),
            activity("o", "oura", &none),
        ]});
        let withheld = filter_json(&Registered, &mut payload, MODEL);
        let activities = payload["activities"].as_array().expect("an array");

        assert_eq!(activities.len(), 4, "zepp and huawei never reach the model");
        assert_eq!(activities[0], activity("g", "garmin", &none));
        for (item, (id, source)) in
            activities[1..]
                .iter()
                .zip([("s", "strava"), ("w", "whoop"), ("o", "oura")])
        {
            let expected = json!({
                "id": id, "provider": "nolio", "source": source, "sport_type": "Ride",
                "start_date": "2026-09-30T06:00:00Z", "duration_seconds": 3600
            });
            assert_eq!(item, &expected, "{source}: existence only");
        }
        assert_eq!((withheld.dropped, withheld.reduced), (2, 3));
    }

    #[test]
    fn direct_whoop_keeps_measurements_and_loses_its_scores() {
        let mut payload = json!([{
            "provider": "whoop", "recovery_score": 34.0, "sleep_score": 80.0,
            "daily_strain": 12.1, "hrv_ms": 62.0, "resting_heart_rate": 48
        }]);
        let withheld = filter_json(&Registered, &mut payload, MODEL);
        assert_eq!(
            payload,
            json!([{ "provider": "whoop", "hrv_ms": 62.0, "resting_heart_rate": 48 }])
        );
        assert_eq!(withheld.reduced, 1);
    }

    #[cfg(all(feature = "provider-whoop", feature = "provider-strava"))]
    #[test]
    fn the_registry_resolves_each_policy_through_its_descriptor() {
        let registry = ProviderRegistry::new();
        assert_eq!(registry.ai_policy("whoop"), Some(&WHOOP));
        assert_eq!(
            registry.ai_policy("strava").copied(),
            Some(SourcePolicy::ALLOW_ALL)
        );
        assert_eq!(registry.ai_policy("not-a-provider"), None);
        assert_eq!(
            registry.transport_policy("whoop"),
            Some(TransportPolicy::FirstPartyOnly),
            "WHOOP data never leaves Dravr's own surfaces (carnet#766)"
        );
        assert_eq!(
            registry.transport_policy("strava"),
            Some(TransportPolicy::FirstPartyOnly),
            "Strava API Policy §5.16"
        );
        assert_eq!(registry.transport_policy("not-a-provider"), None);
        assert!(
            registry.cache_ttls().is_empty(),
            "no shipped provider caps how long a copy is held"
        );
        // WHOOP's §4 bars training on its data; Wahoo's restriction (iii)
        // bars aggregating user information until a lawyer reads it.
        let barred: &[&str] = if cfg!(feature = "provider-wahoo") {
            &["wahoo", "whoop"]
        } else {
            &["whoop"]
        };
        assert_eq!(registry.cross_athlete_learning_barred(), barred);
    }

    /// The sciotte Strava backend is registered as `sciotte`, and every
    /// sciotte backend stamps its items `sciotte` with the scraped service as
    /// `source`: each item is governed by the backend that scraped it.
    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn a_scraped_item_is_governed_by_the_backend_that_scraped_it() {
        let registry = ProviderRegistry::new();
        assert_eq!(
            registry.transport_policy("sciotte"),
            Some(TransportPolicy::FirstPartyOnly),
            "the Strava backend's own reads"
        );
        for scraped in ["strava", "Strava", "sciotte"] {
            assert_eq!(
                registry.item_transport_policy("sciotte", Some(scraped)),
                Some(TransportPolicy::FirstPartyOnly),
                "{scraped}"
            );
        }
        for (scraped, backend) in [
            ("garmin", "sciotte_garmin"),
            ("trainingpeaks", "sciotte_trainingpeaks"),
            ("coros", "sciotte_coros"),
        ] {
            assert!(registry.transport_policy(backend).is_some(), "{backend}");
            assert_eq!(
                registry.item_transport_policy("sciotte", Some(scraped)),
                registry.transport_policy(backend),
                "{scraped} is {backend}'s to govern, not the Strava backend's"
            );
        }
    }

    /// Each descriptor whose terms were read returns its decided policy, and
    /// the names its data is stamped with resolve to it (carnet#767).
    #[cfg(all(
        feature = "provider-garmin",
        feature = "provider-sciotte",
        feature = "provider-intervals-icu"
    ))]
    #[test]
    fn each_read_provider_declares_its_transport_policy() {
        let registry = ProviderRegistry::new();
        for (name, policy) in [
            ("garmin", GARMIN_TRANSPORT),
            ("sciotte_garmin", GARMIN_TRANSPORT),
            ("sciotte_coros", COROS_TRANSPORT),
            ("sciotte_trainingpeaks", TRAININGPEAKS_TRANSPORT),
            ("intervals_icu", INTERVALS_ICU_TRANSPORT),
            ("trainingpeaks", TRAININGPEAKS_TRANSPORT),
            ("coros", COROS_TRANSPORT),
        ] {
            assert_eq!(registry.transport_policy(name), Some(policy), "{name}");
        }
    }

    /// Where each provider's items may go over an external transport, as the
    /// gate decides from their stamps.
    #[cfg(all(
        feature = "provider-garmin",
        feature = "provider-sciotte",
        feature = "provider-intervals-icu"
    ))]
    #[test]
    fn the_gate_reads_each_stamp_through_the_registry() {
        use pierre_core::ai_policy::first_party_only;

        let registry = ProviderRegistry::new();
        for (provider, source) in [
            ("garmin", None),
            ("sciotte_garmin", None),
            ("sciotte", Some("garmin")),
            ("sciotte", Some("coros")),
            ("sciotte", Some("trainingpeaks")),
            ("sciotte_trainingpeaks", None),
            ("trainingpeaks", None),
            ("intervals_icu", Some("coros")),
        ] {
            assert!(
                first_party_only(&registry, provider, source),
                "{provider} / {source:?} stays first-party"
            );
        }
        for (provider, source) in [
            ("intervals_icu", None),
            ("intervals_icu", Some("garmin")),
            ("intervals_icu", Some("GARMIN")),
            ("intervals_icu", Some("wahoo")),
        ] {
            assert!(
                !first_party_only(&registry, provider, source),
                "{provider} / {source:?} is served over every transport"
            );
        }
        assert_eq!(
            registry.relayed_transport_policy("intervals_icu", "garmin"),
            Some(TransportPolicy::AnyTransport)
        );
        assert_eq!(registry.relayed_transport_policy("sciotte", "garmin"), None);
    }

    /// A scraper and the provider it reads hold one set of terms, so the name
    /// both answer to resolves the same whichever descriptor serves it.
    #[test]
    fn a_reader_declares_the_terms_of_the_service_it_reads() {
        let registry = ProviderRegistry::new();
        for name in registry.supported_providers() {
            let reader = registry.get_descriptor(name).expect("registered");
            let Some(origin) = reader.origin() else {
                continue;
            };
            assert_eq!(
                registry.transport_policy(origin),
                Some(reader.transport_policy()),
                "{name} reads {origin}"
            );
            assert_eq!(
                registry.ai_policy(origin),
                Some(reader.ai_policy()),
                "{name} reads {origin}"
            );
        }
    }

    /// Pinned on purpose: seven days is fixed by Nolio's API terms §7.1, a
    /// value this codebase does not control.
    #[test]
    fn a_nolio_copy_is_held_at_most_seven_days() {
        assert_eq!(NOLIO_CACHE_TTL.as_secs(), 7 * 86_400);
    }

    #[test]
    fn direct_strava_and_garmin_pass_untouched() {
        let original = json!([
            {"provider": "strava", "name": "Tempo", "average_heart_rate": 160},
            {"provider": "garmin", "name": "Hills"}
        ]);
        let mut payload = original.clone();
        assert!(filter_json(&Registered, &mut payload, MODEL).is_empty());
        assert_eq!(payload, original);
    }
}
