// ABOUTME: Single-use invitations an operator mails to an address with no account — issue, revoke, list live, build the link
// ABOUTME: The invitee redeems one through AuthService::accept_invitation, choosing their own password

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
//! The token is a `<selector>.<verifier>` pair from [`crate::link_token`]:
//! only the selector and the verifier's SHA-256 reach the database, and the
//! whole token exists once, in the [`IssuedInvitation`] handed to whoever
//! mails it. Nothing here logs it.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::UserInvitation;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

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
}

impl fmt::Debug for IssuedInvitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedInvitation")
            .field("email", &self.email)
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl IssuedInvitation {
    /// The link the invitee follows, on the web app at `frontend_url`.
    #[must_use]
    pub fn url(&self, frontend_url: &str) -> String {
        invitation_url(frontend_url, &self.token)
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
/// address bar and clears it before anything else loads.
#[must_use]
fn invitation_url(frontend_url: &str, token: &str) -> String {
    format!(
        "{}{ACCEPT_INVITATION_PATH}#token={}",
        frontend_url.trim_end_matches('/'),
        urlencoding::encode(token)
    )
}

/// Issue a single-use invitation to an address with no account.
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
        )
        .await?;
    info!(invited_by = ?invited_by, "Invitation issued");

    Ok(InviteOutcome::Issued(IssuedInvitation {
        email: invitation.email,
        token: generated.token,
        expires_at: invitation.expires_at,
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
        let built = invitation_url("https://app.example", "Sel.Ver");
        let key = built
            .split_once('#')
            .and_then(|(_, fragment)| fragment.split_once('='))
            .map(|(key, _)| key)
            .expect("the link carries a key=value fragment");
        assert!(
            web.contains(&format!(".get('{key}')")),
            "the web app must read the `{key}` fragment key the link carries"
        );
    }

    #[test]
    fn the_link_carries_the_token_in_the_fragment_of_the_accept_page() {
        let url = invitation_url("https://app.dravr.ai", "AbCd1234.EfGh5678");
        assert_eq!(
            url,
            "https://app.dravr.ai/accept-invite#token=AbCd1234.EfGh5678"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_frontend_url_does_not_double_the_separator() {
        let url = invitation_url("https://app.dravr.ai/", "sel.ver");
        assert_eq!(url, "https://app.dravr.ai/accept-invite#token=sel.ver");
    }

    #[test]
    fn a_token_outside_the_alphabet_is_percent_encoded_rather_than_breaking_the_link() {
        let url = invitation_url("https://app.dravr.ai", "a b&c#d");
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
        };
        assert_eq!(
            issued.url("http://localhost:3000"),
            "http://localhost:3000/accept-invite#token=Sel1.Ver2"
        );
    }
}
