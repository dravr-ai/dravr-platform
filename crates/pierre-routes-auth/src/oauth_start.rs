// ABOUTME: The precondition every OAuth start passes: the provider's required notice, and the offered consents the athlete ticked
// ABOUTME: Shared by the launch route, the mobile init and the hosted picker's init, so none mints an authorization URL without it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What an OAuth start carries beyond the provider, and the precondition
//! every route that mints an authorization URL applies to it.

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_services::provider_notice::{record_connect_consents, require_notice_accepted};
use serde::Deserialize;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// What a client carries on an OAuth start request.
///
/// `tos_consent` is `true` once the athlete ticked the provider's notice
/// (WHOOP's owner authorization) on this attempt. The launch routes are
/// browser navigations, so it rides the query string where the credential
/// login sends it in the JSON body.
///
/// `consents` names, comma-separated, the boxes the athlete ticked among the
/// consents the connect offers without requiring them (Garmin's AI use and
/// workout transfer, carnet#1157 and carnet#1390); see
/// [`record_connect_consents`].
///
/// `sign_in_blocked` is `true` when the athlete's sign-in for the provider
/// could not complete (a provider challenge, or the sign-in service down) and
/// the client fell back to the OAuth: the start then authorizes even a dead
/// grant, instead of turning it back into the sign-in that just failed
/// (carnet#1667, amending carnet#1494).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct OAuthStartQuery {
    /// The athlete accepted the provider's notice on this attempt.
    #[serde(default)]
    pub tos_consent: bool,
    /// The offered consents the athlete ticked, comma-separated.
    #[serde(default)]
    pub consents: Option<String>,
    /// The athlete's sign-in could not complete, so this start is its fallback.
    #[serde(default)]
    pub sign_in_blocked: bool,
}

/// The consents a connect's query names, comma-separated, without blanks.
pub(crate) fn query_consents(consents: Option<&str>) -> Vec<String> {
    consents
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Refuse to begin `provider`'s OAuth flow until the account has accepted the
/// notice in force for it, recording the acceptance this start carries and
/// the offered consents the athlete ticked ([`record_connect_consents`]).
///
/// The OAuth-start face of [`require_notice_accepted`], shared by every route
/// that mints an authorization URL — the launch route, the mobile init and
/// the hosted connect picker's init — so none can hand out a URL the
/// precondition has not passed. The refusal names the provider by its
/// registered display name.
///
/// # Errors
/// Returns the precondition's refusal, or the store's error.
pub(crate) async fn require_oauth_start_notice(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    tos_consent: bool,
    consents: &[String],
) -> Result<(), AppError> {
    let brand = resources
        .provider_registry
        .get_descriptor(provider)
        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned());
    require_notice_accepted(
        &resources.repos,
        tenant_id.as_uuid(),
        user_id,
        provider,
        &brand,
        tos_consent,
    )
    .await?;
    record_connect_consents(
        &resources.repos,
        tenant_id.as_uuid(),
        user_id,
        provider,
        consents,
    )
    .await
}
