// ABOUTME: A provider is read on a coached athlete's behalf because its descriptor declares COACH_ROSTER, whatever its name
// ABOUTME: Pins the registry's delegated factory: the coach's credential and the athlete id it binds, and the refusals before any build
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The delegated factory is provider-neutral (carnet#720). A coaching
//! platform registered under any name — here a stand-in, `coachhub` — is
//! built for one athlete from the coach's stored credential, an OAuth grant
//! or an API key alike. A provider is read that way only when its descriptor
//! declares [`ProviderCapabilities::COACH_ROSTER`], and a refused athlete id
//! never reaches the provider's factory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_providers::core::{
    ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    ProviderFactory,
};
use pierre_providers::delegation::{CoachRoster, DelegatedReads};
use pierre_providers::errors::{AppError, AppResult, ErrorCode};
use pierre_providers::models::{
    Activity, ActivityBuilder, Athlete, RosterAthlete, SportType, Stats,
};
use pierre_providers::pagination::{CursorPage, PaginationParams};
use pierre_providers::spi::{OAuthEndpoints, OAuthParams, OAuthRefresh};
use pierre_providers::{
    global_registry, ProviderCapabilities, ProviderDescriptor, ProviderRegistry,
};

/// The stand-in coaching platform.
const COACHHUB: &str = "coachhub";
/// The same provider, registered under a descriptor that declares no roster.
const UNDECLARED: &str = "coachhub_undeclared";
/// A provider declaring the roster whose factory builds no delegated reads.
const FACTORYLESS: &str = "coachhub_factoryless";

/// The athlete a coach reads.
const ATHLETE: &str = "a201";

/// A descriptor declaring `capabilities` under `name`.
struct Descriptor {
    name: &'static str,
    capabilities: ProviderCapabilities,
}

impl ProviderDescriptor for Descriptor {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        "CoachHub"
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

/// The stand-in provider: whose activities it reads is the athlete it was
/// built for, through the credential it was handed.
struct CoachHub {
    config: ProviderConfig,
    athlete: Option<String>,
    credentials: Mutex<Option<OAuth2Credentials>>,
}

#[async_trait]
impl FitnessProvider for CoachHub {
    fn name(&self) -> &'static str {
        COACHHUB
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        *self.credentials.lock().unwrap() = Some(credentials);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        self.credentials.lock().unwrap().is_some()
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let athlete = self.athlete.as_deref().unwrap_or("coach");
        Ok(vec![ActivityBuilder::new(
            format!("{athlete}:1"),
            "Easy run",
            SportType::Run,
            Utc::now() - Duration::days(1),
            1_800,
            COACHHUB,
        )
        .build()])
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }

    async fn read_coach_roster(&self) -> AppResult<CoachRoster> {
        Ok(CoachRoster {
            account_email: Some("coach@coachhub.test".to_owned()),
            athletes: vec![RosterAthlete {
                id: ATHLETE.to_owned(),
                name: Some("Alex".to_owned()),
                email: Some("alex@coachhub.test".to_owned()),
            }],
        })
    }
}

/// The stand-in's factory, counting every provider it builds.
struct Factory {
    built: Arc<AtomicUsize>,
    delegated: bool,
}

impl Factory {
    fn build(&self, config: ProviderConfig, athlete: Option<&str>) -> Box<dyn FitnessProvider> {
        self.built.fetch_add(1, Ordering::SeqCst);
        Box::new(CoachHub {
            config,
            athlete: athlete.map(str::to_owned),
            credentials: Mutex::new(None),
        })
    }
}

impl ProviderFactory for Factory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(self.build(config, None))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[COACHHUB]
    }

    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        self.delegated.then_some(self as &dyn DelegatedReads)
    }
}

impl DelegatedReads for Factory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        if athlete_id.is_empty() || !athlete_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(AppError::invalid_input(format!(
                "{athlete_id:?} is not a CoachHub athlete id"
            )));
        }
        Ok(())
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        self.check_athlete_id(athlete_id)?;
        Ok(self.build(config, Some(athlete_id)))
    }
}

/// A registry holding the stand-in under its three registrations, and the
/// count of providers its factories built.
fn registry() -> (ProviderRegistry, Arc<AtomicUsize>) {
    let built = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    for (name, capabilities, delegated) in [
        (
            COACHHUB,
            ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::COACH_ROSTER),
            true,
        ),
        (UNDECLARED, ProviderCapabilities::ACTIVITIES, true),
        (
            FACTORYLESS,
            ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::COACH_ROSTER),
            false,
        ),
    ] {
        let descriptor = Descriptor { name, capabilities };
        registry.set_default_config(name, descriptor.to_config());
        registry.register_descriptor(name, Box::new(descriptor));
        registry.register_factory(
            name,
            Box::new(Factory {
                built: Arc::clone(&built),
                delegated,
            }),
        );
    }
    (registry, built)
}

fn credentials(kind: CredentialKind, secret: &str) -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: Some(secret.to_owned()),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
        kind,
        request_budget: None,
    }
}

fn recent() -> ActivityQueryParams {
    ActivityQueryParams {
        limit: Some(10),
        offset: None,
        before: None,
        after: None,
    }
}

#[tokio::test]
async fn a_declared_platform_reads_the_named_athlete_through_the_coachs_credential() {
    let (registry, built) = registry();
    for (kind, secret) in [
        (CredentialKind::OAuthBearer, "coach-oauth-token"),
        (CredentialKind::ApiKey, "coach-api-key"),
    ] {
        let provider = registry
            .create_delegated_provider(COACHHUB, ATHLETE, credentials(kind, secret))
            .await
            .expect("a provider declaring a coach roster reads a coached athlete");
        assert!(
            provider.is_authenticated().await,
            "built holding the coach's credential"
        );
        let activities = provider
            .get_activities_with_params(&recent())
            .await
            .unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].id(), format!("{ATHLETE}:1"));
    }
    assert_eq!(built.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_refused_athlete_id_never_reaches_the_factory() {
    let (registry, built) = registry();
    for refused in ["", "a1/../a2", "a1?athlete=a2"] {
        let Err(error) = registry
            .create_delegated_provider(
                COACHHUB,
                refused,
                credentials(CredentialKind::OAuthBearer, "coach-oauth-token"),
            )
            .await
        else {
            panic!("{refused:?} must be refused");
        };
        assert_eq!(error.code, ErrorCode::InvalidInput, "{refused:?}");
        assert!(registry.check_delegated_athlete(COACHHUB, refused).is_err());
    }
    assert!(registry.check_delegated_athlete(COACHHUB, ATHLETE).is_ok());
    assert_eq!(built.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_a_descriptor_declaring_the_roster_is_read_on_anyones_behalf() {
    let (registry, built) = registry();
    let Err(undeclared) = registry
        .create_delegated_provider(
            UNDECLARED,
            ATHLETE,
            credentials(CredentialKind::OAuthBearer, "coach-oauth-token"),
        )
        .await
    else {
        panic!("a factory offering delegated reads is not enough without the capability");
    };
    assert_eq!(undeclared.code, ErrorCode::InvalidInput);
    assert_eq!(
        undeclared.sanitized_message(),
        format!("{UNDECLARED} cannot be read on behalf of a coached athlete")
    );

    let Err(factoryless) = registry
        .create_delegated_provider(
            FACTORYLESS,
            ATHLETE,
            credentials(CredentialKind::OAuthBearer, "coach-oauth-token"),
        )
        .await
    else {
        panic!("a declared roster with no delegated factory cannot be built");
    };
    assert_eq!(factoryless.code, ErrorCode::InternalError);
    assert_eq!(built.load(Ordering::SeqCst), 0);

    let declared = registry.coach_roster_providers();
    assert!(declared.contains(&COACHHUB));
    assert!(declared.contains(&FACTORYLESS));
    assert!(!declared.contains(&UNDECLARED));
    assert!(
        declared.windows(2).all(|pair| pair[0] <= pair[1]),
        "{declared:?}"
    );
}

#[test]
fn every_built_in_coaching_platform_builds_its_delegated_reads() {
    let registry = global_registry();
    for name in registry.coach_roster_providers() {
        if let Err(error) = registry.check_delegated_athlete(name, "1") {
            // An id the platform refuses is its own rule; a missing factory
            // is a descriptor declaring what the provider cannot do.
            assert_eq!(error.code, ErrorCode::InvalidInput, "{name}: {error}");
        }
    }
    #[cfg(feature = "provider-sciotte")]
    assert!(registry
        .coach_roster_providers()
        .contains(&"sciotte_trainingpeaks"));
    #[cfg(feature = "provider-intervals-icu")]
    assert!(registry.coach_roster_providers().contains(&"intervals_icu"));
    for other in [
        "strava",
        "sciotte",
        "sciotte_garmin",
        "sciotte_coros",
        "garmin",
    ] {
        assert!(
            !registry.coach_roster_providers().contains(&other),
            "{other}"
        );
    }
}
