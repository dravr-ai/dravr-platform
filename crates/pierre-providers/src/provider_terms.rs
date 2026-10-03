// ABOUTME: What each provider's terms allow for its data — the declared AI SourcePolicy and TransportPolicy per provider
// ABOUTME: WHOOP keeps its scores out of prompts; Nolio restricts connector data by source and stays first-party (§6.9)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Declared provider terms.
//!
//! An AI policy for each provider whose terms restrict AI use, and a transport
//! policy for each whose terms keep its data inside Dravr's own surfaces.
//!
//! A descriptor returns them from
//! [`ProviderDescriptor::ai_policy`](crate::spi::ProviderDescriptor::ai_policy)
//! and
//! [`ProviderDescriptor::transport_policy`](crate::spi::ProviderDescriptor::transport_policy);
//! a provider absent here has no restriction. Adding a provider's terms is a
//! policy here and one descriptor line — no tool changes.
//!
//! Strava's June 2026 terms bar AI use, but direct Strava data stays allowed
//! by product decision (2026-10-02) until that question is settled on its own.
//! No shipped provider's descriptor declares a transport policy, so each is
//! served over every transport.

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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(feature = "provider-whoop", feature = "provider-strava"))]
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
                "whoop" | "garmin" | "strava" => Some(TransportPolicy::AnyTransport),
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
            Some(TransportPolicy::AnyTransport),
            "every shipped provider is served over every transport"
        );
        assert_eq!(
            registry.transport_policy("strava"),
            Some(TransportPolicy::AnyTransport)
        );
        assert_eq!(registry.transport_policy("not-a-provider"), None);
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
