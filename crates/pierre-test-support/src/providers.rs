// ABOUTME: The provider registry a test builds, with every endpoint the server calls at a refused origin
// ABOUTME: Shared by the server's suites and every other crate's tests that build a ProviderRegistry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_config::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_providers::core::ProviderConfig;
use pierre_providers::registry::ProviderRegistry;
use url::{Host, Url};

/// Where a test server sends every call a test names no stand-in for: a
/// loopback port nothing listens on (1 is reserved for TCPMUX, and no test
/// binds it), so the connection is refused at once.
///
/// A registry built with no override holds each provider's real token, API
/// and revocation URLs, and a test that stood one of them in still sent the
/// others to the real host with its fixture credentials: a refresh answered
/// by a stub was followed by a read of `api.prod.whoop.com`, and an expired
/// Garmin grant was refreshed at `diauth.garmin.com` (carnet#1615).
///
/// The one `ServerConfig::default` asks the weather vendors at.
pub const REFUSED_ORIGIN: &str = ServerConfig::REFUSED_ORIGIN;

/// Whether a request to `url` would leave the machine: it names a host, and
/// that host is neither `localhost` nor a loopback address. Empty text, or
/// text that is no URL, names nothing a client can connect to.
#[must_use]
pub fn leaves_the_machine(url: &str) -> bool {
    match Url::parse(url).ok().as_ref().and_then(Url::host) {
        Some(Host::Domain(name)) => name != "localhost",
        Some(Host::Ipv4(address)) => !address.is_loopback(),
        Some(Host::Ipv6(address)) => !address.is_loopback(),
        None => false,
    }
}

/// A provider registry a suite builds itself, every endpoint the server
/// calls on a real host refused.
///
/// The token, API and revocation URLs that name a real host are sent to
/// [`REFUSED_ORIGIN`], as the server tests' `offline_config` does for a
/// context's.
///
/// `ProviderRegistry::new()` holds each provider's real endpoints, so a
/// suite that built one and stood a single provider in was one refresh, read
/// or disconnect from every other provider's real host. A suite names its
/// stand-in on what this returns (`set_default_config`,
/// `with_endpoint_overrides`); the authorize URL, which only the athlete's
/// browser follows, keeps the provider's own. The server's
/// `startup_channel_egress_test` reads its test sources and fails on a
/// registry built any other way (carnet#1615).
#[must_use]
pub fn offline_provider_registry() -> ProviderRegistry {
    offline_registry(ProviderRegistry::new())
}

/// `registry` with every endpoint the server calls (token, API, revocation)
/// that names a real host sent to [`REFUSED_ORIGIN`]; a loopback stand-in
/// and the authorize URL stay as they are.
///
/// For a suite that registers a provider's real descriptor, or sets a
/// configuration built from one, after [`offline_provider_registry`]: that
/// puts the provider's real endpoints back, and this takes them out of the
/// finished registry.
#[must_use]
pub fn offline_registry(mut registry: ProviderRegistry) -> ProviderRegistry {
    for provider in registry.supported_providers() {
        let Some(mut config) = registry.default_config(provider).cloned() else {
            continue;
        };
        for url in [&mut config.token_url, &mut config.api_base_url] {
            if leaves_the_machine(url) {
                REFUSED_ORIGIN.clone_into(url);
            }
        }
        if config.revoke_url.as_deref().is_some_and(leaves_the_machine) {
            config.revoke_url = Some(REFUSED_ORIGIN.to_owned());
        }
        registry.set_default_config(provider, config);
    }
    registry
}

/// `provider`'s configuration as [`offline_provider_registry`] holds it.
///
/// For a suite that points one endpoint at its stand-in and puts the
/// configuration back with `set_default_config`: the endpoints it does not
/// name stay refused, where the provider's own `default_config()` would
/// bring its real token and revocation URLs back.
///
/// # Errors
///
/// Returns an error for a provider the registry does not carry.
pub fn offline_provider_config(provider: &str) -> AppResult<ProviderConfig> {
    offline_provider_registry()
        .default_config(provider)
        .cloned()
        .ok_or_else(|| AppError::internal(format!("the provider registry carries no {provider}")))
}
