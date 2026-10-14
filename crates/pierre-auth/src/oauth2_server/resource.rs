// ABOUTME: RFC 8707 resource indicators — which resource server an access token is minted for
// ABOUTME: Checks a requested `resource` against the MCP and satellite resources and names the audience it binds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Matches a requested `resource` against the resources this server mints for.
//!
//! A client names the resource server it wants a token for with the RFC 8707
//! `resource` parameter, on `/oauth2/authorize` and `/oauth2/token`. This
//! authorization server mints for the platform's MCP resource server, known by
//! each origin that serves `/mcp` (`MCP_RESOURCE_URL` and its aliases): the
//! one a client dialed is what `/.well-known/oauth-protected-resource`
//! publishes to it. It also mints for each satellite MCP server named in
//! `OAUTH2_SATELLITE_RESOURCES`, which verifies the token against this
//! server's JWKS and accepts only its own audience. A request naming anything
//! else is refused with `invalid_target` rather than answered with a token no
//! resource server would accept — or, worse, one a different server would.

use url::Url;

use super::models::OAuth2Error;

/// The audience a token requested for `requested` is bound to, when this
/// authorization server serves the resource known by each of `served`.
///
/// `requested` must be an absolute `http`/`https` URI with no fragment (RFC
/// 8707 §2), query or credentials, on the origin of one of `served` and at or
/// below its path — the MCP SDKs send either the published identifier itself,
/// often normalized with a trailing `/`, or the MCP endpoint's URL beneath it.
/// The audience is that identifier verbatim: the string the metadata published
/// to the client and the resource server checks.
///
/// # Errors
/// `invalid_target` when `requested` is malformed or names another resource.
pub fn bound_audience(served: &[&str], requested: &str) -> Result<String, OAuth2Error> {
    let requested_url = Url::parse(requested).map_err(|_| {
        OAuth2Error::invalid_target("The resource parameter must be an absolute URI")
    })?;
    if !matches!(requested_url.scheme(), "http" | "https") {
        return Err(OAuth2Error::invalid_target(
            "The resource parameter must be an http or https URI",
        ));
    }
    if requested_url.fragment().is_some() {
        return Err(OAuth2Error::invalid_target(
            "The resource parameter must not include a fragment",
        ));
    }
    if requested_url.query().is_some()
        || !requested_url.username().is_empty()
        || requested_url.password().is_some()
    {
        return Err(OAuth2Error::invalid_target(
            "The resource parameter must not include a query or credentials",
        ));
    }

    // The most specific identifier wins: a satellite served under a path of
    // the platform's own origin is the satellite, not the platform.
    served
        .iter()
        .filter(|served| names_resource(&requested_url, served))
        .max_by_key(|served| served.trim_end_matches('/').len())
        .map(|served| (*served).to_owned())
        .ok_or_else(|| {
            OAuth2Error::invalid_target(
                "The requested resource is not served by this authorization server",
            )
        })
}

/// Whether `requested` is on the origin of `served` and at or below its path.
fn names_resource(requested: &Url, served: &str) -> bool {
    let Ok(served_url) = Url::parse(served) else {
        return false;
    };
    let served_path = served_url.path().trim_end_matches('/');
    let requested_path = requested.path();
    let within_served = requested_path.trim_end_matches('/') == served_path
        || requested_path
            .strip_prefix(served_path)
            .is_some_and(|rest| rest.starts_with('/'));
    requested.origin() == served_url.origin() && within_served
}

/// The resource servers an authorization server mints for.
///
/// The platform's MCP resource server answers to several identifiers (each
/// origin that serves `/mcp`), and they are interchangeable: a grant bound to
/// one may mint for another, which is what keeps the clients of a moved host
/// working. Each satellite is its own resource server, known by one
/// identifier, and never interchangeable with the platform or with another
/// satellite.
#[derive(Debug, Clone, Copy)]
pub struct ResourceServers<'a> {
    /// Every identifier of the platform's MCP resource server.
    pub platform: &'a [&'a str],
    /// One identifier per satellite MCP resource server.
    pub satellites: &'a [&'a str],
}

impl ResourceServers<'_> {
    /// Every identifier this authorization server mints for: the platform's,
    /// then the satellites'.
    fn identifiers(&self) -> Vec<&str> {
        self.platform
            .iter()
            .chain(self.satellites)
            .copied()
            .collect()
    }

    /// Whether `a` and `b` name the same resource server.
    fn same_server(&self, a: &str, b: &str) -> bool {
        a == b || (self.platform.contains(&a) && self.platform.contains(&b))
    }
}

/// The audience of an access token minted from a grant bound to `granted`,
/// when the token request names `requested` (RFC 8707 §2.2).
///
/// A request that names no resource gets the grant's. One that names a
/// resource may move the token only within the resource server the grant was
/// authorized for: a grant bound to the platform can mint for any of the
/// platform's identifiers, a grant bound to a satellite for that satellite
/// alone. A grant bound to no resource (the platform audience) can be narrowed
/// to the platform's identifiers, never to a satellite the athlete did not
/// consent to.
///
/// # Errors
/// `invalid_target` when `requested` is refused, or names a resource server
/// other than the grant's; `invalid_grant` when `granted` is no longer an
/// identifier this server mints for, so the client authorizes again instead of
/// receiving a token no resource server accepts.
pub fn token_audience(
    servers: &ResourceServers<'_>,
    requested: Option<&str>,
    granted: Option<&str>,
) -> Result<Option<String>, OAuth2Error> {
    let identifiers = servers.identifiers();
    if granted.is_some_and(|granted| !identifiers.contains(&granted)) {
        return Err(OAuth2Error::invalid_grant(
            "The resource this grant was authorized for is no longer served here",
        ));
    }
    let Some(requested) = requested else {
        return Ok(granted.map(str::to_owned));
    };
    let bound = bound_audience(&identifiers, requested)?;
    let within_grant = granted.map_or_else(
        || servers.platform.contains(&bound.as_str()),
        |granted| servers.same_server(granted, &bound),
    );
    if within_grant {
        Ok(Some(bound))
    } else {
        Err(OAuth2Error::invalid_target(
            "The requested resource is not the resource server this grant was authorized for",
        ))
    }
}
