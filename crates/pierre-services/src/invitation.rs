// ABOUTME: Single-use invitations an operator mails to an address with no account — issue, revoke, list, build the link
// ABOUTME: A coach-seat invitation also grants coach access and records the coach role when it is redeemed (carnet#1524)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The operator side of an invitation.
//!
//! An invitation goes with a standing pre-approval ([`crate::pre_approval`]):
//! the operator allows an address and asks for it to be told, and the mail
//! carries a single-use link to the web app's accept page, where the invitee
//! chooses their own password and the account is created
//! [`UserStatus::Active`](pierre_core::models::UserStatus::Active) with
//! `approved_by` set to the inviting operator while their account exists. No
//! password ever crosses a side channel, and the link opens nothing for anyone
//! who is not reading that inbox.
//!
//! A super-admin may also invite a coach the team recruited
//! ([`InvitationKind::CoachSeat`], carnet#1524). It is the same link, locked
//! to the same address; redeeming it also settles what a coach otherwise
//! meets on the way in ([`settle_coach_seat`]): coach access
//! (`manages_roster`, an operator grant by the inviter — the decision ADR-018
//! leaves to a super-admin, taken before sign-up rather than after a
//! request) and the coach role, so the coach meets neither the role question
//! nor the coach-access request.
//!
//! The token is a `<selector>.<verifier>` pair from [`crate::link_token`]:
//! only the selector and the verifier's SHA-256 reach the database, and the
//! whole token exists once, in the [`IssuedInvitation`] handed to whoever
//! mails it. Nothing here logs it.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{InvitationKind, TenantId, UserInvitation};
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::admin_ops::set_user_manages_roster;
use crate::intake::{
    release_coach_only_agent, STATUS_COMPLETE, STATUS_NOT_APPLICABLE, STEP_ABOUT_YOU, STEP_PARQ,
    STEP_PROFILE_TYPE,
};
use crate::link_token::generate_link_token;
use crate::pre_approval::valid_email;

/// How long an invitation link stays redeemable. Long enough for a person who
/// reads mail weekly; an operator re-sends past it.
pub const INVITATION_TTL_DAYS: i64 = 7;

/// Invitations one address may be sent in a rolling hour, counted from the
/// invitation table. An operator re-sending is normal; re-sending in a loop is
/// mail an inbox did not ask for.
pub const INVITATIONS_PER_ADDRESS_PER_HOUR: i64 = 5;

/// The web app's page an invitation link opens.
const ACCEPT_INVITATION_PATH: &str = "/accept-invite";

/// The onboarding step a coach's seat is settled on.
///
/// Only a redeemed coach-seat invitation records it on the server
/// ([`settle_coach_seat`]): the wizard's seat step is a device flag, and the
/// onboarding `PUT` route does not accept it, so a server row for it is the
/// invitation's.
pub const STEP_COACH_SEAT: &str = "coach_seat";

/// A freshly issued invitation: what the mail carries.
///
/// Deliberately not `Debug`: `token` is the only copy of the secret, and a
/// `{:?}` anywhere would put it in a log.
pub struct IssuedInvitation {
    /// The normalized address it was issued to.
    pub email: String,
    /// The `<selector>.<verifier>` token the link carries. Never stored whole.
    pub token: String,
    /// When the link stops working.
    pub expires_at: DateTime<Utc>,
    /// What redeeming it gives besides the account.
    pub kind: InvitationKind,
}

impl fmt::Debug for IssuedInvitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedInvitation")
            .field("email", &self.email)
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("kind", &self.kind)
            .finish()
    }
}

impl IssuedInvitation {
    /// The link the invitee follows, on the web app at `frontend_url`.
    #[must_use]
    pub fn url(&self, frontend_url: &str) -> String {
        invitation_url(frontend_url, &self.token, self.kind)
    }
}

/// What [`issue`] did.
#[derive(Debug)]
pub enum InviteOutcome {
    /// An invitation was issued; any earlier live one for the address is revoked.
    Issued(IssuedInvitation),
    /// The address already has an account, so there is nothing to invite it to.
    AccountExists,
    /// The address was sent [`INVITATIONS_PER_ADDRESS_PER_HOUR`] invitations
    /// in the last hour; nothing was issued.
    BudgetSpent,
}

/// The accept link for `token` on the web app at `frontend_url`.
///
/// The token rides in the fragment, not the query: a fragment never reaches a
/// server log, a proxy or a `Referer` header, and the page reads it from the
/// address bar and clears it before anything else loads. A coach-seat
/// invitation's link also names its kind there (`kind=coach_seat`), so the
/// page words the form for a coach before anything is redeemed. It is a
/// wording hint only: what redeeming grants is the stored invitation's kind,
/// never the link's.
#[must_use]
fn invitation_url(frontend_url: &str, token: &str, kind: InvitationKind) -> String {
    let base = format!(
        "{}{ACCEPT_INVITATION_PATH}#token={}",
        frontend_url.trim_end_matches('/'),
        urlencoding::encode(token)
    );
    match kind {
        InvitationKind::Account => base,
        InvitationKind::CoachSeat => format!("{base}&kind={}", kind.label()),
    }
}

/// Issue a single-use invitation of `kind` to an address with no account.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] for a malformed
/// address, or the repository's error when a lookup, the count or the insert
/// fails.
pub async fn issue(
    repos: &RepositoryRegistry,
    email: &str,
    invited_by: Option<Uuid>,
    kind: InvitationKind,
) -> AppResult<InviteOutcome> {
    let email = valid_email(email)?;

    if repos.users.get_by_email(&email).await?.is_some() {
        return Ok(InviteOutcome::AccountExists);
    }

    let sent = repos
        .user_invitations
        .count_recent(&email, Utc::now() - Duration::hours(1))
        .await?;
    if sent >= INVITATIONS_PER_ADDRESS_PER_HOUR {
        warn!(
            sent,
            "Invitation not issued: the address's hourly budget is spent"
        );
        return Ok(InviteOutcome::BudgetSpent);
    }

    let generated = generate_link_token();
    let invitation = repos
        .user_invitations
        .issue(
            &email,
            invited_by,
            &generated.selector,
            &generated.verifier_hash,
            Utc::now() + Duration::days(INVITATION_TTL_DAYS),
            kind,
        )
        .await?;
    info!(invited_by = ?invited_by, kind = kind.label(), "Invitation issued");

    Ok(InviteOutcome::Issued(IssuedInvitation {
        email: invitation.email,
        token: generated.token,
        expires_at: invitation.expires_at,
        kind,
    }))
}

/// Revoke the live invitation an address holds, so its link stops working.
/// Returns whether there was one.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] for a malformed
/// address, or the repository's error when the update fails.
pub async fn revoke(repos: &RepositoryRegistry, email: &str) -> AppResult<bool> {
    let email = valid_email(email)?;
    let revoked = repos.user_invitations.revoke_for_email(&email).await?;
    if revoked {
        info!("Invitation revoked");
    }
    Ok(revoked)
}

/// Revoke the coach-seat invitation an address holds, so its link stops
/// working. A plain invitation the address holds instead is left alone.
/// Returns whether there was one.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] for a malformed
/// address, or the repository's error when the update fails.
pub async fn revoke_coach_seat(repos: &RepositoryRegistry, email: &str) -> AppResult<bool> {
    let email = valid_email(email)?;
    let revoked = repos
        .user_invitations
        .revoke_coach_seat_for_email(&email)
        .await?;
    if revoked {
        info!("Coach-seat invitation revoked");
    }
    Ok(revoked)
}

/// One coach-seat invitation as the console's coach-access queue lists it.
#[derive(Debug, Clone, Serialize)]
pub struct CoachSeatInvitationView {
    /// The invited address.
    pub email: String,
    /// The operator who sent it, when attributable.
    pub invited_by: Option<Uuid>,
    /// That operator's address, while their account exists.
    pub invited_by_email: Option<String>,
    /// When it was sent.
    pub created_at: DateTime<Utc>,
    /// When its link stops (or stopped) working.
    pub expires_at: DateTime<Utc>,
    /// Its link no longer works: the operator sends it again.
    pub expired: bool,
}

/// Every coach-seat invitation neither redeemed nor revoked, oldest first,
/// expired ones included so the console can send them again.
///
/// # Errors
///
/// Returns the repository's error when the listing fails.
pub async fn list_coach_seat_invitations(
    repos: &RepositoryRegistry,
) -> AppResult<Vec<CoachSeatInvitationView>> {
    let now = Utc::now();
    let mut views = Vec::new();
    for invitation in repos
        .user_invitations
        .list_outstanding_coach_seats()
        .await?
    {
        let invited_by_email = match invitation.invited_by {
            Some(id) => repos.users.get_global(id).await?.map(|user| user.email),
            None => None,
        };
        views.push(CoachSeatInvitationView {
            expired: invitation.expires_at <= now,
            email: invitation.email,
            invited_by: invitation.invited_by,
            invited_by_email,
            created_at: invitation.created_at,
            expires_at: invitation.expires_at,
        });
    }
    Ok(views)
}

/// Settle what a coach-seat invitation decided for the account just created
/// from it, so the coach meets none of it on the way in:
///
/// - coach access: `manages_roster` as an operator grant by `operator`, the
///   inviter (`None` when their account is gone), through the one operator
///   path ([`set_user_manages_roster`]);
/// - the coach role (`users.coaches_others`), and the role question answered
///   as a coach who does not train: the athlete steps are recorded
///   `not_applicable` as the wizard records them, and the athlete's agent
///   sign-up picked is released as every surface releases it;
/// - the onboarding steps the two settle, recorded `complete`: the role
///   question ([`STEP_PROFILE_TYPE`]) and the seat ([`STEP_COACH_SEAT`]).
///   What is left is the coach's group and its athlete invite.
///
/// # Errors
///
/// Returns the repository's error for the first write that fails; the writes
/// before it stay.
pub async fn settle_coach_seat(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    operator: Option<Uuid>,
) -> AppResult<()> {
    set_user_manages_roster(repos, user_id, true, operator).await?;
    repos.users.set_coaches_others(user_id, true).await?;

    let user_key = user_id.to_string();
    let tenant = tenant_id.to_string();
    let steps = [
        (STEP_ABOUT_YOU, STATUS_NOT_APPLICABLE),
        (STEP_PARQ, STATUS_NOT_APPLICABLE),
        (STEP_PROFILE_TYPE, STATUS_COMPLETE),
        (STEP_COACH_SEAT, STATUS_COMPLETE),
    ];
    for (step_id, status) in steps {
        repos
            .user_onboarding
            .set_onboarding_step(&user_key, step_id, status, None, Some(&tenant))
            .await?;
    }
    release_coach_only_agent(
        repos.tenants.as_ref(),
        user_id,
        STEP_PARQ,
        STATUS_NOT_APPLICABLE,
    )
    .await?;

    info!(
        user_id = %user_id,
        operator_user_id = ?operator,
        "Coach-seat invitation settled: coach access granted and the coach role recorded"
    );
    Ok(())
}

/// Every invitation still redeemable now, keyed by address.
///
/// # Errors
///
/// Returns the repository's error when the listing fails.
pub(crate) async fn live_by_email(
    repos: &RepositoryRegistry,
) -> AppResult<HashMap<String, UserInvitation>> {
    Ok(repos
        .user_invitations
        .list_live(Utc::now())
        .await?
        .into_iter()
        .map(|invitation| (invitation.email.clone(), invitation))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The web app's side of the link, relative to this crate.
    const WEB_INVITATION_LINK: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../frontend/src/utils/tokenLink.ts"
    );

    /// A parity check between two artefacts nothing at runtime joins: this
    /// crate builds the link, the web app routes it and reads its fragment, so
    /// the test reads the web app's module and compares it with the constant
    /// and the fragment key the link is built from here.
    #[test]
    fn the_web_app_routes_the_path_and_reads_the_fragment_the_link_carries() {
        let web = fs::read_to_string(WEB_INVITATION_LINK).expect("the web app's link module");
        assert!(
            web.contains(&format!(
                "export const ACCEPT_INVITATION_PATH = '{ACCEPT_INVITATION_PATH}';"
            )),
            "the web app must route {ACCEPT_INVITATION_PATH}"
        );
        let built = invitation_url("https://app.example", "Sel.Ver", InvitationKind::CoachSeat);
        let fragment = built
            .split_once('#')
            .map(|(_, fragment)| fragment)
            .expect("the link carries a fragment");
        for pair in fragment.split('&') {
            let (key, value) = pair
                .split_once('=')
                .expect("each fragment part is key=value");
            assert!(
                web.contains(&format!(".get('{key}')")),
                "the web app must read the `{key}` fragment key the link carries"
            );
            if key == "kind" {
                assert!(
                    web.contains(&format!("'{value}'")),
                    "the web app must know the `{value}` kind the link names"
                );
            }
        }
    }

    #[test]
    fn the_link_carries_the_token_in_the_fragment_of_the_accept_page() {
        let url = invitation_url(
            "https://app.dravr.ai",
            "AbCd1234.EfGh5678",
            InvitationKind::Account,
        );
        assert_eq!(
            url,
            "https://app.dravr.ai/accept-invite#token=AbCd1234.EfGh5678"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_frontend_url_does_not_double_the_separator() {
        let url = invitation_url("https://app.dravr.ai/", "sel.ver", InvitationKind::Account);
        assert_eq!(url, "https://app.dravr.ai/accept-invite#token=sel.ver");
    }

    #[test]
    fn a_token_outside_the_alphabet_is_percent_encoded_rather_than_breaking_the_link() {
        let url = invitation_url("https://app.dravr.ai", "a b&c#d", InvitationKind::Account);
        assert_eq!(
            url,
            "https://app.dravr.ai/accept-invite#token=a%20b%26c%23d"
        );
    }

    #[test]
    fn an_issued_invitation_links_to_its_own_token() {
        let issued = IssuedInvitation {
            email: "coach@example.com".to_owned(),
            token: "Sel1.Ver2".to_owned(),
            expires_at: Utc::now(),
            kind: InvitationKind::Account,
        };
        assert_eq!(
            issued.url("http://localhost:3000"),
            "http://localhost:3000/accept-invite#token=Sel1.Ver2"
        );
    }

    #[test]
    fn a_coach_seat_link_names_its_kind_after_the_token() {
        let issued = IssuedInvitation {
            email: "coach@example.com".to_owned(),
            token: "Sel1.Ver2".to_owned(),
            expires_at: Utc::now(),
            kind: InvitationKind::CoachSeat,
        };
        assert_eq!(
            issued.url("https://app.dravr.ai"),
            "https://app.dravr.ai/accept-invite#token=Sel1.Ver2&kind=coach_seat"
        );
    }
}
