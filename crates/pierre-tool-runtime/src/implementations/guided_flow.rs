// ABOUTME: Which guided flow owns the turn, and whether it refuses a plan save
// ABOUTME: Lives apart from any tool so every surface that resolves a walk asks it the same way

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which guided flow owns an athlete's turn.
//!
//! An interview must not write a training plan, and `save_training_plan`
//! refuses a self-scope save while one owns the conversation. That refusal is
//! the enforcement; the walk directive, last in the prompt's tail, is the
//! instruction. Advertisement is deliberately NOT narrowed: every surface lists
//! the same tools whether or not a walk is running, because the tools block is
//! the head of every cached prefix and changing it on a walk's first and last
//! turn re-wrote the whole prefix (carnet#1256).

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, GuidedFlow, OnboardingState, TenantId};
use pierre_database::RepositoryRegistry;
use serde_json::Value;
use tracing::warn;

/// Newest conversations consulted when resolving an athlete's guided-flow state
/// without a conversation in scope. An interview keeps its conversation the
/// most recently updated one, so a running walk is always inside this window.
const GUIDED_FLOW_SCAN_LIMIT: i64 = 50;

/// The guided flow that owns the turn FOR THIS USER, when one does.
///
/// The flow, not a verdict: whether it refuses a plan save is
/// [`flow_refuses_plan_save`]'s question, asked of what this returns.
///
/// The conversation is authoritative when the call arrives through the chat
/// pipeline, which puts its id (and, since room walks exist, the tenant that
/// owns its row) in scope. A `tools/call` on the `/mcp` endpoint has neither,
/// so the state is resolved from the athlete's conversations instead — the
/// `conversation_id` argument is model-supplied and cannot be trusted to
/// answer a question about whether this same model may write.
///
/// `conversation_ref` is the `(id, owning tenant)` pair from
/// [`crate::context::ToolExecutionContext::conversation_ref`]. It matters when
/// `conv` is `None` despite a conversation id being in scope: a shared room
/// files its row under the channel tenant, so the caller's own-tenant lookup
/// missed it — this predicate then retries under the owning tenant, which is
/// exactly the case where a room walk's refusal used to fall through to the
/// scan and silently never fire.
///
/// A walk bound to a subject refuses that member alone: everyone else
/// on the thread is not mid-interview, and refusing the room's human coach a
/// plan save because their athlete is calibrating would be the wrong refusal.
///
/// `save_training_plan` is its one production caller. It is public because
/// `room_guided_walk_test.rs` drives its conversation-tenant fallback against
/// real room rows, which needs a full server context this crate cannot build.
///
/// With no conversation in scope at all — a bare `/mcp` `tools/call` — the
/// athlete's newest conversations are scanned, first under the caller's home
/// tenant and then under every tenant that holds one of the athlete's channel
/// links (see [`walk_tenants`]). A room's per-member row is filed under the
/// bot tenant that owns the channel, which is exactly the tenant its link
/// lives under, so an active ROOM walk refuses on this path too instead of
/// being invisible to a home-tenant-only scan. Each scan is its own
/// `(user_id, tenant_id)`-scoped query.
///
/// # Errors
///
/// Propagates a repository failure, so a save that cannot tell whether a walk
/// is running fails rather than writing a plan into one.
pub async fn active_guided_flow(
    repos: &RepositoryRegistry,
    conv: Option<&ConversationRecord>,
    conversation_ref: Option<(&str, TenantId)>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<Option<GuidedFlow>> {
    if let Some(conv) = conv {
        return Ok(walk_binds(conv.onboarding_state.as_deref(), user_id));
    }
    if let Some((conv_id, conv_tenant)) = conversation_ref {
        if conv_tenant != tenant {
            if let Some(conv) = repos
                .chat
                .get_conversation(conv_id, user_id, conv_tenant)
                .await?
            {
                return Ok(walk_binds(conv.onboarding_state.as_deref(), user_id));
            }
        }
    }
    for scan_tenant in walk_tenants(repos, tenant, user_id).await? {
        let states = repos
            .chat
            .list_user_onboarding_states(user_id, scan_tenant, GUIDED_FLOW_SCAN_LIMIT)
            .await?;
        if let Some(flow) = states.iter().find_map(|raw| walk_binds(Some(raw), user_id)) {
            return Ok(Some(flow));
        }
    }
    Ok(None)
}

/// The tenants an athlete's guided walks can live under, home tenant first.
///
/// A DM or web conversation is filed under the athlete's own tenant. A shared
/// room's per-member row is filed under the channel/bot tenant
/// (`resolve_linked_session` keeps group sessions there), and an athlete only
/// ever reaches a room through a channel link stored under that same bot
/// tenant — ingress resolves the sender with `get_channel_link(bot_tenant, ..)`
/// and drops the message when no link exists. So the athlete's own links name
/// every other tenant a walk of theirs can be in, and nothing else.
///
/// The link listing is the user-scoped read `list_channel_links_for_user`
/// documents (a user id is narrower than a tenant: each row names its owner),
/// and the scans it seeds stay `(user_id, tenant_id)`-scoped, so no other
/// member's row — and no tenant the athlete holds nothing in — is ever read.
async fn walk_tenants(
    repos: &RepositoryRegistry,
    home: TenantId,
    user_id: &str,
) -> AppResult<Vec<TenantId>> {
    let mut tenants = vec![home];
    for link in repos.messaging.list_channel_links_for_user(user_id).await? {
        let Some(link_tenant) = link
            .get("tenant_id")
            .and_then(Value::as_str)
            .and_then(|raw| TenantId::parse_str(raw).ok())
        else {
            warn!("channel link carries no valid tenant; skipping it in the guided-flow scan");
            continue;
        };
        if !tenants.contains(&link_tenant) {
            tenants.push(link_tenant);
        }
    }
    Ok(tenants)
}

/// The flow that binds this user, when one does.
///
/// Returns the flow rather than a bool because whether a walk refuses a plan
/// save is a property *of that walk*: an interview must not write a plan, and
/// a walk whose whole purpose is to write one must. Discarding the flow here
/// is what made the refusal flow-agnostic, and a writing flow unbuildable.
fn walk_binds(raw: Option<&str>, user_id: &str) -> Option<GuidedFlow> {
    OnboardingState::from_column(raw).and_then(|state| {
        state
            .subject_user_id
            .as_deref()
            .is_none_or(|subject| subject == user_id)
            .then_some(state.flow)
    })
}

/// Whether `save_training_plan` refuses a self-scope save while `flow` owns
/// the turn.
///
/// Every walk is an interview — it asks a question and records the answer —
/// so every one of them refuses. It takes the flow because the rule is about
/// what a flow is *for*: one that exists to write a plan cannot be denied the
/// save, and a flow-agnostic rule is what made such a flow unbuildable.
///
/// [`GuidedFlow::Fortnight`] is that flow, and it is the reason this is a
/// function. It never owns a turn today — `/fortnight` decides in Rust and
/// retires its marker in the same breath — so the answer it gives is not read
/// on any live path; it is stated here because the alternative is deciding it
/// implicitly, and a rail whose whole purpose is to save two weeks must never
/// inherit a refusal it was never considered for.
#[must_use]
pub(crate) const fn flow_refuses_plan_save(flow: GuidedFlow) -> bool {
    flow.is_interview()
}

#[cfg(test)]
mod tests {
    use super::flow_refuses_plan_save;
    use pierre_core::models::GuidedFlow;

    /// The four interviews keep their refusal when the fifth flow does not.
    ///
    /// The suppressions and the refusal read one predicate
    /// (`GuidedFlow::is_interview`), so a mistake there is a single edit that
    /// silently disarms the 2026-07-24 guard for every walk at once. This is
    /// the test that fires if that happens.
    #[test]
    fn every_interview_refuses_a_plan_save() {
        for flow in [
            GuidedFlow::Pillars,
            GuidedFlow::Calibration,
            GuidedFlow::Intake,
            GuidedFlow::Season,
        ] {
            assert!(
                flow.is_interview(),
                "bug: {flow:?} is an interview and must stay one"
            );
            assert!(
                flow_refuses_plan_save(flow),
                "bug: {flow:?} asks a question and records the answer — writing a \
                 plan mid-interview is the 2026-07-24 derail"
            );
        }
    }

    /// The fortnight rail is the flow this predicate was made a function for:
    /// a rail that exists to save two weeks and is refused the save would tell
    /// the athlete it wrote them and write nothing.
    #[test]
    fn the_fortnight_rail_may_save() {
        assert!(
            !GuidedFlow::Fortnight.is_interview(),
            "bug: the rail is the one flow that is not an interview"
        );
        assert!(
            !flow_refuses_plan_save(GuidedFlow::Fortnight),
            "bug: a flow whose whole purpose is to write a plan cannot be refused the save"
        );
    }
}
