// ABOUTME: Service Provider Interface (SPI) for pluggable provider architecture
// ABOUTME: Defines the contract that external provider crates must implement for registration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Provider Service Provider Interface (SPI)
//!
//! This module defines the contract that external provider crates must implement
//! to integrate with the Pierre MCP Server. The SPI enables true pluggability by
//! allowing providers to be developed, compiled, and registered independently.
//!
//! ## Key Concepts
//!
//! - **`ProviderDescriptor`**: Describes provider capabilities (OAuth, sleep tracking, etc.)
//! - **`OAuthEndpoints`**: OAuth configuration for providers requiring authentication
//! - **`ProviderBundle`**: Complete provider package for registration
//!
//! ## Example: Implementing a Custom Provider
//!
//! ```rust,no_run
//! use pierre_providers::spi::{
//!     OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
//!     RefreshClientAuth,
//! };
//!
//! pub struct WhoopDescriptor;
//!
//! impl ProviderDescriptor for WhoopDescriptor {
//!     fn name(&self) -> &'static str {
//!         "whoop"
//!     }
//!
//!     fn display_name(&self) -> &'static str {
//!         "WHOOP"
//!     }
//!
//!     fn capabilities(&self) -> ProviderCapabilities {
//!         // Use bitflags combinators for provider capabilities
//!         ProviderCapabilities::full_health()
//!     }
//!
//!     fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
//!         Some(OAuthEndpoints {
//!             auth_url: "https://api.prod.whoop.com/oauth/oauth2/auth",
//!             token_url: "https://api.prod.whoop.com/oauth/oauth2/token",
//!             revoke_url: Some("https://api.prod.whoop.com/developer/v2/user/access"),
//!         })
//!     }
//!
//!     fn oauth_params(&self) -> Option<OAuthParams> {
//!         Some(OAuthParams {
//!             scope_separator: " ",
//!             use_pkce: true,
//!             additional_auth_params: &[],
//!         })
//!     }
//!
//!     fn oauth_refresh(&self) -> Option<OAuthRefresh> {
//!         // WHOOP only returns a new refresh token when `scope=offline` is sent.
//!         Some(OAuthRefresh {
//!             client_auth: RefreshClientAuth::RequestBody,
//!             extra_form: &[("scope", "offline")],
//!         })
//!     }
//!
//!     fn api_base_url(&self) -> &'static str {
//!         "https://api.prod.whoop.com/developer/v2"
//!     }
//!
//!     fn default_scopes(&self) -> &'static [&'static str] {
//!         &["read:profile", "read:workout", "read:sleep", "read:recovery"]
//!     }
//! }
//! ```

use super::core::{FitnessProvider, ProviderConfig};
#[cfg(feature = "provider-whoop")]
use crate::provider_ai_terms;
#[cfg(feature = "provider-whoop")]
use crate::utils::WHOOP_REFRESH_EXTRA_FORM;
use pierre_core::ai_policy::SourcePolicy;
#[cfg(feature = "provider-garmin")]
use pierre_core::constants::oauth::{
    GARMIN_API_BASE_URL, GARMIN_AUTH_URL, GARMIN_DEREGISTRATION_URL, GARMIN_TOKEN_URL,
};
use std::fmt;

#[cfg(feature = "provider-intervals-icu")]
use crate::intervals_icu_provider::{
    DEFAULT_API_BASE_URL as INTERVALS_ICU_API_BASE_URL, DEFAULT_SCOPES as INTERVALS_ICU_SCOPES,
    OAUTH_AUTHORIZE_URL as INTERVALS_ICU_AUTHORIZE_URL,
    OAUTH_REVOKE_URL as INTERVALS_ICU_REVOKE_URL, OAUTH_TOKEN_URL as INTERVALS_ICU_TOKEN_URL,
};

/// OAuth endpoint configuration for providers requiring authentication
#[derive(Debug, Clone)]
pub struct OAuthEndpoints {
    /// OAuth authorization endpoint URL
    pub auth_url: &'static str,
    /// OAuth token endpoint URL
    pub token_url: &'static str,
    /// Optional token revocation endpoint URL
    pub revoke_url: Option<&'static str>,
}

/// OAuth flow parameters specific to each provider
#[derive(Debug, Clone)]
pub struct OAuthParams {
    /// Scope separator character ("," for Strava, " " for WHOOP)
    pub scope_separator: &'static str,
    /// Whether to use PKCE (Proof Key for Code Exchange)
    pub use_pkce: bool,
    /// Additional query parameters for authorization URL
    /// Example: Strava needs `"approval_prompt=force"`
    pub additional_auth_params: &'static [(&'static str, &'static str)],
}

/// How a refresh request presents the client's credentials to the token
/// endpoint. RFC 6749 section 2.3.1 allows both and leaves the choice to the
/// vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshClientAuth {
    /// `client_id` and `client_secret` as fields of the form body.
    RequestBody,
    /// `Authorization: Basic base64(client_id:client_secret)`, with neither
    /// credential in the form body.
    BasicHeader,
}

/// The refresh grant a provider's token endpoint takes: what the platform
/// posts to renew an expired access token, beyond the standard
/// `grant_type=refresh_token` and `refresh_token` fields.
///
/// The token endpoint itself is the descriptor's
/// [`OAuthEndpoints::token_url`], as the registry's default configuration
/// carries it after the `PIERRE_<PROVIDER>_TOKEN_URL` override.
#[derive(Debug, Clone, Copy)]
pub struct OAuthRefresh {
    /// Where the client credentials travel.
    pub client_auth: RefreshClientAuth,
    /// Form fields the vendor requires beyond the standard ones.
    pub extra_form: &'static [(&'static str, &'static str)],
}

impl OAuthRefresh {
    /// The `OAuth2` default: client credentials in the form body and no
    /// vendor field.
    pub const STANDARD: Self = Self {
        client_auth: RefreshClientAuth::RequestBody,
        extra_form: &[],
    };
}

bitflags::bitflags! {
    /// Provider capability flags using bitflags for efficient storage
    ///
    /// Indicates which features a provider supports. Used by the system to
    /// route requests to appropriate providers and generate accurate tool descriptions.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct ProviderCapabilities: u8 {
        /// Provider requires OAuth authentication
        const OAUTH = 0b0000_0001;
        /// Provider supports activity/workout data
        const ACTIVITIES = 0b0000_0010;
        /// Provider supports sleep tracking data
        const SLEEP_TRACKING = 0b0000_0100;
        /// Provider supports recovery/readiness metrics
        const RECOVERY_METRICS = 0b0000_1000;
        /// Provider supports health metrics (weight, HRV, etc.)
        const HEALTH_METRICS = 0b0001_0000;
        /// Provider supports continuous 24/7 data sync (sleep, recovery, HR, steps)
        const CONTINUOUS_DATA = 0b0010_0000;
        /// Fetching one activity's DETAIL costs about as much as fetching the
        /// list — true for HTTP APIs, false for a headless-browser scrape
        /// where each detail is a full page navigation.
        ///
        /// `get_activities` auto-promotes small result sets to detailed, an
        /// N+1 that is milliseconds on an API and minutes on a scrape: a live
        /// 2026-08-12 Telegram turn spent 3m41s of its 4m37s scraping 30
        /// detail pages one at a time, after the list had already returned in
        /// 37s. Absent by default, so a new provider is assumed expensive and
        /// stays fast until it opts in.
        const CHEAP_ACTIVITY_DETAIL = 0b0100_0000;
        /// Provider reads the workouts its calendar plans for the athlete —
        /// what a coach or a plan prescribed for a day — through
        /// `FitnessProvider::list_planned_workouts`.
        ///
        /// This is the last free bit of the `u8`: the next capability widens
        /// the storage type first.
        const PLANNED_WORKOUTS = 0b1000_0000;
    }
}

impl ProviderCapabilities {
    /// Create capabilities for an activity-only provider (like Strava)
    #[must_use]
    pub const fn activity_only() -> Self {
        Self::OAUTH
            .union(Self::ACTIVITIES)
            .union(Self::CHEAP_ACTIVITY_DETAIL)
    }

    /// Create capabilities for a full health provider (like Garmin)
    #[must_use]
    pub const fn full_health() -> Self {
        Self::OAUTH
            .union(Self::ACTIVITIES)
            .union(Self::SLEEP_TRACKING)
            .union(Self::RECOVERY_METRICS)
            .union(Self::HEALTH_METRICS)
            .union(Self::CONTINUOUS_DATA)
            .union(Self::CHEAP_ACTIVITY_DETAIL)
    }

    /// Check if OAuth is required
    #[must_use]
    pub const fn requires_oauth(&self) -> bool {
        self.contains(Self::OAUTH)
    }

    /// Check if activities are supported
    #[must_use]
    pub const fn supports_activities(&self) -> bool {
        self.contains(Self::ACTIVITIES)
    }

    /// Check if sleep tracking is supported
    #[must_use]
    pub const fn supports_sleep(&self) -> bool {
        self.contains(Self::SLEEP_TRACKING)
    }

    /// Check if recovery metrics are supported
    #[must_use]
    pub const fn supports_recovery(&self) -> bool {
        self.contains(Self::RECOVERY_METRICS)
    }

    /// Check if health metrics are supported
    #[must_use]
    pub const fn supports_health(&self) -> bool {
        self.contains(Self::HEALTH_METRICS)
    }

    /// Check if continuous data sync is supported
    #[must_use]
    pub const fn supports_continuous_data(&self) -> bool {
        self.contains(Self::CONTINUOUS_DATA)
    }

    /// Check if the provider reads the workouts its calendar plans
    #[must_use]
    pub const fn supports_planned_workouts(&self) -> bool {
        self.contains(Self::PLANNED_WORKOUTS)
    }
}

/// Describes a provider's identity and capabilities
///
/// This trait is the primary interface for provider metadata. External provider
/// crates implement this trait to describe what they support.
pub trait ProviderDescriptor: Send + Sync {
    /// Unique provider identifier (e.g., "strava", "garmin", "whoop")
    ///
    /// This must be lowercase, alphanumeric, and match the provider name used
    /// in configuration and API requests.
    fn name(&self) -> &'static str;

    /// Human-readable display name (e.g., "Strava", "Garmin Connect", "WHOOP")
    fn display_name(&self) -> &'static str;

    /// Provider capabilities (OAuth, sleep tracking, etc.)
    fn capabilities(&self) -> ProviderCapabilities;

    /// OAuth endpoints if provider requires authentication
    ///
    /// Returns `None` for providers that don't require OAuth (e.g., synthetic provider).
    fn oauth_endpoints(&self) -> Option<OAuthEndpoints>;

    /// OAuth flow parameters specific to this provider
    ///
    /// Returns `None` for providers that don't require OAuth (e.g., synthetic provider).
    /// Defines provider-specific OAuth behavior like scope separators and additional parameters.
    fn oauth_params(&self) -> Option<OAuthParams>;

    /// The refresh grant the platform posts to renew this provider's expired
    /// access tokens, or `None` when it has none the platform can use.
    ///
    /// Required, with no default: a provider whose tokens the platform should
    /// refresh says how, and one it should not says so. An expired token of a
    /// provider answering `None` is a reconnect.
    fn oauth_refresh(&self) -> Option<OAuthRefresh>;

    /// Whether the provider-side owner id, the id this provider's push events
    /// name the athlete by, is missing from its token response and has to be
    /// read from its API with the access token (the provider's `get_athlete`).
    /// The OAuth callback and the platform refresh then read it
    /// ([`crate::owner_id::owner_id_for_access_token`]) and store it with the
    /// token. `false` by default: Strava returns the owner inline.
    fn owner_id_from_api(&self) -> bool {
        false
    }

    /// Base URL for provider API calls
    fn api_base_url(&self) -> &'static str;

    /// Default OAuth scopes to request
    ///
    /// Returns an empty slice for providers without OAuth.
    fn default_scopes(&self) -> &'static [&'static str];

    /// What of this provider's data its terms let a model see, per upstream
    /// source. No restriction unless the provider's terms set one; the
    /// declared policies live in [`crate::provider_ai_terms`].
    fn ai_policy(&self) -> &'static SourcePolicy {
        &SourcePolicy::ALLOW_ALL
    }

    /// Whether this provider requires OAuth authentication
    fn requires_oauth(&self) -> bool {
        self.capabilities().requires_oauth()
    }

    /// Whether this provider supports sleep tracking
    fn supports_sleep(&self) -> bool {
        self.capabilities().supports_sleep()
    }

    /// Whether this provider supports recovery metrics
    fn supports_recovery(&self) -> bool {
        self.capabilities().supports_recovery()
    }

    /// Whether this provider supports health metrics
    fn supports_health(&self) -> bool {
        self.capabilities().supports_health()
    }

    /// Whether this provider supports continuous 24/7 data sync
    fn supports_continuous_data(&self) -> bool {
        self.capabilities().supports_continuous_data()
    }

    /// Build a `ProviderConfig` from this descriptor
    ///
    /// Uses the descriptor's endpoints and scopes to create a configuration
    /// suitable for provider instantiation.
    fn to_config(&self) -> ProviderConfig {
        let (auth_url, token_url, revoke_url) = self.oauth_endpoints().map_or_else(
            || {
                // Synthetic/test providers use placeholder URLs
                (
                    format!("http://localhost/{}/auth", self.name()),
                    format!("http://localhost/{}/token", self.name()),
                    None,
                )
            },
            |endpoints| {
                (
                    endpoints.auth_url.to_owned(),
                    endpoints.token_url.to_owned(),
                    endpoints.revoke_url.map(str::to_owned),
                )
            },
        );

        ProviderConfig {
            name: self.name().to_owned(),
            auth_url,
            token_url,
            api_base_url: self.api_base_url().to_owned(),
            revoke_url,
            default_scopes: self
                .default_scopes()
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }
}

/// Factory function type for creating provider instances
pub type ProviderFactoryFn = fn(ProviderConfig) -> Box<dyn FitnessProvider>;

/// Complete provider bundle for registration
///
/// Combines a provider descriptor with its factory function for easy registration.
/// External crates export a function that returns this bundle.
pub struct ProviderBundle {
    /// Provider descriptor with metadata and capabilities
    pub descriptor: Box<dyn ProviderDescriptor>,
    /// Factory function for creating provider instances
    pub factory: ProviderFactoryFn,
}

impl ProviderBundle {
    /// Create a new provider bundle
    pub fn new(descriptor: Box<dyn ProviderDescriptor>, factory: ProviderFactoryFn) -> Self {
        Self {
            descriptor,
            factory,
        }
    }

    /// Get the provider name from the descriptor
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.descriptor.name()
    }

    /// Create a provider instance using the factory and descriptor's config
    #[must_use]
    pub fn create_provider(&self) -> Box<dyn FitnessProvider> {
        let config = self.descriptor.to_config();
        (self.factory)(config)
    }

    /// Create a provider instance with custom config
    #[must_use]
    pub fn create_provider_with_config(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        (self.factory)(config)
    }
}

impl fmt::Debug for ProviderBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderBundle")
            .field("name", &self.descriptor.name())
            .field("display_name", &self.descriptor.display_name())
            .field("capabilities", &self.descriptor.capabilities())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// Built-in Provider Descriptors (conditionally compiled)
// ============================================================================

/// Strava provider descriptor
#[cfg(feature = "provider-strava")]
pub struct StravaDescriptor;

#[cfg(feature = "provider-strava")]
impl ProviderDescriptor for StravaDescriptor {
    fn name(&self) -> &'static str {
        "strava"
    }

    fn display_name(&self) -> &'static str {
        "Strava"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::activity_only()
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: "https://www.strava.com/oauth/authorize",
            token_url: "https://www.strava.com/oauth/token",
            revoke_url: Some("https://www.strava.com/oauth/revoke"),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: ",",
            use_pkce: true,
            additional_auth_params: &[("approval_prompt", "force")],
        })
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        Some(OAuthRefresh::STANDARD)
    }

    fn api_base_url(&self) -> &'static str {
        "https://www.strava.com/api/v3"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &["activity:read_all"]
    }
}

/// Garmin provider descriptor
#[cfg(feature = "provider-garmin")]
pub struct GarminDescriptor;

#[cfg(feature = "provider-garmin")]
impl ProviderDescriptor for GarminDescriptor {
    fn name(&self) -> &'static str {
        "garmin"
    }

    fn display_name(&self) -> &'static str {
        "Garmin Connect"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_health()
    }

    /// Garmin's `OAuth2` PKCE flow, per the Garmin Connect Developer Program
    /// "OAuth2.0 PKCE Specification". Garmin has no revocation endpoint; the
    /// revoke URL is the user deregistration a disconnect must call.
    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: GARMIN_AUTH_URL,
            token_url: GARMIN_TOKEN_URL,
            revoke_url: Some(GARMIN_DEREGISTRATION_URL),
        })
    }

    /// PKCE is required: the authorization carries `code_challenge` with
    /// `code_challenge_method=S256`, and the code exchange the `code_verifier`.
    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: ",",
            use_pkce: true,
            additional_auth_params: &[],
        })
    }

    /// Garmin's refresh grant: `client_id` and `client_secret` in the form
    /// body beside `grant_type=refresh_token` and the refresh token, with no
    /// other field. Garmin returns a new refresh token on every refresh, which
    /// the platform's compare-and-swap write stores.
    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        Some(OAuthRefresh::STANDARD)
    }

    /// Garmin's token response carries no user id; it is served at `user/id`.
    fn owner_id_from_api(&self) -> bool {
        true
    }

    fn api_base_url(&self) -> &'static str {
        GARMIN_API_BASE_URL
    }

    /// None: Garmin's scope is fixed server-side and its authorization
    /// request takes no `scope` parameter, so none is sent.
    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// WHOOP provider descriptor
///
/// WHOOP is a full health provider supporting sleep, recovery, workouts,
/// and body measurements through their v2 Developer API (v1 was
/// decommissioned by WHOOP in October 2025).
#[cfg(feature = "provider-whoop")]
pub struct WhoopDescriptor;

#[cfg(feature = "provider-whoop")]
impl ProviderDescriptor for WhoopDescriptor {
    fn name(&self) -> &'static str {
        "whoop"
    }

    fn display_name(&self) -> &'static str {
        "WHOOP"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_health()
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: "https://api.prod.whoop.com/oauth/oauth2/auth",
            token_url: "https://api.prod.whoop.com/oauth/oauth2/token",
            revoke_url: Some("https://api.prod.whoop.com/developer/v2/user/access"),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: " ", // WHOOP uses space-separated scopes
            use_pkce: true,
            additional_auth_params: &[],
        })
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        Some(OAuthRefresh {
            client_auth: RefreshClientAuth::RequestBody,
            extra_form: WHOOP_REFRESH_EXTRA_FORM,
        })
    }

    /// WHOOP's token response carries no user id; it is served at
    /// `user/profile/basic`.
    fn owner_id_from_api(&self) -> bool {
        true
    }

    fn api_base_url(&self) -> &'static str {
        "https://api.prod.whoop.com/developer/v2"
    }

    fn ai_policy(&self) -> &'static SourcePolicy {
        &provider_ai_terms::WHOOP
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[
            "offline",
            "read:profile",
            "read:body_measurement",
            "read:workout",
            "read:sleep",
            "read:recovery",
            "read:cycles",
        ]
    }
}

/// COROS provider descriptor
///
/// COROS is a GPS sports watch manufacturer whose partner API offers
/// activities and daily health summaries.
///
/// Note: COROS API documentation is private. OAuth endpoints are placeholders
/// until official documentation is received. Apply for access at:
/// <https://support.coros.com/hc/en-us/articles/17085887816340>
#[cfg(feature = "provider-coros")]
pub struct CorosDescriptor;

#[cfg(feature = "provider-coros")]
impl ProviderDescriptor for CorosDescriptor {
    fn name(&self) -> &'static str {
        "coros"
    }

    fn display_name(&self) -> &'static str {
        "COROS"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // LIMITATION(registre#509): `CorosDescriptor::capabilities` advertises activities
        // only: health data is read from dravr-enforme's synced rows, and the COROS health
        // sync reads the Training Hub session on `sciotte_coros`; the partner API's signed
        // daily push has no receiver.
        ProviderCapabilities::OAUTH.union(ProviderCapabilities::ACTIVITIES)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        // Placeholder URLs - update when COROS provides official API documentation
        Some(OAuthEndpoints {
            auth_url: "https://open.coros.com/oauth2/authorize",
            token_url: "https://open.coros.com/oauth2/token",
            revoke_url: Some("https://open.coros.com/oauth2/revoke"),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: " ", // Placeholder - update when docs received
            use_pkce: true,
            additional_auth_params: &[],
        })
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        // LIMITATION(registre#509): `CorosDescriptor::oauth_refresh` declares no refresh
        // grant: the token endpoint above is a placeholder, and the partner API's refresh
        // grant is undocumented.
        None
    }

    fn api_base_url(&self) -> &'static str {
        // Placeholder URL - update when COROS provides official API documentation
        "https://open.coros.com/api/v1"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        // Placeholder scopes - update when docs received
        &["read:workouts", "read:sleep", "read:daily"]
    }
}

/// Sciotte web scraping provider descriptor
///
/// Sciotte uses browser-based web scraping via CDP to capture session cookies
/// and extract activity data from fitness platform HTML pages (Strava, Garmin, etc.).
/// It does not use OAuth — authentication happens via a streamed browser login session.
#[cfg(feature = "provider-sciotte")]
pub struct SciotteDescriptor;

#[cfg(feature = "provider-sciotte")]
impl ProviderDescriptor for SciotteDescriptor {
    fn name(&self) -> &'static str {
        "sciotte"
    }

    fn display_name(&self) -> &'static str {
        "Strava"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Sciotte uses browser login (not OAuth) and provides activity data
        ProviderCapabilities::ACTIVITIES
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None // Sciotte uses browser-based session cookies, not OAuth
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None // Sciotte uses browser-based session cookies, not OAuth
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        // Sciotte runs in-process via dravr-sciotte — no external API
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[] // No OAuth scopes — browser session-based
    }
}

/// Sciotte Garmin Connect web scraping provider descriptor
#[cfg(feature = "provider-sciotte")]
pub struct SciotteGarminDescriptor;

#[cfg(feature = "provider-sciotte")]
impl ProviderDescriptor for SciotteGarminDescriptor {
    fn name(&self) -> &'static str {
        "sciotte_garmin"
    }

    fn display_name(&self) -> &'static str {
        "Garmin"
    }

    /// Activities are scraped on demand; the night's sleep, resting heart
    /// rate, overnight HRV, stress, Body Battery, body metrics and `VO2max`
    /// are synced from the same session's daily summary by the health sync.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::SLEEP_TRACKING)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Sciotte TrainingPeaks web scraping provider descriptor.
///
/// Activities and the calendar's planned workouts, and never
/// `CHEAP_ACTIVITY_DETAIL`: a TrainingPeaks workout's detail is a browser
/// round trip per workout on the scraper service, the same N+1 the Garmin and
/// Strava mirrors ration. The planned read is one scraper call per window.
#[cfg(feature = "provider-sciotte")]
pub struct SciotteTrainingPeaksDescriptor;

#[cfg(feature = "provider-sciotte")]
impl ProviderDescriptor for SciotteTrainingPeaksDescriptor {
    fn name(&self) -> &'static str {
        "sciotte_trainingpeaks"
    }

    fn display_name(&self) -> &'static str {
        "TrainingPeaks"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::PLANNED_WORKOUTS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Sciotte COROS Training Hub web scraping provider descriptor
#[cfg(feature = "provider-sciotte")]
pub struct SciotteCorosDescriptor;

#[cfg(feature = "provider-sciotte")]
impl ProviderDescriptor for SciotteCorosDescriptor {
    fn name(&self) -> &'static str {
        "sciotte_coros"
    }

    fn display_name(&self) -> &'static str {
        "COROS"
    }

    /// Activities are scraped on demand; the resting heart rate, sleep HRV
    /// and `VO2max` are synced from the Training Hub's daily analysis by the
    /// health sync. The Training Hub carries no sleep sessions.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Intervals.icu endurance-analytics provider descriptor.
///
/// Athletes link through the Dravr OAuth app (the redirect flow every OAuth
/// provider shares), or by pasting their athlete id + personal API key, which
/// stays for coaches and power users. Its tokens never expire and it has no
/// refresh grant, so a refused token is a reconnect.
#[cfg(feature = "provider-intervals-icu")]
pub struct IntervalsIcuDescriptor;

#[cfg(feature = "provider-intervals-icu")]
impl ProviderDescriptor for IntervalsIcuDescriptor {
    fn name(&self) -> &'static str {
        "intervals_icu"
    }

    fn display_name(&self) -> &'static str {
        "Intervals.icu"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Activities, plus the daily wellness feed dravr-enforme syncs into
        // sleep (duration), recovery (HRV, resting HR, the athlete's note) and
        // body (weight) rows. The cheap-detail flag is named because detail is
        // one more HTTP GET, not a browser page load.
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
            .union(ProviderCapabilities::SLEEP_TRACKING)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: INTERVALS_ICU_AUTHORIZE_URL,
            token_url: INTERVALS_ICU_TOKEN_URL,
            revoke_url: Some(INTERVALS_ICU_REVOKE_URL),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            // `ACTIVITY:READ,WELLNESS:READ`, as Intervals.icu documents it.
            scope_separator: ",",
            // Intervals.icu documents a plain code exchange with the client
            // secret, and no PKCE.
            use_pkce: false,
            additional_auth_params: &[],
        })
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // Its tokens never expire and it has no refresh grant
    }

    fn api_base_url(&self) -> &'static str {
        INTERVALS_ICU_API_BASE_URL
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        INTERVALS_ICU_SCOPES
    }
}
