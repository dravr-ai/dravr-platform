// ABOUTME: Grant or decline a coach-access request from its Slack notice, for a linked active super-admin
// ABOUTME: Settles the card on a decision; a refused tap keeps the buttons and says why under them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_services::coach_access;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use super::ops_card::{
    authorize_super_admin, keep_card_with_reason, room_text, settled_card, SuperAdminDecision,
};
use super::{update_card, CoachAccessDecision, SlackAction};
use crate::mcp::resources::ServerContext;

/// Grant or decline the coach-access request a tap names, for the
/// super-admin behind the Slack identity that tapped.
///
/// The room is not enough here: a grant gives `manages_roster`, which only a
/// super-admin decides (the console's coach-access routes are super-admin
/// only), so the clicker must also resolve to an active super-admin
/// ([`authorize_super_admin`]). The decision goes through
/// [`coach_access::grant`] / [`coach_access::decline`] with that admin as
/// `decided_by`, as the console's does, so `coach_access.granted` /
/// `.declined` fire the same way.
///
/// A settled request — decided now, decided before, or gone — turns the
/// card's buttons into the outcome. A refused tap, or a decision that failed,
/// keeps the buttons for whoever may still decide and says why under them.
/// Anything but a decision taken now is also the route's answer, so the
/// route logs it.
pub(super) async fn answer_coach_access_tap(
    resources: &ServerContext,
    action: &SlackAction,
    decision: CoachAccessDecision,
    request_id: Uuid,
) -> AppResult<(StatusCode, Json<Value>)> {
    let decider = format!("@{}", action.slack_username);

    let operator = match authorize_super_admin(
        resources,
        &action.slack_user_id,
        SuperAdminDecision::CoachAccess,
    )
    .await
    {
        Ok(operator) => operator,
        Err(refused) => {
            keep_card_with_reason(
                resources,
                action,
                &format!("{decider}: {}", room_text(&refused)),
            );
            return Err(refused);
        }
    };

    match decide_coach_access(resources, decision, request_id, operator, &decider).await {
        Ok(outcome) => {
            info!(
                request_id = %request_id,
                operator_user_id = %operator,
                slack_user = %action.slack_user_id,
                "Coach access request decided via Slack action"
            );
            let card = settled_card(
                &action.message_blocks,
                &format!(":white_check_mark: {outcome}"),
            );
            update_card(resources, action, &card);
            Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
        }
        Err(e)
            if matches!(
                e.code,
                ErrorCode::ResourceAlreadyExists | ErrorCode::ResourceNotFound
            ) =>
        {
            let card = settled_card(&action.message_blocks, &format!(":x: {}", room_text(&e)));
            update_card(resources, action, &card);
            Err(e)
        }
        Err(e) => {
            keep_card_with_reason(
                resources,
                action,
                &format!(
                    "{decider}: the decision failed, try again ({})",
                    room_text(&e)
                ),
            );
            Err(e)
        }
    }
}

/// Take `decision` on the request for `operator`, returning the outcome line
/// the card shows.
async fn decide_coach_access(
    resources: &ServerContext,
    decision: CoachAccessDecision,
    request_id: Uuid,
    operator: Uuid,
    decider: &str,
) -> AppResult<String> {
    let repos = &resources.common.repos;
    match decision {
        CoachAccessDecision::Grant => {
            let outcome = coach_access::grant(repos, request_id, Some(operator)).await?;
            Ok(if outcome.attached_group.is_some() {
                format!("Coach access granted by {decider}; they now coach their group")
            } else {
                format!("Coach access granted by {decider}")
            })
        }
        CoachAccessDecision::Decline => {
            coach_access::decline(repos, request_id, Some(operator)).await?;
            Ok(format!("Coach access declined by {decider}"))
        }
    }
}
