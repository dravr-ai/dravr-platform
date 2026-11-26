// ABOUTME: An authorization request's RFC 9396 details, resolved against the resource its tokens are for
// ABOUTME: A preset grant reads from the platform's own MCP or A2A resource; the details a grant stored are what its tokens echo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::OAuthClientGrant;
use tracing::error;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::authorization_details::{
    parse_authorization_details, resolve, ResolvedDetails,
};
use crate::oauth2_server::models::{AuthorizeRequest, OAuth2Error};
use crate::oauth2_server::step_up_scope::{one_off_details, StepUpScope};

impl OAuth2AuthorizationServer {
    /// The resources a preset grant on a request bound to `resource` may read
    /// from: that resource when it is the platform's own MCP or A2A resource,
    /// every MCP origin when the request names none, nothing for a satellite.
    pub(super) fn preset_sources<'a>(&'a self, resource: Option<&'a str>) -> Vec<&'a str> {
        match resource {
            Some(resource) if self.is_platform_resource(Some(resource)) => vec![resource],
            Some(_) => Vec::new(),
            None => self.resources.iter().map(String::as_str).collect(),
        }
    }

    /// The `authorization_details` a step-up scope asks for, read from the
    /// first source a preset on `resource` reads; `None` for a request
    /// naming no step-up scope.
    ///
    /// # Errors
    /// `invalid_request` for a request naming both a step-up scope and its
    /// own details, `invalid_scope` for a step-up on a resource no preset
    /// reads (a satellite's).
    pub(super) fn step_up_details(
        &self,
        step_up: Option<&StepUpScope>,
        request: &AuthorizeRequest,
        resource: Option<&str>,
    ) -> Result<Option<String>, OAuth2Error> {
        let Some(step_up) = step_up else {
            return Ok(None);
        };
        if request
            .authorization_details
            .as_deref()
            .is_some_and(|raw| !raw.is_empty())
        {
            return Err(OAuth2Error::invalid_request(
                "a step-up scope and authorization_details cannot be asked together",
            ));
        }
        let source = self
            .preset_sources(resource)
            .first()
            .copied()
            .ok_or_else(|| {
                OAuth2Error::invalid_scope("a step-up is asked of the platform's own resource only")
            })?;
        Ok(Some(one_off_details(step_up, source)))
    }

    /// The request's `authorization_details`, parsed and resolved for tokens
    /// bound to `resource`; `None` when it asks for none.
    ///
    /// # Errors
    /// `invalid_authorization_details` as
    /// [`parse_authorization_details`] and [`resolve`] refuse them, and for
    /// a request bound to a satellite, whose tokens no preset governs.
    pub(super) fn resolve_request_details(
        &self,
        raw: Option<&str>,
        resource: Option<&str>,
    ) -> Result<Option<ResolvedDetails>, OAuth2Error> {
        let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
            return Ok(None);
        };
        let requested = parse_authorization_details(raw)?;
        // A step-up preset is granted only as a child of a live grant, after
        // a fresh sign-in (carnet#1544 slice 4b); while the step-up screen is
        // not offered (its words are not in the catalogue) it is never
        // resolved here, so no consent grants it.
        if requested.requires_step_up() && !self.step_up_offered {
            return Err(OAuth2Error::invalid_authorization_details(
                "step-up is not offered yet",
            ));
        }
        resolve(&requested, &self.preset_sources(resource)).map(Some)
    }
}

/// The details `grant` was approved for, as its tokens echo them; `None` for
/// a plain grant, or when no grant governs the token.
///
/// # Errors
/// `server_error` when a stored document does not read: a grant whose terms
/// are unknown mints nothing.
pub(super) fn granted_details(
    grant: Option<&OAuthClientGrant>,
) -> Result<Option<ResolvedDetails>, OAuth2Error> {
    grant
        .and_then(|grant| grant.authorization_details.as_deref())
        .map(|document| {
            ResolvedDetails::from_document(document).map_err(|e| {
                error!(error = %e, "A consent grant's authorization details do not read");
                OAuth2Error::server_error("Failed to read the consent grant")
            })
        })
        .transpose()
}
