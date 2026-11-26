// ABOUTME: The Host Profile T2 basis a step-up stands on: a client's metadata declaration, or its own per-request claim
// ABOUTME: One rule for who may be told to step up and what the approval records (owl PDPP §6.2 declared_cimd, declared_request)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Host Profile basis (carnet#1544 slice 4b, owl PDPP §6.1, §6.2)
//!
//! A step-up child opens T2 only for a host that declared it handles T2.
//! Two carriers declare it, and a client stands on exactly one:
//!
//! - **`declared_cimd`** (carrier A): a client identified by a metadata
//!   document declares a `mcp_host_profile` member in it, read when the
//!   document is fetched. A document declaring a lower tier, or none, never
//!   stands for a step-up: the declaration a client publishes is the
//!   stricter carrier, and its own requests cannot raise it.
//! - **`declared_request`** (carrier B; ChefFamille 2026-11-26): a client
//!   registered by DCR has no metadata document, so it declares on its own
//!   requests, in the MCP client extension [`HOST_PROFILE_EXTENSION`]. owl
//!   requires the claim be seen on the request that triggered consent:
//!   `/mcp` records the T2 claim of a call a step-up would cure
//!   ([`may_step_up`]), and the approval stands on it while it is at most
//!   [`HOST_PROFILE_CLAIM_MAX_AGE_SECS`] old ([`step_up_basis`]).
//!
//! Either way the step-up still needs a fresh sign-in on Dravr's page, T3
//! never reaches an outside client, and every later call is checked against
//! its own per-request claim ([`GrantCeiling::opens_t2`]).
//!
//! [`GrantCeiling::opens_t2`]: super::grant_ceiling::GrantCeiling::opens_t2
//! [`HOST_PROFILE_CLAIM_MAX_AGE_SECS`]: pierre_core::models::HOST_PROFILE_CLAIM_MAX_AGE_SECS

use dravr_barrage::Tier;
use dravr_tronc::dates::Timestamp;
use pierre_core::models::{HostProfileBasis, HostProfileClaim};
use pierre_database::backends::OAuth2ServerRepository;
use pierre_database::repositories::HostProfileClaimRepository;
use serde_json::Value;
use tracing::warn;

use super::cimd::{host_profile_tier, is_metadata_document_id};

/// The MCP client extension a host declares its Host Profile with, per
/// request (owl PDPP §6.2 carrier B).
pub const HOST_PROFILE_EXTENSION: &str = "app.dravr.vault/host-profile";

/// The Host Profile tier a request's client capabilities declare.
///
/// Read from the capabilities (its
/// `_meta["io.modelcontextprotocol/clientCapabilities"]`) at
/// `extensions["app.dravr.vault/host-profile"]`, `{"tier_max": "t2", …}`;
/// `None` when they declare none or a value this server does not read,
/// which opens nothing above T1.
#[must_use]
pub fn request_host_tier(capabilities: &Value) -> Option<Tier> {
    let declared = capabilities
        .get("extensions")?
        .get(HOST_PROFILE_EXTENSION)?
        .get("tier_max")?
        .as_str()?;
    match declared.to_ascii_uppercase().as_str() {
        "T0" => Some(Tier::T0),
        "T1" => Some(Tier::T1),
        "T2" => Some(Tier::T2),
        _ => None,
    }
}

/// The Host Profile T2 basis a step-up of `client_id` stands on at `now`.
///
/// With the tier it declares: its metadata document's declaration
/// (`declared`) when it has one, else, for a client with no metadata
/// document, the per-request `claim` seen under its parent grant. `None`
/// when neither stands at T2.
#[must_use]
pub fn step_up_basis(
    client_id: &str,
    declared: Option<&str>,
    claim: Option<&HostProfileClaim>,
    now: Timestamp,
) -> Option<(HostProfileBasis, String)> {
    if let Some(declared) = declared {
        return host_profile_tier(declared)
            .filter(|tier| tier.eq_ignore_ascii_case("T2"))
            .map(|_| (HostProfileBasis::DeclaredCimd, "T2".to_owned()));
    }
    // A metadata document client declaring nothing stays without a basis:
    // its published declaration is the carrier it is held to.
    if is_metadata_document_id(client_id) {
        return None;
    }
    claim
        .filter(|claim| claim.stands_at(now))
        .map(|_| (HostProfileBasis::DeclaredRequest, "T2".to_owned()))
}

/// A call a step-up would cure, as [`may_step_up`] reads it.
#[derive(Debug, Clone, Copy)]
pub struct CurableCall<'call> {
    /// The calling client
    pub client_id: &'call str,
    /// The calling user
    pub user_id: &'call str,
    /// The grant the call's token names: the step-up's parent
    pub client_grant_id: &'call str,
    /// The tier the call's own Host Profile claim declares
    pub request_tier: Option<Tier>,
}

/// Whether the client making `call` may be told to step up: the one rule
/// `/mcp`'s 403 challenge and the in-band `step_up_required` refusal follow.
///
/// A client whose metadata document declares Host Profile T2 may. A client
/// with no metadata document may when this call declares T2 on its own
/// request: the claim is then recorded under the call's parent grant, for
/// the approval to stand on ([`step_up_basis`]). Every other client may not,
/// and neither may one whose declaration or claim cannot be read or kept.
pub async fn may_step_up(
    oauth2: &dyn OAuth2ServerRepository,
    claims: &dyn HostProfileClaimRepository,
    call: CurableCall<'_>,
) -> bool {
    let declared = match oauth2.client_host_profile(call.client_id).await {
        Ok(declared) => declared,
        Err(e) => {
            warn!(client_id = call.client_id, error = %e, "A client's Host Profile could not be read");
            return false;
        }
    };
    if declared.is_some() || is_metadata_document_id(call.client_id) {
        return step_up_basis(call.client_id, declared.as_deref(), None, Timestamp::now())
            .is_some();
    }
    if call.request_tier != Some(Tier::T2) {
        return false;
    }
    let claim = HostProfileClaim {
        client_grant_id: call.client_grant_id.to_owned(),
        tier_max: "T2".to_owned(),
        seen_at: Timestamp::now(),
    };
    claims
        .record_host_profile_claim(call.user_id, &claim)
        .await
        .inspect_err(|e| warn!(client_id = call.client_id, error = %e, "A Host Profile claim could not be recorded"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Elapsed;
    use pierre_core::models::HOST_PROFILE_CLAIM_MAX_AGE_SECS;
    use serde_json::json;

    const DCR: &str = "dcr-client-id";
    const CIMD: &str = "https://host.example/oauth/client-metadata";

    fn claim(tier: &str, seen_at: Timestamp) -> HostProfileClaim {
        HostProfileClaim {
            client_grant_id: "parent".to_owned(),
            tier_max: tier.to_owned(),
            seen_at,
        }
    }

    #[test]
    fn a_dcr_client_stands_on_its_own_fresh_t2_claim_only() {
        let now = Timestamp::now();
        assert_eq!(
            step_up_basis(
                DCR,
                None,
                Some(&claim("T2", now - Elapsed::seconds(60))),
                now
            ),
            Some((HostProfileBasis::DeclaredRequest, "T2".to_owned())),
            "bug: a fresh claim did not stand"
        );
        assert_eq!(
            step_up_basis(DCR, None, None, now),
            None,
            "bug: no claim stood"
        );
        assert_eq!(
            step_up_basis(DCR, None, Some(&claim("T1", now)), now),
            None,
            "bug: a T1 claim stood"
        );
        let stale = now - Elapsed::seconds(HOST_PROFILE_CLAIM_MAX_AGE_SECS + 1);
        assert_eq!(
            step_up_basis(DCR, None, Some(&claim("T2", stale)), now),
            None,
            "bug: a stale claim stood"
        );
    }

    #[test]
    fn a_metadata_document_client_stands_on_its_declaration_alone() {
        let now = Timestamp::now();
        let fresh = claim("T2", now);
        assert_eq!(
            step_up_basis(CIMD, Some(r#"{"tier_max":"t2","version":1}"#), None, now),
            Some((HostProfileBasis::DeclaredCimd, "T2".to_owned())),
            "bug: a declared T2 host"
        );
        assert_eq!(
            step_up_basis(
                CIMD,
                Some(r#"{"tier_max":"T1","version":1}"#),
                Some(&fresh),
                now
            ),
            None,
            "bug: a request claim raised a T1 declaration"
        );
        assert_eq!(
            step_up_basis(CIMD, None, Some(&fresh), now),
            None,
            "bug: a metadata document declaring nothing stood on a request claim"
        );
    }

    #[test]
    fn a_requests_host_profile_tier_is_read_from_its_extension() {
        let declaring =
            |tier: &str| json!({"extensions": {HOST_PROFILE_EXTENSION: {"tier_max": tier}}});
        assert_eq!(request_host_tier(&declaring("t2")), Some(Tier::T2));
        assert_eq!(request_host_tier(&declaring("T1")), Some(Tier::T1));
        assert_eq!(request_host_tier(&declaring("T3")), None, "bug: T3 read");
        assert_eq!(request_host_tier(&json!({})), None);
    }
}
