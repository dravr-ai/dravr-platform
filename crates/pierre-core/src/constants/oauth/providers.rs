// ABOUTME: OAuth provider identifiers and validation functions
// ABOUTME: Centralizes provider name constants to eliminate hardcoded strings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OAuth provider constants
//!
//! Note: For dynamic provider discovery, prefer using `ProviderRegistry::supported_providers()`
//! instead of the static `all()` function. The registry respects feature flags and includes
//! externally registered providers.

/// Strava fitness provider identifier
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-strava` feature.
pub const STRAVA: &str = "strava";

/// Garmin fitness provider identifier
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-garmin` feature.
pub const GARMIN: &str = "garmin";

/// Terra unified fitness provider identifier (150+ wearables)
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-terra` feature.
pub const TERRA: &str = "terra";

/// WHOOP fitness provider identifier
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-whoop` feature.
pub const WHOOP: &str = "whoop";

/// COROS fitness provider identifier (GPS sports watches)
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-coros` feature.
pub const COROS: &str = "coros";

/// Intervals.icu endurance analytics provider identifier.
///
/// Links by OAuth or by the athlete's personal API key (HTTP Basic
/// `API_KEY:<key>`), gated behind `provider-intervals-icu` feature.
pub const INTERVALS_ICU: &str = "intervals_icu";

/// Intervals.icu default OAuth scopes.
///
/// Completed activities and wellness to read, the calendar to write planned
/// sessions into, and the athlete's settings (zones, thresholds) to read.
/// Intervals.icu joins them with commas.
pub const INTERVALS_ICU_DEFAULT_SCOPES: &[&str] = &[
    "ACTIVITY:READ",
    "WELLNESS:READ",
    "CALENDAR:WRITE",
    "SETTINGS:READ",
];

/// Sciotte web scraping provider identifier.
///
/// Browser-based data extraction, gated behind `provider-sciotte` feature.
pub const SCIOTTE: &str = "sciotte";

/// Sciotte Garmin web scraping provider identifier.
///
/// Browser-based Garmin Connect data extraction, gated behind `provider-sciotte` feature.
pub const SCIOTTE_GARMIN: &str = "sciotte_garmin";

/// TrainingPeaks user-facing provider identifier.
///
/// TrainingPeaks has no OAuth backend inside Pierre — its partner API is
/// closed — so this name never has a factory of its own. It exists so the
/// backend resolver can hide the scrape mirror behind it, exactly as `garmin`
/// hides `sciotte_garmin`.
pub const TRAININGPEAKS: &str = "trainingpeaks";

/// Sciotte TrainingPeaks web scraping provider identifier.
///
/// Browser-based TrainingPeaks calendar extraction, gated behind `provider-sciotte` feature.
pub const SCIOTTE_TRAININGPEAKS: &str = "sciotte_trainingpeaks";

/// Sciotte COROS web scraping provider identifier.
///
/// Browser-based COROS Training Hub data extraction, gated behind
/// `provider-sciotte` feature. The mirror of `coros` until the partner API
/// (carnet#509) is approved.
pub const SCIOTTE_COROS: &str = "sciotte_coros";

// LIMITATION(registre#657): no `NOLIO` provider identifier, descriptor or factory exists, so a
// coach whose roster and plans live in Nolio (OAuth 2.0 API at nolio.io) cannot connect it.

// LIMITATION(registre#713): no `VEKTA` provider identifier, descriptor or factory exists, so a
// coach or athlete whose sessions, CP/W' and sleep live in Vekta (bearer-key API at
// api.joinvekta.com) cannot connect it.

/// Which accounts a provider's notice is asked of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAudience {
    /// Every account that connects the provider, whatever its feature flags:
    /// the provider's own terms bind every connection, so no operator switch
    /// can waive the notice.
    EveryAccount,
    /// Only the accounts the `provider_exposure_notice` feature flag arms,
    /// per tenant or per user; an account the flag leaves off is never asked.
    FlagArmedAccounts,
}

/// A notice a provider requires the account to accept before connecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderNotice {
    /// The backend the notice guards.
    pub backend: &'static str,
    /// The notice's dated version; an acceptance of any other version is no
    /// acceptance of this one.
    pub version: &'static str,
    /// Which accounts are asked for it.
    pub audience: NoticeAudience,
    /// What accepting it gives Dravr, and so whether it can be withdrawn.
    pub kind: NoticeKind,
}

/// What accepting a provider's notice gives Dravr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// An acknowledgement of a risk the provider's terms of use create. It
    /// gates the connect and nothing after it, so there is nothing to
    /// withdraw: a disconnect ends the exposure.
    TermsExposure,
    /// Consent to hand the provider's data to an AI model, which the
    /// provider's terms let the athlete withdraw at any time, as simply as it
    /// was given. While it is not given, or once it is withdrawn, no model
    /// reads that provider's data; the athlete still sees what is stored and
    /// the connection stays live.
    AiConsent,
}

impl ProviderNotice {
    /// Whether the account can withdraw its acceptance once given: only a
    /// consent to AI use can be.
    #[must_use]
    pub const fn withdrawable(&self) -> bool {
        matches!(self.kind, NoticeKind::AiConsent)
    }
}

/// Every notice a provider requires the account to accept before connecting,
/// by the backend it guards.
///
/// Two kinds of notice share this table and the one acceptance record behind
/// it:
///
/// - **Terms-of-use exposure.** A provider read by signing in with the
///   athlete's own account, against its terms of use, states the risk before
///   the credentials: TrainingPeaks (Terms of Use section 13) and COROS (Terms
///   of Service sections 4 and 7). Asked of the accounts the
///   `provider_exposure_notice` flag arms
///   ([`NoticeAudience::FlagArmedAccounts`]). A [`NoticeKind::TermsExposure`]:
///   nothing to withdraw.
/// - **Owner authorization.** WHOOP's API Terms of Use (section 4, effective
///   2026-10-06) let Dravr store WHOOP Data, compute from it and hand it to
///   the coach only as the data's owner expressly authorizes; the athlete
///   gives that authorization before the WHOOP OAuth flow begins, and health
///   sync keeps no WHOOP record for an account that has not. The terms bind
///   every WHOOP connection, so it is asked of every account
///   ([`NoticeAudience::EveryAccount`]). A [`NoticeKind::AiConsent`]: the
///   athlete withdraws it from the privacy settings or the connection card,
///   after which no model reads WHOOP data and health sync keeps no new WHOOP
///   record until it is given again (carnet#726).
///
/// A connect to one of these backends — a credential login or the start of an
/// OAuth flow — is refused until an account asked for the notice accepts its
/// current version; the web and mobile connect surfaces and the hosted pages
/// show it. An account that accepted a version connects again without being
/// asked. Bump a provider's version whenever its notice text changes on any
/// surface or in any locale, so every account is asked again.
pub const PROVIDER_NOTICES: [ProviderNotice; 3] = [
    ProviderNotice {
        backend: SCIOTTE_TRAININGPEAKS,
        version: "2026-09-24",
        audience: NoticeAudience::FlagArmedAccounts,
        kind: NoticeKind::TermsExposure,
    },
    ProviderNotice {
        backend: SCIOTTE_COROS,
        version: "2026-09-24",
        audience: NoticeAudience::FlagArmedAccounts,
        kind: NoticeKind::TermsExposure,
    },
    ProviderNotice {
        backend: WHOOP,
        version: "2026-09-25",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::AiConsent,
    },
];

/// The attribution shown beside data a Garmin device recorded.
///
/// intervals.icu's API terms (effective 2025-10-23) require it "in the form
/// and manner required by Garmin's brand guidelines" wherever information
/// derived from Garmin-sourced data is displayed (carnet#521); Nolio's terms
/// carry the same duty for `source=garmin`.
pub const GARMIN_ATTRIBUTION: &str = "Garmin";

/// The attribution an activity must be shown with, by its upstream `source`:
/// [`GARMIN_ATTRIBUTION`] for Garmin-sourced data, `None` otherwise.
#[must_use]
pub fn source_attribution(source: Option<&str>) -> Option<&'static str> {
    source
        .filter(|source| source.eq_ignore_ascii_case(GARMIN))
        .map(|_| GARMIN_ATTRIBUTION)
}

/// The longest device model an attribution carries; a provider's device name
/// is free text, and the attribution is shown beside a title.
const DEVICE_MODEL_MAX_CHARS: usize = 40;

/// Whether a recording device's name names a Garmin device, as intervals.icu's
/// API terms identify one: the name contains "garmin", in any case.
#[must_use]
pub fn garmin_device(device_name: Option<&str>) -> bool {
    device_name.is_some_and(|name| name.to_ascii_lowercase().contains(GARMIN))
}

/// Whether an activity holds Garmin-sourced data: relayed from Garmin
/// (`source`), or recorded on a Garmin device whichever service relayed it
/// (`device_name`).
#[must_use]
fn garmin_recorded(source: Option<&str>, device_name: Option<&str>) -> bool {
    source_attribution(source).is_some() || garmin_device(device_name)
}

/// The attribution an activity must be shown with, beside its title.
///
/// Garmin's brand guidelines name the device: `Garmin Forerunner 965` when
/// `device_name` names a Garmin device and its model; plain
/// [`GARMIN_ATTRIBUTION`] for Garmin-sourced data whose device is unknown;
/// `None` for anything else.
#[must_use]
pub fn activity_attribution(source: Option<&str>, device_name: Option<&str>) -> Option<String> {
    if !garmin_recorded(source, device_name) {
        return None;
    }
    let model = device_name
        .filter(|name| garmin_device(Some(name)))
        .map(garmin_device_model)
        .unwrap_or_default();
    Some(if model.is_empty() {
        GARMIN_ATTRIBUTION.to_owned()
    } else {
        format!("{GARMIN_ATTRIBUTION} {model}")
    })
}

/// The model a Garmin device name carries: the name without the brand, the
/// punctuation left around it, control characters or redundant spacing, at most
/// [`DEVICE_MODEL_MAX_CHARS`] characters. Empty when nothing but the brand
/// is named.
fn garmin_device_model(device_name: &str) -> String {
    let lower = device_name.to_ascii_lowercase();
    let mut rest = String::with_capacity(device_name.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find(GARMIN) {
        rest.push_str(&device_name[cursor..cursor + found]);
        rest.push(' ');
        cursor += found + GARMIN.len();
    }
    rest.push_str(&device_name[cursor..]);
    let words: Vec<&str> = rest
        .split(|c: char| c.is_whitespace() || c.is_control() || c == '_')
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .collect();
    words
        .join(" ")
        .chars()
        .take(DEVICE_MODEL_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The backends whose notice is a consent to AI use
/// ([`NoticeKind::AiConsent`]): while an account has not given it, or once it
/// withdrew it, no model reads that backend's data.
pub fn ai_consent_backends() -> impl Iterator<Item = &'static str> {
    PROVIDER_NOTICES
        .iter()
        .filter(|notice| notice.kind == NoticeKind::AiConsent)
        .map(|notice| notice.backend)
}

/// The notice `backend` requires, or `None` when the provider asks for none.
#[must_use]
pub fn provider_notice(backend: &str) -> Option<&'static ProviderNotice> {
    PROVIDER_NOTICES
        .iter()
        .find(|notice| notice.backend == backend)
}

/// The current notice version for `backend`, or `None` when the provider
/// asks for no notice.
#[must_use]
pub fn provider_terms_version(backend: &str) -> Option<&'static str> {
    provider_notice(backend).map(|notice| notice.version)
}

/// Synthetic fitness provider identifier (for testing)
/// Note: Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-synthetic` feature.
pub const SYNTHETIC: &str = "synthetic";

/// Synthetic sleep provider identifier (for cross-provider testing).
///
/// Used to simulate a second provider that provides sleep data while
/// the primary synthetic provider provides activity data.
/// Provider name constants are always available for configuration;
/// the provider implementation is gated behind `provider-synthetic` feature.
pub const SYNTHETIC_SLEEP: &str = "synthetic_sleep";

/// Whether `provider` answers from locally generated data and so needs no
/// connection, token, or session of any kind.
///
/// Deliberately narrower than "does not use OAuth". `sciotte`, `sciotte_garmin`
/// and `sciotte_trainingpeaks` skip OAuth but run on a browser session the
/// athlete still has to establish, and `coros` is simply unconfigured — a request naming any
/// of those from an athlete with nothing connected must still be refused, or
/// the refusal that sends them to connect never fires. Only the synthetic
/// providers make their data up on the spot.
///
/// Used by the dispatch chokepoint to stand aside for demo and seeded accounts.
/// Membership is a closed list on purpose: keying the bypass on a negative
/// ("not known to need OAuth") would hand a way past the gate to every
/// unrecognized string, including the `"all"` sentinel and any provider name an
/// LLM invents.
#[must_use]
pub fn is_credential_free(provider: &str) -> bool {
    matches!(provider, SYNTHETIC | SYNTHETIC_SLEEP)
}

/// Get statically-known OAuth providers
///
/// **Deprecated**: Use `crate::providers::get_supported_providers()` instead,
/// which respects feature flags and includes externally registered providers.
#[must_use]
#[deprecated(
    since = "0.2.0",
    note = "Use crate::providers::get_supported_providers() for dynamic provider discovery"
)]
pub const fn all() -> &'static [&'static str] {
    // This is a compile-time constant, so we include all potential providers
    // For runtime checking, use the registry
    &["strava", "garmin", "whoop", "coros", "synthetic", "sciotte"]
}

/// Check if a provider is statically known
///
/// **Deprecated**: Use `crate::providers::is_provider_supported()` instead,
/// which respects feature flags and includes externally registered providers.
#[must_use]
#[deprecated(
    since = "0.2.0",
    note = "Use crate::providers::is_provider_supported() for dynamic provider validation"
)]
#[allow(deprecated)]
pub fn is_supported(provider: &str) -> bool {
    all().contains(&provider)
}

/// Token type recorded in `user_oauth_tokens.token_type` for sciotte
/// browser-session credentials.
///
/// OAuth exchanges record the provider's own token type (`"Bearer"` in
/// practice). A session row's `access_token` holds the serialized browser
/// session JSON the sciotte scraper restores — it is unusable as a Bearer
/// token, and a Bearer token is unusable as a session.
pub const TOKEN_TYPE_SESSION: &str = "session";

/// Strava default scopes (comma-separated as per Strava API requirements)
pub const STRAVA_DEFAULT_SCOPES: &str = "activity:read_all";

/// Garmin's `OAuth2` PKCE authorization endpoint.
///
/// Garmin Connect Developer Program "OAuth2.0 PKCE Specification": the user
/// is sent here with `response_type=code`, `client_id`, `code_challenge` and
/// `code_challenge_method=S256`. The provider descriptor, the registry, the
/// tenant OAuth client and the server configuration all read Garmin's
/// endpoints from these constants, since `pierre-auth` and `pierre-config`
/// sit below the provider crate and cannot read its descriptor.
pub const GARMIN_AUTH_URL: &str = "https://connect.garmin.com/oauth2Confirm";

/// Garmin's `OAuth2` token endpoint, for both the code exchange and the
/// refresh grant. Garmin takes the client credentials in the form body and
/// rotates the refresh token on every refresh.
pub const GARMIN_TOKEN_URL: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";

/// Garmin Health (wellness) API base URL; calls carry the access token as `Bearer`.
pub const GARMIN_API_BASE_URL: &str = "https://apis.garmin.com/wellness-api/rest";

/// Garmin user deregistration endpoint.
///
/// Garmin has no token-revocation endpoint: a disconnect withdraws consent
/// with a `DELETE` on the user's registration, authenticated with the user's
/// access token as `Bearer`.
pub const GARMIN_DEREGISTRATION_URL: &str =
    "https://apis.garmin.com/wellness-api/rest/user/registration";

/// Terra default scopes (data types)
pub const TERRA_DEFAULT_SCOPES: &str = "activity,sleep,body,daily,nutrition";

/// WHOOP default scopes (space-separated as per WHOOP API requirements)
/// - `offline`: Required for refresh tokens
/// - `read:profile`: Access to user profile information
/// - `read:body_measurement`: Access to height, weight, max heart rate
/// - `read:workout`: Access to workout/activity data
/// - `read:sleep`: Access to sleep data
/// - `read:recovery`: Access to recovery scores
/// - `read:cycles`: Access to cycle data (strain, recovery aggregation)
pub const WHOOP_DEFAULT_SCOPES: &str =
    "offline read:profile read:body_measurement read:workout read:sleep read:recovery read:cycles";

/// COROS default scopes (placeholder - update when API docs received).
///
/// COROS API documentation is private. Apply at:
/// <https://support.coros.com/hc/en-us/articles/17085887816340>
///
/// Known data types from Terra integration: activities, sleep, daily summaries.
pub const COROS_DEFAULT_SCOPES: &str = "read:workouts read:sleep read:daily";

#[cfg(test)]
mod tests {
    use super::{activity_attribution, garmin_recorded};

    #[test]
    fn a_garmin_device_is_named_by_its_model() {
        for (device, expected) in [
            ("Garmin Forerunner 965", "Garmin Forerunner 965"),
            ("GARMIN EDGE 1050", "Garmin EDGE 1050"),
            ("garmin_fenix_8", "Garmin fenix 8"),
            ("Edge 840 (Garmin)", "Garmin Edge 840"),
            ("  Garmin   Venu\t3 ", "Garmin Venu 3"),
        ] {
            assert_eq!(
                activity_attribution(None, Some(device)).as_deref(),
                Some(expected),
                "{device}"
            );
        }
    }

    #[test]
    fn garmin_data_without_a_named_model_carries_the_brand_alone() {
        assert_eq!(
            activity_attribution(Some("garmin"), None).as_deref(),
            Some("Garmin")
        );
        assert_eq!(
            activity_attribution(None, Some("Garmin")).as_deref(),
            Some("Garmin")
        );
        assert_eq!(
            activity_attribution(Some("garmin"), Some("Wahoo ELEMNT BOLT")).as_deref(),
            Some("Garmin"),
            "a device that is not Garmin's is never named as one"
        );
    }

    #[test]
    fn a_garmin_recording_another_service_relayed_is_attributed() {
        assert!(garmin_recorded(Some("strava"), Some("Garmin Edge 840")));
        assert_eq!(
            activity_attribution(Some("strava"), Some("Garmin Edge 840")).as_deref(),
            Some("Garmin Edge 840")
        );
    }

    #[test]
    fn anything_else_carries_no_attribution() {
        assert!(!garmin_recorded(None, None));
        assert_eq!(activity_attribution(Some("strava"), None), None);
        assert_eq!(activity_attribution(None, Some("COROS PACE 4")), None);
    }

    #[test]
    fn a_long_device_name_is_capped() {
        let long = format!("Garmin {}", "X".repeat(200));
        let attribution = activity_attribution(None, Some(&long)).unwrap_or_default();
        assert_eq!(attribution.chars().count(), "Garmin ".len() + 40);
    }
}
