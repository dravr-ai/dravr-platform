// ABOUTME: Admin access at consent: the one way an OAuth grant comes to hold `admin` (carnet#1511, step 12)
// ABOUTME: Ticked by a live admin, bound to the platform /mcp or /a2a resource, kept only while the user is an admin

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin access at consent
//!
//! `admin` is never asked for and never published: registration, a client
//! metadata document and the client-credentials grant go on refusing it
//! through [`OAuthScope::requested_grant`], and `scopes_supported` lists the
//! delegable scopes alone. The authorization request is the one exception,
//! and only so far as to drop it: a client that names `admin` is checked as if
//! it had not ([`without_admin`]), so a non-admin is never refused for it and
//! never holds it.
//!
//! An admin is offered an unticked Admin access box on the consent screen
//! instead (JF, Q20). The consent route reads the admin role live, reads the
//! box, and hands the decision here as an [`AdminAccess`]; nothing in the form
//! is taken as a scope. The grant then holds `admin` only when the request is
//! bound to the platform's own `/mcp` or `/a2a` resource
//! ([`CheckedAuthorization::admin_grantable`]): a token with no resource, or
//! one for a satellite, never carries it.
//!
//! Every mint reads the role again ([`OAuth2AuthorizationServer::delegated_grant_for`]):
//! a user who is no longer an admin, or whose account cannot be read, is
//! minted the grant without `admin`, and a refresh writes the successor
//! without it, so a later promotion never restores it without the screen.

use pierre_core::permissions::scopes::OAuthScope;
use tracing::{info, warn};
use uuid::Uuid;

use super::{CheckedAuthorization, OAuth2AuthorizationServer};

/// What the signed-in user decided about Admin access on the consent screen,
/// as the consent route read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdminAccess {
    /// No admin access: the box was not ticked, not offered, or the user is
    /// not an admin. Every path that shows no consent screen passes this.
    #[default]
    Withheld,
    /// An admin, read live by the route, ticked the box.
    Ticked,
}

/// The `scope` of an authorization request without `admin`.
///
/// `admin` is never asked for: it is offered on the consent screen. So a
/// request naming it is checked as if it had not, rather than refused with
/// `invalid_scope` (JF, Q20). What remains goes through the shared
/// [`OAuthScope::requested_grant`] unchanged.
#[must_use]
pub fn without_admin(scope: Option<&str>) -> Option<String> {
    let admin = OAuthScope::Admin.as_str();
    scope.map(|scope| {
        scope
            .split_whitespace()
            .filter(|name| *name != admin)
            .collect::<Vec<_>>()
            .join(" ")
    })
}

impl OAuth2AuthorizationServer {
    /// Whether `resource`, the audience an authorization is bound to, is the
    /// platform's own MCP or A2A resource server: the only audiences an
    /// `admin` grant is minted for. `None` (no resource named) and a
    /// satellite are not.
    pub(super) fn is_platform_resource(&self, resource: Option<&str>) -> bool {
        resource.is_some_and(|resource| {
            self.resources
                .iter()
                .chain(&self.a2a_resources)
                .any(|served| served == resource)
        })
    }

    /// The scope a checked authorization is granted once the user decided
    /// `access`: the checked scope, plus `admin` when an admin ticked the box
    /// and the authorization is bound to the platform's own resource.
    ///
    /// The consent route records this exact string as the grant and the code
    /// carries it, so the two cannot disagree.
    #[must_use]
    pub fn granted_scope(checked: &CheckedAuthorization, access: AdminAccess) -> String {
        if access != AdminAccess::Ticked || !checked.admin_grantable {
            return checked.scope.clone();
        }
        let mut granted = OAuthScope::parse_granted(&checked.scope);
        granted.push(OAuthScope::Admin);
        OAuthScope::render_granted(&granted)
    }

    /// The grant a token minted from a stored code or refresh token carries
    /// for `user_id`: the stored `scope`'s names, with `admin` kept only while
    /// the user is an admin, read now, and the grant is bound to the platform's
    /// own resource.
    ///
    /// A user who cannot be read, or who is gone, is not an admin here: unlike
    /// the provider list a token describes, the admin role authorizes.
    ///
    /// Holding every scope never makes a delegated token a session: a route
    /// that reads no scope tells a third party from the athlete by the client
    /// the token names ([`Claims::is_session`](crate::auth::Claims::is_session)).
    pub(super) async fn delegated_grant_for(
        &self,
        user_id: Uuid,
        scope: Option<&str>,
        resource: Option<&str>,
    ) -> Vec<OAuthScope> {
        let mut granted = OAuthScope::parse_granted(scope.unwrap_or_default());
        if !granted.contains(&OAuthScope::Admin) {
            return granted;
        }
        let keeps_admin = self.is_platform_resource(resource) && self.is_live_admin(user_id).await;
        if !keeps_admin {
            info!(%user_id, "admin dropped from a delegated grant: no longer an admin, or not bound to the platform");
            granted.retain(|scope| *scope != OAuthScope::Admin);
        }
        granted
    }

    /// Whether `user_id` is an admin now. A read that fails or finds no user
    /// answers no.
    async fn is_live_admin(&self, user_id: Uuid) -> bool {
        // SECURITY: Global lookup — OAuth2 token minting, no tenant context.
        match self.users.get_global(user_id).await {
            Ok(user) => user.is_some_and(|user| user.role.is_admin()),
            Err(e) => {
                warn!(%user_id, "admin role could not be read at mint; minting without admin: {e}");
                false
            }
        }
    }
}

/// Whether a stored grant `scope` holds `admin`.
pub(super) fn holds_admin(scope: Option<&str>) -> bool {
    OAuthScope::parse_granted(scope.unwrap_or_default()).contains(&OAuthScope::Admin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_naming_admin_is_checked_as_if_it_had_not() {
        assert_eq!(
            without_admin(Some("fitness:read admin profile:read")).as_deref(),
            Some("fitness:read profile:read"),
            "bug: admin was kept, or another scope dropped with it"
        );
        assert_eq!(
            without_admin(Some("admin")).as_deref(),
            Some(""),
            "only admin named: the rest is empty and reads as the default grant"
        );
        assert_eq!(
            without_admin(Some("fitness:read")).as_deref(),
            Some("fitness:read"),
            "bug: a request without admin was changed"
        );
        assert_eq!(
            without_admin(None),
            None,
            "bug: a request naming no scope was given one"
        );
        assert_eq!(
            without_admin(Some("administrator")).as_deref(),
            Some("administrator"),
            "bug: a name that merely starts with admin was dropped instead of refused"
        );
    }

    fn checked(scope: &str, admin_grantable: bool) -> CheckedAuthorization {
        CheckedAuthorization {
            scope: scope.to_owned(),
            resource: None,
            client_name: None,
            consent_every_time: false,
            unstored_client: None,
            client_host_profile: None,
            admin_grantable,
            details: None,
            agent_write: None,
            preset_source: None,
        }
    }

    #[test]
    fn admin_is_granted_only_ticked_and_on_the_platforms_resource() {
        let bound = checked("fitness:read profile:read", true);
        assert_eq!(
            OAuth2AuthorizationServer::granted_scope(&bound, AdminAccess::Ticked),
            "fitness:read profile:read admin",
            "bug: a ticked box on the platform's resource did not grant admin"
        );
        assert_eq!(
            OAuth2AuthorizationServer::granted_scope(&bound, AdminAccess::Withheld),
            "fitness:read profile:read",
            "bug: an unticked box granted admin"
        );
        let unbound = checked("fitness:read profile:read", false);
        assert_eq!(
            OAuth2AuthorizationServer::granted_scope(&unbound, AdminAccess::Ticked),
            "fitness:read profile:read",
            "bug: admin granted for a request not bound to the platform's resource"
        );
    }

    #[test]
    fn a_stored_scope_holding_admin_is_told_apart() {
        assert!(
            holds_admin(Some("fitness:read admin")),
            "bug: a stored grant holding admin was not told apart"
        );
        assert!(
            !holds_admin(Some("fitness:read profile:read")),
            "bug: a grant without admin read as holding it"
        );
        assert!(!holds_admin(None), "bug: no stored scope read as admin");
    }
}
