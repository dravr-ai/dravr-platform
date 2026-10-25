// ABOUTME: Handler for /group consent [coach] yes|no — whether a member shares their training with the group's members, or its coach (joining grants that)
// ABOUTME: From a direct message it never guesses a group: a withdrawal reaches all of the member's groups, a grant asks which unless there is one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_COACH_CONSENT_UPDATED, KEY_GROUP_COACH_CONSENT_UPDATED_ALL,
    KEY_GROUP_CONSENT_PICK_GROUP, KEY_GROUP_CONSENT_UPDATED, KEY_GROUP_CONSENT_UPDATED_ALL,
    KEY_GROUP_CONSENT_USAGE, KEY_GROUP_NOT_A_MEMBER, KEY_GROUP_PEER_SHARING_OFF,
    KEY_GROUP_PEER_SHARING_ON,
};
use pierre_core::errors::AppError;
use tracing::info;

use crate::group::{resolve_consent_targets, CallerGroupStanding, ConsentTargets, TargetGroup};
use crate::{CommandHandler, PlatformCommandContext};

/// The subcommand word that points `/group consent` at the coach.
const COACH_AUDIENCE: &str = "coach";

/// Who a `/group consent` decision is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharingAudience {
    /// The group's other members — `peer_sharing_consent`, opt-in.
    Peers,
    /// The group's human coach — `coach_sharing_consent`, granted on joining.
    Coach,
}

/// Parse the arguments of `/group consent` into the audience and the choice,
/// or `None` when they match neither form.
fn parse_consent_args(args: &[String]) -> Option<(SharingAudience, bool)> {
    let mut words = args.iter().map(|a| a.trim().to_lowercase());
    let first = words.next()?;
    let (audience, choice) = if first == COACH_AUDIENCE {
        (SharingAudience::Coach, words.next()?)
    } else {
        (SharingAudience::Peers, first)
    };
    if words.next().is_some() {
        return None;
    }
    let consent = match choice.as_str() {
        "yes" | "on" | "true" | "1" => true,
        "no" | "off" | "false" | "0" => false,
        _ => return None,
    };
    Some((audience, consent))
}

/// Handler for `/group consent [coach] yes|no` — the member's two sharing
/// decisions.
///
/// Both act on the groups [`resolve_consent_targets`] names: the group bound
/// to the chat the command was typed in, refusing rather than retargeting when
/// a shared room cannot name one. A direct message names no group, and the
/// command never falls back to the one touched last: a withdrawal applies to
/// every group the member belongs to, and a grant to their one group, or else
/// asks which ([`decide_direct`]).
///
/// - `/group consent yes|no` updates `coaching_group_members.peer_sharing_consent`:
///   whether the group's other members see the requester's training. Opt-in,
///   and honoured only while the group's `peer_data_sharing` is on.
/// - `/group consent coach yes|no` updates `coach_sharing_consent`: whether the
///   group's human coach reads it. Joining grants it (ADR-002); this is the
///   member's way to revoke it, and to grant it back.
#[derive(Debug)]
pub struct GroupConsentHandler;

#[async_trait]
impl CommandHandler for GroupConsentHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let Some((audience, consent_choice)) = parse_consent_args(&ctx.args) else {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_CONSENT_USAGE,
                locale,
                &[],
            )));
        };

        let targets = match resolve_consent_targets(ctx).await? {
            ConsentTargets::Bound(group) => vec![group],
            ConsentTargets::Direct(groups) => match decide_direct(&groups, consent_choice) {
                DirectDecision::Apply => groups,
                DirectDecision::AskWhichGroup => {
                    let names = joined_names(groups.iter().map(|g| g.name.as_str()));
                    return Ok(CommandResponse::text(reg.render(
                        KEY_GROUP_CONSENT_PICK_GROUP,
                        locale,
                        &[&names],
                    )));
                }
            },
        };

        let groups = &ctx.ctx.repos().groups;
        let mut applied: Vec<String> = Vec::with_capacity(targets.len());
        for target in targets {
            let group_id_str = target.id.to_string();
            let rows_affected = match audience {
                SharingAudience::Peers => {
                    groups
                        .update_peer_sharing_consent(&group_id_str, ctx.user_id, consent_choice)
                        .await?
                }
                SharingAudience::Coach => {
                    groups
                        .update_coach_sharing_consent(&group_id_str, ctx.user_id, consent_choice)
                        .await?
                }
            };

            info!(
                user_id = %ctx.user_id,
                group_id = %group_id_str,
                group_name = %target.name,
                audience = ?audience,
                consent_choice,
                rows_affected,
                source = target.source,
                "Applied /group consent"
            );

            // A bound group the caller is not in writes nothing; a group from
            // their own list writes nothing only if they left it meanwhile.
            if rows_affected {
                applied.push(target.name);
            }
        }

        if applied.is_empty() {
            return Err(AppError::not_found(reg.render(
                KEY_GROUP_NOT_A_MEMBER,
                locale,
                &[],
            )));
        }

        let state_key = if consent_choice {
            KEY_GROUP_PEER_SHARING_ON
        } else {
            KEY_GROUP_PEER_SHARING_OFF
        };
        let state = reg.render(state_key, locale, &[]);
        let body = if let [group_name] = applied.as_slice() {
            let body_key = match audience {
                SharingAudience::Peers => KEY_GROUP_CONSENT_UPDATED,
                SharingAudience::Coach => KEY_GROUP_COACH_CONSENT_UPDATED,
            };
            reg.render(body_key, locale, &[&state, group_name])
        } else {
            let body_key = match audience {
                SharingAudience::Peers => KEY_GROUP_CONSENT_UPDATED_ALL,
                SharingAudience::Coach => KEY_GROUP_COACH_CONSENT_UPDATED_ALL,
            };
            let names = joined_names(applied.iter().map(String::as_str));
            reg.render(body_key, locale, &[&state, &names])
        };

        Ok(CommandResponse::text(body))
    }

    /// Acts on the conversation's group and refuses a non-member — the consent
    /// write reports zero rows and `execute` turns that into "not a member".
    /// In a direct message it acts on the caller's own groups, so any group
    /// will do.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some() || (standing.is_direct_message && standing.highest.is_some())
    }
}

/// What a `/group consent` typed in a direct message does with the caller's
/// groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectDecision {
    /// Apply the choice to every one of them.
    Apply,
    /// Change nothing and ask which group the caller means.
    AskWhichGroup,
}

/// Withdrawing reaches every group: stopping more sharing than meant is the
/// safe failure, and the reply names each group. Granting from a direct
/// message reaches a group only when it is the caller's one group; with
/// several, the caller says which, in that group's chat or with the switch in
/// its info.
fn decide_direct(groups: &[TargetGroup], consent: bool) -> DirectDecision {
    if !consent || groups.len() == 1 {
        DirectDecision::Apply
    } else {
        DirectDecision::AskWhichGroup
    }
}

/// The group names a reply lists, in the order the caller's groups came.
fn joined_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names.collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use pierre_core::models::TenantId;
    use uuid::Uuid;

    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn groups(count: usize) -> Vec<TargetGroup> {
        (0..count)
            .map(|i| TargetGroup {
                id: Uuid::new_v4(),
                name: format!("Group {i}"),
                tenant_id: TenantId::from_uuid(Uuid::new_v4()),
                source: "list_groups_for_user_all",
            })
            .collect()
    }

    #[test]
    fn a_withdrawal_from_a_direct_message_reaches_every_group() {
        for count in 1..=3 {
            assert_eq!(
                decide_direct(&groups(count), false),
                DirectDecision::Apply,
                "{count} groups"
            );
        }
    }

    #[test]
    fn a_grant_from_a_direct_message_reaches_only_a_lone_group() {
        assert_eq!(decide_direct(&groups(1), true), DirectDecision::Apply);
        assert_eq!(
            decide_direct(&groups(2), true),
            DirectDecision::AskWhichGroup
        );
    }

    #[test]
    fn the_reply_lists_the_groups_in_order() {
        assert_eq!(
            joined_names(["Tri club", "Track"].into_iter()),
            "Tri club, Track"
        );
    }

    #[test]
    fn a_bare_choice_is_about_the_peers() {
        assert_eq!(
            parse_consent_args(&args(&["yes"])),
            Some((SharingAudience::Peers, true))
        );
        assert_eq!(
            parse_consent_args(&args(&["OFF"])),
            Some((SharingAudience::Peers, false))
        );
    }

    #[test]
    fn the_coach_word_points_the_choice_at_the_coach() {
        assert_eq!(
            parse_consent_args(&args(&["coach", "no"])),
            Some((SharingAudience::Coach, false))
        );
        assert_eq!(
            parse_consent_args(&args(&["Coach", "yes"])),
            Some((SharingAudience::Coach, true))
        );
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        for bad in [
            &[][..],
            &["coach"][..],
            &["maybe"][..],
            &["coach", "maybe"][..],
            &["yes", "please"][..],
            &["coach", "no", "really"][..],
        ] {
            assert_eq!(parse_consent_args(&args(bad)), None, "{bad:?}");
        }
    }
}
