// ABOUTME: A token's live step-up child as the token response and introspection state it: its details and step_up_result
// ABOUTME: The token names the parent; this says what the child beside it reads, how the athlete stepped up and the host basis

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The step-up child a token reads beside its parent (carnet#1544 slice 4b).
//!
//! A token always names the parent grant (`client_grant_id`); its ceiling
//! includes the parent's live child. The token response and introspection
//! state that child as a [`StepUpEcho`]: its own id and details, when it
//! ends, owl's `step_up_result` (`satisfied_at`, `acr`, `device: null`) and
//! the Host Profile basis it was approved on.

use pierre_core::errors::AppResult;
use pierre_core::models::{ChildGrant, HostProfileBasis, StandingChild, StepUpResult, StepUpTerms};
use serde::Serialize;

use super::authorization_details::ResolvedDetails;

/// The Host Profile a child was approved on (owl §6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostProfileEcho {
    /// How the host satisfied it
    pub basis: HostProfileBasis,
    /// The tier it declared
    pub tier: String,
}

/// A live step-up child, as a token's response and introspection state it.
#[derive(Debug, Clone, Serialize)]
pub struct StepUpEcho {
    /// The child's own grant id
    pub grant_id: String,
    /// What the child was approved for (RFC 9396)
    pub authorization_details: ResolvedDetails,
    /// When it stops reading, RFC 3339
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// How the athlete stepped up (owl §4.2)
    pub step_up_result: StepUpResult,
    /// The Host Profile it stands on
    pub host_profile: HostProfileEcho,
}

impl StepUpEcho {
    /// The echo of the child a grant's standing carries.
    ///
    /// # Errors
    /// An internal error when its stored details do not read.
    pub fn of_standing(child: &StandingChild) -> AppResult<Self> {
        Self::of(
            &child.id,
            &child.authorization_details,
            child.expires_at.map(|at| at.to_rfc3339()),
            &child.terms,
        )
    }

    /// The echo of a child row.
    ///
    /// # Errors
    /// An internal error when it stored no details, or they do not read.
    pub fn of_child(child: &ChildGrant) -> AppResult<Self> {
        let document = child.grant.authorization_details.as_deref().unwrap_or("");
        Self::of(
            &child.grant.id,
            document,
            child.grant.expires_at.map(|at| at.to_rfc3339()),
            &child.terms,
        )
    }

    fn of(
        grant_id: &str,
        document: &str,
        expires_at: Option<String>,
        terms: &StepUpTerms,
    ) -> AppResult<Self> {
        Ok(Self {
            grant_id: grant_id.to_owned(),
            authorization_details: ResolvedDetails::from_document(document)?,
            expires_at,
            step_up_result: StepUpResult::of(terms),
            host_profile: HostProfileEcho {
                basis: terms.host_profile_basis,
                tier: terms.host_profile_tier.clone(),
            },
        })
    }
}
