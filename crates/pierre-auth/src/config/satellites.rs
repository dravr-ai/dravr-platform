// ABOUTME: The satellite MCP servers the authorization server mints for (OAUTH2_SATELLITE_RESOURCES)
// ABOUTME: Parsed from the environment and refused at startup when one could be taken for the platform
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use url::Url;

use super::oauth::OAuth2ServerConfig;

/// The platform's MCP endpoint path on each of its origins.
const PLATFORM_MCP_PATH: &str = "/mcp";

/// The satellite resource identifiers `configured` lists, comma-separated:
/// each trimmed, without a trailing `/`, blanks and repeats dropped, in the
/// order given. Their shape is checked by
/// [`OAuth2ServerConfig::validate_satellite_resources`] at startup.
pub(super) fn resolve_satellite_resources(configured: Option<&str>) -> Vec<String> {
    let mut resources: Vec<String> = Vec::new();
    for resource in configured
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim().trim_end_matches('/'))
        .filter(|value| !value.is_empty())
    {
        if !resources.iter().any(|known| known == resource) {
            resources.push(resource.to_owned());
        }
    }
    resources
}

/// `url`'s path without a trailing `/`: `""` for the root.
fn normalized_path(url: &Url) -> &str {
    url.path().trim_end_matches('/')
}

/// Whether a satellite at `path` on a platform origin sits at or above the
/// platform's `/mcp` endpoint, where a request the platform's clients send
/// (the origin itself, or its `/mcp`) would bind the satellite instead.
fn shadows_platform_mcp(path: &str) -> bool {
    path.is_empty()
        || PLATFORM_MCP_PATH == path
        || PLATFORM_MCP_PATH
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl OAuth2ServerConfig {
    /// Refuse a satellite resource identifier that cannot be a token audience.
    ///
    /// Each must be an absolute `http`/`https` URL with a host and no query,
    /// fragment or credentials (a path is allowed: a satellite's identifier is
    /// typically its `/mcp` endpoint), `https` when `require_https`. On any of
    /// the platform's own origins — compared parsed, so a default port, a
    /// host's case or a trailing `/` changes nothing — its path must lie
    /// strictly beside or below the platform's `/mcp`: at the origin, at
    /// `/mcp` or above it, the platform's own authorizations would bind the
    /// satellite. The refusal names the identifier redacted, as every URL a
    /// log carries is.
    ///
    /// # Errors
    /// Returns an invalid-input error naming the identifier and its defect.
    pub fn validate_satellite_resources(&self, require_https: bool) -> AppResult<()> {
        let platform_origins: Vec<_> = self
            .mcp_resources()
            .into_iter()
            .filter_map(|own| Url::parse(own).ok())
            .map(|own| own.origin())
            .collect();
        for resource in &self.satellite_resources {
            let shown = redact_url(resource);
            let url = Url::parse(resource).map_err(|e| {
                AppError::invalid_input(format!(
                    "OAUTH2_SATELLITE_RESOURCES entry must be an absolute URL ({e}): {shown}"
                ))
            })?;
            let scheme_ok = match url.scheme() {
                "https" => true,
                "http" => !require_https,
                _ => false,
            };
            let shape_ok = url.host().is_some()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none();
            if !scheme_ok || !shape_ok {
                return Err(AppError::invalid_input(format!(
                    "OAUTH2_SATELLITE_RESOURCES entry must be an https URL (http outside \
                     production) with a host and no query, fragment or credentials: {shown}"
                )));
            }
            if platform_origins.contains(&url.origin())
                && shadows_platform_mcp(normalized_path(&url))
            {
                return Err(AppError::invalid_input(format!(
                    "OAUTH2_SATELLITE_RESOURCES entry sits at or above the platform's own \
                     {PLATFORM_MCP_PATH} on one of its origins: {shown}"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satellite_resources_are_trimmed_deduplicated_and_kept_in_order() {
        let got = resolve_satellite_resources(None);
        assert!(got.is_empty(), "expected empty, got {got:?}");
        let got = resolve_satellite_resources(Some(" , "));
        assert!(got.is_empty(), "expected empty, got {got:?}");
        assert_eq!(
            resolve_satellite_resources(Some(
                " https://commere.dravr.ai/mcp/ ,https://enforme.dravr.ai/mcp,\
                 https://commere.dravr.ai/mcp"
            )),
            vec![
                "https://commere.dravr.ai/mcp",
                "https://enforme.dravr.ai/mcp"
            ]
        );
    }

    fn with_satellites(satellites: &[&str]) -> OAuth2ServerConfig {
        OAuth2ServerConfig {
            mcp_resource_url: "https://api.dravr.ai".to_owned(),
            satellite_resources: satellites.iter().map(|s| (*s).to_owned()).collect(),
            ..OAuth2ServerConfig::default()
        }
    }

    #[test]
    fn satellites_are_never_among_the_identifiers_the_platform_answers_to() {
        let config = with_satellites(&["https://commere.dravr.ai/mcp"]);
        assert_eq!(config.mcp_resources(), vec!["https://api.dravr.ai"]);
    }

    #[test]
    fn a_satellite_on_its_own_host_or_beside_the_platforms_mcp_is_accepted() {
        for accepted in [
            "https://commere.dravr.ai/mcp",
            "https://commere.dravr.ai",
            "https://api.dravr.ai/commere/mcp",
            "https://api.dravr.ai/mcp/commere",
        ] {
            with_satellites(&[accepted])
                .validate_satellite_resources(true)
                .unwrap_or_else(|e| panic!("{accepted}: {e}"));
        }
        with_satellites(&["http://127.0.0.1:3200/mcp"])
            .validate_satellite_resources(false)
            .unwrap();
    }

    #[test]
    fn a_satellite_that_could_be_taken_for_the_platform_is_refused() {
        for refused in [
            "https://api.dravr.ai",
            "https://api.dravr.ai/",
            "https://api.dravr.ai:443",
            "https://API.dravr.ai",
            "https://api.dravr.ai/mcp",
            "https://api.dravr.ai/mcp/",
            "https://API.dravr.ai:443/mcp",
            "http://commere.dravr.ai/mcp",
            "https://commere.dravr.ai/mcp?x=1",
            "https://commere.dravr.ai/mcp#frag",
            "https://user@commere.dravr.ai/mcp",
            "commere.dravr.ai/mcp",
        ] {
            assert!(
                with_satellites(&[refused])
                    .validate_satellite_resources(true)
                    .is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_refusal_never_prints_the_credentials_an_entry_carries() {
        let refused = with_satellites(&["https://user:hunter2@commere.dravr.ai/mcp"])
            .validate_satellite_resources(true)
            .unwrap_err()
            .to_string();
        assert!(!refused.contains("hunter2"), "{refused}");
        assert!(refused.contains("commere.dravr.ai"), "{refused}");
    }
}
