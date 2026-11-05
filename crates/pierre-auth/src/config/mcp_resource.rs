// ABOUTME: The identifiers of the MCP resource server (MCP_RESOURCE_URL and its aliases) and the metadata URL of each
// ABOUTME: A resource is an origin or an origin plus the MCP endpoint's path; the origin form stays an alias so issued tokens keep working
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::iter;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use url::Url;

use super::oauth::OAuth2ServerConfig;

/// The well-known path of the RFC 9728 protected resource metadata document.
const PROTECTED_RESOURCE_WELL_KNOWN: &str = "/.well-known/oauth-protected-resource";

/// The origin form of `canonical` when it carries a path, else nothing.
///
/// A resource published with the MCP endpoint's path (`https://mcp.dravr.ai/mcp`)
/// keeps its bare origin as another identifier: tokens minted before the path
/// was published are bound to the origin, and the root metadata document a
/// client connected through still names it.
fn origin_form(canonical: &str) -> Option<String> {
    Url::parse(canonical)
        .ok()
        .filter(|url| url.path() != "/")
        .map(|url| url.origin().ascii_serialization())
}

/// The other identifiers that serve the MCP resource `canonical`.
///
/// First the origin form of `canonical`, when `canonical` carries a path.
/// Then `base_url`, when it is set, is itself an origin
/// (`scheme://host[:port]`, nothing after) and differs from `canonical`. A
/// `base_url` with a path is no resource identifier a client could have dialed
/// `/mcp` under, so it names no alias.
///
/// Split out of [`OAuth2ServerConfig::from_env`] so it is testable without
/// mutating the process environment, like `resolve_mcp_resource_url`.
#[must_use]
pub fn resolve_mcp_resource_aliases(canonical: &str, base_url: Option<&str>) -> Vec<String> {
    let from_base = base_url
        .map(|url| url.trim().trim_end_matches('/'))
        .filter(|url| *url != canonical)
        .filter(|url| {
            Url::parse(url).is_ok_and(|parsed| {
                matches!(parsed.scheme(), "http" | "https")
                    && parsed.host().is_some()
                    && parsed.path() == "/"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
            })
        })
        .map(str::to_owned);
    let candidates = origin_form(canonical).into_iter().chain(from_base);
    let mut aliases: Vec<String> = Vec::new();
    for alias in candidates {
        if alias != canonical && !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    aliases
}

/// The RFC 9728 §3.1 protected resource metadata URL of `resource`.
///
/// The well-known path goes between the host and the path of the identifier,
/// so `https://mcp.dravr.ai/mcp` is described at
/// `https://mcp.dravr.ai/.well-known/oauth-protected-resource/mcp` and a bare
/// origin at `<origin>/.well-known/oauth-protected-resource`.
#[must_use]
pub fn protected_resource_metadata_url(resource: &str) -> String {
    let trimmed = resource.trim_end_matches('/');
    let path = Url::parse(trimmed)
        .map(|url| url.path().trim_end_matches('/').to_owned())
        .unwrap_or_default();
    let origin = trimmed.strip_suffix(path.as_str()).unwrap_or(trimmed);
    format!("{origin}{PROTECTED_RESOURCE_WELL_KNOWN}{path}")
}

/// `host[:port]` of `resource`, lowercased; `None` when it is no URL.
fn authority_of(resource: &str) -> Option<String> {
    let url = Url::parse(resource).ok()?;
    let name = url.host_str()?;
    Some(
        url.port()
            .map_or_else(|| name.to_owned(), |port| format!("{name}:{port}"))
            .to_ascii_lowercase(),
    )
}

/// The path of `resource` without a trailing `/`: `""` for an origin.
fn path_of(resource: &str) -> String {
    Url::parse(resource)
        .map(|url| url.path().trim_end_matches('/').to_owned())
        .unwrap_or_default()
}

impl OAuth2ServerConfig {
    /// Refuse an `mcp_resource_url` that cannot be published as a resource
    /// identifier.
    ///
    /// It must be an absolute `http`/`https` URL: an origin, or an origin and
    /// the path of the MCP endpoint, with no query, fragment or credentials.
    /// RFC 9728 §3.3 has a client reject a `resource` that differs from the URL
    /// it fetched the metadata for, and the document is served at the
    /// well-known path for that exact identifier (origin or path-suffixed), so
    /// a path-bearing value needs the path-suffixed document the routes
    /// serve. `require_https` is set in production, where a bearer token must
    /// never cross a plain-HTTP hop.
    ///
    /// # Errors
    /// Returns an invalid-input error naming the defect.
    pub fn validate_mcp_resource_url(&self, require_https: bool) -> AppResult<()> {
        let raw = &self.mcp_resource_url;
        let value = redact_url(raw);
        let url = Url::parse(raw).map_err(|e| {
            AppError::invalid_input(format!(
                "MCP_RESOURCE_URL must be an absolute URL ({e}): {value}"
            ))
        })?;
        match url.scheme() {
            "https" => {}
            "http" if !require_https => {}
            "http" => {
                return Err(AppError::invalid_input(format!(
                    "MCP_RESOURCE_URL must use HTTPS in production: {value}"
                )))
            }
            _ => {
                return Err(AppError::invalid_input(format!(
                    "MCP_RESOURCE_URL must be an http or https URL: {value}"
                )))
            }
        }
        let is_resource = url.host().is_some()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none();
        if is_resource {
            Ok(())
        } else {
            Err(AppError::invalid_input(format!(
                "MCP_RESOURCE_URL must be an origin, optionally with the MCP endpoint's path \
                 (scheme://host[:port][/path]), with no query, fragment or credentials: {value}"
            )))
        }
    }

    /// Every identifier of the MCP resource server: `mcp_resource_url` first,
    /// then its aliases.
    #[must_use]
    pub fn mcp_resources(&self) -> Vec<&str> {
        iter::once(self.mcp_resource_url.as_str())
            .chain(self.mcp_resource_aliases.iter().map(String::as_str))
            .collect()
    }

    /// The MCP resource identifier a client that dialed `host` knows the
    /// server by: the first whose `host[:port]` it is, else `mcp_resource_url`.
    ///
    /// `host` is the dialed authority as a proxy forwards it (the first entry
    /// of an `X-Forwarded-Host` list) or the `Host` header. Only a configured
    /// identifier is ever returned, so a forged header can at most pick
    /// another of this server's own names.
    #[must_use]
    pub fn mcp_resource_for_host(&self, host: Option<&str>) -> &str {
        let Some(host) = host
            .and_then(|host| host.split(',').next())
            .map(str::trim)
            .filter(|host| !host.is_empty())
        else {
            return &self.mcp_resource_url;
        };
        self.mcp_resources()
            .into_iter()
            .find(|resource| authority_of(resource).is_some_and(|a| a.eq_ignore_ascii_case(host)))
            .unwrap_or(&self.mcp_resource_url)
    }

    /// The identifier the root metadata document
    /// (`/.well-known/oauth-protected-resource`) publishes to a client that
    /// dialed `host`: the bare origin form when the server answers to one on
    /// that host, since RFC 9728 §3.3 has the client compare `resource` to
    /// the URL it derived the document URL from. A client connected before
    /// the path was published holds exactly this.
    #[must_use]
    pub fn mcp_root_resource_for_host(&self, host: Option<&str>) -> &str {
        let anchor = self.mcp_resource_for_host(host);
        let authority = authority_of(anchor);
        self.mcp_resources()
            .into_iter()
            .find(|resource| authority_of(resource) == authority && path_of(resource).is_empty())
            .unwrap_or(anchor)
    }

    /// The identifier a path-suffixed metadata document describes: the one on
    /// the dialed host whose path is `path` (`mcp` or `/mcp`). `None` when this
    /// server publishes no such resource, so the route answers 404 rather than
    /// a document naming a resource the client did not ask about.
    #[must_use]
    pub fn mcp_resource_at(&self, host: Option<&str>, path: &str) -> Option<&str> {
        let wanted = format!("/{}", path.trim_matches('/'));
        let authority = authority_of(self.mcp_resource_for_host(host));
        self.mcp_resources()
            .into_iter()
            .find(|resource| authority_of(resource) == authority && path_of(resource) == wanted)
    }
}
