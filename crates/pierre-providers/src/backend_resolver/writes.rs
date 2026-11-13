// ABOUTME: Writes of a mirror pair: which backend carries a write, and the grant a write still owes
// ABOUTME: Split out of backend_resolver (carnet#1494): Garmin's grant writes what its session cannot

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Writes of a mirror pair: the backend a write goes to ([`write_backend`]),
//! and the grant a write still owes beside the athlete's session
//! ([`grant_owed`], JF's directive 8).

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use super::{mirror_backend_for, read_plan, user_facing_name, Half, PairHalves, OAUTH_SERVED};
use crate::registry::ProviderRegistry;
use crate::spi::ProviderCapabilities;

/// The backend a write carrying `capability` (a calendar write, a planned
/// workout read) goes to when the athlete names `requested`.
///
/// Chosen within the pair by capability, never by routing alone: the first
/// backend of the athlete's [`read_plan`] that carries it, so a workout sent
/// to Garmin keeps going to the Garmin grant while the session serves the
/// reads. When neither does, the plan's primary, whose refusal names the
/// capability. A mirror named directly and a provider with no mirror are
/// returned unchanged, as [`resolve_backend`] returns them.
///
/// # Errors
///
/// The [`grant_owed_refusal`] when only the pair's grant carries
/// `capability` and the grant is [`grant_owed`] beside the athlete's session
/// (JF's directive 8): the write is never sent to a session that cannot make
/// it, and the refusal names the authorization to connect again. An athlete
/// holding neither half gets the plan's primary, whose refusal says the
/// provider is not connected.
pub async fn write_backend(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    requested: &str,
    capability: ProviderCapabilities,
) -> AppResult<String> {
    if mirror_backend_for(requested).is_none() {
        return Ok(requested.to_owned());
    }
    let plan = read_plan(repos, user_id, tenant_id, requested).await;
    if let Some(backend) = plan.carrying(registry, capability) {
        return Ok(backend.to_owned());
    }
    match grant_owed(repos, registry, user_id, tenant_id, requested).await {
        Some(grant)
            if registry
                .get_capabilities(grant)
                .is_some_and(|carried| carried.contains(capability)) =>
        {
            Err(grant_owed_refusal(registry, grant))
        }
        _ => Ok(plan.primary),
    }
}

/// What only a pair's own OAuth grant writes and its mirror cannot: the
/// training calendar (Garmin's workout and calendar push).
const GRANT_ONLY_WRITES: ProviderCapabilities = ProviderCapabilities::CALENDAR_WRITE;

/// `details.reason` of [`grant_owed_refusal`].
pub const GRANT_OWED_REASON: &str = "grant_owed";

/// The OAuth grant of `provider`'s pair when it writes what the pair's
/// mirror cannot, else `None`. Accepts either half of a pair.
///
/// Garmin's: its session reads everything the athlete records, and only its
/// grant pushes workouts and writes the calendar. So Garmin connects and
/// reconnects by its hosted sign-in first and its OAuth grant after it, and
/// the grant is owed while it is not live (JF's directive 8). Strava's grant
/// writes nothing its session cannot, so it is never owed.
#[must_use]
pub fn grant_writes_beside_mirror(
    registry: &ProviderRegistry,
    provider: &str,
) -> Option<&'static str> {
    let user_facing = user_facing_name(provider);
    let mirror = mirror_backend_for(user_facing)?;
    let grant = OAUTH_SERVED
        .iter()
        .copied()
        .find(|served| *served == user_facing)?;
    let writes = |backend: &str| {
        registry
            .get_capabilities(backend)
            .map_or_else(ProviderCapabilities::empty, |carried| {
                carried.intersection(GRANT_ONLY_WRITES)
            })
    };
    (!writes(grant).difference(writes(mirror)).is_empty()).then_some(grant)
}

/// The grant `provider`'s athlete still owes for its writes in this tenant.
///
/// A [`grant_writes_beside_mirror`] grant whose half is not live (absent,
/// flagged or revoked) while the athlete is signed in to the pair's session
/// (held and not revoked). `None` when the provider has no such grant, the
/// athlete's grant is live, or no session is held: an athlete with no
/// session connects by the hosted sign-in first, which then goes on to the
/// grant (JF's directive 8), so nothing points them at the grant alone. A
/// store that cannot be read reads the grant as live: nothing is asked on a
/// guess.
pub async fn grant_owed(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<&'static str> {
    let grant = grant_writes_beside_mirror(registry, provider)?;
    let mirror = mirror_backend_for(grant)?;
    let halves = PairHalves::read(repos, user_id, tenant_id, grant, mirror).await;
    (halves.session.signed_in() && halves.grant != Half::Live).then_some(grant)
}

/// Whether `backend` is a [`grant_writes_beside_mirror`] grant standing
/// beside a session that serves its provider: held, and its connection
/// active, in this tenant.
///
/// Such a grant's death, or its withdrawal, leaves the reads going and the
/// writes only it makes owed, so its athlete is told that much and given the
/// grant's own authorization (JF's directive 8). A flagged session does not
/// serve, though it still routes the reads to its sign-in: "your activities
/// are still syncing" would then be false. `false` for a mirror, for a grant
/// that writes nothing its mirror cannot (Strava's), and for a grant with no
/// serving session beside it, whose notice is the ordinary reconnect. A store
/// that cannot be read reads every held row as live, as [`read_plan`] reads
/// it.
pub async fn grant_writes_beside_serving_session(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    let Some(grant) = grant_writes_beside_mirror(registry, backend) else {
        return false;
    };
    let Some(mirror) = mirror_backend_for(grant) else {
        return false;
    };
    grant == backend
        && PairHalves::read(repos, user_id, tenant_id, grant, mirror)
            .await
            .session
            == Half::Live
}

/// The refusal of a write only `grant`, the pair's own authorization, makes
/// while the athlete's grant is not live.
///
/// `NoProviderConnected`, which reaches the model verbatim, with
/// `details.action` `connect_provider`, `details.reason`
/// [`GRANT_OWED_REASON`] and `details.provider` the grant: the caller that
/// can mint the grant's authorization link adds it.
#[must_use]
pub fn grant_owed_refusal(registry: &ProviderRegistry, grant: &str) -> AppError {
    let display = registry.get_display_name(grant).unwrap_or(grant);
    let mut refusal = AppError::new(
        ErrorCode::NoProviderConnected,
        format!(
            "{display} takes workouts and calendar writes only through its own authorization \
             ({grant}), which the athlete has not connected or must connect again. Their \
             {display} sign-in keeps the reads coming; this write needs {display} connected \
             again through its authorization."
        ),
    );
    refusal.details = Some(Box::new(serde_json::json!({
        "action": "connect_provider",
        "reason": GRANT_OWED_REASON,
        "provider": grant,
    })));
    refusal
}
