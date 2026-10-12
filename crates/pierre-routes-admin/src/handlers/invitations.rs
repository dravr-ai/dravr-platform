// ABOUTME: The invitation half of the pre-approval allow — issue a single-use link, mail it, report what became of it
// ABOUTME: The token goes to the notifier and nowhere else; the operator is told the outcome in the allow's own sentence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::AppResult;
use pierre_middleware::redaction::mask_email;
use pierre_services::invitation::{
    self, InviteOutcome, IssuedInvitation, INVITATIONS_PER_ADDRESS_PER_HOUR,
};
use pierre_services::pre_approval::{self, AllowOutcome};
use pierre_services::user_approval::UserApprovalNotifier;

use crate::context::AdminApiContext;

/// What an allow did about the invitation it may have been asked to send.
pub(super) enum InviteReport {
    /// No invitation was asked for.
    NotAsked,
    /// A single-use invitation was issued and mailed.
    Sent {
        /// When its link stops working.
        expires_at: DateTime<Utc>,
    },
    /// The address already has an account, so there is nothing to invite.
    AccountExists,
    /// The address's hourly invitation budget is spent.
    BudgetSpent,
    /// No notifier is wired, so nothing could carry the link.
    NoNotifier,
    /// The invitation email did not go out, so its link was withdrawn.
    NotDelivered,
}

impl InviteReport {
    /// Whether an invitation was mailed.
    pub(super) const fn invited(&self) -> bool {
        matches!(self, Self::Sent { .. })
    }

    /// When the sent invitation's link stops working.
    pub(super) const fn expires_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Sent { expires_at } => Some(*expires_at),
            _ => None,
        }
    }

    /// The clause the operator reads after the allow's own sentence: empty
    /// when no invitation was asked for, and otherwise what became of it.
    pub(super) fn sentence(&self) -> String {
        match self {
            Self::NotAsked => String::new(),
            Self::Sent { expires_at } => format!(
                " — invitation sent: its link works once, until {}",
                expires_at.format("%Y-%m-%d %H:%M UTC")
            ),
            Self::AccountExists => {
                " — no invitation sent: the address already has an account".to_owned()
            }
            Self::BudgetSpent => format!(
                " — no invitation sent: {INVITATIONS_PER_ADDRESS_PER_HOUR} already went to \
                 this address in the last hour"
            ),
            Self::NoNotifier => " — no invitation sent: this server has no mailer wired".to_owned(),
            Self::NotDelivered => {
                " — no invitation sent: its email could not be delivered, so the link was \
                 withdrawn"
                    .to_owned()
            }
        }
    }
}

/// Issue and mail a single-use invitation when one was asked for and the
/// address has no account, logging which way it went.
///
/// Only an address with no account is invited. A pending one was just
/// approved by the allow and gets that announcement instead; an active or
/// suspended account already exists, so a "create your account" link would be
/// wrong. Issuing a new invitation revokes any earlier one for the address, so
/// asking again is how an operator re-sends.
///
/// # Errors
///
/// Returns the repository's error when the invitation cannot be issued.
pub(super) async fn send_invite_if_eligible(
    ctx: &AdminApiContext,
    result: &pre_approval::AllowResult,
    send_invite: bool,
    admin_token: &ValidatedAdminToken,
) -> AppResult<InviteReport> {
    if !send_invite {
        return Ok(InviteReport::NotAsked);
    }
    let recipient = mask_email(&result.email);
    if !matches!(
        result.outcome,
        AllowOutcome::Recorded | AllowOutcome::AlreadyAllowed
    ) {
        info!(
            %recipient,
            outcome = ?result.outcome,
            "Invitation requested but not sent: the address already has an account"
        );
        return Ok(InviteReport::AccountExists);
    }
    let Some(notifier) = ctx.approval_notifier.as_ref() else {
        warn!(%recipient, "No approval notifier wired — invitation not issued");
        return Ok(InviteReport::NoNotifier);
    };
    issue_and_mail(ctx, notifier.as_ref(), &result.email, admin_token).await
}

/// Issue the invitation and hand it to the notifier.
///
/// # Errors
///
/// Returns the repository's error when the invitation cannot be issued, or an
/// undelivered one cannot be withdrawn.
async fn issue_and_mail(
    ctx: &AdminApiContext,
    notifier: &dyn UserApprovalNotifier,
    email: &str,
    admin_token: &ValidatedAdminToken,
) -> AppResult<InviteReport> {
    match invitation::issue(&ctx.repos, email, admin_token.operator_user_id).await? {
        InviteOutcome::Issued(issued) => mail(ctx, notifier, &issued, admin_token).await,
        InviteOutcome::AccountExists => {
            info!(
                recipient = %mask_email(email),
                "Invitation not issued: the address registered meanwhile"
            );
            Ok(InviteReport::AccountExists)
        }
        InviteOutcome::BudgetSpent => Ok(InviteReport::BudgetSpent),
    }
}

/// Mail an issued invitation, raising `user.invited` for the operators'
/// channel once it went out. The token goes to the notifier and nowhere else,
/// so an invitation the notifier could not mail is withdrawn at once: nobody
/// holds its link, and the listing must not show it as outstanding.
///
/// # Errors
///
/// Returns the repository's error when an undelivered invitation cannot be
/// withdrawn.
async fn mail(
    ctx: &AdminApiContext,
    notifier: &dyn UserApprovalNotifier,
    issued: &IssuedInvitation,
    admin_token: &ValidatedAdminToken,
) -> AppResult<InviteReport> {
    if !notifier.notify_user_invited(issued).await {
        invitation::revoke(&ctx.repos, &issued.email).await?;
        warn!(
            recipient = %mask_email(&issued.email),
            "Invitation email not delivered — its link was withdrawn"
        );
        return Ok(InviteReport::NotDelivered);
    }
    info!(
        target: "notify",
        event = "user.invited",
        invited_by = %admin_token.service_name,
        "user invited by address"
    );
    Ok(InviteReport::Sent {
        expires_at: issued.expires_at,
    })
}
