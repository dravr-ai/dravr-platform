// ABOUTME: An athlete's standing policy: "always allow this app this preset", read where the consent screen would be skipped
// ABOUTME: A renewal it allows is recorded as a grant citing it; unrelated to StandingGrant, which says whether a grant still stands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Standing policies (carnet#1544, design row 35, owl PDPP §6.4)
//!
//! An athlete who approves a preset grant may tick "Always allow this app
//! this preset": the authorization server then renews that grant without the
//! consent screen, as long as a request asks for exactly the terms the
//! athlete approved, under the consent text they read. Policies are
//! allow-only: nothing here ever refuses on the athlete's behalf.
//!
//! A policy is immutable, as a grant is: approving again with the box ticked
//! revokes the live policy and stores a new one. Revoking a grant leaves its
//! policy; removing the policy stops the renewals and leaves live grants as
//! they are.
//!
//! The name is unrelated to `StandingGrant` and `ClientGrantStanding` (the
//! resource servers' cache of whether a grant still stands).

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};

use super::StandingPolicyRef;

/// One row of `oauth_standing_policies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthStandingPolicy {
    /// The policy's id, cited by every grant it renews
    pub id: String,
    /// The tenant the athlete's grants live in
    pub tenant_id: String,
    /// The athlete
    pub user_id: String,
    /// The client the athlete always allows
    pub client_id: String,
    /// The preset it allows (`training_coach`, `sleep_coach`)
    pub preset: String,
    /// The OAuth scope approved with the box: a renewal must ask for exactly
    /// this scope
    pub scope: String,
    /// The resolved details' terms the athlete approved with the box ticked
    /// (the window not frozen): what a renewal must equal
    pub authorization_details: String,
    /// The review digest the box was ticked under
    pub consent_version: String,
    /// The language of the consent text it was ticked under
    pub consent_locale: String,
    /// When the athlete ticked the box
    pub created_at: Timestamp,
    /// When it was removed or replaced; `None` while it stands
    pub revoked_at: Option<Timestamp>,
}

impl OAuthStandingPolicy {
    /// The policy as a renewed grant's receipt cites it.
    #[must_use]
    pub fn reference(&self) -> StandingPolicyRef {
        StandingPolicyRef {
            id: self.id.clone(),
            consent_version: self.consent_version.clone(),
        }
    }
}
