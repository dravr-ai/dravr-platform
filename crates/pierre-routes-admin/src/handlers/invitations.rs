// ABOUTME: The invitation half of the pre-approval allow — issue a single-use link, mail it, report what became of it
// ABOUTME: The token goes to the notifier, or to a super-admin who asked for the link itself; never to a log
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The invitation an allow may send.
//!
//! The invitation's token goes to the notifier and nowhere else, with one
//! exception: a super-admin may ask for the link itself (`return_link`), and
//! then it is answered to them instead of mailed. That is for operator-driven
//! end-to-end testing on a deployed environment, where the test accounts'
//! addresses have no mailbox anyone reads; a super-admin can already mint any
//! account's access, so handing them the link grants nothing new. The link is
//! not mailed then (a mail that failed would withdraw the very link just
//! answered), no notifier is needed, and no `user.invited` event is raised,
//! since a test account is not an invitation the operators' channel counts.
//! Neither the token nor the link ever reaches the operator's sentence or a
//! log line: only the response's `invite_url` carries it.

use axum::http::StatusCode;
use axum::response::Response;
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

use super::strava_pool::deny_if_not_super_admin;
use super::types::AllowEmailRequest;
use super::user_removal::refusal;
use crate::context::AdminApiContext;

/// How an issued invitation reaches the invitee.
pub(super) enum Delivery<'ctx> {
    /// Mailed through the approval notifier.
    Mail,
    /// Answered to the super-admin who asked, as a link on the web app at
    /// `frontend_url`.
    Link {
        /// The web app the link opens.
        frontend_url: &'ctx str,
    },
}

/// How the allow in `request` delivers its invitation, or the refusal it
/// takes before anything is written: 403 for `return_link` from a token below
/// super-admin, 400 for `return_link` without `send_invite`, 503 for
/// `return_link` on a server with no frontend URL to build the link on.
pub(super) fn delivery<'ctx>(
    ctx: &'ctx AdminApiContext,
    request: &AllowEmailRequest,
    admin_token: &ValidatedAdminToken,
) -> Result<Delivery<'ctx>, Box<Response>> {
    if !request.return_link {
        return Ok(Delivery::Mail);
    }
    if let Some(denied) = deny_if_not_super_admin(admin_token) {
        return Err(Box::new(denied));
    }
    if !request.send_invite {
        return Err(Box::new(refusal(
            StatusCode::BAD_REQUEST,
            "return_link requires send_invite: there is no link without an invitation".to_owned(),
            None,
        )));
    }
    ctx.frontend_url
        .as_deref()
        .map(|frontend_url| Delivery::Link { frontend_url })
        .ok_or_else(|| {
            Box::new(refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "This server has no frontend URL to build an invitation link on".to_owned(),
                None,
            ))
        })
}

/// What an allow did about the invitation it may have been asked to send.
pub(super) enum InviteReport {
    /// No invitation was asked for.
    NotAsked,
    /// A single-use invitation was issued and mailed.
    Sent {
        /// When its link stops working.
        expires_at: DateTime<Utc>,
    },
    /// A single-use invitation was issued and its link answered to the
    /// super-admin who asked, not mailed.
    Link {
        /// The accept link, token in its fragment.
        url: String,
        /// When the link stops working.
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
    /// Whether an invitation is outstanding: mailed, or its link answered.
    pub(super) const fn invited(&self) -> bool {
        matches!(self, Self::Sent { .. } | Self::Link { .. })
    }

    /// When the outstanding invitation's link stops working.
    pub(super) const fn expires_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Sent { expires_at } | Self::Link { expires_at, .. } => Some(*expires_at),
            _ => None,
        }
    }

    /// The accept link, when the super-admin asked for it.
    pub(super) fn invite_url(&self) -> Option<&str> {
        match self {
            Self::Link { url, .. } => Some(url),
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
            Self::Link { expires_at, .. } => format!(
                " — invitation link issued, not mailed: it works once, until {}",
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

/// Issue a single-use invitation when one was asked for and the address has
/// no account, and deliver it as `delivery` says, logging which way it went.
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
    delivery: &Delivery<'_>,
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
    let carrier = match delivery {
        Delivery::Link { frontend_url } => Carrier::Operator { frontend_url },
        Delivery::Mail => {
            let Some(notifier) = ctx.approval_notifier.as_ref() else {
                warn!(%recipient, "No approval notifier wired — invitation not issued");
                return Ok(InviteReport::NoNotifier);
            };
            Carrier::Notifier(notifier.as_ref())
        }
    };
    issue_and_deliver(ctx, carrier, &result.email, admin_token).await
}

/// Who an issued invitation's token goes to.
enum Carrier<'ctx> {
    /// The notifier, which mails it.
    Notifier(&'ctx dyn UserApprovalNotifier),
    /// The super-admin who asked, as a link on the web app at `frontend_url`.
    Operator { frontend_url: &'ctx str },
}

/// Issue the invitation and hand it to `carrier`.
///
/// # Errors
///
/// Returns the repository's error when the invitation cannot be issued, or an
/// undelivered one cannot be withdrawn.
async fn issue_and_deliver(
    ctx: &AdminApiContext,
    carrier: Carrier<'_>,
    email: &str,
    admin_token: &ValidatedAdminToken,
) -> AppResult<InviteReport> {
    match invitation::issue(&ctx.repos, email, admin_token.operator_user_id).await? {
        InviteOutcome::Issued(issued) => match carrier {
            Carrier::Notifier(notifier) => mail(ctx, notifier, &issued, admin_token).await,
            Carrier::Operator { frontend_url } => {
                info!(
                    recipient = %mask_email(email),
                    token_id = %admin_token.token_id,
                    "Invitation link issued to the super-admin who asked, not mailed"
                );
                Ok(InviteReport::Link {
                    url: issued.url(frontend_url),
                    expires_at: issued.expires_at,
                })
            }
        },
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
/// channel once it went out. A mailed token goes to the notifier and nowhere else,
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
