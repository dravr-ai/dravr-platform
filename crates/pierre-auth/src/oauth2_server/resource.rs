// ABOUTME: RFC 8707 resource indicators — which resource server an access token is minted for
// ABOUTME: Checks a requested `resource` against the configured MCP resource and names the audience it binds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Matches a requested `resource` against the MCP resource this server serves.
//!
//! A client names the resource server it wants a token for with the RFC 8707
//! `resource` parameter, on `/oauth2/authorize` and `/oauth2/token`. This
//! authorization server mints for exactly one: the MCP resource server whose
//! identifier it publishes as `resource` in
//! `/.well-known/oauth-protected-resource`. A request naming anything else is
//! refused with `invalid_target` rather than answered with a token no resource
//! server would accept — or, worse, one a different server would.

use url::Url;

use super::models::OAuth2Error;

/// The audience a token requested for `requested` is bound to, when this
/// authorization server serves the resource `served`.
///
/// `requested` must be an absolute `http`/`https` URI with no fragment (RFC
/// 8707 §2), query or credentials, on the origin of `served` and at or below
/// its path — the MCP SDKs send either the published identifier itself, often
/// normalized with a trailing `/`, or the MCP endpoint's URL beneath it. Every
/// such URI names the one resource server, so the audience is `served`
/// verbatim: the string the metadata publishes and the resource server checks.
///
/// # Errors
/// `invalid_target` when `requested` is malformed or names another resource.
pub fn bound_audience(served: &str, requested: &str) -> Result<String, OAuth2Error> {
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

    let Ok(served_url) = Url::parse(served) else {
        return Err(OAuth2Error::invalid_target(
            "This authorization server publishes no resource a request can name",
        ));
    };
    let served_path = served_url.path().trim_end_matches('/');
    let requested_path = requested_url.path();
    let within_served = requested_path.trim_end_matches('/') == served_path
        || requested_path
            .strip_prefix(served_path)
            .is_some_and(|rest| rest.starts_with('/'));

    if requested_url.origin() == served_url.origin() && within_served {
        Ok(served.to_owned())
    } else {
        Err(OAuth2Error::invalid_target(
            "The requested resource is not served by this authorization server",
        ))
    }
}

/// The audience of an access token minted from a grant bound to `granted`,
/// when the token request names `requested` (RFC 8707 §2.2).
///
/// A request that names no resource gets the grant's. One that names a
/// resource must name the one this server serves — which is also the only
/// resource a grant can be bound to, so a token request can narrow an unbound
/// grant to it and can never move a bound grant elsewhere.
///
/// # Errors
/// `invalid_target` when `requested` is refused; `invalid_grant` when
/// `granted` is no longer the resource this server serves, so the client
/// authorizes again instead of receiving a token no resource server accepts.
pub fn token_audience(
    served: &str,
    requested: Option<&str>,
    granted: Option<&str>,
) -> Result<Option<String>, OAuth2Error> {
    if granted.is_some_and(|granted| granted != served) {
        return Err(OAuth2Error::invalid_grant(
            "The resource this grant was authorized for is no longer served here",
        ));
    }
    requested.map_or_else(
        || Ok(granted.map(str::to_owned)),
        |requested| bound_audience(served, requested).map(Some),
    )
}
