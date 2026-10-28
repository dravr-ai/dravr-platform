// ABOUTME: The terms descriptor of an imported archive (strava_archive): its display name, and no restriction on any reader
// ABOUTME: Registered as a descriptor only, never a factory, so the key can never be connected or fetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Imported archives as a provider key
//!
//! An athlete's own export of a service's history (carnet#1218) is held by
//! dravr-barrage and served from the activity cache under its own key,
//! [`STRAVA_ARCHIVE`], never under the live provider's. The descriptor gives
//! that key its terms: the copy is the athlete's own, so a model, MCP, A2A and
//! an API key may read it alike, it is never evicted for age, and no
//! connection's consent or disconnect reaches it.
//!
//! It is registered with [`crate::registry::ProviderRegistry::register_descriptor`]
//! only. Connectability is read from the factories, so the key is never
//! offered as a connection and nothing ever fetches it.

use pierre_core::constants::oauth_providers::STRAVA_ARCHIVE;
use pierre_core::transport::TransportPolicy;

use crate::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};

/// The terms of a Strava history the athlete imported from their own export.
#[derive(Debug)]
pub struct StravaArchiveDescriptor;

impl ProviderDescriptor for StravaArchiveDescriptor {
    fn name(&self) -> &'static str {
        STRAVA_ARCHIVE
    }

    fn display_name(&self) -> &'static str {
        "Strava (imported)"
    }

    /// Recorded activities, with their samples and laps: what the export holds.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::RECORDED_ACTIVITIES)
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

    /// The athlete's own copy: served over every transport, whatever Strava's
    /// own terms say about the data Dravr fetches from Strava.
    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::AnyTransport
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::ai_policy::first_party_only;
    use pierre_core::constants::oauth_providers::{STRAVA, STRAVA_ARCHIVE};

    use crate::backend_resolver::brand_name;
    use crate::registry::ProviderRegistry;

    #[test]
    fn an_imported_strava_history_is_readable_everywhere_and_never_connectable() {
        let registry = ProviderRegistry::new();
        assert!(
            !first_party_only(&registry, STRAVA_ARCHIVE, None),
            "the athlete's own export is served over every transport"
        );
        assert_eq!(
            brand_name(&registry, STRAVA_ARCHIVE),
            Some("Strava (imported)"),
            "its display name comes from its descriptor"
        );
        assert!(
            !registry.is_supported(STRAVA_ARCHIVE),
            "a terms descriptor alone is never a connection"
        );
        assert!(
            registry.get_descriptor(STRAVA).is_some(),
            "the live provider keeps its own descriptor"
        );
    }
}
