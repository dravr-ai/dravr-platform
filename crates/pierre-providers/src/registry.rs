// ABOUTME: Provider registry for managing all fitness data providers in a centralized way
// ABOUTME: Handles provider instantiation, configuration, and lookup with proper error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::core::{FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory};
use crate::delegation::DelegatedReads;
use crate::request_budget::{ProviderRateLimiter, RequestBudget};
use crate::spi::{ProviderBundle, ProviderCapabilities, ProviderDescriptor};
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "provider-terra",
    feature = "provider-wahoo",
    feature = "provider-whoop",
    feature = "provider-coros"
))]
use pierre_auth::config::oauth::load_provider_env_config;
use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "provider-terra",
    feature = "provider-whoop",
    feature = "provider-coros",
    feature = "provider-sciotte",
    feature = "provider-intervals-icu",
    feature = "provider-wahoo"
))]
use pierre_core::constants::oauth as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use std::{
    collections::HashMap,
    convert::AsRef,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tracing::info;
use uuid::Uuid;

// Conditional imports for provider-specific types
#[cfg(feature = "provider-coros")]
use crate::coros_provider::CorosProviderFactory;
#[cfg(feature = "provider-garmin")]
use crate::garmin_provider::GarminProviderFactory;
#[cfg(feature = "provider-intervals-icu")]
use crate::intervals_icu_provider::{
    default_config as intervals_icu_default_config, IntervalsIcuProviderFactory,
};
#[cfg(feature = "provider-sciotte")]
use crate::sciotte_provider::{
    SciotteCorosProviderFactory, SciotteGarminProviderFactory, SciotteProviderFactory,
    SciotteTarget, SciotteTrainingPeaksProviderFactory,
};
#[cfg(feature = "provider-coros")]
use crate::spi::CorosDescriptor;
#[cfg(feature = "provider-garmin")]
use crate::spi::GarminDescriptor;
#[cfg(feature = "provider-intervals-icu")]
use crate::spi::IntervalsIcuDescriptor;
#[cfg(feature = "provider-strava")]
use crate::spi::StravaDescriptor;
#[cfg(feature = "provider-whoop")]
use crate::spi::WhoopDescriptor;
#[cfg(feature = "provider-sciotte")]
use crate::spi::{
    SciotteCorosDescriptor, SciotteDescriptor, SciotteGarminDescriptor,
    SciotteTrainingPeaksDescriptor,
};
#[cfg(feature = "provider-strava")]
use crate::strava_provider::StravaProviderFactory;
#[cfg(feature = "provider-terra")]
use crate::terra::constants::{
    TERRA_API_BASE_URL, TERRA_DEAUTH_URL, TERRA_TOKEN_URL, TERRA_WIDGET_SESSION_URL,
};
#[cfg(feature = "provider-terra")]
use crate::terra::{TerraDataCache, TerraDescriptor, TerraProviderFactory};
#[cfg(feature = "provider-wahoo")]
use crate::wahoo_descriptor::WahooDescriptor;
#[cfg(feature = "provider-wahoo")]
use crate::wahoo_provider::WahooProviderFactory;
#[cfg(feature = "provider-whoop")]
use crate::whoop_provider::WhoopProviderFactory;

/// Factory wrapper for bundle-based provider registration
struct BundleFactory {
    factory_fn: super::spi::ProviderFactoryFn,
}

impl ProviderFactory for BundleFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok((self.factory_fn)(config))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[] // Bundle-based providers don't use this method
    }
}

/// Global provider registry that manages all available fitness providers
pub struct ProviderRegistry {
    factories: HashMap<&'static str, Box<dyn ProviderFactory>>,
    default_configs: HashMap<&'static str, ProviderConfig>,
    descriptors: HashMap<&'static str, Box<dyn ProviderDescriptor>>,
    /// The request budgets every provider call is admitted against, per
    /// signing OAuth app. `None` counts nothing (a registry built for a test).
    request_limiter: Option<Arc<ProviderRateLimiter>>,
}

impl ProviderRegistry {
    /// Create a new provider registry with default providers
    ///
    /// Providers are configured from environment variables with fallback to hardcoded defaults.
    /// See `load_provider_env_config()` for environment variable format.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            factories: HashMap::new(),
            default_configs: HashMap::new(),
            descriptors: HashMap::new(),
            request_limiter: None,
        };

        // Register all enabled providers
        Self::register_strava(&mut registry);
        Self::register_garmin(&mut registry);
        Self::register_terra(&mut registry);
        Self::register_whoop(&mut registry);
        Self::register_coros(&mut registry);
        Self::register_sciotte(&mut registry);
        Self::register_sciotte_garmin(&mut registry);
        Self::register_sciotte_trainingpeaks(&mut registry);
        Self::register_sciotte_coros(&mut registry);
        Self::register_intervals_icu(&mut registry);
        Self::register_wahoo(&mut registry);

        // Log registered providers at startup
        let providers = registry.supported_providers().join(", ");
        info!(
            "Provider registry initialized with {} provider(s): [{}]",
            registry.factories.len(),
            providers
        );

        registry
    }

    /// Register Strava provider with environment-based configuration
    #[cfg(feature = "provider-strava")]
    fn register_strava(registry: &mut Self) {
        registry.register_factory(oauth_providers::STRAVA, Box::new(StravaProviderFactory));
        registry.register_descriptor(oauth_providers::STRAVA, Box::new(StravaDescriptor));
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::STRAVA,
            "https://www.strava.com/oauth/authorize",
            "https://www.strava.com/oauth/token",
            "https://www.strava.com/api/v3",
            Some("https://www.strava.com/oauth/revoke"),
            &[oauth_providers::STRAVA_DEFAULT_SCOPES.to_owned()],
        );
        registry.set_default_config(
            oauth_providers::STRAVA,
            ProviderConfig {
                name: oauth_providers::STRAVA.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-strava"))]
    fn register_strava(_registry: &mut Self) {}

    /// Register Garmin provider with environment-based configuration
    #[cfg(feature = "provider-garmin")]
    fn register_garmin(registry: &mut Self) {
        registry.register_factory(oauth_providers::GARMIN, Box::new(GarminProviderFactory));
        registry.register_descriptor(oauth_providers::GARMIN, Box::new(GarminDescriptor));
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::GARMIN,
            oauth_providers::GARMIN_AUTH_URL,
            oauth_providers::GARMIN_TOKEN_URL,
            oauth_providers::GARMIN_API_BASE_URL,
            Some(oauth_providers::GARMIN_DEREGISTRATION_URL),
            // Garmin's scope is fixed server-side; its authorization takes none.
            &[],
        );
        registry.set_default_config(
            oauth_providers::GARMIN,
            ProviderConfig {
                name: oauth_providers::GARMIN.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-garmin"))]
    fn register_garmin(_registry: &mut Self) {}

    /// Register Terra provider with environment-based configuration
    #[cfg(feature = "provider-terra")]
    fn register_terra(registry: &mut Self) {
        let terra_cache = global_terra_cache();
        registry.register_factory(
            oauth_providers::TERRA,
            Box::new(TerraProviderFactory::new(terra_cache)),
        );
        registry.register_descriptor(oauth_providers::TERRA, Box::new(TerraDescriptor));
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::TERRA,
            TERRA_WIDGET_SESSION_URL,
            TERRA_TOKEN_URL,
            TERRA_API_BASE_URL,
            Some(TERRA_DEAUTH_URL),
            &oauth_providers::TERRA_DEFAULT_SCOPES
                .split(',')
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        registry.set_default_config(
            oauth_providers::TERRA,
            ProviderConfig {
                name: oauth_providers::TERRA.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-terra"))]
    fn register_terra(_registry: &mut Self) {}

    /// Register WHOOP provider with environment-based configuration
    #[cfg(feature = "provider-whoop")]
    fn register_whoop(registry: &mut Self) {
        registry.register_factory(oauth_providers::WHOOP, Box::new(WhoopProviderFactory));
        registry.register_descriptor(oauth_providers::WHOOP, Box::new(WhoopDescriptor));
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::WHOOP,
            "https://api.prod.whoop.com/oauth/oauth2/auth",
            "https://api.prod.whoop.com/oauth/oauth2/token",
            "https://api.prod.whoop.com/developer/v2",
            Some("https://api.prod.whoop.com/developer/v2/user/access"),
            &oauth_providers::WHOOP_DEFAULT_SCOPES
                .split(' ')
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        registry.set_default_config(
            oauth_providers::WHOOP,
            ProviderConfig {
                name: oauth_providers::WHOOP.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-whoop"))]
    fn register_whoop(_registry: &mut Self) {}

    /// Register COROS provider with environment-based configuration
    ///
    /// Note: COROS API documentation is private. OAuth endpoints are placeholders
    /// until official documentation is received.
    #[cfg(feature = "provider-coros")]
    fn register_coros(registry: &mut Self) {
        registry.register_factory(oauth_providers::COROS, Box::new(CorosProviderFactory));
        registry.register_descriptor(oauth_providers::COROS, Box::new(CorosDescriptor));
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::COROS,
            // Placeholder URLs - update when COROS provides official API documentation
            "https://open.coros.com/oauth2/authorize",
            "https://open.coros.com/oauth2/token",
            "https://open.coros.com/api/v1",
            Some("https://open.coros.com/oauth2/revoke"),
            &oauth_providers::COROS_DEFAULT_SCOPES
                .split(' ')
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        registry.set_default_config(
            oauth_providers::COROS,
            ProviderConfig {
                name: oauth_providers::COROS.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-coros"))]
    fn register_coros(_registry: &mut Self) {}

    /// Register Sciotte web scraping provider
    ///
    /// Sciotte uses browser-based session cookies for authentication (not OAuth).
    /// It runs as a separate sidecar service and provides activity data scraped
    /// from fitness platform HTML pages (Strava, Garmin, etc.).
    #[cfg(feature = "provider-sciotte")]
    fn register_sciotte(registry: &mut Self) {
        registry.register_factory(oauth_providers::SCIOTTE, Box::new(SciotteProviderFactory));
        registry.register_descriptor(oauth_providers::SCIOTTE, Box::new(SciotteDescriptor));
        // Sciotte runs in-process via dravr-sciotte library — no OAuth, no external URLs
        registry.set_default_config(
            oauth_providers::SCIOTTE,
            ProviderConfig {
                name: oauth_providers::SCIOTTE.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: vec![],
            },
        );
    }

    #[cfg(not(feature = "provider-sciotte"))]
    fn register_sciotte(_registry: &mut Self) {}

    /// Register Sciotte Garmin Connect provider (in-process web scraping)
    #[cfg(feature = "provider-sciotte")]
    fn register_sciotte_garmin(registry: &mut Self) {
        registry.register_factory(
            oauth_providers::SCIOTTE_GARMIN,
            Box::new(SciotteGarminProviderFactory),
        );
        registry.register_descriptor(
            oauth_providers::SCIOTTE_GARMIN,
            Box::new(SciotteGarminDescriptor),
        );
        registry.set_default_config(
            oauth_providers::SCIOTTE_GARMIN,
            ProviderConfig {
                name: oauth_providers::SCIOTTE_GARMIN.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: vec![],
            },
        );
    }

    #[cfg(not(feature = "provider-sciotte"))]
    fn register_sciotte_garmin(_registry: &mut Self) {}

    /// Register Sciotte TrainingPeaks provider (web scraping on the scraper service)
    #[cfg(feature = "provider-sciotte")]
    fn register_sciotte_trainingpeaks(registry: &mut Self) {
        registry.register_factory(
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            Box::new(SciotteTrainingPeaksProviderFactory),
        );
        registry.register_descriptor(
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            Box::new(SciotteTrainingPeaksDescriptor),
        );
        registry.set_default_config(
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            ProviderConfig {
                name: oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: vec![],
            },
        );
    }

    #[cfg(not(feature = "provider-sciotte"))]
    fn register_sciotte_trainingpeaks(_registry: &mut Self) {}

    /// Register the COROS Training Hub mirror (sciotte scraping), the
    /// `coros` backend until the partner API (carnet#509) is approved.
    #[cfg(feature = "provider-sciotte")]
    fn register_sciotte_coros(registry: &mut Self) {
        registry.register_factory(
            oauth_providers::SCIOTTE_COROS,
            Box::new(SciotteCorosProviderFactory),
        );
        registry.register_descriptor(
            oauth_providers::SCIOTTE_COROS,
            Box::new(SciotteCorosDescriptor),
        );
        registry.set_default_config(
            oauth_providers::SCIOTTE_COROS,
            ProviderConfig {
                name: oauth_providers::SCIOTTE_COROS.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: vec![],
            },
        );
    }

    #[cfg(not(feature = "provider-sciotte"))]
    fn register_sciotte_coros(_registry: &mut Self) {}

    /// Register the Intervals.icu provider with environment-based configuration.
    ///
    /// Athletes link through the OAuth app or by pasting their athlete id + API
    /// key; both call the same API base. The OAuth endpoints honour the
    /// `PIERRE_INTERVALS_ICU_*` overrides every OAuth provider reads.
    #[cfg(feature = "provider-intervals-icu")]
    fn register_intervals_icu(registry: &mut Self) {
        registry.register_factory(
            oauth_providers::INTERVALS_ICU,
            Box::new(IntervalsIcuProviderFactory),
        );
        registry.register_descriptor(
            oauth_providers::INTERVALS_ICU,
            Box::new(IntervalsIcuDescriptor),
        );
        let defaults = intervals_icu_default_config();
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::INTERVALS_ICU,
            &defaults.auth_url,
            &defaults.token_url,
            &defaults.api_base_url,
            defaults.revoke_url.as_deref(),
            &defaults.default_scopes,
        );
        registry.set_default_config(
            oauth_providers::INTERVALS_ICU,
            ProviderConfig {
                name: defaults.name,
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-intervals-icu"))]
    fn register_intervals_icu(_registry: &mut Self) {}

    /// Register the Wahoo provider with environment-based configuration
    /// (`PIERRE_WAHOO_*` overrides, as every OAuth provider reads them).
    #[cfg(feature = "provider-wahoo")]
    fn register_wahoo(registry: &mut Self) {
        registry.register_factory(oauth_providers::WAHOO, Box::new(WahooProviderFactory));
        registry.register_descriptor(oauth_providers::WAHOO, Box::new(WahooDescriptor));
        let scopes: Vec<String> = oauth_providers::WAHOO_DEFAULT_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect();
        let (auth_url, token_url, api_base_url, revoke_url, scopes) = load_provider_env_config(
            oauth_providers::WAHOO,
            oauth_providers::WAHOO_AUTH_URL,
            oauth_providers::WAHOO_TOKEN_URL,
            oauth_providers::WAHOO_API_BASE_URL,
            Some(oauth_providers::WAHOO_DEAUTHORIZE_URL),
            &scopes,
        );
        registry.set_default_config(
            oauth_providers::WAHOO,
            ProviderConfig {
                name: oauth_providers::WAHOO.to_owned(),
                auth_url,
                token_url,
                api_base_url,
                revoke_url,
                default_scopes: scopes,
            },
        );
    }

    #[cfg(not(feature = "provider-wahoo"))]
    fn register_wahoo(_registry: &mut Self) {}

    /// Register a provider factory
    pub fn register_factory(
        &mut self,
        provider_name: &'static str,
        factory: Box<dyn ProviderFactory>,
    ) {
        self.factories.insert(provider_name, factory);
    }

    /// Set default configuration for a provider
    pub fn set_default_config(&mut self, provider_name: &'static str, config: ProviderConfig) {
        self.default_configs.insert(provider_name, config);
    }

    /// The default configuration a provider was registered with — its
    /// endpoints after the `PIERRE_<PROVIDER>_*_URL` environment overrides
    /// were applied — without instantiating the provider.
    #[must_use]
    pub fn default_config(&self, provider_name: &str) -> Option<&ProviderConfig> {
        self.default_configs.get(provider_name)
    }

    /// Register a provider descriptor
    pub fn register_descriptor(
        &mut self,
        provider_name: &'static str,
        descriptor: Box<dyn ProviderDescriptor>,
    ) {
        self.descriptors.insert(provider_name, descriptor);
    }

    /// Register a complete provider bundle (factory + descriptor + config)
    ///
    /// This is the preferred method for external provider crates to register their providers.
    /// It handles factory registration, descriptor storage, and default configuration.
    pub fn register_provider_bundle(&mut self, bundle: ProviderBundle) {
        let name = bundle.name();
        // We need to leak the string to get a &'static str
        // This is safe because provider names are expected to live for the program's lifetime
        let static_name: &'static str = Box::leak(name.to_owned().into_boxed_str());

        self.factories.insert(
            static_name,
            Box::new(BundleFactory {
                factory_fn: bundle.factory,
            }),
        );
        self.default_configs
            .insert(static_name, bundle.descriptor.to_config());
        self.descriptors.insert(static_name, bundle.descriptor);

        info!("Registered external provider: {}", static_name);
    }

    /// Get list of supported provider names
    #[must_use]
    pub fn supported_providers(&self) -> Vec<&'static str> {
        self.factories.keys().copied().collect()
    }

    /// Check if a provider is supported
    #[must_use]
    pub fn is_supported(&self, provider_name: &str) -> bool {
        self.factories.contains_key(provider_name)
    }

    /// Check if a provider requires OAuth authentication
    #[must_use]
    pub fn requires_oauth(&self, provider_name: &str) -> bool {
        self.descriptors
            .get(provider_name)
            .is_some_and(|d| d.requires_oauth())
    }

    /// Check if a provider supports sleep tracking
    #[must_use]
    pub fn supports_sleep(&self, provider_name: &str) -> bool {
        self.descriptors
            .get(provider_name)
            .is_some_and(|d| d.supports_sleep())
    }

    /// Check if a provider supports recovery metrics
    #[must_use]
    pub fn supports_recovery(&self, provider_name: &str) -> bool {
        self.descriptors
            .get(provider_name)
            .is_some_and(|d| d.supports_recovery())
    }

    /// Get provider capabilities
    #[must_use]
    pub fn get_capabilities(&self, provider_name: &str) -> Option<ProviderCapabilities> {
        self.descriptors
            .get(provider_name)
            .map(|d| d.capabilities())
    }

    /// Get provider display name
    #[must_use]
    pub fn get_display_name(&self, provider_name: &str) -> Option<&'static str> {
        self.descriptors
            .get(provider_name)
            .map(|d| d.display_name())
    }

    /// Get provider descriptor for OAuth and API configuration
    #[must_use]
    pub fn get_descriptor(&self, provider_name: &str) -> Option<&dyn ProviderDescriptor> {
        self.descriptors.get(provider_name).map(AsRef::as_ref)
    }
}

impl ProviderRegistry {
    /// The descriptor whose terms govern data stamped `name`: the one
    /// registered under it, or else the one whose backend reads that service
    /// ([`ProviderDescriptor::origin`]) — `trainingpeaks` planned workouts and
    /// `coros` sources answer to the scraper that reads them (carnet#767).
    /// Several readers of one service are first in name order; the registry
    /// test pins that they declare the same terms.
    pub(crate) fn terms_descriptor(&self, name: &str) -> Option<&dyn ProviderDescriptor> {
        if let Some(descriptor) = self.descriptors.get(name) {
            return Some(descriptor.as_ref());
        }
        self.descriptors
            .iter()
            .filter(|(_, d)| d.origin().is_some_and(|origin| origin == name))
            .min_by_key(|(registered, _)| **registered)
            .map(|(_, d)| d.as_ref())
    }
}

impl ProviderTerms for ProviderRegistry {
    fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
        self.terms_descriptor(provider)
            .map(ProviderDescriptor::ai_policy)
    }

    fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
        self.terms_descriptor(provider)
            .map(ProviderDescriptor::transport_policy)
    }

    fn relayed_transport_policy(&self, relay: &str, source: &str) -> Option<TransportPolicy> {
        self.terms_descriptor(relay)?
            .transport_by_source()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(source))
            .map(|(_, policy)| *policy)
    }

    /// Every sciotte backend stamps its items `sciotte` with the scraped
    /// service as `source`, while `sciotte` is also the name of its Strava
    /// backend: a Garmin activity it scraped is the `sciotte_garmin` backend's
    /// to govern, not the Strava one's (carnet#765). An unstamped read names
    /// its backend itself.
    #[cfg(feature = "provider-sciotte")]
    fn item_transport_policy(
        &self,
        provider: &str,
        source: Option<&str>,
    ) -> Option<TransportPolicy> {
        let backend = match source {
            Some(scraped) if provider == oauth_providers::SCIOTTE => {
                SciotteTarget::from_target_param(&scraped.to_ascii_lowercase()).provider_name()
            }
            _ => provider,
        };
        self.transport_policy(backend)
    }
}

impl ProviderRegistry {
    /// Every registered provider whose terms cap how long a copy of its data
    /// may be held, with that cap, sorted by name: what the cache TTL sweep
    /// enforces.
    #[must_use]
    pub fn cache_ttls(&self) -> Vec<(&'static str, Duration)> {
        let mut ttls: Vec<(&'static str, Duration)> = self
            .descriptors
            .iter()
            .filter_map(|(name, d)| d.cache_ttl().map(|ttl| (*name, ttl)))
            .collect();
        ttls.sort_unstable_by_key(|(name, _)| *name);
        ttls
    }

    /// Every registered provider whose terms bar learning from its data for
    /// other athletes, sorted: whose connected athletes the archetype
    /// aggregation leaves out.
    #[must_use]
    pub fn cross_athlete_learning_barred(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .descriptors
            .iter()
            .filter(|(_, d)| d.bars_cross_athlete_learning())
            .map(|(name, _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// Get all providers that support OAuth
    #[must_use]
    pub fn oauth_providers(&self) -> Vec<&'static str> {
        self.descriptors
            .iter()
            .filter(|(_, d)| d.requires_oauth())
            .map(|(name, _)| *name)
            .collect()
    }

    /// Get all providers that read the workouts their calendar plans, by
    /// registered name, sorted so a message that lists them reads the same
    /// on every call.
    #[must_use]
    pub fn planned_workout_providers(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .descriptors
            .iter()
            .filter(|(_, d)| d.capabilities().supports_planned_workouts())
            .map(|(name, _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// Get all providers whose training calendar accepts writes, by
    /// registered name, sorted: the order a caller picks a push target by
    /// when an athlete has several connected.
    #[must_use]
    pub fn calendar_write_providers(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .descriptors
            .iter()
            .filter(|(_, d)| d.capabilities().supports_calendar_write())
            .map(|(name, _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// This registry, admitting every provider call it hands credentials for
    /// against `limiter`'s per-app budgets.
    #[must_use]
    pub fn with_request_limiter(mut self, limiter: Arc<ProviderRateLimiter>) -> Self {
        self.request_limiter = Some(limiter);
        self
    }

    /// The budget of `app` (the signing OAuth client's id), with the app's own
    /// `daily_limit` when it registered one, for credentials to carry; `None`
    /// when this registry counts nothing.
    #[must_use]
    pub fn request_budget(&self, app: &str, daily_limit: Option<u32>) -> Option<RequestBudget> {
        self.request_limiter
            .as_ref()
            .map(|limiter| RequestBudget::new(Arc::clone(limiter), app.to_owned(), daily_limit))
    }

    /// The budget of one athlete's grant to `app`: the app's windows as
    /// [`Self::request_budget`] has them, and the windows the provider keeps
    /// for each grant, counted for `account`, the provider's id for the
    /// athlete. `None` when this registry counts nothing.
    #[must_use]
    pub fn grant_budget(
        &self,
        app: &str,
        daily_limit: Option<u32>,
        account: &str,
    ) -> Option<RequestBudget> {
        self.request_limiter.as_ref().map(|limiter| {
            RequestBudget::for_grant(
                Arc::clone(limiter),
                app.to_owned(),
                daily_limit,
                account.to_owned(),
            )
        })
    }

    /// The budget of the personal API key that belongs to `account`, the
    /// provider's id for its account: the windows the provider keeps for
    /// each key. `None` when this registry counts nothing.
    #[must_use]
    pub fn api_key_budget(&self, account: &str) -> Option<RequestBudget> {
        self.request_limiter
            .as_ref()
            .map(|limiter| RequestBudget::for_api_key(Arc::clone(limiter), account.to_owned()))
    }

    /// Create a provider instance with default configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the provider is not supported or no default configuration exists.
    pub fn create_provider(&self, provider_name: &str) -> AppResult<Box<dyn FitnessProvider>> {
        let factory = self.factories.get(provider_name).ok_or_else(|| {
            AppError::invalid_input(format!("Unsupported provider: {provider_name}"))
        })?;

        let config = self
            .default_configs
            .get(provider_name)
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "No default configuration for provider: {provider_name}"
                ))
            })?
            .clone();

        factory.create(config)
    }

    /// Refuse an athlete id `provider_name` could not have issued, before it
    /// is stored on a link or reaches a URL.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider that cannot be read on
    /// anyone's behalf, or for an id the provider refuses.
    pub fn check_delegated_athlete(&self, provider_name: &str, athlete_id: &str) -> AppResult<()> {
        self.delegated_reads(provider_name)?
            .check_athlete_id(athlete_id)
    }

    /// Create a provider that reads one coached athlete, `athlete_id`,
    /// through `coach_credentials`: the coach account's own stored
    /// credential, an OAuth grant or an API key alike
    /// ([`CredentialKind`](crate::core::CredentialKind)).
    ///
    /// Only a provider whose descriptor declares
    /// [`ProviderCapabilities::COACH_ROSTER`] is built this way. A coaching
    /// platform's coach account reads each athlete who shares with it by that
    /// athlete's id; the provider built names `athlete_id` on every read and
    /// refuses a detail id outside that athlete. The athlete id is checked
    /// before the provider is built, so a refused id reaches no URL.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider that cannot be read on
    /// anyone's behalf, for an athlete id it refuses, or when the provider
    /// has no default configuration; or the provider's refusal of the
    /// credentials.
    pub async fn create_delegated_provider(
        &self,
        provider_name: &str,
        athlete_id: &str,
        coach_credentials: OAuth2Credentials,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let reads = self.delegated_reads(provider_name)?;
        let config = self
            .default_configs
            .get(provider_name)
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "No default configuration for provider: {provider_name}"
                ))
            })?
            .clone();
        let provider = reads.create_delegated(config, athlete_id)?;
        provider.set_credentials(coach_credentials).await?;
        Ok(provider)
    }

    /// Every provider a coach account reads its athletes through (those whose
    /// descriptor declares [`ProviderCapabilities::COACH_ROSTER`]) by
    /// registered name, sorted so every call lists them in one order.
    #[must_use]
    pub fn coach_roster_providers(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .descriptors
            .iter()
            .filter(|(_, d)| d.capabilities().supports_coach_roster())
            .map(|(name, _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// The delegated-read capability of `provider_name`: its descriptor
    /// declares [`ProviderCapabilities::COACH_ROSTER`], and its factory says
    /// how to build the provider fixed to one athlete.
    fn delegated_reads(&self, provider_name: &str) -> AppResult<&dyn DelegatedReads> {
        let declared = self
            .get_capabilities(provider_name)
            .is_some_and(|caps| caps.supports_coach_roster());
        if !declared {
            return Err(AppError::invalid_input(format!(
                "{provider_name} cannot be read on behalf of a coached athlete"
            )));
        }
        self.factories
            .get(provider_name)
            .and_then(|factory| factory.delegated_reads())
            .ok_or_else(|| {
                AppError::internal(format!(
                    "{provider_name} declares a coach roster but its factory builds no delegated reads"
                ))
            })
    }

    /// Create a provider instance with custom configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the provider is not supported.
    pub fn create_provider_with_config(
        &self,
        provider_name: &str,
        config: ProviderConfig,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let factory = self.factories.get(provider_name).ok_or_else(|| {
            AppError::invalid_input(format!("Unsupported provider: {provider_name}"))
        })?;

        factory.create(config)
    }

    /// Create a provider on behalf of one user of one tenant
    ///
    /// The provider itself is tenant-agnostic; the tenant and user are recorded
    /// in the log line that attributes the construction.
    ///
    /// # Errors
    ///
    /// Returns an error if the provider is not supported or no default configuration exists.
    pub fn create_tenant_provider(
        &self,
        provider_name: &str,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let provider = self.create_provider(provider_name)?;
        info!(
            provider = provider.name(),
            %tenant_id,
            %user_id,
            "Created provider for tenant user"
        );
        Ok(provider)
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global provider registry instance (singleton)
///
/// Note: For test isolation, prefer creating local `ProviderRegistry::new()` instances
/// instead of using this global singleton. Tests that use the global singleton will
/// share state and cannot customize provider configuration per-test.
static REGISTRY: OnceLock<Arc<ProviderRegistry>> = OnceLock::new();

/// Get the global provider registry
///
/// This should be used in production code for convenience. For tests requiring isolation,
/// use `ProviderRegistry::new()` directly to create test-specific instances.
#[must_use]
pub fn global_registry() -> Arc<ProviderRegistry> {
    REGISTRY
        .get_or_init(|| Arc::new(ProviderRegistry::new()))
        .clone() // Safe: Arc clone for provider registry access
}

/// Convenience function to create a provider using the global registry
///
/// For test isolation, prefer creating a local `ProviderRegistry` instance and calling
/// `registry.create_provider()` instead of using this global function.
///
/// # Errors
///
/// Returns an error if the provider is not supported or no default configuration exists.
pub fn create_provider(provider_name: &str) -> AppResult<Box<dyn FitnessProvider>> {
    global_registry().create_provider(provider_name)
}

/// Convenience function to create a tenant provider using the global registry
///
/// For test isolation, prefer creating a local `ProviderRegistry` instance and calling
/// `registry.create_tenant_provider()` instead of using this global function.
///
/// # Errors
///
/// Returns an error if the provider is not supported or no default configuration exists.
pub fn create_tenant_provider(
    provider_name: &str,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Box<dyn FitnessProvider>> {
    global_registry().create_tenant_provider(provider_name, tenant_id, user_id)
}

/// Convenience function to check if a provider is supported
///
/// Uses the global registry. For test isolation, create a local `ProviderRegistry` instance.
#[must_use]
pub fn is_provider_supported(provider_name: &str) -> bool {
    global_registry().is_supported(provider_name)
}

/// Convenience function to get all supported providers
///
/// Uses the global registry. For test isolation, create a local `ProviderRegistry` instance.
#[must_use]
pub fn get_supported_providers() -> Vec<&'static str> {
    global_registry().supported_providers()
}

/// Create a new provider registry with external provider bundles
///
/// This function creates a new registry instance with both built-in providers
/// (based on feature flags) and any additional external provider bundles.
///
/// # Example
///
/// ```rust,no_run
/// use pierre_providers::registry::create_registry_with_external_providers;
/// use pierre_providers::spi::ProviderBundle;
///
/// // External provider crate would provide a function like:
/// // fn whoop_provider_bundle() -> ProviderBundle { ... }
///
/// let external_bundles: Vec<ProviderBundle> = vec![
///     // whoop_provider_bundle(),
/// ];
/// let registry = create_registry_with_external_providers(external_bundles);
/// ```
#[must_use]
pub fn create_registry_with_external_providers(bundles: Vec<ProviderBundle>) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    for bundle in bundles {
        registry.register_provider_bundle(bundle);
    }
    registry
}

// ============================================================================
// Terra Global Cache (conditionally compiled)
// ============================================================================

/// Global Terra data cache instance
///
/// Terra uses a webhook-based model where data is pushed to your endpoint.
/// This global cache stores webhook data and makes it available to `TerraProvider`
/// instances for the `FitnessProvider` trait implementation.
#[cfg(feature = "provider-terra")]
static TERRA_CACHE: OnceLock<Arc<TerraDataCache>> = OnceLock::new();

/// Get the global Terra data cache
///
/// Returns a shared reference to the Terra webhook data cache.
/// Use this cache with `TerraWebhookHandler` to store incoming webhook data.
#[cfg(feature = "provider-terra")]
#[must_use]
pub fn global_terra_cache() -> Arc<TerraDataCache> {
    TERRA_CACHE
        .get_or_init(|| Arc::new(TerraDataCache::new_in_memory()))
        .clone() // Safe: Arc clone for shared cache access
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built-in factory that builds delegated reads is read on a coached
    /// athlete's behalf only through its descriptor's `COACH_ROSTER`, so one
    /// that declares nothing would build reads no one can reach.
    #[test]
    fn every_built_in_delegated_factory_declares_a_coach_roster() {
        let registry = ProviderRegistry::new();
        for (name, factory) in &registry.factories {
            if factory.delegated_reads().is_some() {
                assert!(
                    registry
                        .get_capabilities(name)
                        .is_some_and(|caps| caps.supports_coach_roster()),
                    "{name} builds delegated reads but declares no coach roster"
                );
            }
        }
    }
}
