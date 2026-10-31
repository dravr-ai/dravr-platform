// ABOUTME: The terms descriptor of each key holding the athlete's own data: imported archives, uploads, manual entries
// ABOUTME: Registered as descriptors only, never factories, so such a key can never be connected or fetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Imported archives as provider keys
//!
//! An athlete's own export of a service's history (carnet#1218) is held by
//! dravr-barrage and served under its own key, one per service
//! ([`STRAVA_ARCHIVE`], [`GARMIN_ARCHIVE`]), never under
//! the live provider's. Each key's descriptor gives it its terms: the copy is
//! the athlete's own (portability data), so a model, MCP, A2A and an API key
//! may read it alike, it is never evicted for age, and no connection's consent
//! or disconnect reaches it.
//!
//! The terms are declared, not left to the absence of rules: a key with no
//! descriptor passes the AI filter only because nothing names it, which is no
//! policy (Athlete Data Consolidation §3.2). [`own_data_descriptors`] declares
//! one per archive key, and a test holds the two lists together.
//!
//! The same holds for the two other keys the athlete's own data is filed
//! under: an uploaded file ([`UPLOAD`]) and a reading entered by hand
//! ([`ATHLETE`]). [`own_data_descriptors`] declares all of them.
//!
//! They are registered with
//! [`crate::registry::ProviderRegistry::register_descriptor`] only.
//! Connectability is read from the factories, so an archive key is never
//! offered as a connection and nothing ever fetches it.

use pierre_core::constants::oauth_providers::{ATHLETE, GARMIN_ARCHIVE, STRAVA_ARCHIVE, UPLOAD};
use pierre_core::transport::TransportPolicy;

use crate::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};

/// The terms of a key holding the athlete's own data: a history imported
/// from their own export of a service, an upload, a manual entry.
#[derive(Debug)]
pub struct ArchiveDescriptor {
    key: &'static str,
    display_name: &'static str,
    capabilities: ProviderCapabilities,
}

/// One descriptor per key holding the athlete's own data, each with what it
/// holds.
///
/// - A Strava export holds recorded activities, with their samples and laps.
/// - A Garmin Connect export holds those too, and the athlete's wellness:
///   sleep, the day's recovery readings, body readings and activity totals.
/// - An upload holds the recordings in the files the athlete uploaded.
/// - A manual entry holds the readings the athlete typed in.
#[must_use]
pub fn own_data_descriptors() -> Vec<ArchiveDescriptor> {
    let recorded =
        ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::RECORDED_ACTIVITIES);
    vec![
        ArchiveDescriptor {
            key: STRAVA_ARCHIVE,
            display_name: "Strava (imported)",
            capabilities: recorded,
        },
        ArchiveDescriptor {
            key: GARMIN_ARCHIVE,
            display_name: "Garmin (imported)",
            capabilities: recorded
                .union(ProviderCapabilities::SLEEP_TRACKING)
                .union(ProviderCapabilities::RECOVERY_METRICS)
                .union(ProviderCapabilities::HEALTH_METRICS),
        },
        ArchiveDescriptor {
            key: UPLOAD,
            display_name: "Uploaded file",
            capabilities: recorded,
        },
        ArchiveDescriptor {
            key: ATHLETE,
            display_name: "Entered by you",
            capabilities: ProviderCapabilities::SLEEP_TRACKING
                .union(ProviderCapabilities::RECOVERY_METRICS)
                .union(ProviderCapabilities::HEALTH_METRICS),
        },
    ]
}

impl ProviderDescriptor for ArchiveDescriptor {
    fn name(&self) -> &'static str {
        self.key
    }

    fn display_name(&self) -> &'static str {
        self.display_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    /// The athlete's own copy: served over every transport, whatever the
    /// service's own terms say about the data Dravr fetches from it.
    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::AnyTransport
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::ai_policy::first_party_only;
    use pierre_core::constants::oauth_providers::{
        ATHLETE, GARMIN, GARMIN_ARCHIVE, STRAVA, STRAVA_ARCHIVE, USER_PROVIDED_PROVIDERS,
    };

    use super::own_data_descriptors;
    use crate::backend_resolver::brand_name;
    use crate::registry::ProviderRegistry;
    use crate::spi::{ProviderCapabilities, ProviderDescriptor};

    #[test]
    fn every_imported_history_is_readable_everywhere_and_never_connectable() {
        let registry = ProviderRegistry::new();
        for (key, brand) in [
            (STRAVA_ARCHIVE, "Strava (imported)"),
            (GARMIN_ARCHIVE, "Garmin (imported)"),
        ] {
            assert!(
                !first_party_only(&registry, key, None),
                "{key}: the athlete's own export is served over every transport"
            );
            assert_eq!(brand_name(&registry, key), Some(brand), "{key}");
            assert!(
                !registry.is_supported(key),
                "{key}: a terms descriptor alone is never a connection"
            );
        }
        // Each live provider compiled in keeps its own descriptor beside its
        // export's (Garmin is behind `provider-garmin`, off by default).
        let live = [
            (STRAVA, cfg!(feature = "provider-strava")),
            (GARMIN, cfg!(feature = "provider-garmin")),
        ];
        for (key, compiled) in live {
            assert_eq!(
                registry.get_descriptor(key).is_some(),
                compiled,
                "{key}: the live provider keeps its own descriptor"
            );
        }
    }

    /// One descriptor per key of the athlete's own data, so a new such key
    /// cannot reach the registry with no declared terms.
    #[test]
    fn every_own_data_key_has_exactly_one_descriptor() {
        let mut names: Vec<&str> = own_data_descriptors()
            .iter()
            .map(ProviderDescriptor::name)
            .collect();
        let mut keys: Vec<&str> = USER_PROVIDED_PROVIDERS.to_vec();
        keys.push(ATHLETE);
        names.sort_unstable();
        keys.sort_unstable();
        assert_eq!(
            names, keys,
            "bug: an own-data key has no descriptor, or two"
        );
    }

    #[test]
    fn a_garmin_export_declares_its_wellness() {
        let garmin = own_data_descriptors()
            .into_iter()
            .find(|d| d.name() == GARMIN_ARCHIVE)
            .expect("garmin_archive descriptor");
        for wellness in [
            ProviderCapabilities::SLEEP_TRACKING,
            ProviderCapabilities::RECOVERY_METRICS,
            ProviderCapabilities::HEALTH_METRICS,
        ] {
            assert!(garmin.capabilities().contains(wellness), "{wellness:?}");
        }
    }
}
