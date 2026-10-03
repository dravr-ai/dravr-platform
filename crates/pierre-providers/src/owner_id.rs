// ABOUTME: Reads the provider-side owner id behind an access token, for providers whose token response leaves it out
// ABOUTME: One lookup for the OAuth callback and the refresh path, driven by the provider descriptor (WHOOP, Garmin)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//!
//! A provider's push events name the athlete by the provider's own user id,
//! so a stored token without it can never be matched to one. Strava returns
//! the id inline in its token response; WHOOP and Garmin do not, and serve it
//! from their API instead (WHOOP's `user/profile/basic`, Garmin's `user/id`).
//! A descriptor that answers
//! [`ProviderDescriptor::owner_id_from_api`](crate::spi::ProviderDescriptor::owner_id_from_api)
//! has it read here, with the fresh access token, through the provider's own
//! `get_athlete`: the same request path, circuit breaker and
//! `PIERRE_<PROVIDER>_API_BASE_URL` seam as every other call to it.

use crate::core::{CredentialKind, OAuth2Credentials};
use crate::errors::AppResult;
use crate::registry::ProviderRegistry;
use crate::spi::ProviderDescriptor;

/// Read the owner id behind `access_token` when `provider`'s token response
/// carries none.
///
/// `Ok(None)` when the provider declares no such lookup (or is not
/// registered): its token response is where the id comes from, if anywhere.
/// The access token is the only credential set: a bearer read never
/// refreshes, so no client id, secret, refresh token or expiry is needed, and
/// the provider instance is dropped afterwards.
///
/// # Errors
///
/// Returns the registry's error when the provider cannot be built and the
/// provider's own error when the read fails (a rejected token, a transport
/// failure).
pub async fn owner_id_for_access_token(
    registry: &ProviderRegistry,
    provider: &str,
    access_token: &str,
) -> AppResult<Option<String>> {
    let declared = registry
        .get_descriptor(provider)
        .is_some_and(ProviderDescriptor::owner_id_from_api);
    if !declared {
        return Ok(None);
    }
    let client = registry.create_provider(provider)?;
    client
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(access_token.to_owned()),
            refresh_token: None,
            expires_at: None,
            scopes: Vec::new(),
            kind: CredentialKind::OAuthBearer,
        })
        .await?;
    Ok(Some(client.get_athlete().await?.id))
}
