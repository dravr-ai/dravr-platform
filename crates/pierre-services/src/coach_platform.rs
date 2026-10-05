// ABOUTME: The coaching platforms a coach's own account reads group athletes through — TrainingPeaks and Intervals.icu
// ABOUTME: Each platform reads its coach's roster and hands a delegated read the coach's credential; the link flow is shared
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach platforms
//!
//! A coach whose athletes train on a coaching platform reads them through
//! the coach's own account there: `TrainingPeaks` through the coach's scraper
//! session, Intervals.icu through the coach's personal API key. The linking
//! flow (`crate::delegated_connections`) and the read path are the same for
//! both; what differs is behind [`CoachPlatform`]:
//!
//! - how the coach's roster is read, and what the account must be for it to
//!   be read at all ([`CoachPlatform::read_roster`]);
//! - which credential a delegated read is handed
//!   ([`CoachPlatform::coach_credentials`]);
//! - whether a coach account keeps a calendar of its own
//!   ([`CoachPlatform::own_calendar_refusal`]).
//!
//! Every platform here is a provider whose factory offers delegated reads
//! ([`pierre_providers::delegation::DelegatedReads`]), which is what builds
//! the provider fixed to one athlete.

use async_trait::async_trait;
use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, SCIOTTE_TRAININGPEAKS};
use pierre_core::errors::AppResult;
use pierre_core::models::{ProviderAccountRole, RosterAthlete, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::sciotte_provider::SciotteTarget;
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
        let provider = read.registry.create_provider(INTERVALS_ICU)?;
        provider.set_credentials(credentials).await?;
        let roster = match provider.read_coach_roster().await {
            Ok(roster) => roster,
            Err(e) if e.provider_auth_required_provider().is_some() => {
                return Err(Refusal::ReconnectNeeded.error(Some(self)));
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
            return Err(refusal.error(Some(self)));
        }
        Ok(roster.athletes)
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

/// Every coaching platform, in the order a coach's connected one is chosen.
pub static COACH_PLATFORMS: [&dyn CoachPlatform; 2] =
    [&TrainingPeaksPlatform, &IntervalsIcuPlatform];

/// The coaching platform `provider` names, by backend or by the name the
/// user knows it under; `None` for any other provider.
#[must_use]
pub fn coach_platform(provider: &str) -> Option<&'static dyn CoachPlatform> {
    COACH_PLATFORMS
        .iter()
        .copied()
        .find(|platform| platform.backend() == provider || platform.user_facing() == provider)
}

/// The coaching platform a coach's roster read is about: the one `requested`
/// names, else the first in [`COACH_PLATFORMS`] order the coach holds a
/// credential of their own for.
///
/// # Errors
///
/// Returns the `unsupported_provider` refusal when `requested` names no
/// coaching platform, the `coach_platform_not_connected` refusal when nothing
/// was requested and nothing is connected, or a repository error.
pub async fn roster_platform(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    requested: Option<&str>,
) -> AppResult<&'static dyn CoachPlatform> {
    match requested {
        Some(name) => coach_platform(name).ok_or_else(|| Refusal::UnsupportedProvider.error(None)),
        None => connected_coach_platform(repos, coach_user_id, coach_tenant).await,
    }
}

/// The coaching platform `coach_user_id` holds a credential of their own for
/// in `coach_tenant`, the first in [`COACH_PLATFORMS`] order.
///
/// # Errors
///
/// Returns the `coach_platform_not_connected` refusal when they hold none,
/// or the repository error when the credentials cannot be read.
async fn connected_coach_platform(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
) -> AppResult<&'static dyn CoachPlatform> {
    for platform in COACH_PLATFORMS {
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

    use super::*;

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
        for (name, backend) in [
            ("trainingpeaks", SCIOTTE_TRAININGPEAKS),
            (SCIOTTE_TRAININGPEAKS, SCIOTTE_TRAININGPEAKS),
            (INTERVALS_ICU, INTERVALS_ICU),
        ] {
            assert_eq!(
                coach_platform(name).map(CoachPlatform::backend),
                Some(backend)
            );
        }
        for other in ["strava", "sciotte", "garmin", "sciotte_garmin"] {
            assert!(coach_platform(other).is_none(), "{other}");
        }
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
