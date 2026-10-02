// ABOUTME: Whose data a provider push event is about: the token's owner, or the member a coach's confirmed link names
// ABOUTME: A coach platform names the athlete while the token is the coach's, so the lookup falls back to delegated connections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Webhook owner resolution
//!
//! A provider's push event names a provider-side id and nothing else: no
//! platform user, no tenant. Two shapes of provider exist:
//!
//! - the athlete connected the provider themselves, so the id was captured
//!   into `provider_user_id` when their token was stored and resolves to that
//!   one `(user, tenant)`;
//! - a coach platform names the **athlete** while the token belongs to the
//!   **coach**. The athlete holds no token here; they are a group member the
//!   coach linked and who confirmed the link, and the event resolves through
//!   that confirmed `delegated_connections` row to the member, to be read
//!   with the coach's connection.
//!
//! The token owner is tried first and wins. A link that is only proposed, or
//! that ended, resolves to nobody, and so does one the group relation no
//! longer backs (the member left, the group is archived, the coach was
//! replaced): the same relation the member's read path follows. So does one
//! whose roster athlete is not the member by email
//! ([`link_binding`]): the member's read refuses that link, so an event
//! through it would announce data the member cannot read. An event is never
//! broadcast: an id that names no one, or more than one member, is dropped.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::delegated_connections::unbound_link_reason;
use crate::trainingpeaks_accounts::link_binding;

/// The coach connection a delegated member's data is read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoachConnection {
    /// The confirmed `delegated_connections` row the event resolved through.
    pub link_id: Uuid,
    /// The coach whose stored provider session serves the read.
    pub coach_user_id: Uuid,
    /// The tenant holding that session.
    pub coach_tenant_id: TenantId,
}

/// The platform user a provider push event is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookOwner {
    /// The user whose data the event announces.
    pub user_id: Uuid,
    /// The tenant that data lives in: the token's tenant for a direct owner,
    /// the tenant the member confirmed in for a delegated one.
    pub tenant_id: String,
    /// `None` when the user owns the provider token; the coach connection to
    /// read with when the user is a member linked through their coach.
    pub read_through: Option<CoachConnection>,
}

/// Resolve the user a provider-side id names, for a push event of `provider`.
///
/// Returns `None` when the id names no token owner and no single confirmed,
/// still-backed delegated link whose athlete is the member by email.
///
/// Neither lookup filters by tenant: the event carries none, so the
/// provider-side id is the key and the tenant is read from the matched row.
///
/// # Errors
/// Returns a database error when either lookup fails, or when the member or
/// their email verification cannot be read to check the link's binding.
pub async fn resolve_webhook_owner(
    repos: &RepositoryRegistry,
    provider: &str,
    provider_user_id: &str,
) -> AppResult<Option<WebhookOwner>> {
    if let Some((user_id, tenant_id)) = repos
        .oauth_tokens
        .find_user_by_provider_user_id(provider, provider_user_id)
        .await?
    {
        return Ok(Some(WebhookOwner {
            user_id,
            tenant_id,
            read_through: None,
        }));
    }

    let links = repos
        .delegated_connections
        .list_confirmed_for_athlete(provider, provider_user_id)
        .await?;
    let [link] = links.as_slice() else {
        if !links.is_empty() {
            warn!(
                provider = %provider,
                provider_user_id = %provider_user_id,
                links = links.len(),
                "webhook athlete id is confirmed under more than one coach; not guessing a member"
            );
        }
        return Ok(None);
    };
    // The schema admits no confirmed row without the member's tenant; a row
    // that carries none has nowhere to land the event, so it names nobody.
    let Some(member_tenant) = link.member_tenant_id else {
        warn!(link_id = %link.id, "confirmed delegated link carries no member tenant");
        return Ok(None);
    };
    // The check the member's read path makes before it reads through the
    // link, and the one their provider card names its refusal with.
    if let Some(reason) = unbound_link_reason(link_binding(repos, link).await?) {
        info!(
            link_id = %link.id,
            reason,
            "webhook athlete's confirmed link does not bind its member by email; the event \
             names nobody"
        );
        return Ok(None);
    }

    Ok(Some(WebhookOwner {
        user_id: link.member_user_id,
        tenant_id: member_tenant.to_string(),
        read_through: Some(CoachConnection {
            link_id: link.id,
            coach_user_id: link.coach_user_id,
            coach_tenant_id: link.coach_tenant_id,
        }),
    }))
}
