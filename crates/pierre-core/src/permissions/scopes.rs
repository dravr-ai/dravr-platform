// ABOUTME: The OAuth scope vocabulary this resource server defines, publishes and enforces
// ABOUTME: Five coarse names, each derived from a tool capability flag — never a per-tool table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The OAuth 2.1 scope vocabulary.
//!
//! Three names — `fitness:read`, `activities:read`, `profile:read` — were
//! published in both metadata documents and enforced nowhere: an
//! advertised-but-empty surface. This module is the vocabulary those names
//! should have named, and it is deliberately *coarse*.
//!
//! Coarse, because the alternatives are worse. A scope per tool is ~110 names
//! no athlete can read on a consent screen. A scope per capability flag would
//! publish `requires_tenant` and `requires_provider` as things to consent to —
//! those flags are runtime requirements, not permissions. Only four of the
//! platform's flags describe *what a caller may do*, so those four are the
//! whole vocabulary, and the mapping from them is mechanical. A per-tool table
//! would drift from what the tools declare; a derivation cannot.
//!
//! `activities:read` is gone. Nothing mapped to it — activities are fitness
//! data, reached under `fitness:read` — so publishing it asked clients to
//! request a grant this server would never check.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};

/// One name in this server's OAuth scope vocabulary.
///
/// The wire form is the `:`-separated string, which is what appears in an
/// authorization request, in the `scope` claim of a minted token, in
/// `scopes_supported` on both metadata documents, and in the `scope` parameter
/// of an RFC 6750 `insufficient_scope` challenge. [`Self::as_str`] is the one
/// place that spelling lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OAuthScope {
    /// Read the athlete's fitness data — activities, stats, sleep, analytics.
    #[serde(rename = "fitness:read")]
    FitnessRead,
    /// Create or modify the athlete's fitness data — goals, plans, logged work.
    #[serde(rename = "fitness:write")]
    FitnessWrite,
    /// Read who the athlete is — profile, configuration, which providers are linked.
    #[serde(rename = "profile:read")]
    ProfileRead,
    /// Change who the athlete is — profile fields, configuration, provider links.
    #[serde(rename = "profile:write")]
    ProfileWrite,
    /// Operate the server. Held by operators, never delegated to an integration.
    #[serde(rename = "admin")]
    Admin,
}

impl OAuthScope {
    /// Every scope this server defines, in declaration order.
    ///
    /// `scopes_supported` is served from this constant (its delegable part,
    /// [`Self::delegable_as_str`]) rather than a literal list, so a scope
    /// cannot be enforced without being published or published without being
    /// enforceable — which is exactly how the three names this replaces came
    /// to mean nothing. `admin` is the one name enforced but not published: a
    /// client can never be granted it, so advertising it would only invite a
    /// request the authorization server has to refuse.
    pub const ALL: [Self; 5] = [
        Self::FitnessRead,
        Self::FitnessWrite,
        Self::ProfileRead,
        Self::ProfileWrite,
        Self::Admin,
    ];

    /// The grant carried by a credential that is the athlete acting directly —
    /// a signed-in session, their own API key, a linked messaging channel —
    /// rather than a third party acting on their behalf.
    ///
    /// Every scope, `admin` included, and that is not a grant of admin. Scope
    /// answers *how narrow is this delegation*; the role gate answers *who is
    /// this*, and both still have to pass. A direct credential is not narrowed
    /// at all, so withholding `admin` here would encode the role decision in
    /// the wrong axis — and then an admin's own API key would be refused an
    /// admin tool for the wrong reason, or a non-admin's session would look
    /// like it had been granted something it had not.
    ///
    /// Only an OAuth grant is narrower than this, which is the whole point: the
    /// delegation axis exists for third parties.
    #[must_use]
    pub fn self_grant() -> Vec<Self> {
        Self::ALL.to_vec()
    }

    /// The grant a client gets when it registers or authorizes without asking
    /// for one (RFC 7591 §3.1.1, RFC 6749 §3.3).
    ///
    /// Read-only, and read-only deliberately: a client that never asked for
    /// anything has not been consented to writing. The consent screen renders
    /// this same list, so what the athlete is shown and what the client is
    /// issued cannot disagree — they were two literals before, and the literals
    /// still named `activities:read`, a scope nothing checked.
    #[must_use]
    pub fn default_grant() -> Vec<Self> {
        vec![Self::FitnessRead, Self::ProfileRead]
    }

    /// Whether a third party may be granted this scope.
    ///
    /// Everything but `admin`. Operating the server stays on the operator's
    /// own credential and is never delegated to an integration, so the
    /// authorization server refuses it in a registration or an authorization
    /// request, leaves it out of every token it mints, and does not publish it.
    #[must_use]
    pub const fn is_delegable(self) -> bool {
        !matches!(self, Self::Admin)
    }

    /// Whether `granted` is the whole [`Self::self_grant`]: the athlete acting
    /// directly rather than a third party acting for them.
    ///
    /// This is how a route that reads no scope tells the two apart. It is sound
    /// because a delegation is always strictly narrower: every first-party
    /// credential is minted with every scope, and no grant the authorization
    /// server mints holds `admin` ([`Self::is_delegable`]).
    #[must_use]
    pub fn is_self_grant(granted: &[Self]) -> bool {
        Self::ALL.iter().all(|scope| granted.contains(scope))
    }

    /// The grant a client *asks for* — at registration (RFC 7591 §2) or at
    /// authorization (RFC 6749 §3.3) — or [`Self::default_grant`] when it
    /// names none.
    ///
    /// Stricter than [`Self::parse_granted`], which reads a grant that was
    /// already minted. A request is where an unknown name is the client's error
    /// to hear about, and where `admin` is refused outright rather than
    /// dropped: it is never delegated.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] naming the first scope that is unknown or
    /// not delegable. The message is written for the client that sent it.
    pub fn requested_grant(scope: Option<&str>) -> AppResult<Vec<Self>> {
        let mut requested = Vec::new();
        for name in scope.unwrap_or_default().split_whitespace() {
            let parsed = Self::from_str(name)?;
            if !parsed.is_delegable() {
                return Err(AppError::invalid_input(format!(
                    "the '{parsed}' scope is never delegated to an application"
                )));
            }
            requested.push(parsed);
        }
        if requested.is_empty() {
            return Ok(Self::default_grant());
        }
        requested.sort_unstable();
        requested.dedup();
        Ok(requested)
    }

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FitnessRead => "fitness:read",
            Self::FitnessWrite => "fitness:write",
            Self::ProfileRead => "profile:read",
            Self::ProfileWrite => "profile:write",
            Self::Admin => "admin",
        }
    }

    /// The scopes a client can be granted, as wire strings — what
    /// `scopes_supported` publishes on both metadata documents.
    ///
    /// A spec-following MCP client requests exactly this list at registration
    /// and at authorization (the MCP scope-selection strategy falls back to the
    /// protected resource's `scopes_supported`), so a name listed here that the
    /// authorization server refuses would fail every such client's connection.
    #[must_use]
    pub fn delegable_as_str() -> Vec<&'static str> {
        Self::ALL
            .iter()
            .filter(|scope| scope.is_delegable())
            .map(|scope| scope.as_str())
            .collect()
    }

    /// Parse a space-delimited `scope` string, the RFC 6749 §3.3 wire form.
    ///
    /// This is how a third party's grant enters the system — an OAuth access
    /// token's claim, or an A2A client-credentials token's. Such a caller acts
    /// FOR the athlete rather than as them, so its grant is whatever it was
    /// issued and never [`Self::self_grant`]: the delegation axis exists for
    /// exactly this case.
    ///
    ///
    /// Unknown names are dropped rather than refused. A resource server reads
    /// this from a token another party minted; refusing the whole grant because
    /// one name is unrecognised would deny a caller whose remaining scopes are
    /// perfectly sufficient. The authorization endpoint is where an unknown
    /// scope is an `invalid_scope` error — that check already exists and runs
    /// against the client's registered scope.
    #[must_use]
    pub fn parse_granted(scope: &str) -> Vec<Self> {
        let mut granted: Vec<Self> = scope
            .split_whitespace()
            .filter_map(|name| Self::from_str(name).ok())
            .collect();
        granted.sort_unstable();
        granted.dedup();
        granted
    }

    /// Render a grant back to the space-delimited wire form.
    #[must_use]
    pub fn render_granted(granted: &[Self]) -> String {
        granted
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Display for OAuthScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OAuthScope {
    type Err = AppError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "fitness:read" => Ok(Self::FitnessRead),
            "fitness:write" => Ok(Self::FitnessWrite),
            "profile:read" => Ok(Self::ProfileRead),
            "profile:write" => Ok(Self::ProfileWrite),
            "admin" => Ok(Self::Admin),
            other => Err(AppError::invalid_input(format!(
                "unknown OAuth scope '{other}'"
            ))),
        }
    }
}
