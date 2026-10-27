// ABOUTME: Handler for /group invite, and the invite issuer /agent invite shares with it
// ABOUTME: Invites to the conversation's group, or to the one a button names by id, for its managers only
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_core::errors::AppError;

#[cfg(not(feature = "tools-groups"))]
use pierre_contremaitre::messaging_strings::KEY_GROUP_INVITE_UNAVAILABLE;
#[cfg(feature = "tools-groups")]
use pierre_contremaitre::messaging_strings::{KEY_COACH_INVITE_BODY, KEY_GROUP_INVITE_BODY};
use pierre_contremaitre::messaging_strings::{KEY_GROUP_INVITE_FORBIDDEN, KEY_GROUP_NOT_A_MEMBER};
use pierre_core::models::groups::{GroupInviteKind, GroupRole};
use pierre_core::models::TenantId;
#[cfg(not(feature = "tools-groups"))]
use tracing::info;
use uuid::Uuid;

use crate::group::{resolve_target_group, CallerGroupStanding, TargetGroup};
use crate::{CommandHandler, PlatformCommandContext};

/// Resolve the group a `/group invite <group-id>` names by id.
///
/// That is the button `/group create` puts on its answer, and it must invite
/// to the group just made wherever it is pressed: in Home's personal thread,
/// which names no group, and in a messaging DM, where the fallback would pick
/// whichever group the caller touched last (carnet#1187). The id alone finds
/// the row, as it does for `/agent assign`: a coaching group spans tenants,
/// so what admits the caller is the membership row [`issue_group_invite`]
/// checks next, and the invite is filed under the row's own tenant. An
/// unknown id answers "not a member", the answer [`issue_group_invite`] gives
/// a group the caller is not in, so the reply never tells a stranger whether
/// a group exists.
async fn named_group(
    ctx: &PlatformCommandContext,
    group_id: Uuid,
) -> Result<TargetGroup, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let group = ctx
        .ctx
        .repos()
        .groups
        .get_group(&group_id.to_string(), ctx.tenant_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, ctx.locale.as_str(), &[]))
        })?;
    let tenant_id = TenantId::parse_str(&group.tenant_id)
        .map_err(|e| AppError::internal(format!("Invalid group tenant: {e}")))?;
    Ok(TargetGroup {
        id: group.id,
        name: group.name,
        tenant_id,
        source: "named_group_id",
    })
}

/// Issue an invite of `kind` on the caller's behalf, refusing anyone who may
/// not manage the group's members.
///
/// The group is the conversation's, or the one `named` names when the
/// command carried its id ([`named_group`]). The one implementation behind
/// two triggers: `/group invite [agent]` and `/agent invite` resolve the
/// conversation's group through the same ladder, enforce the same role and
/// file the invite through the same `GroupService` path, so the two
/// spellings cannot drift.
///
/// LIMITATION(registre#1059): `issue_group_invite` serves owners and admins only, so an athlete cannot invite their own coach from a personal thread.
///
/// # Errors
///
/// Returns an error when the caller has no group to invite into, is not a
/// member of it, or the invite cannot be persisted.
pub(crate) async fn issue_group_invite(
    ctx: &PlatformCommandContext,
    kind: GroupInviteKind,
    named: Option<Uuid>,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    let group = match named {
        Some(group_id) => named_group(ctx, group_id).await?,
        None => resolve_target_group(ctx).await?,
    };

    // Check admin role
    let member = ctx
        .ctx
        .repos()
        .groups
        .get_member(&group.id.to_string(), ctx.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

    if !GroupInviteHandler::permits(member.role) {
        return Ok(CommandResponse::text(reg.render(
            KEY_GROUP_INVITE_FORBIDDEN,
            locale,
            &[],
        )));
    }

    #[cfg(feature = "tools-groups")]
    {
        file_invite(ctx, &group, kind).await
    }
    #[cfg(not(feature = "tools-groups"))]
    {
        Ok(file_invite(ctx, &group, kind))
    }
}

/// File the invite through `GroupService` and render the body for its kind.
#[cfg(feature = "tools-groups")]
async fn file_invite(
    ctx: &PlatformCommandContext,
    group: &TargetGroup,
    kind: GroupInviteKind,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    // The invite is filed under the tenant that owns the group, not the
    // issuer's — in a shared room those differ.
    let invite = ctx
        .ctx
        .group_service()
        .create_invite(group.id, ctx.user_id, group.tenant_id, Some(7), None, kind)
        .await?;

    // A coach invite attaches its redeemer as the group's human coach rather
    // than adding an athlete, and the body says so.
    let key = match kind {
        GroupInviteKind::Coach => KEY_COACH_INVITE_BODY,
        GroupInviteKind::Member => KEY_GROUP_INVITE_BODY,
    };
    Ok(CommandResponse::text(reg.render(
        key,
        locale,
        &[&group.name, &invite.code, &invite.code],
    )))
}

/// A build without the groups feature has no `GroupService` to file an
/// invite through, so the caller is told invites are unavailable.
#[cfg(not(feature = "tools-groups"))]
fn file_invite(
    ctx: &PlatformCommandContext,
    group: &TargetGroup,
    kind: GroupInviteKind,
) -> CommandResponse {
    let reg = ctx.ctx.messaging_strings_registry();
    info!(
        group_id = %group.id,
        kind = kind.as_str(),
        "Group invite requested on a build without the groups feature"
    );
    CommandResponse::text(reg.render(KEY_GROUP_INVITE_UNAVAILABLE, ctx.locale.as_str(), &[]))
}

/// Handler for `/group invite` — generate invite link
#[derive(Debug)]
pub struct GroupInviteHandler;

impl GroupInviteHandler {
    /// The single authority on who may invite. `execute` checks it against the
    /// membership row it fetched; `is_available` checks it against the role
    /// `/help` already resolved for the same group. `/agent invite` lists
    /// itself on the same answer.
    pub(crate) fn permits(role: GroupRole) -> bool {
        role.can_manage_members()
    }

    /// What `/group invite [coach] [group-id]` asks for. `coach` as the first
    /// word issues a coach invite (attaches the redeemer as the group's human
    /// coach); any other first word, or none, a standard athlete-membership
    /// invite. A group id as the last word names the group to invite to —
    /// how `/group create`'s button reaches the group it made (carnet#1187);
    /// without one the conversation's group is invited to.
    fn requested(args: &[String]) -> (GroupInviteKind, Option<Uuid>) {
        let kind = if args
            .first()
            .is_some_and(|a| a.trim().eq_ignore_ascii_case("coach"))
        {
            GroupInviteKind::Coach
        } else {
            GroupInviteKind::Member
        };
        let named = args.last().and_then(|a| Uuid::parse_str(a.trim()).ok());
        (kind, named)
    }
}

#[async_trait]
impl CommandHandler for GroupInviteHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let (kind, named) = Self::requested(&ctx.args);
        issue_group_invite(ctx, kind, named).await
    }

    /// Acts on the conversation's group, so the caller's role *there* decides.
    /// The form that names its group is a button's postback, not a command
    /// anyone types, so `/help` keeps listing by the conversation's group.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some_and(Self::permits)
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::groups::GroupInviteKind;
    use uuid::Uuid;

    use super::GroupInviteHandler;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn group_invite_reads_its_kind_first_and_a_named_group_last() {
        let group = Uuid::new_v4();
        let id = group.to_string();

        assert_eq!(
            GroupInviteHandler::requested(&args(&[])),
            (GroupInviteKind::Member, None),
            "a bare invite is a member invite to the conversation's group"
        );
        assert_eq!(
            GroupInviteHandler::requested(&args(&["Coach"])),
            (GroupInviteKind::Coach, None),
            "`coach` asks for a coach invite, whatever its case"
        );
        assert_eq!(
            GroupInviteHandler::requested(&args(&[&id])),
            (GroupInviteKind::Member, Some(group)),
            "a lone id names the group, the button's form"
        );
        assert_eq!(
            GroupInviteHandler::requested(&args(&["coach", &id])),
            (GroupInviteKind::Coach, Some(group)),
            "the kind comes first and the group last"
        );
        assert_eq!(
            GroupInviteHandler::requested(&args(&["friends"])),
            (GroupInviteKind::Member, None),
            "a word that is no id leaves the conversation's group to resolve"
        );
    }
}
