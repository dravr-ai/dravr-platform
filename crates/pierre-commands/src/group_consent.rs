// ABOUTME: Handler for /group consent — whether a member shares their training data with the group's coach and with its members
// ABOUTME: `/group consent yes|no` sets peer sharing; `/group consent coach yes|no` sets coach sharing, which joining grants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_COACH_CONSENT_UPDATED, KEY_GROUP_CONSENT_UPDATED, KEY_GROUP_CONSENT_USAGE,
    KEY_GROUP_NOT_A_MEMBER, KEY_GROUP_PEER_SHARING_OFF, KEY_GROUP_PEER_SHARING_ON,
};
use pierre_core::errors::AppError;
use tracing::info;

use crate::group::{resolve_target_group, CallerGroupStanding};
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
/// Both act on the group [`resolve_target_group`] names: the group bound to
/// the chat the command was typed in, refusing rather than retargeting when a
/// shared room cannot name one.
///
/// - `/group consent yes|no` updates `coaching_group_members.peer_sharing_consent`:
///   whether the group's other members see the requester's training. Opt-in,
///   and honoured only while the group's `peer_data_sharing` is on.
/// - `/group consent coach yes|no` updates `coach_sharing_consent`: whether the
///   group's human coach reads it. Joining grants it (ADR-002); this is the
///   member's way to revoke it, and to grant it back.
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

        let target = resolve_target_group(ctx).await?;
        let group_id_str = target.id.to_string();
        let group_name = target.name;

        let groups = &ctx.ctx.repos().groups;
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
            group_name = %group_name,
            audience = ?audience,
            consent_choice,
            rows_affected,
            source = target.source,
            "Applied /group consent"
        );

        if !rows_affected {
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
        let body_key = match audience {
            SharingAudience::Peers => KEY_GROUP_CONSENT_UPDATED,
            SharingAudience::Coach => KEY_GROUP_COACH_CONSENT_UPDATED,
        };
        let body = reg.render(body_key, locale, &[&state, &group_name]);

        Ok(CommandResponse::text(body))
    }

    /// Acts on the conversation's group and refuses a non-member — the consent
    /// write reports zero rows and `execute` turns that into "not a member".
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
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
