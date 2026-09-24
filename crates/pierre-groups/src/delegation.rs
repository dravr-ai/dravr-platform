// ABOUTME: The one place a delegated connection ends: the link row, then the member's delegated connection and cache
// ABOUTME: Every lifecycle event (leave, removal, archive, detach, disconnect, own login) ends links through DelegationStore
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Ending delegated connections
//!
//! A confirmed delegated connection has two traces besides its row: the
//! member's `delegated` provider connection, which makes the provider card
//! read as connected and lets the read path elect the provider, and the
//! activities read through the coach's session, cached under the member. A
//! link that ends must take both with it, exactly as a disconnect takes a
//! member's own connection and cache, or the card keeps saying "connected"
//! for a read that no longer happens and the member keeps workouts they
//! withdrew consent to.
//!
//! Every way a link ends goes through [`DelegationStore`], so the release
//! cannot be forgotten at one of them: the member leaving or being removed,
//! the group archived, the coach detached, either side disconnecting the
//! provider, the member's own login taking over, either side ending it
//! directly, and the provider no longer listing the athlete on the coach's
//! roster. A link that was only ever proposed has no trace to release.
//!
//! The link row is ended first. Should a release step fail after it, the
//! member's read path still fails closed: it only follows a confirmed link.

use std::slice;
use std::sync::Arc;

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::{DelegatedConnection, DelegationEndReason, DelegationStatus, TenantId};
use pierre_database::repositories::{
    ActivityCacheRepository, CoachingGroupRepository, DelegatedConnectionRepository,
    ProviderConnectionRepository,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::user_facing_name;
use tracing::{info, warn};
use uuid::Uuid;

/// Ends delegated connections and releases what a confirmed one left behind.
pub struct DelegationStore {
    /// The `delegated_connections` rows.
    links: Arc<dyn DelegatedConnectionRepository>,
    /// Holds the member's `delegated` provider connection a confirmed link
    /// registered.
    connections: Arc<dyn ProviderConnectionRepository>,
    /// Holds the activities read through the coach's session, filed under the
    /// member.
    activity_cache: Arc<dyn ActivityCacheRepository>,
    /// The groups and memberships a link's standing is read from.
    groups: Arc<dyn CoachingGroupRepository>,
}

/// What became of a member's confirmed link the read path no longer follows
/// ([`DelegationStore::end_unbacked_for_member`]).
#[derive(Debug, Clone)]
pub enum UnbackedLink {
    /// The link ended, for the reason its relation broke.
    Ended(Box<DelegatedConnection>),
    /// The relation backs the link again: a lifecycle change landed between
    /// the refused read and this look. Nothing ended.
    Backed,
    /// No confirmed link names the member there: it ended earlier.
    Gone,
}

impl DelegationStore {
    /// A store over the registry's repositories.
    #[must_use]
    pub fn new(repos: &RepositoryRegistry) -> Self {
        Self {
            links: Arc::clone(&repos.delegated_connections),
            connections: Arc::clone(&repos.provider_connections),
            activity_cache: Arc::clone(&repos.activity_cache),
            groups: Arc::clone(&repos.groups),
        }
    }

    /// End every live link naming `member_user_id` in `group_id`: the member
    /// left the group, or a group admin removed them. Returns the ended rows.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the links cannot be ended or a
    /// confirmed one's delegated connection cannot be removed.
    pub async fn end_for_member_leaving(
        &self,
        group_id: Uuid,
        member_user_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
    ) -> AppResult<Vec<DelegatedConnection>> {
        let ended = self
            .links
            .end_for_group_member(group_id, member_user_id, actor, reason, Utc::now())
            .await?;
        self.release(&ended, reason).await?;
        Ok(ended)
    }

    /// End every live link of `group_id`: the group was archived, or its coach
    /// detached or replaced. Returns the ended rows.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the links cannot be ended or a
    /// confirmed one's delegated connection cannot be removed.
    pub async fn end_for_group(
        &self,
        group_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
    ) -> AppResult<Vec<DelegatedConnection>> {
        let ended = self
            .links
            .end_for_group(group_id, actor, reason, Utc::now())
            .await?;
        self.release(&ended, reason).await?;
        Ok(ended)
    }

    /// End every live link served by `coach_user_id`'s `provider` session in
    /// `coach_tenant`: the coach disconnected the provider. Returns the ended
    /// rows.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the links cannot be ended or a
    /// confirmed one's delegated connection cannot be removed.
    pub async fn end_for_coach(
        &self,
        coach_user_id: Uuid,
        coach_tenant: TenantId,
        provider: &str,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
    ) -> AppResult<Vec<DelegatedConnection>> {
        let ended = self
            .links
            .end_for_coach(
                coach_user_id,
                coach_tenant,
                provider,
                actor,
                reason,
                Utc::now(),
            )
            .await?;
        self.release(&ended, reason).await?;
        Ok(ended)
    }

    /// End `member_user_id`'s confirmed `provider` link in `member_tenant`:
    /// the member disconnected the provider, signed in to it themselves, or
    /// the provider no longer lists them on the coach's roster. Returns the
    /// ended rows (at most one).
    ///
    /// # Errors
    ///
    /// Returns the repository error when the link cannot be ended or its
    /// delegated connection cannot be removed.
    pub async fn end_confirmed_for_member(
        &self,
        member_user_id: Uuid,
        member_tenant: TenantId,
        provider: &str,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
    ) -> AppResult<Vec<DelegatedConnection>> {
        let ended = self
            .links
            .end_confirmed_for_member(
                member_user_id,
                member_tenant,
                provider,
                actor,
                reason,
                Utc::now(),
            )
            .await?;
        self.release(&ended, reason).await?;
        Ok(ended)
    }

    /// End every live link, proposed or confirmed, that `coach_user_id`'s
    /// `provider` session in `coach_tenant` holds for an athlete `roster` no
    /// longer lists: the provider dropped them from the coach's roster. The
    /// system ends them, so no actor is recorded. Returns the ended rows.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the links cannot be read or ended,
    /// or a confirmed one's delegated connection cannot be removed.
    pub async fn end_off_coach_roster(
        &self,
        coach_user_id: Uuid,
        coach_tenant: TenantId,
        provider: &str,
        roster: &[&str],
    ) -> AppResult<Vec<DelegatedConnection>> {
        let mut ended = Vec::new();
        for link in self
            .links
            .list_live_for_coach(coach_user_id, coach_tenant, provider)
            .await?
            .into_iter()
            .filter(|link| !roster.contains(&link.provider_athlete_id.as_str()))
        {
            if let Some(link) = self
                .end_one(
                    link.id,
                    coach_user_id,
                    None,
                    DelegationEndReason::NotOnRoster,
                )
                .await?
            {
                ended.push(link);
            }
        }
        Ok(ended)
    }

    /// End `member_user_id`'s confirmed `provider` link in `member_tenant`
    /// that the group relation no longer backs, for the reason it broke.
    ///
    /// The reason is what broke: the group is gone or inactive ([`DelegationEndReason::GroupArchived`]), its
    /// coach is no longer the link's ([`DelegationEndReason::CoachDetached`]),
    /// or the member is no longer in it ([`DelegationEndReason::MemberLeft`]).
    /// The system ends it, so no actor is recorded.
    ///
    /// This is the read path's refusal made durable: a lifecycle change that
    /// did not end the link itself is found when the member next reads. The
    /// reason is read in the order the relation is checked by the read path's
    /// statement, group first.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the link, its group or the
    /// membership cannot be read, or the link cannot be ended or released.
    pub async fn end_unbacked_for_member(
        &self,
        member_user_id: Uuid,
        member_tenant: TenantId,
        provider: &str,
    ) -> AppResult<UnbackedLink> {
        let Some(link) = self
            .links
            .list_live_for_member(member_user_id)
            .await?
            .into_iter()
            .find(|link| {
                link.provider == provider
                    && link.status == DelegationStatus::Confirmed
                    && link.member_tenant_id == Some(member_tenant)
            })
        else {
            return Ok(UnbackedLink::Gone);
        };
        let Some(reason) = self.broken_relation(&link).await? else {
            return Ok(UnbackedLink::Backed);
        };
        Ok(self
            .end_one(link.id, member_user_id, None, reason)
            .await?
            .map_or(UnbackedLink::Gone, |link| {
                UnbackedLink::Ended(Box::new(link))
            }))
    }

    /// Why the group relation no longer backs `link`, or `None` when it does.
    async fn broken_relation(
        &self,
        link: &DelegatedConnection,
    ) -> AppResult<Option<DelegationEndReason>> {
        let group_id = link.group_id.to_string();
        let group = self
            .groups
            .get_group(&group_id, link.coach_tenant_id)
            .await?
            .filter(|group| group.is_active);
        let Some(group) = group else {
            return Ok(Some(DelegationEndReason::GroupArchived));
        };
        if group.coach_user_id != Some(link.coach_user_id) {
            return Ok(Some(DelegationEndReason::CoachDetached));
        }
        if self
            .groups
            .get_member(&group_id, link.member_user_id)
            .await?
            .is_none()
        {
            return Ok(Some(DelegationEndReason::MemberLeft));
        }
        Ok(None)
    }

    /// End the one live link `id`, which `participant_user_id` must be the
    /// coach or the member of. `actor` is who ended it; `None` for the system.
    /// Returns the ended row, or `None` when no live link with that id names
    /// the participant.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the link cannot be ended or its
    /// delegated connection cannot be removed.
    pub async fn end_one(
        &self,
        id: Uuid,
        participant_user_id: Uuid,
        actor: Option<Uuid>,
        reason: DelegationEndReason,
    ) -> AppResult<Option<DelegatedConnection>> {
        let ended = self
            .links
            .end_one(id, participant_user_id, actor, reason, Utc::now())
            .await?;
        if let Some(link) = &ended {
            self.release(slice::from_ref(link), reason).await?;
        }
        Ok(ended)
    }

    /// Release what each ended link that had been confirmed left behind: the
    /// member's delegated provider connection and the activities read through
    /// the coach's session. A link only ever proposed left nothing.
    ///
    /// The connection removal is the step that stops the card reading as
    /// connected, so its failure is returned; the cache purge is best-effort,
    /// as on a disconnect, since an undeleted row still ages out through
    /// retention pruning.
    async fn release(
        &self,
        ended: &[DelegatedConnection],
        reason: DelegationEndReason,
    ) -> AppResult<()> {
        for link in ended {
            let (Some(_), Some(member_tenant)) = (link.confirmed_at, link.member_tenant_id) else {
                continue;
            };
            let member = link.member_user_id;
            let provider = link.provider.as_str();
            self.connections
                .remove_delegated_connection(member, member_tenant, provider)
                .await?;
            if let Err(e) = self
                .activity_cache
                .delete_provider_activities(member, &member_tenant, provider)
                .await
            {
                warn!(
                    user_id = %member,
                    backend = %provider,
                    error = %e,
                    "Failed to delete activities read through an ended delegated connection; rows age out via retention pruning"
                );
            }
            info!(
                target: "notify",
                event = "provider.disconnected",
                provider = %user_facing_name(provider),
                backend = %provider,
                user_id = %member,
                tenant_id = %member_tenant,
                reason = reason.as_str(),
                delegated = true,
                "delegated connection ended"
            );
        }
        Ok(())
    }
}
