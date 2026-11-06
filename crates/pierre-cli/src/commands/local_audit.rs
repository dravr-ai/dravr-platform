// ABOUTME: The admin audit trail for pierre-cli commands that write the database directly (--database-url)
// ABOUTME: One audit_events row per write command, success or failure; reads write nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli` reaches a deployed server over the admin API for most of what
//! it does, and those calls are recorded by the server's audit layer. The
//! commands below open the database themselves instead, so they write their
//! own row of the same trail (carnet#1391), with the source `cli_database`:
//! there is no signed-in operator, so the row names the machine's account
//! (`$USER`) in its metadata, and never a password.
//!
//! Not covered, because they run before the repository registry this module
//! writes through exists: `seed` (reference and demo data, and the first-run
//! bootstrap) and `key rotate-dek` / `key rewrap-kek` (each bootstraps its own
//! key management against the database; existing data stays decryptable).
//! `user mint-session` writes only to a local `SQLite` file and is refused
//! against any other database.

use std::env;

use pierre_database::repositories::{AuditOutcome, AuditSource, NewAuditEvent};
use pierre_database::RepositoryRegistry;
use serde_json::{json, Value};
use tracing::warn;

use crate::commands::garmin::GarminCommand;
#[cfg(feature = "tools-verification")]
use crate::commands::harness::HarnessCommand;
use crate::commands::tenant::TenantCommand;
use crate::commands::zones::ZonesCommand;
use crate::{Command, TokenCommand, ToolCommand, UserCommand};

/// What one write command will record.
#[derive(Debug)]
pub(crate) struct LocalAudit {
    action: &'static str,
    summary: String,
    target: Value,
}

impl LocalAudit {
    fn new(action: &'static str, summary: String, target: Value) -> Self {
        Self {
            action,
            summary,
            target,
        }
    }
}

/// What `command` will record, or `None` for a command that only reads.
pub(crate) fn describe(command: &Command) -> Option<LocalAudit> {
    match command {
        Command::User { action } => describe_user(action),
        Command::Token { action } => describe_token(action),
        Command::Tool { action } => describe_tool(action),
        Command::Tenant { action } => describe_tenant(action),
        Command::Zones {
            action:
                ZonesCommand::Backfill {
                    tenant_id,
                    apply: true,
                    ..
                },
        } => Some(LocalAudit::new(
            "zones.backfill",
            format!("Re-cut stored zones for tenant {tenant_id}"),
            json!({ "tenant_id": tenant_id }),
        )),
        Command::Garmin {
            action:
                GarminCommand::RetireHistorySessions {
                    tenant_id,
                    dry_run: false,
                },
        } => Some(LocalAudit::new(
            "garmin.retire_history_sessions",
            format!("Retired the Garmin sign-in sessions beside a grant in tenant {tenant_id}"),
            json!({ "tenant_id": tenant_id }),
        )),
        #[cfg(feature = "tools-verification")]
        Command::Harness { action } => describe_harness(action),
        _ => None,
    }
}

/// The `user` verbs that write the database.
fn describe_user(action: &UserCommand) -> Option<LocalAudit> {
    let audit = match action {
        UserCommand::Create {
            email, super_admin, ..
        } => LocalAudit::new(
            "user.create_admin",
            format!("Created admin account {email}"),
            json!({ "email": email, "super_admin": super_admin }),
        ),
        UserCommand::Promote { email } => LocalAudit::new(
            "user.promote",
            format!("Promoted {email} to admin"),
            json!({ "email": email }),
        ),
        UserCommand::Demote { email } => LocalAudit::new(
            "user.demote",
            format!("Demoted {email} to a regular user"),
            json!({ "email": email }),
        ),
        UserCommand::SetTier { email, tier, .. } => LocalAudit::new(
            "user.tier.set",
            format!("Set {email}'s tier to {tier}"),
            json!({ "email": email, "tier": tier }),
        ),
        UserCommand::ClearTier { email } => LocalAudit::new(
            "user.tier.clear",
            format!("Cleared {email}'s tier override"),
            json!({ "email": email }),
        ),
        UserCommand::Approve { email, reason } => LocalAudit::new(
            "user.approve",
            format!("Approved {email}"),
            json!({ "email": email, "reason": reason }),
        ),
        UserCommand::Suspend { email, reason } => LocalAudit::new(
            "user.suspend",
            format!("Suspended {email}"),
            json!({ "email": email, "reason": reason }),
        ),
        UserCommand::ResetPassword { email } => LocalAudit::new(
            "user.password_reset_token",
            format!("Issued a password reset token for {email}"),
            json!({ "email": email }),
        ),
        UserCommand::SetRateLimit { email, monthly, .. } => LocalAudit::new(
            "user.rate_limit.set",
            format!("Set {email}'s monthly rate limit"),
            json!({ "email": email, "monthly": monthly }),
        ),
        UserCommand::ClearRateLimit { email } => LocalAudit::new(
            "user.rate_limit.clear",
            format!("Cleared {email}'s rate-limit override"),
            json!({ "email": email }),
        ),
        UserCommand::SetFeature {
            email,
            key,
            enabled,
        } => LocalAudit::new(
            "user.feature.set",
            format!("Set feature {key} for {email}"),
            json!({ "email": email, "key": key, "enabled": enabled }),
        ),
        UserCommand::ClearFeature { email, key } => LocalAudit::new(
            "user.feature.clear",
            format!("Cleared feature {key} for {email}"),
            json!({ "email": email, "key": key }),
        ),
        _ => return None,
    };
    Some(audit)
}

/// The `token` verbs that write the database.
fn describe_token(action: &TokenCommand) -> Option<LocalAudit> {
    let audit = match action {
        TokenCommand::Generate {
            service,
            super_admin,
            tenant_id,
            ..
        } => LocalAudit::new(
            "admin_token.create",
            format!("Generated admin token for {service}"),
            json!({ "service": service, "super_admin": super_admin, "tenant_id": tenant_id }),
        ),
        TokenCommand::Revoke { token_id } => LocalAudit::new(
            "admin_token.revoke",
            format!("Revoked admin token {token_id}"),
            json!({ "token_id": token_id }),
        ),
        TokenCommand::Rotate { token_id, .. } => LocalAudit::new(
            "admin_token.rotate",
            format!("Rotated admin token {token_id}"),
            json!({ "token_id": token_id }),
        ),
        _ => return None,
    };
    Some(audit)
}

/// The `tool` verbs that write the database.
fn describe_tool(action: &ToolCommand) -> Option<LocalAudit> {
    let audit = match action {
        ToolCommand::Enable { email, tool, .. } => LocalAudit::new(
            "user.tool.enable",
            format!("Enabled tool {tool} for {email}"),
            json!({ "email": email, "tool": tool }),
        ),
        ToolCommand::Disable { email, tool, .. } => LocalAudit::new(
            "user.tool.disable",
            format!("Disabled tool {tool} for {email}"),
            json!({ "email": email, "tool": tool }),
        ),
        ToolCommand::Reset { email, tool } => LocalAudit::new(
            "user.tool.reset",
            format!("Reset tool {tool} for {email}"),
            json!({ "email": email, "tool": tool }),
        ),
        _ => return None,
    };
    Some(audit)
}

/// The `tenant` verbs that write the database.
fn describe_tenant(action: &TenantCommand) -> Option<LocalAudit> {
    let audit = match action {
        TenantCommand::SetPlan { email, plan, .. } => LocalAudit::new(
            "tenant.plan.set",
            format!("Set {email}'s tenant plan to {plan}"),
            json!({ "email": email, "plan": plan }),
        ),
        TenantCommand::EnableTool { email, tool, .. }
        | TenantCommand::DisableTool { email, tool, .. }
        | TenantCommand::ResetTool { email, tool, .. } => LocalAudit::new(
            "tenant.tool.override",
            format!("Changed tool {tool} on {email}'s tenant"),
            json!({ "email": email, "tool": tool }),
        ),
        TenantCommand::SetFeature { email, key, .. }
        | TenantCommand::ClearFeature { email, key, .. } => LocalAudit::new(
            "tenant.feature.override",
            format!("Changed feature {key} on {email}'s tenant"),
            json!({ "email": email, "key": key }),
        ),
        TenantCommand::SetOauthApp {
            email, provider, ..
        } => LocalAudit::new(
            "tenant.oauth_app.set",
            format!("Set the {provider} OAuth app on {email}'s tenant"),
            json!({ "email": email, "provider": provider }),
        ),
        _ => return None,
    };
    Some(audit)
}

/// The `harness` verbs that write the database: a verdict backfill that
/// persists, and a memory dedup that applies its merges.
#[cfg(feature = "tools-verification")]
fn describe_harness(action: &HarnessCommand) -> Option<LocalAudit> {
    match action {
        HarnessCommand::BackfillVerdicts {
            tenant_id,
            dry_run: false,
            ..
        } => Some(LocalAudit::new(
            "harness.backfill_verdicts",
            format!("Backfilled claim verdicts for tenant {tenant_id}"),
            json!({ "tenant_id": tenant_id }),
        )),
        HarnessCommand::DedupMemory {
            tenant_id,
            user_id,
            apply: true,
            ..
        } => Some(LocalAudit::new(
            "harness.dedup_memory",
            format!("Folded duplicate memory facts of user {user_id}"),
            json!({ "tenant_id": tenant_id, "user_id": user_id }),
        )),
        _ => None,
    }
}

/// Write the row for a command that ran: `succeeded` is whether it returned
/// without an error. A failed write is logged and never changes the command's
/// own result.
pub(crate) async fn record(repos: &RepositoryRegistry, audit: LocalAudit, succeeded: bool) {
    let outcome = if succeeded {
        AuditOutcome::Success
    } else {
        AuditOutcome::Failed
    };
    let event = NewAuditEvent {
        action: audit.action.to_owned(),
        summary: audit.summary,
        outcome,
        source: AuditSource::CliDatabase,
        actor_user_id: None,
        tenant_id: None,
        ip_address: None,
        user_agent: Some(format!("pierre-cli/{}", env!("CARGO_PKG_VERSION"))),
        metadata: json!({
            "target": audit.target,
            "os_user": env::var("USER").ok(),
        }),
    };
    if let Err(e) = repos.admin_audit.record(&event).await {
        warn!(error = %e, action = %event.action, "Failed to record the command in the admin audit trail");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_command_is_described_and_a_read_is_not() {
        let promote = describe(&Command::User {
            action: UserCommand::Promote {
                email: "ops@example.com".to_owned(),
            },
        })
        .expect("a promote writes the database, so it is audited");
        assert_eq!(promote.action, "user.promote", "bug: wrong action key");
        assert_eq!(
            promote.target["email"], "ops@example.com",
            "bug: the row must name the promoted account"
        );
        assert!(
            describe(&Command::User {
                action: UserCommand::ListAdmins,
            })
            .is_none(),
            "bug: a read-only command must write no audit row"
        );
    }

    #[test]
    fn a_zones_backfill_is_audited_only_when_it_applies() {
        let backfill = |apply| Command::Zones {
            action: ZonesCommand::Backfill {
                tenant_id: "t-1".to_owned(),
                apply,
            },
        };
        assert_eq!(
            describe(&backfill(true)).map(|a| a.action),
            Some("zones.backfill"),
            "bug: an applied backfill rewrites stored zones"
        );
        assert!(
            describe(&backfill(false)).is_none(),
            "bug: a dry run changes nothing and writes no row"
        );
    }

    #[test]
    fn a_garmin_session_retirement_is_audited_only_when_it_writes() {
        let retire = |dry_run| Command::Garmin {
            action: GarminCommand::RetireHistorySessions {
                tenant_id: "t-1".to_owned(),
                dry_run,
            },
        };
        assert_eq!(
            describe(&retire(false)).map(|a| a.action),
            Some("garmin.retire_history_sessions"),
            "bug: a retirement deletes session rows"
        );
        assert!(
            describe(&retire(true)).is_none(),
            "bug: a dry run only lists the sessions and writes no row"
        );
    }
}
