// ABOUTME: The notice version a user must have accepted before connecting a provider, or none
// ABOUTME: Combines the provider's notice and its audience with the provider_exposure_notice flag and the acceptance record

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which provider notice is in force for one account, and the precondition
//! every connect path applies.
//!
//! `TrainingPeaks` and COROS are read through the athlete's own signed-in
//! session, which their terms forbid for third parties; WHOOP's API terms let
//! Dravr keep and compute from WHOOP Data only under its owner's express
//! authorization. Each carries a notice ([`provider_notice`]) whose audience
//! says which accounts are asked for it: WHOOP's binds every account, while
//! the `TrainingPeaks` and COROS notices are asked of the accounts the
//! `provider_exposure_notice` feature flag arms — off by default, so a demo
//! account connects without them, and armed per tenant or per user for the
//! athletes an operator onboards.
//!
//! [`require_notice_accepted`] is the one precondition: the sciotte credential
//! login and every path that begins a WHOOP OAuth flow call it before anything
//! leaves the server. [`outstanding_notice`] is the read the connect cards and
//! health sync share, so what a surface shows, what a connect refuses and what
//! sync keeps cannot disagree.

use pierre_core::constants::oauth_providers::{self, provider_notice, NoticeAudience};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use serde_json::json;
use tracing::{debug, info};
use uuid::Uuid;

/// `details.action` on the precondition's refusal, which a caller matches to
/// tell a notice to accept from any other failure to connect.
pub const NOTICE_REFUSAL_ACTION: &str = "accept_provider_notice";

/// The notice version `user_id` must accept before connecting `backend`.
///
/// `None` when no notice applies: the backend carries none, or its notice is
/// asked only of flag-armed accounts ([`NoticeAudience::FlagArmedAccounts`])
/// and the `provider_exposure_notice` flag is off for this account. A notice
/// for [`NoticeAudience::EveryAccount`] is in force whatever the flag says.
///
/// A flag read that fails resolves to the flag's compile default (off), as
/// every feature flag does.
pub async fn notice_in_force(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
) -> Option<&'static str> {
    let notice = provider_notice(backend)?;
    let asked = match notice.audience {
        NoticeAudience::EveryAccount => true,
        NoticeAudience::FlagArmedAccounts => exposure_notice_armed(repos, tenant_id, user_id).await,
    };
    asked.then_some(notice.version)
}

/// Whether the `provider_exposure_notice` flag arms this account.
async fn exposure_notice_armed(repos: &RepositoryRegistry, tenant_id: Uuid, user_id: Uuid) -> bool {
    match repos
        .feature_flags
        .resolve_for_user(tenant_id, user_id)
        .await
    {
        Ok(flags) => flags
            .get(&FeatureKey::ProviderExposureNotice)
            .copied()
            .unwrap_or_else(|| FeatureKey::ProviderExposureNotice.default_enabled()),
        Err(e) => {
            debug!(%user_id, error = %e, "exposure notice flag unreadable; compile default applies");
            FeatureKey::ProviderExposureNotice.default_enabled()
        }
    }
}

/// The notice version `user_id` must still accept for `backend`.
///
/// The version in force for this account ([`notice_in_force`]) when the
/// account's recorded acceptance is not that version; `None` when no notice
/// applies or the current one is accepted.
///
/// # Errors
/// Returns an error when the acceptance record cannot be read.
pub async fn outstanding_notice(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
) -> AppResult<Option<&'static str>> {
    let Some(current) = notice_in_force(repos, tenant_id, user_id, backend).await else {
        return Ok(None);
    };
    let accepted = repos.users.provider_terms_version(user_id, backend).await?;
    Ok((accepted.as_deref() != Some(current)).then_some(current))
}

/// Whether a connect surface must show `backend`'s notice to this account.
///
/// [`outstanding_notice`], with an unreadable acceptance read as outstanding:
/// showing the notice twice costs a tick, skipping it costs the precondition,
/// which the connect re-checks with its own read.
pub async fn asks_for_notice(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
) -> bool {
    outstanding_notice(repos, tenant_id, user_id, backend)
        .await
        .map_or(true, |outstanding| outstanding.is_some())
}

/// Refuse to begin connecting `backend` until the account has accepted the
/// notice in force for it, recording the acceptance this attempt carries.
///
/// `accepted_now` is the acceptance the connect request itself carries — the
/// `tos_consent` the client sends once the athlete ticked the notice. A
/// backend with no notice, a flag-gated notice for an account the
/// `provider_exposure_notice` flag leaves off, and an account that already
/// accepted the current version all pass without it, including on a
/// reconnect after a disconnect. `brand` is
/// the provider as the refusal names it.
///
/// # Errors
/// Returns [`AppError::invalid_input`] naming the provider, with
/// `details.action` [`NOTICE_REFUSAL_ACTION`] and `details.provider`, when
/// the notice is outstanding and this attempt does not accept it; or the
/// store's error when the acceptance cannot be read or recorded.
///
/// Accepting WHOOP's notice also resets the account's WHOOP sync cursors in
/// this tenant, so the history health sync withheld while it was owed is
/// read again.
pub async fn require_notice_accepted(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
    brand: &str,
    accepted_now: bool,
) -> AppResult<()> {
    let Some(current) = outstanding_notice(repos, tenant_id, user_id, backend).await? else {
        return Ok(());
    };
    if !accepted_now {
        let mut refusal = AppError::invalid_input(format!(
            "Connecting {brand} requires accepting the account notice first"
        ));
        refusal.details = Some(Box::new(json!({
            "action": NOTICE_REFUSAL_ACTION,
            "provider": backend,
        })));
        return Err(refusal);
    }
    repos
        .users
        .record_provider_terms(user_id, backend, current)
        .await?;
    if backend == oauth_providers::WHOOP {
        // Health sync withheld every WHOOP record while this notice was owed,
        // and the WHOOP sync walks its pages newest-first behind a cursor that
        // kept advancing over them. Dropping the cursor makes the next sync —
        // the backfill the reconnect triggers — start from WHOOP's newest page
        // and walk the whole history again, now kept.
        let reset = repos
            .sync_cursors
            .reset_sync_cursors(
                &user_id.to_string(),
                &TenantId::from_uuid(tenant_id),
                oauth_providers::WHOOP,
            )
            .await?;
        info!(
            %user_id,
            %tenant_id,
            cursors_reset = reset,
            "WHOOP owner authorization accepted; the WHOOP sync re-reads its history"
        );
    }
    Ok(())
}
