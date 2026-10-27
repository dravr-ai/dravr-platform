// ABOUTME: Raise the athlete a quota.limit_reached notice names to Enterprise, from its Slack button, for a linked active super-admin
// ABOUTME: The tier write is the shared admin override attributed to that admin; a refused tap keeps the button and says why
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::UserTier;
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use pierre_services::admin_ops;
use pierre_services::usage_counter;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use super::ops_card::{
    authorize_super_admin, keep_card_with_reason, room_text, settled_card, SuperAdminDecision,
};
use super::{update_card, SlackAction};
use crate::mcp::resources::ServerContext;

/// What a Raise to Enterprise tap did to the athlete it names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unlock {
    /// The tier was raised; carries the athlete's email.
    Raised(String),
    /// The athlete was already Enterprise, so nothing was written; carries
    /// the athlete's email.
    AlreadyEnterprise(String),
}

/// Raise the athlete a `quota.limit_reached` tap names to Enterprise, for the
/// super-admin behind the Slack identity that tapped, then turn its card into
/// the outcome.
///
/// The room is not enough here: Enterprise lifts every tier cap, which is
/// unbounded LLM spend, and a tier change is a super-admin's decision on
/// every other surface (the admin tier routes are super-admin only). So the
/// clicker must also resolve to an active super-admin
/// ([`authorize_super_admin`]), and the override row is written with that
/// admin as `set_by` — the same write as
/// `pierre-cli user set <email> --tier enterprise`.
///
/// The button is posted only under a cap the raise lifts ([`raise_can_lift`]):
/// a tier default that Enterprise leaves unbounded, not an admin-config
/// limit override, which outranks any tier and which the raise leaves alone.
/// A tap that finds the athlete already Enterprise (raised by an earlier tap
/// or elsewhere) writes nothing and reports no success.
///
/// A refused tap, or a raise that failed, keeps the button for whoever may
/// still decide and says why under it.
pub(super) async fn answer_quota_unlock_tap(
    resources: &ServerContext,
    action: &SlackAction,
    user_uuid: Uuid,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Attribute the raise to the Slack handle that clicked; the username
    // rides in the interactive payload.
    let raiser = format!("@{}", action.slack_username);

    let operator = match authorize_super_admin(
        resources,
        &action.slack_user_id,
        SuperAdminDecision::RaiseTier,
    )
    .await
    {
        Ok(operator) => operator,
        Err(refused) => {
            keep_card_with_reason(
                resources,
                action,
                &format!("{raiser}: {}", room_text(&refused)),
            );
            return Err(refused);
        }
    };

    let note = format!(
        "raised to enterprise from Slack #{} by {raiser}",
        room_of(action)
    );
    match raise_to_enterprise(&resources.common.repos, user_uuid, note, operator).await {
        Ok(unlock) => {
            info!(
                user_id = %user_uuid,
                operator_user_id = %operator,
                slack_user = %action.slack_user_id,
                outcome = unlock_kind(&unlock),
                "Slack quota action answered"
            );
            let card = settled_card(&action.message_blocks, &outcome_line(&unlock, &raiser));
            update_card(resources, action, &card);
            Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
        }
        Err(e) if e.code == ErrorCode::ResourceNotFound => {
            let card = settled_card(&action.message_blocks, &format!(":x: {}", room_text(&e)));
            update_card(resources, action, &card);
            Err(e)
        }
        Err(e) => {
            keep_card_with_reason(
                resources,
                action,
                &format!("{raiser}: the raise failed, try again ({})", room_text(&e)),
            );
            Err(e)
        }
    }
}

/// The outcome line a settled card shows: a raise is a success; a tap that
/// finds the athlete already Enterprise raised nothing, so it reports none.
fn outcome_line(unlock: &Unlock, raiser: &str) -> String {
    match unlock {
        Unlock::Raised(email) => format!(
            ":white_check_mark: *{email}* raised to Enterprise by {raiser}; \
             the tier caps no longer bind them"
        ),
        Unlock::AlreadyEnterprise(email) => format!(
            ":information_source: *{email}* is already Enterprise; this tap \
             raised nothing ({raiser}), and a cap that still refuses them is an \
             admin limit override"
        ),
    }
}

/// The outcome's name for the log line, without the athlete's email.
const fn unlock_kind(unlock: &Unlock) -> &'static str {
    match unlock {
        Unlock::Raised(_) => "raised",
        Unlock::AlreadyEnterprise(_) => "already_enterprise",
    }
}

/// Raise `user_uuid` to Enterprise through the shared admin tier override,
/// attributed to `operator` with `note`, which also records the marker that
/// keeps a later billing webhook from lowering it again. A user already
/// Enterprise is left as it is.
///
/// # Errors
///
/// `ResourceNotFound` when no user has that id, and the tier write's error
/// when it fails.
async fn raise_to_enterprise(
    repos: &RepositoryRegistry,
    user_uuid: Uuid,
    note: String,
    operator: Uuid,
) -> AppResult<Unlock> {
    let user = repos
        .users
        .get_global(user_uuid)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User {user_uuid}")))?;

    if user.tier == UserTier::Enterprise {
        return Ok(Unlock::AlreadyEnterprise(user.email));
    }

    let updated = admin_ops::set_user_tier(
        repos,
        user_uuid,
        UserTier::Enterprise,
        Some(note),
        Some(operator),
    )
    .await?;

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated.email),
        previous_tier = user.tier.as_str(),
        operator_user_id = %operator,
        "User raised to Enterprise via Slack quota action"
    );

    Ok(Unlock::Raised(updated.email))
}

/// Whether raising the athlete a `quota.limit_reached` notice names to
/// Enterprise lifts the cap that refused them, read off the notice's
/// `limit_type`, `limit` and `tier`.
///
/// A raise lifts a cap only when Enterprise leaves that counter unbounded and
/// the refusing limit is the athlete's tier default. A limit other than the
/// tier default is an admin-config override (user, tenant or system scope),
/// which outranks every tier, so a raise would leave the athlete locked; an
/// athlete already Enterprise can only be refused by one. Either way the card
/// offers no button that would claim to unlock them. An override set to
/// exactly the tier default cannot be told apart from it. A notice missing
/// any of the three fields gets no button.
pub(super) fn raise_can_lift(fields: &HashMap<String, String>) -> bool {
    let (Some(counter), Some(limit), Some(tier)) = (
        fields.get("limit_type"),
        fields
            .get("limit")
            .and_then(|limit| limit.parse::<i64>().ok()),
        fields
            .get("tier")
            .and_then(|tier| tier.parse::<UserTier>().ok()),
    ) else {
        return false;
    };
    tier != UserTier::Enterprise
        && limit == usage_counter::default_limit(counter, &tier)
        && usage_counter::default_limit(counter, &UserTier::Enterprise) == i64::MAX
}

/// The room a tap came from, by name when the payload carries one and by
/// channel id otherwise.
fn room_of(action: &SlackAction) -> &str {
    if action.channel_name.is_empty() {
        &action.channel_id
    } else {
        &action.channel_name
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pierre_core::errors::ErrorCode;
    use pierre_core::models::UserTier;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::{create_test_admin_user, create_test_user};
    use uuid::Uuid;

    use super::{outcome_line, raise_can_lift, raise_to_enterprise, room_of, SlackAction, Unlock};

    fn tap_in(channel_id: &str, channel_name: &str) -> SlackAction {
        SlackAction {
            action_id: "raise_tier_enterprise:00000000-0000-0000-0000-000000000001".to_owned(),
            slack_user_id: "U1".to_owned(),
            slack_username: "operator".to_owned(),
            channel_id: channel_id.to_owned(),
            channel_name: channel_name.to_owned(),
            message_ts: "1.0".to_owned(),
            message_blocks: Vec::new(),
        }
    }

    #[test]
    fn the_audit_note_names_the_room_by_name_and_by_id_without_one() {
        assert_eq!(
            room_of(&tap_in("C0QUOTA", "dravr-quota")),
            "dravr-quota",
            "a payload naming its channel is attributed to that name"
        );
        assert_eq!(
            room_of(&tap_in("C0QUOTA", "")),
            "C0QUOTA",
            "a payload without a channel name is attributed to its id"
        );
    }

    fn notice(limit_type: &str, limit: &str, tier: &str) -> HashMap<String, String> {
        HashMap::from([
            ("user_id".to_owned(), Uuid::new_v4().to_string()),
            ("limit_type".to_owned(), limit_type.to_owned()),
            ("limit".to_owned(), limit.to_owned()),
            ("tier".to_owned(), tier.to_owned()),
        ])
    }

    #[test]
    fn a_raise_is_offered_only_for_a_tier_default_that_enterprise_lifts() {
        for (limit_type, limit, tier, offered, why) in [
            ("daily_messages", "50", "starter", true, "Starter's own cap"),
            (
                "weekly_tokens",
                "25000000",
                "professional",
                true,
                "Professional's own cap",
            ),
            (
                "weekly_tool_calls",
                "500",
                "starter",
                true,
                "tier-keyed, unbounded for Enterprise",
            ),
            (
                "daily_messages",
                "20",
                "starter",
                false,
                "an admin override pins it",
            ),
            (
                "daily_messages",
                "500",
                "starter",
                false,
                "an override that is another tier's number",
            ),
            (
                "daily_tool_calls",
                "300",
                "enterprise",
                false,
                "an Enterprise refusal is an override",
            ),
        ] {
            assert_eq!(
                raise_can_lift(&notice(limit_type, limit, tier)),
                offered,
                "{limit_type}={limit} on {tier}: {why}"
            );
        }
    }

    #[test]
    fn a_notice_missing_what_the_check_reads_offers_no_raise() {
        for missing in ["limit_type", "limit", "tier"] {
            let mut fields = notice("daily_messages", "50", "starter");
            fields.remove(missing);
            assert!(!raise_can_lift(&fields), "no {missing}, no button");
        }
        assert!(
            !raise_can_lift(&notice("daily_messages", "fifty", "starter")),
            "a limit that is not a number offers no button"
        );
        assert!(
            !raise_can_lift(&notice("daily_messages", "50", "gold")),
            "a tier that is not one offers no button"
        );
    }

    #[test]
    fn only_a_raise_reads_as_a_success_on_the_card() {
        let raised = outcome_line(&Unlock::Raised("a@x.test".to_owned()), "@op");
        assert!(raised.starts_with(":white_check_mark:"), "{raised}");
        let already = outcome_line(&Unlock::AlreadyEnterprise("a@x.test".to_owned()), "@op");
        assert!(
            !already.starts_with(":white_check_mark:"),
            "a tap that raised nothing is never a success: {already}"
        );
        assert!(already.contains("raised nothing"), "{already}");
    }

    #[tokio::test]
    async fn a_starter_is_raised_to_enterprise_with_the_note_and_the_operator_on_record() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let athlete = create_test_user(&format!("{}@quota.test", Uuid::new_v4()), None);
        let athlete_id = repos.users.create(&athlete).await.expect("user row");
        // `set_by` references a user row: the operator is a real account.
        let operator = repos
            .users
            .create(&create_test_admin_user(
                &format!("{}@ops.test", Uuid::new_v4()),
                None,
            ))
            .await
            .expect("operator row");

        let unlock = raise_to_enterprise(
            repos,
            athlete_id,
            "raised from Slack #dravr-quota by @op".to_owned(),
            operator,
        )
        .await
        .expect("the raise");

        assert_eq!(
            unlock,
            Unlock::Raised(athlete.email.clone()),
            "a Starter is raised, named by email"
        );
        let stored = repos
            .users
            .get_global(athlete_id)
            .await
            .expect("read back")
            .expect("the athlete");
        assert_eq!(stored.tier, UserTier::Enterprise, "the tier was raised");
        let marker = repos
            .user_tier_overrides
            .get(athlete_id)
            .await
            .expect("read the override")
            .expect("the raise records an admin override");
        assert_eq!(
            marker.tier,
            UserTier::Enterprise,
            "the override pins Enterprise"
        );
        assert_eq!(
            marker.note.as_deref(),
            Some("raised from Slack #dravr-quota by @op"),
            "the audit note is recorded as given"
        );
        assert_eq!(marker.set_by, Some(operator), "the operator is on record");
    }

    #[tokio::test]
    async fn an_athlete_already_enterprise_is_left_as_is() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let athlete = create_test_admin_user(&format!("{}@quota.test", Uuid::new_v4()), None);
        assert_eq!(athlete.tier, UserTier::Enterprise, "fixture precondition");
        let athlete_id = repos.users.create(&athlete).await.expect("user row");

        let unlock = raise_to_enterprise(repos, athlete_id, "note".to_owned(), Uuid::new_v4())
            .await
            .expect("the tap is answered");

        assert_eq!(
            unlock,
            Unlock::AlreadyEnterprise(athlete.email.clone()),
            "an Enterprise athlete is reported as already raised"
        );
        assert!(
            repos
                .user_tier_overrides
                .get(athlete_id)
                .await
                .expect("read the override")
                .is_none(),
            "nothing was written"
        );
    }

    #[tokio::test]
    async fn an_unknown_athlete_is_not_found() {
        let db = create_test_db().await.expect("test database");
        let missing = Uuid::new_v4();

        let error = raise_to_enterprise(
            db.repositories(),
            missing,
            "note".to_owned(),
            Uuid::new_v4(),
        )
        .await
        .expect_err("nobody to raise");

        assert_eq!(error.code, ErrorCode::ResourceNotFound, "{error:?}");
        assert!(
            db.repositories()
                .user_tier_overrides
                .get(missing)
                .await
                .expect("read the override")
                .is_none(),
            "nothing was written"
        );
    }
}
