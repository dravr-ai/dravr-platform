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
use crate::field_support::ActivityFieldSupport;
#[cfg(any(
    feature = "provider-whoop",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-intervals-icu"
))]
use crate::field_support::{ActivityFields as F, StreamChannels as S};
#[cfg(any(
    feature = "provider-whoop",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-intervals-icu"
))]
use crate::provider_terms;
#[cfg(feature = "provider-whoop")]
use crate::utils::WHOOP_REFRESH_EXTRA_FORM;
use pierre_core::ai_policy::SourcePolicy;
#[cfg(feature = "provider-garmin")]
use pierre_core::constants::oauth::{
    GARMIN_API_BASE_URL, GARMIN_AUTH_URL, GARMIN_DEREGISTRATION_URL, GARMIN_TOKEN_URL,
};
use pierre_core::transport::TransportPolicy;
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

/// The refresh grant a provider's token endpoint takes: what the platform
/// posts to renew an expired access token, beyond the standard
/// `grant_type=refresh_token` and `refresh_token` fields.
///
/// The client's credentials always travel as `client_id` and `client_secret`
/// fields of the form body (RFC 6749 section 2.3.1), the shape every shipped
/// vendor takes.
///
/// The token endpoint itself is the descriptor's
/// [`OAuthEndpoints::token_url`], as the registry's default configuration
/// carries it after the `PIERRE_<PROVIDER>_TOKEN_URL` override.
#[derive(Debug, Clone, Copy)]
pub struct OAuthRefresh {
    /// Form fields the vendor requires beyond the standard ones.
    pub extra_form: &'static [(&'static str, &'static str)],
}

impl OAuthRefresh {
    /// The `OAuth2` default: client credentials in the form body and no
    /// vendor field.
    pub const STANDARD: Self = Self { extra_form: &[] };
}

bitflags::bitflags! {
    /// Provider capability flags using bitflags for efficient storage
    ///
    /// Indicates which features a provider supports. Used by the system to
    /// route requests to appropriate providers and generate accurate tool descriptions.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct ProviderCapabilities: u16 {
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
        /// `get_activities` details the activities of the last fourteen days, an
        /// N+1 that is milliseconds on an API and minutes on a scrape: a live
        /// 2026-08-12 Telegram turn spent 3m41s of its 4m37s scraping 30
        /// detail pages one at a time, after the list had already returned in
        /// 37s. Absent by default, so a new provider is assumed expensive and
        /// stays fast until it opts in.
        const CHEAP_ACTIVITY_DETAIL = 0b0100_0000;
        /// Provider reads the workouts its calendar plans for the athlete —
        /// what a coach or a plan prescribed for a day — through
        /// `FitnessProvider::list_planned_workouts`.
        const PLANNED_WORKOUTS = 0b1000_0000;
        /// Provider is a coaching platform whose coach account reads the
        /// athletes who share with it: it lists the coach's roster through
        /// `FitnessProvider::read_coach_roster` and reads one athlete through
        /// the coach's own credential, its factory offering
        /// `ProviderFactory::delegated_reads`. The coach's credential may be
        /// an OAuth grant or an API key (`CredentialKind`).
        const COACH_ROSTER = 0b0000_0001_0000_0000;
        /// Provider writes its training calendar — `list_calendar_events`
        /// (with `calendar_event_detail` when its listing names identity
        /// only), `push_planned_session`, `update_planned_session` and
        /// `delete_planned_sessions` on `FitnessProvider` — so the plan push
        /// and `prescribe_workout` can target it.
        ///
        /// Distinct from `PLANNED_WORKOUTS`, which only reads: a provider can
        /// show the athlete's planned workouts without accepting ours.
        const CALENDAR_WRITE = 0b0000_0010_0000_0000;
        /// Provider's training calendar holds a note pinned to a whole week
        /// (a plan week's focus) as well as sessions. Without it a plan push
        /// sends only the sessions: a calendar of workouts (Wahoo) is never
        /// sent a week note it would refuse on every push.
        const CALENDAR_WEEK_NOTES = 0b0000_0100_0000_0000;
        /// Provider's activities are the recordings themselves — sport,
        /// distance and route as the device or recording app captured them.
        ///
        /// A provider that declares `ACTIVITIES` without this flag serves
        /// workouts it detected from wrist heart rate (WHOOP): no distance
        /// unless its own app recorded GPS, and a sport label a heuristic
        /// chose. On 2026-08-22 that shape answered a 200 km ride as a
        /// distance-less "run" because the athlete had connected WHOOP after
        /// Strava. Activity-source election
        /// (`crate::activity_source::elect_activity_source`) ranks a
        /// provider declaring this ahead of one that only detects, among
        /// connections equally healthy.
        const RECORDED_ACTIVITIES = 0b0000_1000_0000_0000;
        /// Provider delivers its data only by pushing it to Dravr: nothing
        /// can be read from it on request, so its connection is read from
        /// what Dravr stored when each push arrived.
        ///
        /// The Garmin Health and Activity APIs at the Starter tier push
        /// every summary and serve no list, detail or backfill read
        /// (carnet#1155). A list read of such a provider is its cached
        /// window with no live top-up, never written back through the
        /// cache, and its freshness is the last push
        /// ([`ProviderCapabilities::is_push_only`]). A delivery mode, not a
        /// data scope: it names no capability on a provider card.
        const PUSH_ONLY = 0b0001_0000_0000_0000;
        /// Provider lists the athlete's efforts on one segment over a window,
        /// across every activity, through
        /// `FitnessProvider::segment_effort_history`: one call where the
        /// stored details would need every activity read. A read capability,
        /// never a data scope: it names nothing on a provider card.
        const SEGMENT_EFFORT_HISTORY = 0b0010_0000_0000_0000;
    }
}

impl ProviderCapabilities {
    /// Create capabilities for an activity-only provider (like Strava)
    #[must_use]
    pub const fn activity_only() -> Self {
        Self::OAUTH
            .union(Self::ACTIVITIES)
            .union(Self::RECORDED_ACTIVITIES)
            .union(Self::CHEAP_ACTIVITY_DETAIL)
    }

    /// Create capabilities for a full health provider (like Garmin)
    #[must_use]
    pub const fn full_health() -> Self {
        Self::OAUTH
            .union(Self::ACTIVITIES)
            .union(Self::RECORDED_ACTIVITIES)
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

    /// Check if the provider's activities are recordings rather than detections
    #[must_use]
    pub const fn supports_recorded_activities(&self) -> bool {
        self.contains(Self::RECORDED_ACTIVITIES)
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

    /// Check if the provider lists the athlete's efforts on one segment
    #[must_use]
    pub const fn supports_segment_effort_history(&self) -> bool {
        self.contains(Self::SEGMENT_EFFORT_HISTORY)
    }

    /// Check if a coach account reads its athletes through the provider
    #[must_use]
    pub const fn supports_coach_roster(&self) -> bool {
        self.contains(Self::COACH_ROSTER)
    }

    /// Check if the provider accepts writes to its training calendar
    #[must_use]
    pub const fn supports_calendar_write(&self) -> bool {
        self.contains(Self::CALENDAR_WRITE)
    }

    /// Check if a coach account writes the calendars of the athletes it
    /// reads: a coaching platform whose training calendar takes writes
    #[must_use]
    pub const fn supports_coach_calendar_write(&self) -> bool {
        self.contains(Self::COACH_ROSTER.union(Self::CALENDAR_WRITE))
    }

    /// Check if the provider's calendar holds week notes
    #[must_use]
    pub const fn supports_calendar_week_notes(&self) -> bool {
        self.contains(Self::CALENDAR_WEEK_NOTES)
    }

    /// Check if the provider only pushes its data: a read of it is served
    /// from what Dravr stored, never from the provider
    #[must_use]
    pub const fn is_push_only(&self) -> bool {
        self.contains(Self::PUSH_ONLY)
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

    /// What this backend's mapper fills on the activities it returns, field
    /// by field, on a list row and on a detail read ([`crate::field_support`]).
    /// Nothing by default: a backend promises no field until it declares it.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport::NONE
    }

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

    /// Whether a refresh this provider refuses is the athlete withdrawing the
    /// grant on the provider's side.
    ///
    /// A provider that sends no deauthorization notice says so no other way,
    /// and its terms can still require the athlete's data deleted when they
    /// revoke (Wahoo). When `true`, a refusal the platform's refresh confirms
    /// disconnects the athlete and deletes the provider's data, as a
    /// deauthorization notice would; otherwise the connection only waits for
    /// a reconnect. `false` by default.
    fn refused_refresh_is_revocation(&self) -> bool {
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
    /// declared policies live in [`crate::provider_terms`].
    fn ai_policy(&self) -> &'static SourcePolicy {
        &SourcePolicy::ALLOW_ALL
    }

    /// Where this provider's terms let its data be served: only to Dravr's
    /// own surfaces (the default), or over every transport. The default fails
    /// closed: a provider whose terms nobody has read keeps its data off MCP,
    /// A2A and API keys until its descriptor declares
    /// [`TransportPolicy::AnyTransport`]. The declared policies live in
    /// [`crate::provider_terms`].
    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::FirstPartyOnly
    }

    /// The relayed sources this provider's terms settle by name, each with
    /// where its items may be served: the transport counterpart of
    /// [`SourcePolicy::by_source`] (matched case-insensitively). A named
    /// source decides alone; one not named keeps this provider's
    /// [`Self::transport_policy`] and its origin's. Empty by default.
    fn transport_by_source(&self) -> &'static [(&'static str, TransportPolicy)] {
        &[]
    }

    /// The service whose data this descriptor's backend reads, when that is
    /// not the descriptor itself: a scraper names the provider it scrapes
    /// (`sciotte_trainingpeaks` reads `trainingpeaks`). Items stamped with
    /// that service's name — a scraped activity's `source`, a planned
    /// workout's `provider` — resolve to this descriptor's terms when no
    /// descriptor carries the name itself. `None` by default.
    fn origin(&self) -> Option<&'static str> {
        None
    }

    /// Whether this provider's terms bar using its data to improve Dravr's AI
    /// for other athletes: the cross-athlete priors the archetype aggregation
    /// learns from every athlete's playbooks. `false` unless a provider's
    /// terms say so.
    fn bars_cross_athlete_learning(&self) -> bool {
        false
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
#[derive(Debug)]
pub struct StravaDescriptor;

#[cfg(feature = "provider-strava")]
impl ProviderDescriptor for StravaDescriptor {
    /// The API's summary on every list row; the athlete's words, the gear and
    /// the device, laps, splits and every segment effort on the detail read;
    /// nine stream channels.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::WORKOUT_TYPE
                .union(F::TRAINER)
                .union(F::COMMUTE)
                .union(F::AVERAGE_HEART_RATE)
                .union(F::MAX_HEART_RATE)
                .union(F::AVERAGE_POWER)
                .union(F::NORMALIZED_POWER)
                .union(F::MOVING_TIME)
                .union(F::AVERAGE_CADENCE)
                .union(F::SUMMARY_POLYLINE)
                .union(F::SPORT_TYPE_DETAIL),
            detail_only: F::DESCRIPTION
                .union(F::PRIVATE_NOTE)
                .union(F::CALORIES)
                .union(F::GEAR)
                .union(F::DEVICE)
                .union(F::LAPS)
                .union(F::SPLITS)
                .union(F::SEGMENT_EFFORTS)
                .union(F::PERCEIVED_EXERTION),
            streams: S::all(),
        }
    }

    fn name(&self) -> &'static str {
        "strava"
    }

    fn display_name(&self) -> &'static str {
        "Strava"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::activity_only().union(ProviderCapabilities::SEGMENT_EFFORT_HISTORY)
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

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::STRAVA_TRANSPORT
    }
}

/// Garmin provider descriptor
#[cfg(feature = "provider-garmin")]
#[derive(Debug)]
pub struct GarminDescriptor;

#[cfg(feature = "provider-garmin")]
impl ProviderDescriptor for GarminDescriptor {
    /// What Garmin pushes: the summary's readings on every activity, and the
    /// samples, laps and the power derived from them once its details arrive.
    /// No measured push payload carries a description, workout tag or trainer
    /// flag, so none is declared ([`crate::field_support`]).
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::MAX_HEART_RATE)
                .union(F::AVERAGE_CADENCE)
                .union(F::CALORIES)
                .union(F::DEVICE),
            detail_only: F::AVERAGE_POWER.union(F::NORMALIZED_POWER).union(F::LAPS),
            streams: S::all(),
        }
    }

    fn name(&self) -> &'static str {
        "garmin"
    }

    fn display_name(&self) -> &'static str {
        "Garmin Connect"
    }

    /// Every Garmin summary is pushed to Dravr; the Starter tier serves no
    /// list, detail or backfill read, so reads come from what each push
    /// stored ([`ProviderCapabilities::PUSH_ONLY`]). The training calendar is
    /// written through the Training API ([`ProviderCapabilities::CALENDAR_WRITE`]),
    /// for an athlete who granted Dravr `WORKOUT_IMPORT`; the provider refuses
    /// one who did not, by name. Its calendar holds workouts, never week notes.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_health()
            .union(ProviderCapabilities::PUSH_ONLY)
            .union(ProviderCapabilities::CALENDAR_WRITE)
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

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::GARMIN_TRANSPORT
    }
}

/// WHOOP provider descriptor
///
/// WHOOP is a full health provider supporting sleep, recovery, workouts,
/// and body measurements through their v2 Developer API (v1 was
/// decommissioned by WHOOP in October 2025).
#[cfg(feature = "provider-whoop")]
#[derive(Debug)]
pub struct WhoopDescriptor;

#[cfg(feature = "provider-whoop")]
impl ProviderDescriptor for WhoopDescriptor {
    /// A detected workout's score: heart rate, energy and WHOOP's sport name.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::MAX_HEART_RATE)
                .union(F::CALORIES)
                .union(F::SPORT_TYPE_DETAIL),
            detail_only: F::empty(),
            streams: S::empty(),
        }
    }

    fn name(&self) -> &'static str {
        "whoop"
    }

    fn display_name(&self) -> &'static str {
        "WHOOP"
    }

    /// Everything a full health provider reads, but its workouts are the
    /// sessions the strap detected from heart rate, not recordings: no
    /// `RECORDED_ACTIVITIES`, so a healthy recording source the athlete also
    /// connected answers their activity questions first.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::SLEEP_TRACKING)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
            .union(ProviderCapabilities::CONTINUOUS_DATA)
            .union(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
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
        &provider_terms::WHOOP
    }

    /// WHOOP's API Terms bar exposing WHOOP Data to third parties without the
    /// athlete's explicit opt-in, which Dravr does not record (carnet#766).
    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::WHOOP_TRANSPORT
    }

    /// WHOOP's API Terms (§4, effective 2026-10-06) bar using WHOOP Data to
    /// "create, develop, test, train, fine-tune or improve" an AI system.
    /// Per-athlete coaching reads it; learning from it for other athletes
    /// does not (carnet#539 point 5).
    fn bars_cross_athlete_learning(&self) -> bool {
        true
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
#[derive(Debug)]
pub struct CorosDescriptor;

#[cfg(feature = "provider-coros")]
impl ProviderDescriptor for CorosDescriptor {
    /// The partner API's workout summary.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::MAX_HEART_RATE)
                .union(F::CALORIES)
                .union(F::AVERAGE_CADENCE)
                .union(F::AVERAGE_POWER)
                .union(F::SPORT_TYPE_DETAIL),
            detail_only: F::empty(),
            streams: S::empty(),
        }
    }

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
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::RECORDED_ACTIVITIES)
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

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::COROS_TRANSPORT
    }
}

/// The four sciotte backends' descriptors.
#[cfg(feature = "provider-sciotte")]
mod sciotte_descriptors;
#[cfg(feature = "provider-sciotte")]
pub use sciotte_descriptors::{
    SciotteCorosDescriptor, SciotteDescriptor, SciotteGarminDescriptor,
    SciotteTrainingPeaksDescriptor,
};

/// Intervals.icu endurance-analytics provider descriptor.
///
/// Athletes link through the Dravr OAuth app (the redirect flow every OAuth
/// provider shares), or by pasting their athlete id + personal API key, which
/// stays for coaches and power users. Its tokens never expire and it has no
/// refresh grant, so a refused token is a reconnect.
#[cfg(feature = "provider-intervals-icu")]
#[derive(Debug)]
pub struct IntervalsIcuDescriptor;

#[cfg(feature = "provider-intervals-icu")]
impl ProviderDescriptor for IntervalsIcuDescriptor {
    /// The whole activity on every list row, the athlete's words and flags
    /// included; its comment thread on a detail read; seven stream channels.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::DESCRIPTION
                .union(F::TRAINER)
                .union(F::COMMUTE)
                .union(F::AVERAGE_HEART_RATE)
                .union(F::MAX_HEART_RATE)
                .union(F::AVERAGE_POWER)
                .union(F::NORMALIZED_POWER)
                .union(F::MOVING_TIME)
                .union(F::CALORIES)
                .union(F::AVERAGE_CADENCE)
                .union(F::DEVICE)
                .union(F::PERCEIVED_EXERTION),
            detail_only: F::COMMENTS,
            streams: S::TIME
                .union(S::HEART_RATE)
                .union(S::POWER)
                .union(S::CADENCE)
                .union(S::SPEED)
                .union(S::ALTITUDE)
                .union(S::LATLNG),
        }
    }

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
        //
        // `Wellness` rows name no source or device, so their Garmin attribution is
        // inferred from the athlete's most recent intervals.icu activity device
        // (pierre-services `stored_health`, carnet#521).
        //
        // A coach's API key lists the athletes who share with them and reads
        // each at their own path (`IntervalsIcuProviderFactory::delegated_reads`).
        // It also writes its training calendar: it implements the four
        // calendar methods on `FitnessProvider`, and a coach's key writes the
        // calendar of each athlete who made them their coach
        // (`create_delegated_calendar_writer`). The same calendar is read for
        // the workouts and races planned on it, and the activity that
        // completed each (`list_planned_workouts`).
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::RECORDED_ACTIVITIES)
            .union(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
            .union(ProviderCapabilities::SLEEP_TRACKING)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
            .union(ProviderCapabilities::COACH_ROSTER)
            .union(ProviderCapabilities::CALENDAR_WRITE)
            .union(ProviderCapabilities::CALENDAR_WEEK_NOTES)
            .union(ProviderCapabilities::PLANNED_WORKOUTS)
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

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::INTERVALS_ICU_TRANSPORT
    }

    fn transport_by_source(&self) -> &'static [(&'static str, TransportPolicy)] {
        provider_terms::INTERVALS_ICU_TRANSPORT_BY_SOURCE
    }
}
