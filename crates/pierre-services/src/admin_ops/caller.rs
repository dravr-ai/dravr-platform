// ABOUTME: Who acts on an account through the service layer, built once per surface, and the rank rule it carries
// ABOUTME: An admin account takes a platform caller; the account writes take the account from the caller, so none skips the rule

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One value says who is acting on an account (carnet#1589).
//!
//! The account writes of this crate (a status change, an approval, a
//! pre-approval, a password reset, a provider disconnect, an onboarding
//! reset, a deletion) are reached from several surfaces: an admin's console
//! session, an admin token, `pierre-cli` on the host, the Slack sign-up
//! room, a sign-up rule, the account's own holder. Each surface builds an
//! [`AccountCaller`] once. It carries what the writes record and what they
//! refuse:
//!
//! - the account written against the change (`approved_by`, `allowed_by`),
//!   and the name the logs, the announcements and a reset token carry;
//! - the admin whose own session this is, when there is one: no admin
//!   takes itself out of the active status;
//! - whether the caller may act on an admin account: **the rank rule**.
//!
//! The rank rule: suspending, approving, deleting, disconnecting or
//! resetting an admin account takes from an admin, or hands whoever asks,
//! every power. So it takes a platform caller: an admin's console session,
//! a device login, a super-admin service token, or `pierre-cli` on the
//! host. It is the caller the scope rule calls
//! [`ScopeCaller::Platform`](pierre_core::admin::scope::ScopeCaller::Platform).
//! A scoped service token, the Slack room and a sign-up rule are refused an
//! admin account. An account's own holder acts on its own account only,
//! whatever its role: deleting one's own account is every holder's right,
//! an admin's included, and the last active admin is still refused by the
//! deletion itself.
//!
//! The rule is applied in one place, [`AccountCaller::acting_on`], which
//! hands back the account as an [`ActingOn`]. Every account write takes an
//! `ActingOn` and nothing else builds one, so a surface cannot reach a
//! write without the rule having read the account it writes.
//!
//! What the rule reads is the account as the surface loaded it. The writes
//! read the account again for what they guard themselves (its status, the
//! last active admin), not for its role: an account promoted between the
//! surface's read and the write is acted on by a caller the rule would
//! refuse now.

use pierre_core::admin::models::{ValidatedAdminToken, DEVICE_CLI_SERVICE_PREFIX};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::User;
use pierre_database::repositories::{AuditSource, DeletionActor};
use pierre_database::RepositoryRegistry;
use tracing::warn;
use uuid::Uuid;

use super::credentials::{
    record_withdrawn_admin_credentials, AdminStandingLost, StandingChange,
    WithdrawnAdminCredentials,
};

/// The name `pierre-cli` on the host is recorded under.
const HOST_CLI_NAME: &str = "pierre-cli";

/// The name a sign-up rule is logged under.
const SIGN_UP_RULE_NAME: &str = "sign-up rule";

/// Where a validated admin credential came from, for the audit trail.
#[must_use]
pub fn token_audit_source(token: &ValidatedAdminToken) -> AuditSource {
    if token.token_id.starts_with("cookie:") {
        AuditSource::Console
    } else if token.service_name.starts_with(DEVICE_CLI_SERVICE_PREFIX) {
        AuditSource::Cli
    } else {
        AuditSource::Service
    }
}

/// The surface a caller acts through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    /// A validated admin credential: a console session, a device login or
    /// a service token, told apart by [`token_audit_source`].
    AdminCredential(AuditSource),
    /// `pierre-cli` writing the database on the host.
    HostCli,
    /// The Slack sign-up room, whose membership is its authorization.
    SlackRoom,
    /// A rule that admits a sign-up nobody decided: a standing allow, a
    /// signup code, a proven address, an auto-approved domain.
    SignUpRule,
    /// The holder of the account it names, from the app or from a
    /// messaging channel.
    AccountHolder(DeletionActor, Uuid),
}

/// Who is acting on an account. See the module docs.
#[derive(Debug, Clone)]
pub struct AccountCaller {
    surface: Surface,
    /// Whether the caller may act on an admin account.
    platform: bool,
    acting_admin: Option<Uuid>,
    recorded_as: Option<Uuid>,
    name: String,
}

impl AccountCaller {
    /// The caller behind a validated admin credential: an admin's console
    /// session, a device login, or a service token.
    ///
    /// A console session and a device login act as their admin and carry
    /// the super-admin flag. A service token names no admin; the flag
    /// stored on it is its rank.
    #[must_use]
    pub fn of_token(token: &ValidatedAdminToken) -> Self {
        Self {
            surface: Surface::AdminCredential(token_audit_source(token)),
            platform: token.is_super_admin,
            acting_admin: token.operator_user_id,
            recorded_as: token.operator_user_id,
            name: token.service_name.clone(),
        }
    }

    /// `pierre-cli` writing the database on the host: whoever holds the
    /// host shell holds every power, and no admin's session is behind it.
    /// `recorded_as` is the admin its changes are written against, which
    /// it looks up; `None` on a database with no admin yet.
    #[must_use]
    pub fn host_cli(recorded_as: Option<Uuid>) -> Self {
        Self {
            surface: Surface::HostCli,
            platform: true,
            acting_admin: None,
            recorded_as,
            name: HOST_CLI_NAME.to_owned(),
        }
    }

    /// The Slack sign-up room, on a tap by `handle`. The room decides
    /// sign-ups and names no admin account.
    #[must_use]
    pub fn slack_room(handle: impl Into<String>) -> Self {
        Self {
            surface: Surface::SlackRoom,
            platform: false,
            acting_admin: None,
            recorded_as: None,
            name: handle.into(),
        }
    }

    /// A sign-up rule admitting an account. `approver` is the admin the
    /// activation is written against: whoever recorded the allow or minted
    /// the code, while that account exists.
    #[must_use]
    pub fn sign_up_rule(approver: Option<Uuid>) -> Self {
        Self {
            surface: Surface::SignUpRule,
            platform: false,
            acting_admin: None,
            recorded_as: approver,
            name: SIGN_UP_RULE_NAME.to_owned(),
        }
    }

    /// The holder of `account`, from a signed-in session of the app: it
    /// acts on that account and no other.
    #[must_use]
    pub fn account_holder(account: Uuid) -> Self {
        Self::holder(DeletionActor::Athlete, account, "account holder")
    }

    /// The holder of `account`, from a messaging channel.
    #[must_use]
    pub fn account_holder_on_messaging(account: Uuid) -> Self {
        Self::holder(
            DeletionActor::Messaging,
            account,
            "account holder (messaging)",
        )
    }

    fn holder(actor: DeletionActor, account: Uuid, name: &str) -> Self {
        Self {
            surface: Surface::AccountHolder(actor, account),
            platform: false,
            acting_admin: None,
            recorded_as: None,
            name: name.to_owned(),
        }
    }

    /// Take `account` as the one this caller acts on: the rank rule.
    ///
    /// The only way to an [`ActingOn`], which every account write takes.
    /// An account's holder takes its own account, an admin's too, and no
    /// other.
    ///
    /// # Errors
    ///
    /// Returns a permission error when `account` is an admin and the caller
    /// is neither a platform one nor the account's holder, and when an
    /// account holder names another account.
    pub fn acting_on(&self, account: User) -> AppResult<ActingOn<'_>> {
        let holder_of = match self.surface {
            Surface::AccountHolder(_, own) => Some(own),
            Surface::AdminCredential(_)
            | Surface::HostCli
            | Surface::SlackRoom
            | Surface::SignUpRule => None,
        };
        let own_account = holder_of == Some(account.id);
        if !self.platform && !own_account && account.role.is_admin() {
            warn!(
                user_id = %account.id,
                caller = %self.name,
                "Write to an admin account refused: the caller is not a super-admin one"
            );
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Acting on an admin account requires a super-admin token",
            ));
        }
        if holder_of.is_some() && !own_account {
            warn!(
                user_id = %account.id,
                caller = %self.name,
                "An account holder named an account other than its own"
            );
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "An account holder acts on its own account only",
            ));
        }
        Ok(ActingOn {
            caller: self,
            account,
        })
    }

    /// Whether this caller decides sign-ups and nothing else: the Slack
    /// sign-up room and a sign-up rule. Its status writes land only on an
    /// account still pending ([`super::decide_sign_up`]); an admin's
    /// decision moves any status ([`super::transition_user_status`]).
    #[must_use]
    pub const fn decides_sign_ups_only(&self) -> bool {
        match self.surface {
            Surface::SlackRoom | Surface::SignUpRule => true,
            Surface::AdminCredential(_) | Surface::HostCli | Surface::AccountHolder(..) => false,
        }
    }

    /// The admin whose own session or device login this is. `None` for a
    /// caller no admin's session is behind, which has no self to refuse.
    #[must_use]
    pub const fn acting_admin(&self) -> Option<Uuid> {
        self.acting_admin
    }

    /// The account a change is written against (`approved_by`,
    /// `allowed_by`): the acting admin, or the admin a caller with no
    /// session names. `None` for a service token and for the Slack room.
    #[must_use]
    pub const fn recorded_as(&self) -> Option<Uuid> {
        self.recorded_as
    }

    /// The caller's name in logs and announcements: a console admin's
    /// email, a token's service name, a Slack handle, `pierre-cli`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The identity a password reset token is issued under: the acting
    /// admin's id, or the caller's name when no admin is behind it.
    #[must_use]
    pub fn audit_name(&self) -> String {
        self.acting_admin
            .map_or_else(|| self.name.clone(), |admin| admin.to_string())
    }

    /// Who a deletion receipt says asked: the holder by its channel, an
    /// admin surface otherwise. A persisted value.
    #[must_use]
    pub const fn deletion_actor(&self) -> DeletionActor {
        match self.surface {
            Surface::AccountHolder(actor, _) => actor,
            Surface::AdminCredential(_)
            | Surface::HostCli
            | Surface::SlackRoom
            | Surface::SignUpRule => DeletionActor::Operator,
        }
    }

    /// The surface as the audit trail names it; `None` for a sign-up rule
    /// and an account holder, which write no row of the admin trail.
    const fn audit_source(&self) -> Option<AuditSource> {
        match self.surface {
            Surface::AdminCredential(source) => Some(source),
            Surface::HostCli => Some(AuditSource::CliDatabase),
            Surface::SlackRoom => Some(AuditSource::Slack),
            Surface::SignUpRule | Surface::AccountHolder(..) => None,
        }
    }

    /// Put on the trail what taking an account out of admin standing
    /// withdrew ([`record_withdrawn_admin_credentials`]): the row names the
    /// acting admin and the surface. Nothing is written when `withdrawn` is
    /// empty. `account` is `None` for a deleted account.
    pub async fn record_withdrawn(
        &self,
        repos: &RepositoryRegistry,
        withdrawn: &WithdrawnAdminCredentials,
        cause: AdminStandingLost,
        account: Option<Uuid>,
    ) {
        if withdrawn.is_empty() {
            return;
        }
        let Some(source) = self.audit_source() else {
            warn!(
                caller = %self.name,
                cause = cause.as_str(),
                "Admin credentials were withdrawn by a caller the audit trail has no source for"
            );
            return;
        };
        let change = StandingChange {
            cause,
            account,
            actor: self.acting_admin,
            source,
        };
        record_withdrawn_admin_credentials(repos, withdrawn, change).await;
    }
}

/// An account, taken by the caller that acts on it.
///
/// Built only by [`AccountCaller::acting_on`], which applies the rank rule,
/// and taken by every account write. The account is the one the caller
/// read; a write that decides on the account's standing reads it again.
#[derive(Debug)]
pub struct ActingOn<'caller> {
    caller: &'caller AccountCaller,
    account: User,
}

impl ActingOn<'_> {
    /// Who acts.
    #[must_use]
    pub const fn caller(&self) -> &AccountCaller {
        self.caller
    }

    /// The account acted on, as the caller read it.
    #[must_use]
    pub const fn account(&self) -> &User {
        &self.account
    }

    /// The id of the account acted on.
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.account.id
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::admin::models::AdminPermissions;
    use pierre_core::admin::scope::ScopeCaller;
    use pierre_core::permissions::UserRole;

    use super::*;

    fn account(role: UserRole) -> User {
        let mut user = User::new("target@example.com".to_owned(), "hash".to_owned(), None);
        user.role = role;
        user
    }

    fn token(is_super_admin: bool, operator: Option<Uuid>, id: &str, name: &str) -> AccountCaller {
        AccountCaller::of_token(&ValidatedAdminToken {
            token_id: id.to_owned(),
            service_name: name.to_owned(),
            permissions: AdminPermissions::super_admin(),
            is_super_admin,
            tenant_id: None,
            operator_user_id: operator,
            user_info: None,
        })
    }

    fn credential(token_id: &str, service: &str) -> ValidatedAdminToken {
        ValidatedAdminToken {
            token_id: token_id.to_owned(),
            service_name: service.to_owned(),
            permissions: AdminPermissions::default_admin(),
            is_super_admin: true,
            tenant_id: None,
            operator_user_id: Some(Uuid::nil()),
            user_info: None,
        }
    }

    #[test]
    fn the_credential_names_the_surface() {
        assert_eq!(
            token_audit_source(&credential(&format!("cookie:{}", Uuid::nil()), "a@b.c")),
            AuditSource::Console,
            "bug: a cookie session is the console"
        );
        assert_eq!(
            token_audit_source(&credential(
                "t1",
                &format!("{DEVICE_CLI_SERVICE_PREFIX}a@b.c")
            )),
            AuditSource::Cli,
            "bug: a device-login token is pierre-cli"
        );
        assert_eq!(
            token_audit_source(&credential("t2", "canary")),
            AuditSource::Service,
            "bug: any other token is a service"
        );
    }

    /// The rank rule, surface by surface: an admin account is taken by a
    /// console session, a device login, a super-admin token and the host
    /// CLI, and refused to every other caller in the one sentence, the
    /// holder of another account included. A plain account is taken by all
    /// of them but that holder, which takes its own account only.
    #[test]
    fn an_admin_account_is_taken_only_by_a_platform_caller() {
        let admin = Uuid::new_v4();
        let someone_else = Uuid::new_v4();
        let with_the_rank = [
            token(
                true,
                Some(admin),
                &format!("cookie:{admin}"),
                "ops@example.com",
            ),
            token(true, Some(admin), "tok-1", "device-cli:ops"),
            token(true, None, "tok-2", "platform-ops"),
            AccountCaller::host_cli(Some(admin)),
        ];
        let without_it = [
            token(false, None, "tok-3", "support-desk"),
            AccountCaller::slack_room("@reviewer"),
            AccountCaller::sign_up_rule(Some(admin)),
            AccountCaller::account_holder(someone_else),
            AccountCaller::account_holder_on_messaging(someone_else),
        ];
        for caller in &with_the_rank {
            assert!(
                caller.acting_on(account(UserRole::Admin)).is_ok(),
                "{caller:?}"
            );
        }
        for caller in &without_it {
            let refused = caller
                .acting_on(account(UserRole::Admin))
                .expect_err("an admin account takes a platform caller");
            assert!(
                refused.code == ErrorCode::PermissionDenied,
                "{caller:?}: {:?}",
                refused.code
            );
            assert_eq!(
                refused.message, "Acting on an admin account requires a super-admin token",
                "{caller:?}"
            );
        }
        let every_caller = with_the_rank.iter().chain(&without_it);
        for caller in
            every_caller.filter(|caller| caller.deletion_actor() == DeletionActor::Operator)
        {
            assert!(
                caller.acting_on(account(UserRole::User)).is_ok(),
                "{caller:?}"
            );
        }
    }

    /// An account's holder takes its own account whatever its role: an
    /// admin deletes its own account as anyone does (the deletion still
    /// refuses the last active admin). It takes no other account, a plain
    /// one included, in a sentence of its own.
    #[test]
    fn an_account_holder_takes_its_own_account_and_no_other() {
        for role in [UserRole::Admin, UserRole::User] {
            let own = account(role);
            for holder in [
                AccountCaller::account_holder(own.id),
                AccountCaller::account_holder_on_messaging(own.id),
            ] {
                let acting = holder
                    .acting_on(own.clone())
                    .unwrap_or_else(|refused| panic!("{role:?}, {holder:?}: {refused}"));
                assert_eq!(acting.id(), own.id);

                let refused = holder
                    .acting_on(account(UserRole::User))
                    .expect_err("another account is not the holder's");
                assert!(refused.code == ErrorCode::PermissionDenied, "{holder:?}");
                assert_eq!(
                    refused.message, "An account holder acts on its own account only",
                    "{holder:?}"
                );
            }
        }
    }

    /// The rank this value carries and the scope rule's platform caller
    /// are one reading of a credential, made in two places
    /// (`AccountCaller::of_token`, `ValidatedAdminToken::scope_caller`):
    /// whatever tenant a credential is bound to, it takes an admin account
    /// exactly when the scope rule lets it read every tenant. A change to
    /// one that leaves the other behind fails here.
    #[test]
    fn a_credential_holds_the_rank_exactly_when_the_scope_rule_calls_it_platform() {
        const TENANT: &str = "3f0c1a52-0000-4000-8000-000000000001";
        for (is_super_admin, tenant) in [
            (true, None),
            (true, Some(TENANT)),
            (false, None),
            (false, Some(TENANT)),
        ] {
            let credential = ValidatedAdminToken {
                is_super_admin,
                tenant_id: tenant.map(str::to_owned),
                operator_user_id: None,
                ..credential("t1", "desk")
            };
            let platform = credential
                .scope_caller()
                .is_ok_and(|caller| caller == ScopeCaller::Platform);
            let takes_an_admin = AccountCaller::of_token(&credential)
                .acting_on(account(UserRole::Admin))
                .is_ok();
            assert_eq!(
                takes_an_admin, platform,
                "super-admin flag {is_super_admin}, tenant {tenant:?}"
            );
        }
    }

    /// What each surface is recorded as: a console session and a device
    /// login as their admin, a service token and the Slack room as no one,
    /// the host CLI as the admin it looked up without acting as it.
    #[test]
    fn each_surface_carries_its_own_audit_identity() {
        let admin = Uuid::new_v4();
        let console = token(
            true,
            Some(admin),
            &format!("cookie:{admin}"),
            "ops@example.com",
        );
        assert_eq!(console.acting_admin(), Some(admin));
        assert_eq!(console.recorded_as(), Some(admin));
        assert_eq!(console.audit_name(), admin.to_string());
        assert_eq!(console.audit_source(), Some(AuditSource::Console));

        let device = token(true, Some(admin), "tok-1", "device-cli:ops");
        assert_eq!(device.audit_source(), Some(AuditSource::Cli));

        let service = token(false, None, "tok-2", "support-desk");
        assert_eq!(service.acting_admin(), None);
        assert_eq!(service.audit_name(), "support-desk");
        assert_eq!(service.audit_source(), Some(AuditSource::Service));

        let host = AccountCaller::host_cli(Some(admin));
        assert_eq!(host.acting_admin(), None, "no admin's session is behind it");
        assert_eq!(host.recorded_as(), Some(admin));
        assert_eq!(host.audit_name(), "pierre-cli");
        assert_eq!(host.audit_source(), Some(AuditSource::CliDatabase));

        let room = AccountCaller::slack_room("@reviewer");
        assert_eq!((room.acting_admin(), room.recorded_as()), (None, None));
        assert_eq!(room.name(), "@reviewer");
        assert_eq!(room.audit_source(), Some(AuditSource::Slack));

        assert_eq!(
            AccountCaller::sign_up_rule(Some(admin)).recorded_as(),
            Some(admin)
        );
    }

    /// A deletion receipt names an admin surface as `operator` and the
    /// holder by its channel: the values receipts already hold.
    #[test]
    fn a_deletion_receipt_names_the_holder_by_its_channel() {
        assert_eq!(
            AccountCaller::account_holder(Uuid::nil())
                .deletion_actor()
                .as_str(),
            "athlete"
        );
        assert_eq!(
            AccountCaller::account_holder_on_messaging(Uuid::nil())
                .deletion_actor()
                .as_str(),
            "messaging"
        );
        for caller in [
            AccountCaller::host_cli(None),
            token(true, None, "tok-2", "platform-ops"),
        ] {
            assert_eq!(caller.deletion_actor().as_str(), "operator");
        }
    }
}
