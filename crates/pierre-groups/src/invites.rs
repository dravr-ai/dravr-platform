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

use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupInvite, GroupInviteKind, GroupMember, GroupRole,
};
use pierre_core::models::TenantId;
use rand::Rng;
use tracing::info;
use uuid::Uuid;

use crate::service::GroupService;
use crate::setup_timing::record_member_joined;

/// The most accounts one coach's signup code may approve (carnet#1128).
pub(crate) const SIGNUP_INVITE_MAX_USES: i32 = 50;

/// The longest a coach's signup code stays redeemable, in days.
pub(crate) const SIGNUP_INVITE_MAX_DAYS: i64 = 30;

/// Characters every invite and signup code is drawn from: no 0/O or 1/I, so
/// a code read aloud or copied by hand survives. 32 symbols, 5 bits each.
const CODE_CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Length of a coach's signup code: 13 characters of a 32-symbol alphabet is
/// 65 bits, against 40 for a plain 8-character invite. A plain invite only
/// opens a group to an approved account; this one also approves it.
const SIGNUP_CODE_LEN: usize = 13;

/// `len` random characters of the unambiguous invite alphabet: the one
/// generator behind group invites, coaches' signup codes and admins' signup
/// codes.
#[must_use]
pub fn unambiguous_code(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| char::from(CODE_CHARSET[rng.random_range(0..CODE_CHARSET.len())]))
        .collect()
}

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
            if expires < Utc::now() {
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
    /// group's tenant) is enforced by the `/group join` command, which owns
    /// user-repo access. This method owns the group-side business logic: invite
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

    /// Mint a coach's signup code: a member invite of `group_id` that also
    /// approves the pending account redeeming it (carnet#1128).
    ///
    /// Bounded so it cannot become an open door: both caps are mandatory,
    /// at most [`SIGNUP_INVITE_MAX_USES`] uses and [`SIGNUP_INVITE_MAX_DAYS`]
    /// days. Who may mint one (a coach with `manages_roster` who administers
    /// the group) is the route's check.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a missing or out-of-range cap, and
    /// the repository's error when the insert fails.
    pub async fn create_signup_invite(
        &self,
        group_id: Uuid,
        created_by: Uuid,
        tenant_id: TenantId,
        expires_in_days: Option<i64>,
        max_uses: Option<i32>,
    ) -> AppResult<GroupInvite> {
        let days = expires_in_days
            .filter(|days| (1..=SIGNUP_INVITE_MAX_DAYS).contains(days))
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "A signup code needs expires_in_days between 1 and {SIGNUP_INVITE_MAX_DAYS}"
                ))
            })?;
        let uses = max_uses
            .filter(|uses| (1..=SIGNUP_INVITE_MAX_USES).contains(uses))
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "A signup code needs max_uses between 1 and {SIGNUP_INVITE_MAX_USES}"
                ))
            })?;
        let now = Utc::now();
        let invite = GroupInvite {
            id: Uuid::new_v4(),
            group_id,
            tenant_id: tenant_id.to_string(),
            code: unambiguous_code(SIGNUP_CODE_LEN),
            kind: GroupInviteKind::Member,
            created_by,
            expires_at: Some(now + Duration::days(days)),
            max_uses: Some(uses),
            use_count: 0,
            is_active: true,
            approves_signup: true,
            created_at: now,
        };
        let created = self.repo().create_invite(&invite).await?;
        info!(group_id = %group_id, created_by = %created_by, max_uses = uses, days, "Coach signup code minted");
        Ok(created)
    }

    /// Join a group via invite code
    ///
    /// # Errors
    ///
    /// Returns an error if the invite is invalid, expired, full, or the user is already a member.
    pub async fn join_group(
        &self,
        invite_code: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<GroupMember> {
        let invite = self
            .repo()
            .get_invite_by_code(invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        Self::check_invite_usable(&invite)?;

        // Member invites never attach a coach — coach-kind invites are
        // redeemed through `redeem_coach_invite`, which `/group join`
        // dispatches to based on `invite.kind`.
        if invite.kind == GroupInviteKind::Coach {
            return Err(AppError::invalid_input(
                "This is a coach invite — redeem it from the coach flow, not group join",
            ));
        }

        self.admit_member(&invite, user_id, tenant_id, true).await
    }

    /// Join the account that redeemed a coach's signup code to the code's
    /// group (carnet#1128).
    ///
    /// The code's use was claimed when the account entered it, so it is not
    /// counted again, and its expiry no longer applies: an account that
    /// entered the code in time and confirmed its address later still joins.
    /// The group's own checks (archived, full, already a member) do apply.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when the invite is gone or is not an
    /// approving invite, and every error the membership checks return.
    pub async fn join_group_from_signup_code(
        &self,
        invite_id: &str,
        user_id: Uuid,
    ) -> AppResult<GroupMember> {
        let invite = self
            .repo()
            .get_invite_by_id(invite_id)
            .await?
            .filter(|invite| invite.approves_signup && invite.kind == GroupInviteKind::Member)
            .ok_or_else(|| AppError::not_found("This signup code's group invite is gone"))?;
        let tenant_id = TenantId::parse_str(&invite.tenant_id)
            .map_err(|e| AppError::internal(format!("Invite tenant id is not a uuid: {e}")))?;
        self.admit_member(&invite, user_id, tenant_id, false).await
    }

    /// Add `user_id` to the invite's group as an athlete. `count_use` claims
    /// one use of the invite first, in the same statement that checks its
    /// limit, so two joins on the last use cannot both pass.
    async fn admit_member(
        &self,
        invite: &GroupInvite,
        user_id: Uuid,
        tenant_id: TenantId,
        count_use: bool,
    ) -> AppResult<GroupMember> {
        let group = self
            .open_group_for_invite(&invite.group_id.to_string(), tenant_id)
            .await?;

        let current_count = self
            .repo()
            .count_members(&invite.group_id.to_string())
            .await?;
        if current_count >= i64::from(group.max_members) {
            return Err(AppError::invalid_input("This group is full"));
        }

        let existing = self
            .repo()
            .get_member(&invite.group_id.to_string(), user_id)
            .await?;
        if existing.is_some() {
            return Err(AppError::invalid_input(
                "You are already a member of this group",
            ));
        }

        if count_use
            && !self
                .repo()
                .increment_invite_use_count(&invite.id.to_string())
                .await?
        {
            return Err(AppError::invalid_input(
                "This invite has reached its use limit",
            ));
        }

        let now = Utc::now();
        let member = GroupMember {
            id: Uuid::new_v4(),
            group_id: invite.group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        };

        let created = self.repo().add_member(&member).await?;

        // The invite's tenant is the *group's* tenant, which is what the
        // caller passes in — athlete membership is cross-tenant by design, so
        // the adoption metric must follow the group rather than the
        // redeemer's home tenant.
        info!(
            target: "notify",
            event = "group.joined",
            user_id = %user_id,
            tenant_id = %tenant_id,
            group_id = %invite.group_id,
            "user joined coaching group"
        );
        record_member_joined(self.repo(), &created, group.owner_id).await;
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_has_the_length_asked_for_and_no_ambiguous_character() {
        for len in [8, SIGNUP_CODE_LEN] {
            let code = unambiguous_code(len);
            assert_eq!(code.len(), len);
            assert!(code.bytes().all(|b| CODE_CHARSET.contains(&b)), "{code}");
            assert!(!code.contains(['0', 'O', '1', 'I']), "{code}");
        }
    }
}
