// ABOUTME: Email to the operators — who receives it, and the alert for a sign-up awaiting approval
// ABOUTME: Best-effort: a failed or unconfigured send is logged and never fails what raised it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Operator email.
//!
//! The decisions only a super-admin makes (granting coach access, approving a
//! new account) reach them by email as well as by Slack, so a Slack outage, a
//! bot dropped from the room or a renamed channel cannot silence them.

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use pierre_database::repositories::UserRepository;
use pierre_email::ResendEmailService;
use pierre_middleware::mask_email;
use tracing::{info, warn};

/// The admin console's user list under `frontend_url`, where pending accounts
/// and coach-access requests are decided.
#[must_use]
pub fn console_users_url(frontend_url: &str) -> String {
    format!("{}/#users", frontend_url.trim_end_matches('/'))
}

/// The addresses of every active super-admin ([`User::is_active_super_admin`]):
/// a suspended one is left out, since the mail names the applicant.
///
/// # Errors
///
/// Returns the repository's error when the admins cannot be listed.
pub async fn super_admin_emails(users: &dyn UserRepository) -> AppResult<Vec<String>> {
    Ok(users
        .list_admins()
        .await?
        .into_iter()
        .filter(User::is_active_super_admin)
        .map(|a| a.email)
        .collect())
}

/// Emails every super-admin when a new account waits for approval
/// (carnet#1056), next to the `user.pending_approval` Slack ping.
#[derive(Clone)]
pub struct PendingSignupMail {
    /// The process's one Resend service, shared with every other mailer the
    /// server context hands out and with each request's account service.
    email: Arc<ResendEmailService>,
    /// The console's user list, where the account is approved or rejected.
    review_url: Option<String>,
}

impl PendingSignupMail {
    /// A mailer sending through `email`, linking to the console under
    /// `frontend_url` when one is configured.
    #[must_use]
    pub fn new(email: Arc<ResendEmailService>, frontend_url: Option<&str>) -> Self {
        Self {
            email,
            review_url: frontend_url.map(console_users_url),
        }
    }

    /// Email every active super-admin about `applicant`. Best-effort: a
    /// failure is logged per recipient and never reaches the sign-up.
    pub async fn send(&self, users: &dyn UserRepository, applicant: &User) {
        let recipients = match super_admin_emails(users).await {
            Ok(recipients) => recipients,
            Err(e) => {
                warn!(user_id = %applicant.id, error = %e, "Could not list super-admins for a pending sign-up");
                return;
            }
        };
        if recipients.is_empty() {
            warn!(user_id = %applicant.id, "No active super-admin to email about a pending sign-up");
            return;
        }
        for to in &recipients {
            let sent = self
                .email
                .send_signup_awaiting_approval(
                    to,
                    &applicant.email,
                    applicant.display_name.as_deref(),
                    self.review_url.as_deref(),
                )
                .await;
            log_outcome(to, applicant, sent);
        }
    }
}

/// Log one operator email's outcome against the masked recipient.
fn log_outcome(to: &str, applicant: &User, sent: Result<(), AppError>) {
    let recipient = mask_email(to);
    match sent {
        Ok(()) => info!(%recipient, user_id = %applicant.id, "Pending sign-up email sent"),
        Err(e) => {
            warn!(%recipient, user_id = %applicant.id, error = %e, "Failed to send pending sign-up email");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::UserStatus;
    use pierre_core::permissions::UserRole;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::{create_test_admin_user, create_test_user};

    #[test]
    fn the_console_link_lands_on_the_user_list() {
        assert_eq!(
            console_users_url("https://app.dravr.ai/"),
            "https://app.dravr.ai/#users"
        );
    }

    #[tokio::test]
    async fn only_active_super_admins_are_operator_recipients() {
        let db = create_test_db().await.unwrap();
        let users = db.repositories().users.as_ref();

        let mut deciding = create_test_admin_user("deciding@example.com", None);
        deciding.role = UserRole::SuperAdmin;
        let mut departed = create_test_admin_user("departed@example.com", None);
        departed.role = UserRole::SuperAdmin;
        departed.is_active = false;
        let mut suspended = create_test_admin_user("suspended@example.com", None);
        suspended.role = UserRole::SuperAdmin;
        suspended.user_status = UserStatus::Suspended;
        let admin = create_test_admin_user("admin@example.com", None);
        let athlete = create_test_user("athlete@example.com", None);
        for user in [&deciding, &departed, &suspended, &admin, &athlete] {
            users.create(user).await.unwrap();
        }

        assert_eq!(
            super_admin_emails(users).await.unwrap(),
            vec!["deciding@example.com".to_owned()]
        );
    }
}
