// ABOUTME: Handler for /deleteaccount — the messaging path to self-serve account deletion, behind a typed-email step
// ABOUTME: Runs the same removal as the app (grants revoked upstream, every owned row gone) with the same refusals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/deleteaccount` and `/deleteaccount <email>`.
//!
//! The bare command is the warning: what a delete erases, which providers it
//! disconnects, what blocks it, and the exact confirmation to send. The
//! confirmation names the account's email, typed in a direct message so it is
//! never posted to a shared room. A messaging channel authenticates its sender
//! by a verified channel link, so the typed email is the deliberate step a
//! password is in the app — a password is never asked for in a chat
//! transcript.
//!
//! Deletion is [`user_removal::remove_user`], the path the app's
//! `POST /api/user/account-deletion` and the operator console run, so the
//! refusals are the same: an operator account is refused, and rows other
//! people rely on refuse the delete naming each, with nothing touched.

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_DELETE_ACCOUNT_BLOCKED,
    KEY_DELETE_ACCOUNT_BLOCKER_ADMIN_CONFIG_AUDIT,
    KEY_DELETE_ACCOUNT_BLOCKER_ADMIN_CONFIG_OVERRIDE, KEY_DELETE_ACCOUNT_BLOCKER_APPROVED_USER,
    KEY_DELETE_ACCOUNT_BLOCKER_AUTHORED_AGENT, KEY_DELETE_ACCOUNT_BLOCKER_BILLING_SUBSCRIPTION,
    KEY_DELETE_ACCOUNT_BLOCKER_COACHES_COACHING_GROUP,
    KEY_DELETE_ACCOUNT_BLOCKER_CREATED_GROUP_INVITE, KEY_DELETE_ACCOUNT_BLOCKER_LLM_CREDENTIALS,
    KEY_DELETE_ACCOUNT_BLOCKER_OWNS_COACHING_GROUP, KEY_DELETE_ACCOUNT_BLOCKER_OWNS_TENANT,
    KEY_DELETE_ACCOUNT_BLOCKER_TENANT_OAUTH_CREDENTIALS, KEY_DELETE_ACCOUNT_DM_ONLY,
    KEY_DELETE_ACCOUNT_DONE, KEY_DELETE_ACCOUNT_EMAIL_MISMATCH, KEY_DELETE_ACCOUNT_FAILED,
    KEY_DELETE_ACCOUNT_OPERATOR, KEY_DELETE_ACCOUNT_PROVIDERS, KEY_DELETE_ACCOUNT_WARNING,
};
use pierre_core::errors::AppError;
use pierre_core::models::{normalize_email, UserReference, UserReferenceKind};
use pierre_core::permissions::UserRole;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_services::user_removal::{self, UserRemoval};
use std::collections::BTreeSet;
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// The catalogue key a blocker's line reads as.
const fn blocker_key(kind: UserReferenceKind) -> &'static str {
    match kind {
        UserReferenceKind::OwnsCoachingGroup => KEY_DELETE_ACCOUNT_BLOCKER_OWNS_COACHING_GROUP,
        UserReferenceKind::CoachesCoachingGroup => {
            KEY_DELETE_ACCOUNT_BLOCKER_COACHES_COACHING_GROUP
        }
        UserReferenceKind::CreatedGroupInvite => KEY_DELETE_ACCOUNT_BLOCKER_CREATED_GROUP_INVITE,
        UserReferenceKind::AdminConfigOverride => KEY_DELETE_ACCOUNT_BLOCKER_ADMIN_CONFIG_OVERRIDE,
        UserReferenceKind::AdminConfigAudit => KEY_DELETE_ACCOUNT_BLOCKER_ADMIN_CONFIG_AUDIT,
        UserReferenceKind::TenantOAuthCredentials => {
            KEY_DELETE_ACCOUNT_BLOCKER_TENANT_OAUTH_CREDENTIALS
        }
        UserReferenceKind::LlmCredentials => KEY_DELETE_ACCOUNT_BLOCKER_LLM_CREDENTIALS,
        UserReferenceKind::ApprovedUser => KEY_DELETE_ACCOUNT_BLOCKER_APPROVED_USER,
        UserReferenceKind::OwnsTenant => KEY_DELETE_ACCOUNT_BLOCKER_OWNS_TENANT,
        UserReferenceKind::AuthoredAgent => KEY_DELETE_ACCOUNT_BLOCKER_AUTHORED_AGENT,
        UserReferenceKind::BillingSubscription => KEY_DELETE_ACCOUNT_BLOCKER_BILLING_SUBSCRIPTION,
    }
}

/// The refusal naming each blocker, one line each.
fn blocked_text(
    reg: &MessagingStringsRegistry,
    locale: &str,
    blockers: &[UserReference],
) -> String {
    let lines: Vec<String> = blockers
        .iter()
        .map(|b| {
            format!(
                "• {}",
                reg.render(blocker_key(b.kind), locale, &[&b.detail])
            )
        })
        .collect();
    reg.render(KEY_DELETE_ACCOUNT_BLOCKED, locale, &[&lines.join("\n")])
}

/// Handler for `/deleteaccount` — warn, then delete on the typed email.
///
/// LIMITATION(registre#798): `DeleteAccountHandler` leaves the Firebase identity
/// (`users.firebase_uid`) at Google; the server holds no Firebase Admin credential and a
/// chat channel has no Firebase session to delete it with.
pub struct DeleteAccountHandler;

#[async_trait]
impl CommandHandler for DeleteAccountHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let repos = ctx.ctx.repos();
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let user = repos
            .users
            .get_global(ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {}", ctx.user_id)))?;

        if user.is_admin || user.role != UserRole::User {
            return Ok(CommandResponse::text(reg.render(
                KEY_DELETE_ACCOUNT_OPERATOR,
                locale,
                &[],
            )));
        }

        let blockers = repos.users.deletion_blockers(user.id).await?;
        if !blockers.is_empty() {
            return Ok(CommandResponse::text(blocked_text(reg, locale, &blockers)));
        }

        let Some(typed) = ctx.args.first() else {
            let providers: BTreeSet<String> = user_removal::held_providers(repos, user.id)
                .await?
                .into_iter()
                .map(|held| held.provider)
                .collect();
            let mut text = reg.render(KEY_DELETE_ACCOUNT_WARNING, locale, &[&user.email]);
            if !providers.is_empty() {
                let names: Vec<String> = providers.into_iter().collect();
                text.push_str("\n\n");
                text.push_str(&reg.render(
                    KEY_DELETE_ACCOUNT_PROVIDERS,
                    locale,
                    &[&names.join(", ")],
                ));
            }
            return Ok(CommandResponse::rich_text(text));
        };

        if !ctx.is_direct_message {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_DELETE_ACCOUNT_DM_ONLY,
                locale,
                &[],
            )));
        }
        if normalize_email(typed) != normalize_email(&user.email) {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_DELETE_ACCOUNT_EMAIL_MISMATCH,
                locale,
                &[],
            )));
        }

        let runtime = &ctx.tool_runtime;
        let disconnector = OAuthService::new(runtime.data(), runtime.config().clone());
        let outcome = user_removal::remove_user(
            repos,
            Some(&disconnector),
            user.id,
            DisconnectReason::Athlete,
        )
        .await?;
        let text = match outcome {
            UserRemoval::Removed(report) => {
                info!(
                    user_id = %user.id,
                    channel = %ctx.channel_type,
                    disconnected = report.disconnected.len(),
                    "Account deleted by its owner from a chat command"
                );
                reg.render(KEY_DELETE_ACCOUNT_DONE, locale, &[])
            }
            UserRemoval::Blocked(blockers) => blocked_text(reg, locale, &blockers),
            UserRemoval::Interrupted(interruption) => {
                warn!(
                    user_id = %user.id,
                    error = %interruption.error,
                    "Account deletion from a chat command was interrupted"
                );
                reg.render(KEY_DELETE_ACCOUNT_FAILED, locale, &[])
            }
        };
        Ok(CommandResponse::text(text))
    }
}
