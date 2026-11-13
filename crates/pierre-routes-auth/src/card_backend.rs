// ABOUTME: Which backend serves a connected provider card, judged by the read plan routing reads by
// ABOUTME: A session that is not revoked serves the card before its grant, and the serving backend's flag is its reconnect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The backend behind a connected card, by the athlete's read plan
//! (carnet#1302, carnet#1494), and whether it asks for a reconnect.

use std::collections::HashSet;

use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::{self, ReadPlan};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The backend serving `provider_name`'s card for `user_id`, and the
/// athlete's read plan for its provider in `tenant_id` (`None` outside a
/// tenant).
///
/// The row behind the card is the plan's primary when the athlete holds it
/// ([`backend_resolver::read_plan`]): a session that is not revoked before
/// its grant, so the card's reconnect is the session's. When the primary is
/// not held — a dead grant alone, whose plan is the mirror's sign-in — the
/// first connected backend of the pair stands for the card, so the dead
/// grant's flag still reads as its reconnect.
pub(crate) async fn card_backend(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    provider_name: &str,
    connected_providers: &HashSet<String>,
) -> (Option<String>, Option<ReadPlan>) {
    let plan = match tenant_id {
        Some(tid) => Some(
            backend_resolver::read_plan(
                &resources.repos,
                user_id,
                TenantId::from_uuid(tid),
                provider_name,
            )
            .await,
        ),
        None => None,
    };
    let connected = |backend: &str| connected_providers.contains(backend);
    let backend = plan
        .as_ref()
        .map(|plan| plan.primary.clone())
        .filter(|primary| connected(primary))
        .or_else(|| {
            backend_resolver::serving_backends(provider_name)
                .into_iter()
                .find(|backend| connected(backend))
        });
    (backend, plan)
}

/// Whether the athlete's own grant serves `provider_name`'s card first: the
/// plan's primary is the provider's OAuth backend.
pub(crate) fn grant_serves(provider_name: &str, plan: Option<&ReadPlan>) -> bool {
    let user_facing = backend_resolver::user_facing_name(provider_name);
    backend_resolver::mirror_backend_for(user_facing).is_some()
        && plan.is_some_and(|plan| plan.primary == user_facing)
}

/// Whether the card asks for a reconnect, given `connected_backend`, the
/// backend serving it, and `reauth_providers`, the backends flagged for
/// reconnect.
///
/// Only the serving backend counts: its session died (a dead scrape, a
/// refused OAuth refresh), so the UI prompts a reconnect instead of a healthy
/// "Connected". A grant left behind a serving session is not the card's
/// backend, so its flag asks nothing of the athlete: the card reads
/// "Reconnect" only when the backend serving it is flagged.
pub(crate) fn reauth_flag(
    connected_backend: Option<&str>,
    reauth_providers: &HashSet<String>,
) -> bool {
    connected_backend.is_some_and(|backend| reauth_providers.contains(backend))
}
