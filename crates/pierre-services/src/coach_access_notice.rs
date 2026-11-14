// ABOUTME: Tells a coach how their coach-access request was decided — the room line, the in-app notice and the email
// ABOUTME: Best-effort once the decision has landed: a part that cannot be sent is logged and the decision stands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach told how their request was decided (carnet#1521)
//!
//! A super-admin's decision on a coach-access request used to reach only the
//! operators' notify channel: the coach found out by coming back, and only
//! after signing in again. [`CoachAccessNotices`] closes that loop for both
//! decision paths (the console queue and the Slack card), which hand it to
//! [`crate::coach_access::grant`] and [`crate::coach_access::decline`]:
//!
//! - **granted**: a line in every copy of each group the grant made them the
//!   coach of — their own and each athlete's, in each reader's language, the
//!   athletes' saying what the coach now sees and where they stop it — then
//!   an in-app notice whose button opens their coach Home, and an email
//!   naming those groups;
//! - **declined**: an in-app notice and an email, each with the super-admin's
//!   reason when one was given.
//!
//! The room line is written the way the athlete-joined notice is: a
//! platform-authored row in each reader's own copy of the group's chat, so it
//! reads as unread there and never enters the shared transcript, whose
//! entries are attributed to a member and filtered by their consent. A member
//! with no copy yet is told nothing here; the app files one on first open.
//!
//! Every part is best-effort. The decision has landed before any of this
//! runs, so a deployment without a notification or mail service, an account
//! that cannot be read, or a send that fails is logged and the decision
//! stands.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_COACH_ACCESS_ROOM_LINE_ATHLETE, KEY_COACH_ACCESS_ROOM_LINE_COACH,
};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{roster_display_name, AddMessageParams, CoachAccessRequest, User};
use pierre_core::transport::TransportPolicy;
use pierre_core::untrusted::{self, ROSTER_NAME_MAX_CHARS};
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_middleware::redaction::mask_email;
use pierre_notifications::triggers::{trigger_coach_access_declined, trigger_coach_access_granted};
use pierre_notifications::{NotificationService, TenantId as NoticeTenantId};
use tracing::{info, warn};
use uuid::Uuid;

use crate::coach_access;
use crate::locale::user_locale;

/// What a decided coach-access request needs to reach its coach.
///
/// The in-app notification service, the mail service, the string catalogue
/// the room line is rendered from, and the frontend origin the email links
/// under. Built once by the composition root and handed to both decision paths, so
/// the console and the Slack card tell the coach the same way.
pub struct CoachAccessNotices {
    /// The in-app notification facade; `None` when the deployment has none.
    pub notifications: Option<Arc<NotificationService>>,
    /// The transactional mail service; `None` when Resend is unconfigured.
    pub email: Option<Arc<ResendEmailService>>,
    /// The live string catalogue the room line is rendered from.
    pub strings: Arc<MessagingStringsRegistry>,
    /// The public frontend origin the email's button opens under.
    pub frontend_url: Option<String>,
}

impl fmt::Debug for CoachAccessNotices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoachAccessNotices")
            .field("notifications", &self.notifications.is_some())
            .field("email", &self.email.is_some())
            .field("frontend_url", &self.frontend_url)
            .finish_non_exhaustive()
    }
}

impl CoachAccessNotices {
    /// Tell the coach behind the granted `request` that they are a coach now:
    /// the room line in every copy of each of `attached` groups' chats, then
    /// the in-app notice and the email. Where each part landed is logged.
    pub async fn tell_granted(
        &self,
        repos: &RepositoryRegistry,
        request: &CoachAccessRequest,
        attached: &[CoachingGroup],
    ) {
        let Some(coach) = requester(repos, request).await else {
            return;
        };
        let mut room_lines = 0;
        for group in attached {
            match self.post_room_line(repos, &coach, group).await {
                Ok(delivered) => room_lines += delivered,
                Err(e) => warn!(
                    request_id = %request.id,
                    group_id = %group.id,
                    error = %e,
                    "Coach access grant: the room line could not be posted"
                ),
            }
        }
        let notified = match (&self.notifications, feed_tenant(repos, coach.id).await) {
            (Some(service), Some(tenant)) => {
                trigger_coach_access_granted(service, coach.id, tenant);
                true
            }
            _ => false,
        };
        let group_names: Vec<String> = attached.iter().map(|g| g.name.clone()).collect();
        let emailed = self.email_granted(&coach, request.id, &group_names).await;
        info!(
            request_id = %request.id,
            target_user_id = %coach.id,
            groups = attached.len(),
            room_lines,
            notified,
            emailed,
            "Coach access grant told to its coach"
        );
    }

    /// Tell the coach behind the declined `request` that the team said no,
    /// with the reason stored on it when one was given.
    pub async fn tell_declined(&self, repos: &RepositoryRegistry, request: &CoachAccessRequest) {
        let Some(coach) = requester(repos, request).await else {
            return;
        };
        let reason = request.decline_reason.as_deref();
        let notified = match (&self.notifications, feed_tenant(repos, coach.id).await) {
            (Some(service), Some(tenant)) => {
                trigger_coach_access_declined(service, coach.id, tenant, reason);
                true
            }
            _ => false,
        };
        let emailed = self.email_declined(&coach, request.id, reason).await;
        info!(
            request_id = %request.id,
            target_user_id = %coach.id,
            with_reason = reason.is_some(),
            notified,
            emailed,
            "Coach access decline told to its coach"
        );
    }

    /// Write the grant's line into every copy of `group`'s chat: the coach's
    /// own and each live member's, each in its reader's language. Returns how
    /// many copies received it.
    async fn post_room_line(
        &self,
        repos: &RepositoryRegistry,
        coach: &User,
        group: &CoachingGroup,
    ) -> AppResult<usize> {
        // The name is whatever the coach typed: one bounded line, escaped as
        // Markdown since every client draws the row as Markdown.
        let name = untrusted::markdown_literal(&untrusted::display_line(
            &roster_display_name(coach.display_name.as_deref(), &coach.email),
            ROSTER_NAME_MAX_CHARS,
        ));
        let mut readers: Vec<Uuid> = vec![coach.id];
        let mut seen: HashSet<Uuid> = HashSet::from([coach.id]);
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if seen.insert(member.user_id) {
                readers.push(member.user_id);
            }
        }
        let users = repos.users.get_global_many(&readers).await?;
        let group_id = group.id.to_string();
        let mut delivered = 0_usize;
        for reader in readers {
            let Some(user) = users.get(&reader) else {
                continue;
            };
            let key = if reader == coach.id {
                KEY_COACH_ACCESS_ROOM_LINE_COACH
            } else {
                KEY_COACH_ACCESS_ROOM_LINE_ATHLETE
            };
            let text = self.strings.render(key, &user_locale(Some(user)), &[&name]);
            if text.trim().is_empty() {
                continue;
            }
            delivered += write_into_copies(repos, reader, &group_id, &text).await?;
        }
        Ok(delivered)
    }

    /// Send the granted email; whether it went out (each outcome logged).
    async fn email_granted(&self, coach: &User, request_id: Uuid, group_names: &[String]) -> bool {
        let recipient = mask_email(&coach.email);
        let Some(email) = &self.email else {
            warn!(%recipient, %request_id, "Email service not configured — skipping the coach access granted email");
            return false;
        };
        let home = self.frontend_url.as_deref().map(coach_home_url);
        match email
            .send_coach_access_granted(
                &coach.email,
                coach.display_name.as_deref(),
                group_names,
                home.as_deref(),
            )
            .await
        {
            Ok(()) => {
                info!(%recipient, %request_id, "Coach access granted email sent");
                true
            }
            Err(e) => {
                warn!(%recipient, %request_id, error = %e, "Failed to send the coach access granted email");
                false
            }
        }
    }

    /// Send the declined email; whether it went out (each outcome logged).
    async fn email_declined(&self, coach: &User, request_id: Uuid, reason: Option<&str>) -> bool {
        let recipient = mask_email(&coach.email);
        let Some(email) = &self.email else {
            warn!(%recipient, %request_id, "Email service not configured — skipping the coach access declined email");
            return false;
        };
        let ask_again = self.frontend_url.as_deref().map(ask_again_url);
        match email
            .send_coach_access_declined(
                &coach.email,
                coach.display_name.as_deref(),
                reason,
                ask_again.as_deref(),
            )
            .await
        {
            Ok(()) => {
                info!(%recipient, %request_id, "Coach access declined email sent");
                true
            }
            Err(e) => {
                warn!(%recipient, %request_id, error = %e, "Failed to send the coach access declined email");
                false
            }
        }
    }
}

/// Write `text` as a platform-authored row into every copy of the group's
/// chat `reader` owns, one per tenant they hold one in, and return how many
/// received it.
async fn write_into_copies(
    repos: &RepositoryRegistry,
    reader: Uuid,
    group_id: &str,
    text: &str,
) -> AppResult<usize> {
    let reader_id = reader.to_string();
    let mut delivered = 0_usize;
    for tenant in repos.tenants.list_for_user(reader).await? {
        let Some(conversation) = repos
            .chat
            .find_group_conversation(&reader_id, tenant.id, group_id)
            .await?
        else {
            continue;
        };
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: tenant.id,
                conversation_id: &conversation.id,
                user_id: &reader_id,
                role: "assistant",
                content: text,
                // Platform-authored, like the athlete-joined notice: no model
                // answered and no tokens were spent.
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                // A name and nothing derived from provider data.
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
                turn_attempted_write: None,
            })
            .await?;
        delivered += 1;
    }
    Ok(delivered)
}

/// The requester's account, or `None` (logged) when it cannot be read.
async fn requester(repos: &RepositoryRegistry, request: &CoachAccessRequest) -> Option<User> {
    match repos.users.get_global(request.user_id).await {
        Ok(Some(user)) => Some(user),
        Ok(None) => {
            warn!(request_id = %request.id, "Coach access decision not told: the requester's account is gone");
            None
        }
        Err(e) => {
            warn!(request_id = %request.id, error = %e, "Coach access decision not told: requester read failed");
            None
        }
    }
}

/// The tenant the coach's feed is read in — their home tenant
/// ([`coach_access::home_tenant`]) — as the notification pipeline names it;
/// `None` (logged there) when it cannot be read.
async fn feed_tenant(repos: &RepositoryRegistry, user_id: Uuid) -> Option<NoticeTenantId> {
    coach_access::home_tenant(repos, user_id)
        .await
        .map(|tenant| NoticeTenantId(tenant.as_uuid()))
}

/// The coach's Home under `frontend_url`, which a grant opens.
#[must_use]
fn coach_home_url(frontend_url: &str) -> String {
    format!("{}/#home", frontend_url.trim_end_matches('/'))
}

/// Where a declined coach asks again under `frontend_url`: the coaching pane
/// of Settings, which holds the request for a coach with no group too.
#[must_use]
fn ask_again_url(frontend_url: &str) -> String {
    format!("{}/#settings/coaching", frontend_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_email_links_open_coach_home_and_the_coaching_pane() {
        assert_eq!(
            coach_home_url("https://app.dravr.ai/"),
            "https://app.dravr.ai/#home"
        );
        assert_eq!(
            ask_again_url("https://app.dravr.ai"),
            "https://app.dravr.ai/#settings/coaching"
        );
    }
}
