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
//! athletes an admin onboards.
//!
//! [`require_notice_accepted`] is the one precondition: the session credential
//! login and every path that begins a WHOOP OAuth flow call it before anything
//! leaves the server. [`outstanding_notice`] is the read the connect cards and
//! health sync share, so what a surface shows, what a connect refuses and what
//! sync keeps cannot disagree.
//!
//! A notice that is a consent to AI use (WHOOP's, Garmin's) can be withdrawn as
//! simply as it was given ([`withdraw_consent`], [`grant_consent`]); while it is
//! not given, [`under_ai_consent`] keeps every model read on the athlete's
//! behalf away from that provider's data (carnet#726), and
//! [`under_subject_ai_consent`] keeps a read of the athlete made for their
//! human coach under the athlete's choice, never the coach's (carnet#1419).

use std::collections::BTreeSet;
use std::future::Future;

use dravr_barrage::Tier;
use pierre_core::constants::oauth_providers::{
    self, ai_consent_backends, offered_at_connect, provider_notice, AtConnect, NoticeAudience,
    NoticeKind, GARMIN_WORKOUT_TRANSFER,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use serde_json::json;
use tracing::{debug, info, warn};
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
    if provider_notice(backend).is_some_and(|notice| notice.at_connect == AtConnect::Offered) {
        return Ok(());
    }
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
    accept_notice(repos, tenant_id, user_id, backend, current).await
}

/// Record the account's acceptance of `backend`'s notice `current`, the one
/// write behind a connect's acceptance and a settings toggle.
///
/// Accepting WHOOP's notice also resets the account's WHOOP sync cursors, so
/// the history health sync withheld while it was owed is read again.
async fn accept_notice(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
    current: &str,
) -> AppResult<()> {
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

/// Whether `key`'s notice is a withdrawable consent of `kind`.
fn is_consent(key: &str, kind: NoticeKind) -> bool {
    provider_notice(key).is_some_and(|notice| notice.kind == kind && notice.withdrawable())
}

/// What a refusal calls a consent of `kind`.
const fn consent_noun(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::TransferConsent => "consent to send it data",
        NoticeKind::AiConsent | NoticeKind::TermsExposure => "consent to AI use",
    }
}

/// Record the consents the athlete ticked at a connect to `backend`.
///
/// A connect offers its [`AtConnect::Offered`] notices as boxes the athlete
/// may leave unticked, and goes through either way (JF 2026-11-23,
/// carnet#1157). `consents` names the ticked ones by key; each is recorded in
/// its current version, like a settings toggle. An empty list records
/// nothing and withdraws nothing: an unticked box at a reconnect leaves an
/// earlier answer standing.
///
/// # Errors
/// Returns [`AppError::invalid_input`] naming a key the connect to `backend`
/// does not offer, before anything is recorded; or the store's error.
pub async fn record_connect_consents(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
    consents: &[String],
) -> AppResult<()> {
    let offered: Vec<&'static str> = offered_at_connect(backend).collect();
    if let Some(unknown) = consents.iter().find(|key| !offered.contains(&key.as_str())) {
        return Err(AppError::invalid_input(format!(
            "A connect to {backend} offers no consent named {unknown}"
        )));
    }
    for key in consents {
        let Some(current) = notice_in_force(repos, tenant_id, user_id, key).await else {
            continue;
        };
        accept_notice(repos, tenant_id, user_id, key, current).await?;
        info!(%user_id, backend, consent = %key, "consent given at connect");
    }
    Ok(())
}

/// Whether the athlete lets Dravr send workouts to their Garmin Connect
/// calendar ([`GARMIN_WORKOUT_TRANSFER`], carnet#1390).
///
/// An acceptance that cannot be read counts as not given: Dravr then sends
/// nothing, never something the athlete did not allow.
pub async fn garmin_transfer_allowed(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
) -> bool {
    consent_state(
        repos,
        tenant_id,
        user_id,
        GARMIN_WORKOUT_TRANSFER,
        NoticeKind::TransferConsent,
    )
    .await
        == Some(true)
}

/// The providers whose data this account has not consented to hand to AI,
/// and the consent classes
/// ([`CONSENT_CLASSES`](pierre_core::constants::oauth_providers::CONSENT_CLASSES))
/// it has not given.
///
/// Every key whose notice is a consent to AI use
/// ([`NoticeKind::AiConsent`](pierre_core::constants::oauth_providers::NoticeKind::AiConsent)),
/// in force for this account, that the account has not accepted in its current
/// version or has withdrawn (carnet#726).
///
/// An acceptance that cannot be read counts as withheld: a model then reads
/// less than it may, never more.
pub async fn ai_consent_withheld(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
) -> BTreeSet<String> {
    let mut withheld = BTreeSet::new();
    for backend in ai_consent_backends() {
        match outstanding_notice(repos, tenant_id, user_id, backend).await {
            Ok(None) => {}
            Ok(Some(_)) => {
                withheld.insert(backend.to_owned());
            }
            Err(e) => {
                warn!(%user_id, backend, error = %e, "AI consent unreadable; withheld");
                withheld.insert(backend.to_owned());
            }
        }
    }
    withheld
}

/// Run `fut` under the athlete's AI consents.
///
/// `fut` is reads made for a model on this athlete's behalf; under
/// [`ai_consent_withheld`], no model reads a provider's data the athlete has
/// not consented to hand to AI.
pub async fn under_ai_consent<F: Future>(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    fut: F,
) -> F::Output {
    let withheld = ai_consent_withheld(repos, tenant_id, user_id).await;
    ai_scope::with_ai_consent(withheld, fut).await
}

/// Run `fut`, a read of `subject`'s data made for someone else, under
/// `subject`'s own AI consents in `tenant_id`, the tenant their data lives
/// under.
///
/// For a human coach's agent reading the athlete they coach (carnet#1419):
/// the coach's consents say nothing about the athlete's providers, so a
/// provider the athlete withheld from AI never reaches the coach's model, and
/// one the athlete consented to is not hidden because the coach withheld it.
/// The call's output boundary applies the same consents
/// ([`with_subject_consent`](ai_scope::with_subject_consent)). An acceptance
/// that cannot be read counts as withheld ([`ai_consent_withheld`]).
pub async fn under_subject_ai_consent<F: Future>(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    subject: Uuid,
    fut: F,
) -> F::Output {
    let withheld = ai_consent_withheld(repos, tenant_id, subject).await;
    ai_scope::with_subject_consent(withheld, fut).await
}

/// Whether an outside client (MCP, A2A, an API key) may read `user_id`'s
/// `tier` records.
///
/// By the one tier rule ([`tier_readable`](ai_scope::tier_readable)) under
/// their consents: a T3 record only while they give `sensitive_t3`, a T2
/// record never.
///
/// For a route outside the tool executor serving such records to an outside
/// credential, such as the athlete's memory (JF 2026-11-20, carnet#1544).
pub async fn outside_client_reads(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    tier: Tier,
) -> bool {
    let withheld = ai_consent_withheld(repos, tenant_id, user_id).await;
    ai_scope::serve_over(
        Transport::ApiKey,
        ai_scope::with_ai_consent(withheld, async { ai_scope::tier_readable(tier) }),
    )
    .await
}

/// Withdraw this account's `kind` consent recorded under `backend`.
///
/// As simply as it was given (carnet#726). From then on no model reads that
/// provider's data (a consent to AI use), or Dravr sends it nothing (a
/// transfer consent); the athlete still sees what is stored and the
/// connection stays live. Accepting the notice again — from the same
/// setting, or on the next connect — gives it back. `brand` is the provider
/// as a refusal names it.
///
/// Returns whether a consent was standing; withdrawing one already withdrawn
/// is not an error.
///
/// # Errors
/// Returns [`AppError::invalid_input`] when `backend` carries no withdrawable
/// consent of `kind` (a terms-of-use exposure notice has nothing to
/// withdraw); or the store's error.
pub async fn withdraw_consent(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    backend: &str,
    kind: NoticeKind,
    brand: &str,
) -> AppResult<bool> {
    if !is_consent(backend, kind) {
        return Err(AppError::invalid_input(format!(
            "{brand} has no {} to withdraw",
            consent_noun(kind)
        )));
    }
    let withdrawn = repos
        .users
        .withdraw_provider_terms(user_id, backend)
        .await?;
    info!(
        %user_id,
        backend,
        withdrawn,
        ?kind,
        "consent withdrawn"
    );
    Ok(withdrawn)
}

/// Give this account's `kind` consent recorded under `backend`.
///
/// The current version of its notice, from a settings toggle rather than a
/// connect: the same acceptance a connect records ([`accept_notice`]),
/// including WHOOP's sync-cursor reset, so the two paths cannot disagree.
/// Giving one already given records it again, which is not an error.
///
/// # Errors
/// Returns [`AppError::invalid_input`] when `backend` carries no withdrawable
/// consent of `kind`; or the store's error.
pub async fn grant_consent(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
    kind: NoticeKind,
    brand: &str,
) -> AppResult<()> {
    if !is_consent(backend, kind) {
        return Err(AppError::invalid_input(format!(
            "{brand} has no {} to give",
            consent_noun(kind)
        )));
    }
    let Some(current) = notice_in_force(repos, tenant_id, user_id, backend).await else {
        return Ok(());
    };
    accept_notice(repos, tenant_id, user_id, backend, current).await
}

/// This account's answer to the `kind` consent recorded under `backend`.
///
/// `Some(true)` while it is given in the current version, `Some(false)` while
/// it is not (never given, outdated, or withdrawn), `None` when `backend`
/// asks no such consent of this account. An unreadable acceptance reads as
/// not given.
pub async fn consent_state(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
    kind: NoticeKind,
) -> Option<bool> {
    if !is_consent(backend, kind) {
        return None;
    }
    notice_in_force(repos, tenant_id, user_id, backend).await?;
    Some(matches!(
        outstanding_notice(repos, tenant_id, user_id, backend).await,
        Ok(None)
    ))
}
