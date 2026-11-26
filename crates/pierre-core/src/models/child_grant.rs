// ABOUTME: A step-up child grant: the one-off T2 access an athlete approves under a live grant after a fresh sign-in
// ABOUTME: Its step-up result and Host Profile basis, stored on the child row and stated in its receipts and introspection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Step-up child grants (carnet#1544 slice 4b, owl PDPP §6.1, §6.2, §7.6)
//!
//! A step-up approval never edits the grant it is approved under: it records
//! a **child** of the client's live grant, an [`OAuthClientGrant`] row whose
//! [`StepUpTerms::parent_grant_id`] names that parent. The token the client
//! holds keeps naming the parent; the parent's ceiling then includes the
//! child's picked streams at the child's tier, window and budgets.
//!
//! A parent holds at most one live child: approving a new step-up revokes the
//! old child first. Revoking or replacing the parent revokes its child, and a
//! single-use child is consumed by the first call that reads data under it.
//!
//! `owner_verified` is owl's value for a passkey unlock with user
//! verification (PDPP §4.2). Dravr has no passkey: the value is recorded for a
//! sign-in on Dravr's own hosted page, password or Google verified by Dravr,
//! at most [`STEP_UP_MAX_AGE_SECS`] before the approval. That is a recorded
//! deviation (plan revision 17): the value is owl's so a conforming client
//! works, the assurance is JF's 2026-11-25 answer, and a passkey
//! later makes it exact.

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};

use super::OAuthClientGrant;

/// The oldest a hosted-page sign-in may be when the athlete approves a
/// step-up, in seconds (RFC 9470 `max_age`; JF 2026-11-25: five
/// minutes).
pub const STEP_UP_MAX_AGE_SECS: i64 = 300;

/// The step-up assurance a child records (owl §4.2 `acr`).
pub const OWNER_VERIFIED_ACR: &str = "owner_verified";

/// How a host satisfied Host Profile T2 (owl §6.2 `basis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProfileBasis {
    /// A `mcp_host_profile` member in the client's CIMD document, read at
    /// consent (carrier A)
    DeclaredCimd,
    /// The per-request `_meta` host-profile extension, seen on the request
    /// that triggered consent and checked again on every call (carrier B)
    DeclaredRequest,
}

impl HostProfileBasis {
    /// The stored value (`oauth_client_grants.host_profile_basis`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeclaredCimd => "declared_cimd",
            Self::DeclaredRequest => "declared_request",
        }
    }

    /// The basis a stored value names; `None` for a value this server never
    /// writes, which then opens nothing.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "declared_cimd" => Some(Self::DeclaredCimd),
            "declared_request" => Some(Self::DeclaredRequest),
            _ => None,
        }
    }
}

/// What a step-up child records beside an ordinary grant's columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepUpTerms {
    /// The live grant the child was approved under
    pub parent_grant_id: String,
    /// When the athlete signed in on Dravr's hosted page (RFC 9470
    /// `auth_time`)
    pub auth_time: Timestamp,
    /// The assurance recorded: [`OWNER_VERIFIED_ACR`]
    pub acr: String,
    /// How the host satisfied Host Profile T2
    pub host_profile_basis: HostProfileBasis,
    /// The highest tier the host declared it handles (`T2`)
    pub host_profile_tier: String,
}

impl StepUpTerms {
    /// Whether these terms open T2: the athlete stepped up as owl asks and the
    /// host declared T2. The athlete's own `sensitive_t2` switch, and the
    /// per-request claim, are asked on every call besides.
    #[must_use]
    pub fn opens_t2(&self) -> bool {
        self.acr == OWNER_VERIFIED_ACR && self.host_profile_tier.eq_ignore_ascii_case("t2")
    }
}

/// A child grant: its row, and its step-up terms.
#[derive(Debug, Clone)]
pub struct ChildGrant {
    /// The row, as every grant's
    pub grant: OAuthClientGrant,
    /// What the step-up recorded
    pub terms: StepUpTerms,
}

/// The step-up a receipt or introspection states (owl §4.2 `step_up_result`):
/// when and how the athlete stepped up. `device` is always null: no device
/// key signs a Dravr approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepUpResult {
    /// When the athlete signed in, RFC 3339
    pub satisfied_at: String,
    /// The assurance recorded
    pub acr: String,
    /// The approving device; none
    pub device: Option<String>,
}

impl StepUpResult {
    /// The result `terms` record.
    #[must_use]
    pub fn of(terms: &StepUpTerms) -> Self {
        Self {
            satisfied_at: terms.auth_time.to_rfc3339(),
            acr: terms.acr.clone(),
            device: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(acr: &str, tier: &str) -> StepUpTerms {
        StepUpTerms {
            parent_grant_id: "parent".to_owned(),
            auth_time: Timestamp::now(),
            acr: acr.to_owned(),
            host_profile_basis: HostProfileBasis::DeclaredCimd,
            host_profile_tier: tier.to_owned(),
        }
    }

    #[test]
    fn only_owner_verified_and_a_t2_host_open_t2() {
        assert!(terms(OWNER_VERIFIED_ACR, "T2").opens_t2(), "bug: closed");
        assert!(terms(OWNER_VERIFIED_ACR, "t2").opens_t2(), "bug: case");
        assert!(
            !terms("password", "T2").opens_t2(),
            "bug: any acr opened T2"
        );
        assert!(
            !terms(OWNER_VERIFIED_ACR, "T1").opens_t2(),
            "bug: a T1 host opened T2"
        );
    }

    #[test]
    fn a_stored_basis_reads_back_and_an_unknown_one_opens_nothing() {
        for basis in [
            HostProfileBasis::DeclaredCimd,
            HostProfileBasis::DeclaredRequest,
        ] {
            assert_eq!(
                HostProfileBasis::parse(basis.as_str()),
                Some(basis),
                "bug: round trip"
            );
        }
        assert_eq!(
            HostProfileBasis::parse("assessed"),
            None,
            "bug: an unbuilt basis read"
        );
    }

    #[test]
    fn a_step_up_result_names_no_device() {
        let result = StepUpResult::of(&terms(OWNER_VERIFIED_ACR, "T2"));
        assert_eq!(result.acr, OWNER_VERIFIED_ACR, "bug: acr");
        assert_eq!(result.device, None, "bug: a device was claimed");
    }
}
