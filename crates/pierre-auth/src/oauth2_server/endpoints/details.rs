// ABOUTME: An authorization request's RFC 9396 details, resolved against the resource its tokens are for
// ABOUTME: A preset grant reads from the platform's own MCP or A2A resource; the details a grant stored are what its tokens echo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::OAuthClientGrant;
use tracing::error;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::authorization_details::{
    parse_authorization_details, resolve, GrantDocument, ResolvedAgentWrite, ResolvedDetails,
};
use crate::oauth2_server::models::{AuthorizeRequest, OAuth2Error};
use crate::oauth2_server::step_up_scope::{agent_write_details, one_off_details, StepUpScope};

/// What a request's RFC 9396 details ask for, resolved and bound.
#[derive(Debug, Default)]
pub struct RequestDetails {
    /// The preset read grant; `None` when the request asks none
    pub details: Option<ResolvedDetails>,
    /// The records the client asks to propose; `None` when it asks none
    pub agent_write: Option<ResolvedAgentWrite>,
}

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

    /// The `authorization_details` a step-up scope asks for.
    ///
    /// Read from the first source a preset on `resource` reads; `None` for a
    /// request naming no step-up scope.
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
        if let Some(write) = agent_write_details(step_up)? {
            return Ok(Some(write));
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

    /// The request's `authorization_details`, parsed and resolved for tokens bound to
    /// `resource`, an agent-write detail checked against the binding it asks for
    /// ([`Self::bind_agent_write`]).
    ///
    /// Empty when it asks for none.
    ///
    /// # Errors
    /// `invalid_authorization_details` as
    /// [`parse_authorization_details`] and [`resolve`] refuse them, and for
    /// a request bound to a satellite, whose tokens no preset governs;
    /// `invalid_target` as [`Self::bind_agent_write`] refuses the resource.
    pub(super) fn resolve_request_details(
        &self,
        raw: Option<&str>,
        resource: Option<&str>,
        dpop_jkt: Option<&str>,
    ) -> Result<RequestDetails, OAuth2Error> {
        let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
            self.bind_agent_write(None, false, resource, dpop_jkt)?;
            return Ok(RequestDetails::default());
        };
        let parsed = parse_authorization_details(raw)?;
        let agent_write = self.bind_agent_write(
            parsed.agent_write,
            parsed.data_access.is_some(),
            resource,
            dpop_jkt,
        )?;
        let Some(requested) = parsed.data_access else {
            return Ok(RequestDetails {
                details: None,
                agent_write,
            });
        };
        // A step-up preset is granted only as a child of a live grant, after
        // a fresh sign-in (carnet#1544 slice 4b); while the step-up screen is
        // not offered (its words are not in the catalogue) it is never
        // resolved here, so no consent grants it.
        if requested.requires_step_up() && !self.step_up_offered {
            return Err(OAuth2Error::invalid_authorization_details(
                "step-up is not offered yet",
            ));
        }
        let details = resolve(&requested, &self.preset_sources(resource))?;
        Ok(RequestDetails {
            details: Some(details),
            agent_write,
        })
    }
}

/// The details `grant` was approved for, as its tokens echo them; `None` for
/// a plain grant, or when no grant governs the token. A read grant echoes
/// its resolved preset, an agent-write grant its detail.
///
/// # Errors
/// `server_error` when a stored document does not read: a grant whose terms
/// are unknown mints nothing.
pub(super) fn granted_details(
    grant: Option<&OAuthClientGrant>,
) -> Result<Option<GrantDocument>, OAuth2Error> {
    grant
        .and_then(|grant| grant.authorization_details.as_deref())
        .map(|document| {
            GrantDocument::from_document(document).map_err(|e| {
                error!(error = %e, "A consent grant's authorization details do not read");
                OAuth2Error::server_error("Failed to read the consent grant")
            })
        })
        .transpose()
}
