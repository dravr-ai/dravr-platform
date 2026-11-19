// ABOUTME: The sync providers an orchestrator runs, with Intervals.icu and WHOOP wellness read where the caller names
// ABOUTME: Replaces dravr-enforme's own readers of the two, each built on a fixed host, or takes them out
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use dravr_enforme::config;
use dravr_enforme::error::EnformeResult;
use dravr_enforme::providers;
use dravr_enforme::providers::client::{self, HttpTimeouts};
use dravr_enforme::providers::intervals_icu::{IntervalsIcuProvider, INTERVALS_ICU};
use dravr_enforme::providers::sciotte_reader::DailySummaryReader;
use dravr_enforme::providers::whoop::WhoopProvider;
use dravr_enforme::SyncProvider;
use pierre_core::constants::oauth_providers;
use pierre_providers::registry::ProviderRegistry;

/// Where an orchestrator reads the wellness of the two providers
/// dravr-enforme would otherwise read on a host of its own.
///
/// Each is the API base the server's provider registry holds for the
/// provider, so the wellness is read on the host the link, activity and
/// revocation calls already go to: the provider's own in a deployment, a
/// stand-in or a refused origin in a test (carnet#1615). `None` is a server
/// that registers no such provider: it reads none of its wellness, and the
/// reader is taken out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WellnessOrigins<'registry> {
    /// Intervals.icu's API origin (`https://intervals.icu` unless a
    /// deployment overrides it); the reader's `/api/v1` is appended to it.
    pub intervals_icu_api: Option<&'registry str>,
    /// WHOOP's API base, version path included
    /// (`https://api.prod.whoop.com/developer/v2` unless a deployment
    /// overrides it); the reader appends its paths to it as given.
    pub whoop_api: Option<&'registry str>,
}

impl<'registry> WellnessOrigins<'registry> {
    /// The origins `registry` names: each provider's `api_base_url`, `None`
    /// for a provider this build does not register.
    #[must_use]
    pub fn named_by(registry: &'registry ProviderRegistry) -> Self {
        let api_base = |provider: &str| {
            registry
                .default_config(provider)
                .map(|config| config.api_base_url.as_str())
        };
        Self {
            intervals_icu_api: api_base(oauth_providers::INTERVALS_ICU),
            whoop_api: api_base(oauth_providers::WHOOP),
        }
    }
}

/// The base dravr-enforme's Intervals.icu reader appends its paths to, for
/// an API at the origin `api_base` (the provider registry's
/// `api_base_url`, `https://intervals.icu` unless a deployment overrides it).
fn intervals_icu_wellness_base(api_base: &str) -> String {
    format!("{}/api/v1", api_base.trim_end_matches('/'))
}

/// The base dravr-enforme's WHOOP reader appends its paths to, for an API at
/// `api_base` (the provider registry's `api_base_url`): the same text, which
/// already carries the version path, without a slash at its end.
fn whoop_wellness_base(api_base: &str) -> &str {
    api_base.trim_end_matches('/')
}

/// dravr-enforme's sync providers, with its Intervals.icu and WHOOP readers
/// replaced by ones that ask where `origins` says: the API the server's
/// provider registry names for each.
///
/// dravr-enforme builds those readers on hosts of its own, while the link,
/// activity and calendar calls go where the registry says. Every orchestrator
/// says where the wellness is read, so they stay on one host: the same one
/// in a deployment, a stand-in or a refused origin in a test (carnet#1615).
/// A provider `origins` names no API for is one the server does not
/// register: it reads none of its wellness, and the reader is taken out.
///
/// Each replacement is built as the provider's own `new()` builds it: the
/// same client and deadlines, the credential presented by the same code,
/// and for WHOOP the webhook secret read from `WHOOP_WEBHOOK_SECRET`, blank
/// meaning unset.
pub(super) fn sync_providers(
    reader: &Arc<dyn DailySummaryReader>,
    origins: WellnessOrigins<'_>,
) -> EnformeResult<HashMap<String, Box<dyn SyncProvider>>> {
    let mut registry = providers::build_provider_registry_with_reader(reader)?;
    // One entry per provider name: an insert replaces dravr-enforme's own.
    match origins.intervals_icu_api {
        Some(api_base) => {
            let intervals_icu = IntervalsIcuProvider::with_base_url(
                client::http_client(HttpTimeouts::PROVIDER)?,
                intervals_icu_wellness_base(api_base),
            );
            registry.insert(INTERVALS_ICU.to_owned(), Box::new(intervals_icu));
        }
        None => {
            registry.remove(INTERVALS_ICU);
        }
    }
    match origins.whoop_api {
        Some(api_base) => {
            let whoop = WhoopProvider::with_base_url(
                client::http_client(HttpTimeouts::PROVIDER)?,
                config::env_non_blank("WHOOP_WEBHOOK_SECRET"),
                whoop_wellness_base(api_base),
            );
            registry.insert(whoop.name().to_owned(), Box::new(whoop));
        }
        None => {
            registry.remove(oauth_providers::WHOOP);
        }
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use dravr_enforme::providers::sciotte_reader::{
        AuthSession, DailySummary, ScraperError, ScraperResult,
    };

    /// A scrape reader no test here reads through.
    struct NoScrape;

    #[async_trait]
    impl DailySummaryReader for NoScrape {
        async fn daily_summary(
            &self,
            provider: &str,
            _: &AuthSession,
            _: NaiveDate,
        ) -> ScraperResult<DailySummary> {
            Err(ScraperError::Internal {
                reason: format!("no scrape reader for {provider} in this test"),
            })
        }
    }

    fn names(providers: &HashMap<String, Box<dyn SyncProvider>>) -> Vec<String> {
        let mut names: Vec<String> = providers.keys().cloned().collect();
        names.sort();
        names
    }

    #[test]
    fn a_deployment_reads_the_url_dravr_enforme_reads() {
        // What a deployment hands the storage: the API origin the provider
        // registry holds for Intervals.icu when nothing overrides it.
        let registry = ProviderRegistry::new();
        let named = WellnessOrigins::named_by(&registry)
            .intervals_icu_api
            .expect("the registry names Intervals.icu");
        // dravr-enforme's own reader is built on its private
        // `INTERVALS_ICU_API_BASE`, `https://intervals.icu/api/v1`
        // (src/providers/intervals_icu.rs). The replacement must ask the
        // same URL, so production reads where it read before.
        assert_eq!(
            intervals_icu_wellness_base(named),
            "https://intervals.icu/api/v1"
        );
        assert_eq!(
            intervals_icu_wellness_base("https://intervals.icu"),
            "https://intervals.icu/api/v1"
        );
        // A stand-in, with or without a trailing slash.
        assert_eq!(
            intervals_icu_wellness_base("http://127.0.0.1:4010/"),
            "http://127.0.0.1:4010/api/v1"
        );
    }

    #[test]
    fn a_deployment_reads_whoop_at_the_url_dravr_enforme_reads() {
        // What a deployment hands the storage: the API base the provider
        // registry holds for WHOOP when nothing overrides it.
        let registry = ProviderRegistry::new();
        let origins = WellnessOrigins::named_by(&registry);
        let named = origins.whoop_api.expect("the registry names WHOOP");
        // dravr-enforme's own reader is built on its private
        // `WHOOP_API_BASE`, `https://api.prod.whoop.com/developer/v2`
        // (src/providers/whoop.rs), and appends `/recovery`, `/cycle` and
        // the rest to it. The replacement must be handed the same base, so
        // production reads where it read before.
        assert_eq!(
            whoop_wellness_base(named),
            "https://api.prod.whoop.com/developer/v2"
        );
        // A stand-in, with or without a trailing slash, is used as given.
        assert_eq!(
            whoop_wellness_base("http://127.0.0.1:4010/developer/v2/"),
            "http://127.0.0.1:4010/developer/v2"
        );
        assert_eq!(
            whoop_wellness_base("http://127.0.0.1:4010"),
            "http://127.0.0.1:4010"
        );
    }

    #[test]
    fn the_reader_is_replaced_never_added() {
        let reader: Arc<dyn DailySummaryReader> = Arc::new(NoScrape);
        let enforme_own = providers::build_provider_registry_with_reader(&reader).unwrap();
        assert!(
            enforme_own.contains_key(INTERVALS_ICU),
            "dravr-enforme registers its Intervals.icu reader under {INTERVALS_ICU}: {:?}",
            names(&enforme_own)
        );

        assert!(
            enforme_own.contains_key(oauth_providers::WHOOP),
            "dravr-enforme registers its WHOOP reader under {}: {:?}",
            oauth_providers::WHOOP,
            names(&enforme_own)
        );

        // Named an origin: the same providers, one reader per name, so no
        // second Intervals.icu or WHOOP reader runs beside the first.
        let refused = WellnessOrigins {
            intervals_icu_api: Some("http://127.0.0.1:1"),
            whoop_api: Some("http://127.0.0.1:1"),
        };
        let replaced = sync_providers(&reader, refused).unwrap();
        assert_eq!(names(&replaced), names(&enforme_own));

        // Named none: every other provider, and neither reader left on
        // dravr-enforme's host.
        let removed = sync_providers(&reader, WellnessOrigins::default()).unwrap();
        let mut others = names(&enforme_own);
        others.retain(|name| name != INTERVALS_ICU && name != oauth_providers::WHOOP);
        assert_eq!(names(&removed), others);

        // Named one: the other is taken out, not left where enforme put it.
        let whoop_only = sync_providers(
            &reader,
            WellnessOrigins {
                whoop_api: Some("http://127.0.0.1:1"),
                ..WellnessOrigins::default()
            },
        )
        .unwrap();
        let mut without_intervals = names(&enforme_own);
        without_intervals.retain(|name| name != INTERVALS_ICU);
        assert_eq!(names(&whoop_only), without_intervals);
    }
}
