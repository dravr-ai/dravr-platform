// ABOUTME: A provider key stood in by an account answering from memory, with terms serving every transport
// ABOUTME: For suites that need an MCP or A2A caller to read sessions without reaching a provider's API

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Only the suites of the `it` binary call it.
#![allow(dead_code)]

use std::sync::Arc;

use crate::common::offline_provider_registry;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, Athlete};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::context::ServerContext;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor, ProviderFactoryFn,
};

/// A stand-in for a provider key, with terms that let its data travel over
/// every transport, so what it serves reaches an MCP or A2A caller. The key
/// is what sets the item's source class; the terms are the test's.
pub(crate) struct StandIn {
    pub(crate) name: &'static str,
}

impl ProviderDescriptor for StandIn {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        "Stand-in"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
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
        "http://localhost/stand-in"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::AnyTransport
    }
}

/// The account behind a stand-in, answering with `activities`.
pub(crate) struct Account {
    pub(crate) config: ProviderConfig,
    pub(crate) provider: &'static str,
    pub(crate) activities: Vec<Activity>,
}

#[async_trait]
impl FitnessProvider for Account {
    fn name(&self) -> &'static str {
        self.provider
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Ok(Athlete {
            active_streak: None,
            created_at: None,
            id: format!("{}-athlete", self.provider),
            username: "stand_in".to_owned(),
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: self.provider.to_owned(),
            sex: None,
        })
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(self.activities.clone())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(self.activities.clone(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        self.activities
            .iter()
            .find(|a| a.id() == id)
            .cloned()
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }
}

/// `resources` with `registry` as its provider registry.
pub(crate) fn with_registry(
    resources: &Arc<ServerContext>,
    registry: ProviderRegistry,
) -> Arc<ServerContext> {
    let mut context = (**resources).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

/// The offline registry with `provider` stood in by the account `account`
/// builds.
pub(crate) fn standing_in(provider: &'static str, account: ProviderFactoryFn) -> ProviderRegistry {
    let mut registry = offline_provider_registry();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(StandIn { name: provider }),
        account,
    ));
    registry
}
