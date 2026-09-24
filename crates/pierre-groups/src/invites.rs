// ABOUTME: Invite redemption on GroupService: the checks every invite passes and a coach's attachment
// ABOUTME: Split from service.rs so each file stays under the size ceiling; the methods are GroupService's own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Invite redemption.
//!
//! A member invite adds an athlete ([`GroupService::join_group`]); a coach
//! invite attaches the group's human coach ([`GroupService::redeem_coach_invite`]).
//! Both refuse an inactive, expired or used-up invite, and a group that has
//! been archived since the invite was handed out.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{CoachingGroup, GroupInvite, GroupInviteKind};
use pierre_core::models::TenantId;
use tracing::info;
use uuid::Uuid;

use crate::service::GroupService;

impl GroupService {
    /// Fetch the group an invite points at, refusing one that has been archived.
    ///
    /// `delete_group` archives the row rather than dropping it, and an invite
    /// outlives that archive. Both redemption paths read the invite first and
    /// then loaded the group unconditionally, so a link handed out before the
    /// owner deleted the group still admitted people to it — against a confirm
    /// dialog that promises the group is gone and its members removed.
    pub(crate) async fn open_group_for_invite(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachingGroup> {
        let group = self
            .repo()
            .get_group(group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;
        if !group.is_active {
            return Err(AppError::not_found("This group is no longer available"));
        }
        Ok(group)
    }

    /// Shared invite-validity gate: active, not expired, under its use limit.
    pub(crate) fn check_invite_usable(invite: &GroupInvite) -> AppResult<()> {
        if !invite.is_active {
            return Err(AppError::invalid_input("This invite has been deactivated"));
        }
        if let Some(expires) = invite.expires_at {
            if expires < chrono::Utc::now() {
                return Err(AppError::invalid_input("This invite has expired"));
            }
        }
        if let Some(max) = invite.max_uses {
            if invite.use_count >= max {
                return Err(AppError::invalid_input(
                    "This invite has reached its use limit",
                ));
            }
        }
        Ok(())
    }

    /// Redeem a coach-kind invite, attaching the caller as the group's human
    /// agent (`coach_user_id`).
    ///
    /// Eligibility (the caller is a roster-managing agent and belongs to the
    /// group's tenant) is enforced by the route layer, which owns user-repo
    /// access. This method owns the group-side business logic: invite
    /// validity, the single-agent guard, the attachment write, and the
    /// invite-use increment.
    ///
    /// # Errors
    ///
    /// Returns an error if the invite is invalid/expired/exhausted, is not a
    /// agent invite, the group is missing, or a different agent is already
    /// attached.
    pub async fn redeem_coach_invite(
        &self,
        invite_code: &str,
        coach_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<CoachingGroup> {
        let invite = self
            .repo()
            .get_invite_by_code(invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        Self::check_invite_usable(&invite)?;

        if invite.kind != GroupInviteKind::Coach {
            return Err(AppError::invalid_input(
                "This invite does not grant coach access",
            ));
        }

        let group = self
            .open_group_for_invite(&invite.group_id.to_string(), tenant_id)
            .await?;

        // Single human coach per group (v1). Re-redeeming as the same agent is
        // idempotent; a different agent is rejected so an owner explicitly
        // detaches the current agent first.
        match group.coach_user_id {
            Some(existing) if existing == coach_user_id => return Ok(group),
            Some(_) => {
                return Err(AppError::invalid_input(
                    "This group already has a coach. Remove the current coach first.",
                ));
            }
            None => {}
        }

        let attached = self
            .repo()
            .set_group_coach_user(&invite.group_id.to_string(), Some(coach_user_id), tenant_id)
            .await?;
        if !attached {
            return Err(AppError::internal("Failed to attach coach to group"));
        }
        self.repo()
            .increment_invite_use_count(&invite.id.to_string())
            .await?;

        // Reuses the catalogued `group.joined` event (an agent redeeming a
        // coach-kind invite is still a join); the message distinguishes the
        // agent case for operators. Emitted after the attach succeeds, so
        // re-redeeming the same invite — which returns early above — no
        // longer double-counts the way the route-level emission did.
        info!(
            target: "notify",
            event = "group.joined",
            user_id = %coach_user_id,
            tenant_id = %tenant_id,
            group_id = %invite.group_id,
            "coach joined coaching group"
        );

        self.repo()
            .get_group(&invite.group_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Group not found after coach attach"))
    }
}
