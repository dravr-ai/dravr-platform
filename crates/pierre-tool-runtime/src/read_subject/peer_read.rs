// ABOUTME: A fellow group member's read of a member's data under their peer consent, on the member's own record before it is made
// ABOUTME: Recorded under a tenant the member belongs to, with the transport and support admin a coach read records (carnet#1543)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Peer reads on the member's record
//!
//! A member who shares their training with their peers is read by a fellow
//! member through `get_group_member_activities` and through the group views
//! built from every member's training. Each read is written to the member's
//! `peer_data_reads` record before anything of theirs is read, as a coach's
//! read is written to `coach_data_reads` ([`super::record_coach_read`]): a
//! record that cannot be written refuses the read.
//!
//! The row is kept under a tenant the member belongs to, so their feed, which
//! reads each of their tenants filtered on it, finds it: the tenant of the
//! membership the read went through when the member belongs to it (a member
//! who joined from their own tenant), else the first tenant they belong to (a
//! membership held under the group's tenant). A member who belongs to no
//! tenant, and so has no feed, keeps it under the membership's tenant.

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::GroupMember;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use pierre_database::repositories::NewPeerDataRead;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::context::SUPPORT_SESSION;

/// How the read running now is served: the transport the call declared, and
/// the admin whose support session makes it, if any. What a coach's and a
/// peer's read record alike.
pub(super) fn served_now() -> (Option<Transport>, Option<Uuid>) {
    let impersonator = SUPPORT_SESSION
        .try_with(|support| support.as_ref().map(|grant| grant.impersonator_id))
        .ok()
        .flatten();
    (ai_scope::declared_transport(), impersonator)
}

/// The tenant `membership`'s member keeps the record of a read under.
async fn record_tenant(
    repos: &RepositoryRegistry,
    membership: &GroupMember,
) -> AppResult<TenantId> {
    let tenants = repos.tenants.list_for_user(membership.user_id).await?;
    if let Some(tenant) = tenants
        .iter()
        .find(|tenant| tenant.id.to_string() == membership.tenant_id)
        .or_else(|| tenants.first())
    {
        return Ok(tenant.id);
    }
    TenantId::parse_str(&membership.tenant_id).map_err(|e| {
        AppError::internal(format!(
            "membership of {} names an unreadable tenant {:?}: {e}",
            membership.user_id, membership.tenant_id
        ))
    })
}

/// The row recording `peer`'s read of `membership`'s member through `tool`
/// now.
async fn new_peer_read<'read>(
    repos: &RepositoryRegistry,
    membership: &GroupMember,
    peer: Uuid,
    tool: &'read str,
) -> AppResult<NewPeerDataRead<'read>> {
    let (transport, impersonator) = served_now();
    Ok(NewPeerDataRead {
        tenant: record_tenant(repos, membership).await?,
        member: membership.user_id,
        peer,
        group: membership.group_id,
        tool,
        read_at: Timestamp::now(),
        transport,
        impersonator,
    })
}

/// Put `peer`'s read of `membership`'s member through `tool` on the member's
/// record, before the read is made.
///
/// # Errors
///
/// Returns the repository error from the tenant lookup or the insert, or an
/// internal error for a membership naming an unreadable tenant: the read is
/// refused rather than left unrecorded.
pub async fn record_peer_read(
    repos: &RepositoryRegistry,
    membership: &GroupMember,
    peer: Uuid,
    tool: &str,
) -> AppResult<()> {
    let read = new_peer_read(repos, membership, peer, tool).await?;
    repos.peer_data_reads.record_peer_read(&read).await
}

/// [`record_peer_read`] at most once a UTC day per member, peer, group and
/// source, for the group views a member opens on every turn or page load.
///
/// `true` when the row was written, `false` when that day's row already was.
/// A read in a support session is recorded each time instead, as a coach's
/// is: folded into the peer's own row of the day, the member would never see
/// that support read them.
///
/// # Errors
///
/// As [`record_peer_read`].
pub async fn record_daily_peer_read(
    repos: &RepositoryRegistry,
    membership: &GroupMember,
    peer: Uuid,
    source: &str,
) -> AppResult<bool> {
    let read = new_peer_read(repos, membership, peer, source).await?;
    if read.impersonator.is_some() {
        repos.peer_data_reads.record_peer_read(&read).await?;
        return Ok(true);
    }
    repos.peer_data_reads.record_daily_peer_read(&read).await
}
