// ABOUTME: The coaching platforms a coach's own account reads group athletes through: every provider declaring COACH_ROSTER
// ABOUTME: Each platform reads its coach's roster and hands a delegated read the coach's credential; the link flow is shared
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach platforms
//!
//! A coach whose athletes train on a coaching platform reads them through
//! the coach's own account there: `TrainingPeaks` through the coach's scraper
//! session, Intervals.icu through the coach's personal API key, any other
//! platform through the OAuth grant or API key stored by the human coach. The
//! linking flow (`crate::delegated_connections`) and the read path are the
//! same for all; what differs is behind [`CoachPlatform`]:
//!
//! - how the coach's roster is read, and what the account must be for it to
//!   be read at all ([`CoachPlatform::read_roster`]);
//! - which credential a delegated read is handed
//!   ([`CoachPlatform::coach_credentials`]);
//! - whether a coach account keeps a calendar of its own
//!   ([`CoachPlatform::own_calendar_refusal`]).
//!
//! A provider is a coaching platform because its descriptor declares
//! [`COACH_ROSTER`](pierre_providers::spi::ProviderCapabilities::COACH_ROSTER),
//! never because of its name: the registry's
//! [`coach_roster_providers`](pierre_providers::registry::ProviderRegistry::coach_roster_providers)
//! is the one list ([`coach_platforms`](crate::coach_platform::coach_platforms)).
//! Its factory's delegated reads
//! ([`pierre_providers::delegation::DelegatedReads`]) build the provider fixed
//! to one athlete. `TrainingPeaks` and Intervals.icu keep their own rules
//! (a scraper session and a coach account role; an API key only); every
//! other platform reads its roster with
//! [`read_coach_roster`](pierre_providers::CoreFitnessProvider::read_coach_roster)
//! through the coach's stored credential, as it was stored.

use async_trait::async_trait;
use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, SCIOTTE_TRAININGPEAKS};
use pierre_core::errors::AppResult;
use pierre_core::models::{ProviderAccountRole, RosterAthlete, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::sciotte_provider::SciotteTarget;
#[cfg(doc)]
use pierre_providers::spi::ProviderCapabilities;
use pierre_providers::{CredentialKind, OAuth2Credentials};
use uuid::Uuid;

use crate::delegation_refusal::Refusal;
use crate::trainingpeaks_accounts::{
    account_role, coach_binding, email_binding, record_trainingpeaks_profile, trainingpeaks_profile,
};

/// What a roster read is given: the coach, their stored credential, and the
/// account role recorded on their connection (`None` while never read).
#[derive(Clone, Copy)]
pub struct RosterRead<'a> {
    /// Repositories the read records what it learns in.
    pub repos: &'a RepositoryRegistry,
    /// Builds the provider an API-backed platform reads through.
    pub registry: &'a ProviderRegistry,
    /// The coach whose roster is read.
    pub coach_user_id: Uuid,
    /// The tenant holding the coach's credential.
    pub coach_tenant: TenantId,
    /// The coach's own stored credential for the platform.
    pub token: &'a UserOAuthToken,
    /// The account role recorded on the coach's connection.
    pub recorded_role: Option<ProviderAccountRole>,
}

/// A coaching platform whose coach account reads the athletes who share with
/// it.
#[async_trait]
pub trait CoachPlatform: Send + Sync {
    /// The backend whose stored credential, connection and links are the
    /// platform's (`sciotte_trainingpeaks`, `intervals_icu`).
    fn backend(&self) -> &'static str;

    /// The platform as clients and the athlete name it (`trainingpeaks`,
    /// `intervals_icu`).
    fn user_facing(&self) -> &'static str {
        user_facing_name(self.backend())
    }

    /// The platform's brand, as words name it.
    fn brand(&self) -> &'static str;

    /// Whether a roster cached for a coach whose connection records
    /// `recorded_role` may be served without a live read. A live read is what
    /// checks the account, so a roster is served from the cache only once
    /// nothing is left to learn by reading it again.
    fn serves_cached_roster(&self, recorded_role: Option<ProviderAccountRole>) -> bool;

    /// The athletes the coach's account coaches, read live through the
    /// coach's stored credential, once the account is the coach's own.
    ///
    /// # Errors
    ///
    /// Returns a [`Refusal`] error with its reason when the account cannot
    /// serve a roster to this coach, the provider error when it cannot be
    /// read, or a repository error.
    async fn read_roster(&self, read: RosterRead<'_>) -> AppResult<Vec<RosterAthlete>>;

    /// The credentials a delegated read is handed from the coach's stored
    /// `token`, or `None` when that credential cannot read athletes.
    fn coach_credentials(&self, token: &UserOAuthToken) -> Option<OAuth2Credentials>;

    /// What a read of a coach account's own calendar answers on a platform
    /// whose coach accounts keep none; `None` where a coach account is also
    /// an athlete's, whose own calendar reads as any other.
    fn own_calendar_refusal(&self) -> Option<String>;
}

/// `TrainingPeaks`, read on the scraper service through the coach's session.
///
/// A `TrainingPeaks` coach account keeps no calendar of its own, and only an
/// account `TrainingPeaks` reports as a coach's lists a roster.
struct TrainingPeaksPlatform;

#[async_trait]
impl CoachPlatform for TrainingPeaksPlatform {
    fn backend(&self) -> &'static str {
        SCIOTTE_TRAININGPEAKS
    }

    fn brand(&self) -> &'static str {
        SciotteTarget::TrainingPeaks.brand()
    }

    fn serves_cached_roster(&self, recorded_role: Option<ProviderAccountRole>) -> bool {
        recorded_role == Some(ProviderAccountRole::Coach)
    }

    /// The roster the coach's session reads live, once the account it signed
    /// in to is a coach account whose email is the coach's verified Dravr
    /// email.
    ///
    /// The read records the account's role when it differs from the one
    /// recorded, which grants `manages_roster` to a bound coach account; an
    /// unchanged role re-grants nothing, and a coach account that is not the
    /// coach's own loses the grant either way.
    async fn read_roster(&self, read: RosterRead<'_>) -> AppResult<Vec<RosterAthlete>> {
        let session = serde_json::from_str::<AuthSession>(&read.token.access_token)
            .map_err(|_| Refusal::ReconnectNeeded.error(Some(self)))?;
        let profile = match trainingpeaks_profile(&session).await {
            Ok(profile) => profile,
            Err(e) if e.provider_auth_required_provider().is_some() => {
                return Err(Refusal::ReconnectNeeded.error(Some(self)));
            }
            Err(e) => return Err(e),
        };
        let role = account_role(&profile);
        let binding = match (read.recorded_role == Some(role), role) {
            (true, ProviderAccountRole::Coach) => Some(
                coach_binding(read.repos, read.coach_user_id, read.coach_tenant, &profile).await?,
            ),
            (true, ProviderAccountRole::Athlete) => None,
            (false, _) => {
                record_trainingpeaks_profile(
                    read.repos,
                    read.coach_user_id,
                    read.coach_tenant,
                    &profile,
                )
                .await?
                .binding
            }
        };
        if role != ProviderAccountRole::Coach {
            return Err(Refusal::NotCoachAccount.error(Some(self)));
        }
        if let Some(refusal) = binding.and_then(Refusal::coach_account) {
            return Err(refusal.error(Some(self)));
        }
        Ok(profile
            .coached_athletes
            .into_iter()
            .map(|athlete| RosterAthlete {
                id: athlete.id,
                name: athlete.display_name,
                email: athlete.email,
            })
            .collect())
    }

    /// The coach's scraper session, stored as the token's access token.
    fn coach_credentials(&self, token: &UserOAuthToken) -> Option<OAuth2Credentials> {
        Some(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(token.access_token.clone()),
            refresh_token: None,
            expires_at: token.expires_at,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
    }

    fn own_calendar_refusal(&self) -> Option<String> {
        Some(SciotteTarget::TrainingPeaks.coach_account_refusal())
    }
}

/// Intervals.icu, read through the coach's personal API key.
///
/// Intervals.icu lists a coach's athletes (`GET /api/v1/athletes`) for an API
/// key only, and serves a coach each athlete who shares with them at that
/// athlete's own path. A coach account there trains too, so its own calendar
/// reads as any athlete's.
struct IntervalsIcuPlatform;

#[async_trait]
impl CoachPlatform for IntervalsIcuPlatform {
    fn backend(&self) -> &'static str {
        INTERVALS_ICU
    }

    fn brand(&self) -> &'static str {
        "Intervals.icu"
    }

    // An Intervals.icu account carries no role to learn: every live read
    // checks the same email binding, so a cached roster is as good as one.
    fn serves_cached_roster(&self, _recorded_role: Option<ProviderAccountRole>) -> bool {
        true
    }

    /// The roster the coach's API key lists, once the account the key
    /// belongs to shares an email that is the coach's verified Dravr email.
    async fn read_roster(&self, read: RosterRead<'_>) -> AppResult<Vec<RosterAthlete>> {
        let credentials = self
            .coach_credentials(read.token)
            .ok_or_else(|| Refusal::ApiKeyRequired.error(Some(self)))?;
        api_roster(self, read, credentials).await
    }

    /// The coach's API key, with the coach's own athlete id: an OAuth grant
    /// cannot list a coach's athletes, so it reads none of them.
    fn coach_credentials(&self, token: &UserOAuthToken) -> Option<OAuth2Credentials> {
        let athlete_id = token
            .provider_user_id
            .as_deref()
            .filter(|id| !id.is_empty())?;
        (CredentialKind::from_token_type(&token.token_type) == CredentialKind::ApiKey).then(|| {
            OAuth2Credentials {
                client_id: athlete_id.to_owned(),
                client_secret: String::new(),
                access_token: Some(token.access_token.clone()),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
                kind: CredentialKind::ApiKey,
                request_budget: None,
            }
        })
    }

    fn own_calendar_refusal(&self) -> Option<String> {
        None
    }
}

/// Any other coaching platform: a provider whose descriptor declares
/// [`ProviderCapabilities::COACH_ROSTER`], read through its API with the
/// credential stored by the human coach, an OAuth grant or an API key as its
/// token type says.
///
/// Its accounts carry no role to learn, and a coach account trains too, so
/// its own calendar reads as any athlete's.
#[derive(Debug, Clone, Copy)]
struct ApiCoachPlatform {
    backend: &'static str,
    brand: &'static str,
}

#[async_trait]
impl CoachPlatform for ApiCoachPlatform {
    fn backend(&self) -> &'static str {
        self.backend
    }

    fn brand(&self) -> &'static str {
        self.brand
    }

    // Every live read checks the same email binding, so a cached roster is
    // as good as one read again.
    fn serves_cached_roster(&self, _recorded_role: Option<ProviderAccountRole>) -> bool {
        true
    }

    /// The roster the coach's credential lists, once the account it belongs
    /// to shares an email that is the coach's verified Dravr email.
    async fn read_roster(&self, read: RosterRead<'_>) -> AppResult<Vec<RosterAthlete>> {
        api_roster(self, read, stored_credentials(read.token)).await
    }

    /// The coach's stored credential as it was stored. It carries no refresh
    /// token: an expired grant reads as the coach's to reconnect
    /// ([`crate::delegated_connections::CoachSession::NeedsReconnect`]), never
    /// as the reader's.
    fn coach_credentials(&self, token: &UserOAuthToken) -> Option<OAuth2Credentials> {
        Some(stored_credentials(token))
    }

    fn own_calendar_refusal(&self) -> Option<String> {
        None
    }
}

/// `token`, the coach's stored credential, as a provider is handed it.
fn stored_credentials(token: &UserOAuthToken) -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: Some(token.access_token.clone()),
        refresh_token: None,
        expires_at: token.expires_at,
        scopes: vec![],
        kind: CredentialKind::from_token_type(&token.token_type),
        request_budget: None,
    }
}

/// The roster `platform`'s API lists for the coach's `credentials`, once the
/// account they belong to shares an email that is the coach's verified Dravr
/// email.
///
/// # Errors
///
/// Returns the `ReconnectNeeded` refusal when the platform no longer honours
/// the credentials, a [`Refusal::coach_account`] refusal when the account is
/// not the coach's own, the provider error when the roster cannot be read, or
/// a repository error.
async fn api_roster(
    platform: &dyn CoachPlatform,
    read: RosterRead<'_>,
    credentials: OAuth2Credentials,
) -> AppResult<Vec<RosterAthlete>> {
    let provider = read.registry.create_provider(platform.backend())?;
    provider.set_credentials(credentials).await?;
    let roster = match provider.read_coach_roster().await {
        Ok(roster) => roster,
        Err(e) if e.provider_auth_required_provider().is_some() => {
            return Err(Refusal::ReconnectNeeded.error(Some(platform)));
        }
        Err(e) => return Err(e),
    };
    let binding = email_binding(
        read.repos,
        read.coach_user_id,
        roster.account_email.as_deref(),
    )
    .await?;
    if let Some(refusal) = Refusal::coach_account(binding) {
        return Err(refusal.error(Some(platform)));
    }
    Ok(roster.athletes)
}

/// The coaching platforms chosen first when a coach holds a credential for
/// more than one, in that order; every other one follows by name.
const PREFERRED_PLATFORMS: [&str; 2] = [SCIOTTE_TRAININGPEAKS, INTERVALS_ICU];

/// Every coaching platform `registry` holds: each provider whose descriptor
/// declares [`ProviderCapabilities::COACH_ROSTER`], in the order a coach's
/// connected one is chosen.
#[must_use]
pub fn coach_platforms(registry: &ProviderRegistry) -> Vec<Box<dyn CoachPlatform>> {
    let mut backends = registry.coach_roster_providers();
    // Stable, so the platforms no preference names keep the registry's
    // order by name.
    backends.sort_by_key(|backend| {
        PREFERRED_PLATFORMS
            .iter()
            .position(|preferred| preferred == backend)
            .unwrap_or(PREFERRED_PLATFORMS.len())
    });
    backends
        .into_iter()
        .map(|backend| platform_of(registry, backend))
        .collect()
}

/// The platform `backend`, a provider declaring a coach roster, is read as.
fn platform_of(registry: &ProviderRegistry, backend: &'static str) -> Box<dyn CoachPlatform> {
    match backend {
        SCIOTTE_TRAININGPEAKS => Box::new(TrainingPeaksPlatform),
        INTERVALS_ICU => Box::new(IntervalsIcuPlatform),
        _ => Box::new(ApiCoachPlatform {
            backend,
            brand: registry.get_display_name(backend).unwrap_or(backend),
        }),
    }
}

/// The coaching platform `provider` names in `registry`, by backend or by
/// the name the user knows it under; `None` for a provider whose descriptor
/// declares no coach roster.
#[must_use]
pub fn coach_platform(
    registry: &ProviderRegistry,
    provider: &str,
) -> Option<Box<dyn CoachPlatform>> {
    coach_platforms(registry)
        .into_iter()
        .find(|platform| platform.backend() == provider || platform.user_facing() == provider)
}

/// The coaching platform a coach's roster read is about: the one `requested`
/// names, else the first in [`coach_platforms`] order the coach holds a
/// credential of their own for.
///
/// # Errors
///
/// Returns the `unsupported_provider` refusal when `requested` names no
/// coaching platform, the `coach_platform_not_connected` refusal when nothing
/// was requested and nothing is connected, or a repository error.
pub async fn roster_platform(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    requested: Option<&str>,
) -> AppResult<Box<dyn CoachPlatform>> {
    match requested {
        Some(name) => {
            coach_platform(registry, name).ok_or_else(|| Refusal::UnsupportedProvider.error(None))
        }
        None => connected_coach_platform(repos, registry, coach_user_id, coach_tenant).await,
    }
}

/// The coaching platform `coach_user_id` holds a credential of their own for
/// in `coach_tenant`, the first in [`coach_platforms`] order.
///
/// # Errors
///
/// Returns the `coach_platform_not_connected` refusal when they hold none,
/// or the repository error when the credentials cannot be read.
async fn connected_coach_platform(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
) -> AppResult<Box<dyn CoachPlatform>> {
    for platform in coach_platforms(registry) {
        if repos
            .oauth_tokens
            .get_token(coach_user_id, coach_tenant, platform.backend())
            .await?
            .is_some()
        {
            return Ok(platform);
        }
    }
    Err(Refusal::NotConnected.error(None))
}

#[cfg(test)]
mod tests {
    use pierre_core::models::API_KEY_TOKEN_TYPE;
    use pierre_providers::spi::{
        OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
    };

    use super::*;

    /// A descriptor declaring `capabilities` under `name`.
    struct Declared {
        name: &'static str,
        display_name: &'static str,
        capabilities: ProviderCapabilities,
    }

    impl ProviderDescriptor for Declared {
        fn name(&self) -> &'static str {
            self.name
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
            "http://127.0.0.1:9"
        }

        fn default_scopes(&self) -> &'static [&'static str] {
            &[]
        }
    }

    /// A registry holding exactly the providers named, each declaring its
    /// capabilities: whatever the build's provider features, it decides.
    fn registry(
        declared: &[(&'static str, &'static str, ProviderCapabilities)],
    ) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for (name, display_name, capabilities) in declared {
            registry.register_descriptor(
                name,
                Box::new(Declared {
                    name,
                    display_name,
                    capabilities: *capabilities,
                }),
            );
        }
        registry
    }

    fn coach_roster() -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::COACH_ROSTER)
    }

    fn token(token_type: &str, provider_user_id: Option<&str>) -> UserOAuthToken {
        let mut token = UserOAuthToken::new(
            Uuid::new_v4(),
            TenantId::generate().to_string(),
            INTERVALS_ICU.to_owned(),
            "the-key".to_owned(),
            None,
            None,
            None,
        );
        token.token_type = token_type.to_owned();
        token.provider_user_id = provider_user_id.map(str::to_owned);
        token
    }

    #[test]
    fn a_platform_is_found_by_backend_and_by_the_name_users_know() {
        let registry = registry(&[
            (SCIOTTE_TRAININGPEAKS, "TrainingPeaks", coach_roster()),
            (INTERVALS_ICU, "Intervals.icu", coach_roster()),
            ("strava", "Strava", ProviderCapabilities::activity_only()),
        ]);
        for (name, backend) in [
            ("trainingpeaks", SCIOTTE_TRAININGPEAKS),
            (SCIOTTE_TRAININGPEAKS, SCIOTTE_TRAININGPEAKS),
            (INTERVALS_ICU, INTERVALS_ICU),
        ] {
            assert_eq!(
                coach_platform(&registry, name).map(|platform| platform.backend()),
                Some(backend)
            );
        }
        for other in ["strava", "sciotte", "garmin", "sciotte_garmin"] {
            assert!(coach_platform(&registry, other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_platform_is_one_because_it_declares_a_coach_roster_not_by_its_name() {
        let registry = registry(&[
            ("coachhub", "CoachHub", coach_roster()),
            (
                INTERVALS_ICU,
                "Intervals.icu",
                ProviderCapabilities::ACTIVITIES,
            ),
        ]);
        let platform = coach_platform(&registry, "coachhub").expect("it declares a coach roster");
        assert_eq!(platform.backend(), "coachhub");
        assert_eq!(platform.brand(), "CoachHub");
        assert!(platform.own_calendar_refusal().is_none());
        assert!(
            coach_platform(&registry, INTERVALS_ICU).is_none(),
            "a provider that stops declaring the roster is no coaching platform"
        );
    }

    #[test]
    fn the_preferred_platforms_come_first_and_the_rest_by_name() {
        let registry = registry(&[
            ("zeta_coach", "Zeta", coach_roster()),
            (INTERVALS_ICU, "Intervals.icu", coach_roster()),
            ("alpha_coach", "Alpha", coach_roster()),
            (SCIOTTE_TRAININGPEAKS, "TrainingPeaks", coach_roster()),
        ]);
        let order: Vec<&str> = coach_platforms(&registry)
            .iter()
            .map(|platform| platform.backend())
            .collect();
        assert_eq!(
            order,
            [
                SCIOTTE_TRAININGPEAKS,
                INTERVALS_ICU,
                "alpha_coach",
                "zeta_coach"
            ]
        );
    }

    #[test]
    fn an_api_platform_hands_the_coachs_credential_as_it_was_stored() {
        let platform = ApiCoachPlatform {
            backend: "coachhub",
            brand: "CoachHub",
        };
        let key = platform
            .coach_credentials(&token(API_KEY_TOKEN_TYPE, None))
            .expect("a stored key reads athletes");
        assert_eq!(key.kind, CredentialKind::ApiKey);
        assert_eq!(key.access_token.as_deref(), Some("the-key"));
        assert!(key.refresh_token.is_none());

        let grant = platform
            .coach_credentials(&token("Bearer", Some("c1")))
            .expect("a stored OAuth grant reads athletes");
        assert_eq!(grant.kind, CredentialKind::OAuthBearer);
        assert_eq!(grant.access_token.as_deref(), Some("the-key"));
    }

    #[test]
    fn an_intervals_coach_reads_athletes_through_their_api_key_only() {
        let key = IntervalsIcuPlatform
            .coach_credentials(&token(API_KEY_TOKEN_TYPE, Some("i100")))
            .expect("an API key with its athlete id reads athletes");
        assert_eq!(key.kind, CredentialKind::ApiKey);
        assert_eq!(key.client_id, "i100");
        assert_eq!(key.access_token.as_deref(), Some("the-key"));

        assert!(IntervalsIcuPlatform
            .coach_credentials(&token("Bearer", Some("i100")))
            .is_none());
        assert!(IntervalsIcuPlatform
            .coach_credentials(&token(API_KEY_TOKEN_TYPE, None))
            .is_none());
    }

    #[test]
    fn only_a_trainingpeaks_coach_account_keeps_no_calendar_of_its_own() {
        assert!(TrainingPeaksPlatform.own_calendar_refusal().is_some());
        assert!(IntervalsIcuPlatform.own_calendar_refusal().is_none());
    }
}
