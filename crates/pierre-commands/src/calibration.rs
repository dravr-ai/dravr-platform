// ABOUTME: Handler for /calibrate — start the difficulty-calibration interview on a conversation
// ABOUTME: Supersedes the previous interview's answers by window, snapshots recent load, opens the flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_OPENER, KEY_CALIBRATE_OPENER_ROOM, KEY_CALIBRATE_START_FAILED,
};
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, GuidedFlow, OnboardingState, Pillar, WalkAudience};
use pierre_messaging::commands::CommandResponse;
use pierre_services::recent_load::recent_load_snapshot;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// Profile-JSON key holding the RFC3339 start of the athlete's most recent
/// calibration interview.
///
/// It lives in the user profile rather than the conversation's flow state
/// because a re-run may well happen in a different conversation — a first
/// calibration over Telegram, the next one on the web — and the flow state is
/// per-conversation and cleared on completion.
const PROFILE_KEY_CALIBRATION: &str = "calibration";
/// Sub-key under [`PROFILE_KEY_CALIBRATION`].
const PROFILE_KEY_LAST_STARTED_AT: &str = "last_started_at";

/// Handler for `/calibrate` — tune how hard this athlete's training should be.
///
/// A short guided interview whose answers land as facts that steer later plan
/// generation. Bare `/calibrate` is the only form: an athlete typing `/harder`
/// expects an adjustment to today's plan, not a six-question interview, so that
/// alias is deliberately absent and discovery runs through the coach instead.
///
/// Works in a direct message and in a shared room alike. Typing the command in
/// a room is the athlete's consent to a room-visible interview — the same
/// per-invocation grant `/plan share` established — and the walk binds to them
/// alone: the state carries their `subject_user_id`, so nobody else's message
/// advances it, and their coach follows along read-only. The interview state
/// and opener land on the conversation row's own tenant (the channel tenant in
/// a room), while the athlete-scoped writes below — the supersession window,
/// the profile stamp, the load snapshot — stay under the athlete's own tenant,
/// which is also where the turn pipeline stamps every answer's extraction.
pub struct CalibrateHandler;

/// The previous interview's start, if this athlete has calibrated before.
fn last_calibration_start(profile: Option<&Value>) -> Option<DateTime<Utc>> {
    profile?
        .get(PROFILE_KEY_CALIBRATION)?
        .get(PROFILE_KEY_LAST_STARTED_AT)?
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// The athlete's profile with this interview's start time recorded, preserving
/// every other key.
///
/// `upsert_profile` replaces the whole document, so the existing profile is
/// read and merged rather than overwritten — writing a bare `{"calibration":
/// …}` would drop the athlete's nutrition and equipment blocks.
fn profile_with_calibration_start(profile: Option<Value>, started_at: &str) -> Value {
    let mut doc = match profile {
        Some(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    };
    if let Some(map) = doc.as_object_mut() {
        map.insert(
            PROFILE_KEY_CALIBRATION.to_owned(),
            json!({ PROFILE_KEY_LAST_STARTED_AT: started_at }),
        );
    }
    doc
}

#[async_trait]
impl CommandHandler for CalibrateHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();

        let conversation_id = ctx.conversation_id.as_deref().ok_or_else(|| {
            AppError::invalid_input("/calibrate needs an active conversation to interview in")
        })?;
        let repos = ctx.ctx.repos();
        let user = ctx.user_id.to_string();
        let started_at = Utc::now();

        // Supersede the previous interview's answers before capturing new ones.
        //
        // The window is the only thing that identifies them. Calibration topics
        // cannot be matched by kind and pillar — most land as a `preference` in
        // the training pillar, so a per-topic match would collapse six distinct
        // answers onto one row — and the extractor, not this code, chooses each
        // fact's subject and predicate, so there is no stable natural key to
        // converge on either. Scoping to the training pillar keeps a
        // concurrently-running pillars walk's other five pillars untouched;
        // a training-pillar answer it wrote inside the same window is the one
        // overlap, and losing it costs less than leaving the athlete with two
        // contradictory progression intents both rendered as current.
        //
        // A read that *fails* is not an athlete without a profile, and the two
        // must not collapse: `upsert_profile` below replaces the whole
        // document, so starting from `None` after a failed read writes a bare
        // `{"calibration": …}` over the athlete's nutrition and equipment
        // blocks — the ones `compose_dossier` reads out of this same blob — and
        // supersedes nothing. The command reports the failure instead, which
        // leaves the stored profile exactly as it was.
        let profile = repos
            .profiles
            .get_profile(ctx.user_id)
            .await
            .inspect_err(|e| {
                warn!(
                    error = %e,
                    user_id = %ctx.user_id,
                    "/calibrate could not read the athlete's profile; interview not started"
                );
            })?;
        if let Some(previous) = last_calibration_start(profile.as_ref()) {
            let superseded = repos
                .memory
                .expire_onboarding_facts(
                    ctx.tenant_id,
                    &user,
                    Some(Pillar::TrainingAndMovement),
                    Some(previous),
                    // Pillar-scoped re-screen: no per-question narrowing.
                    None,
                )
                .await?;
            info!(
                user_id = %ctx.user_id,
                superseded,
                previous = %previous,
                "/calibrate re-run superseded the previous interview"
            );
        }

        // Record this interview's window for the *next* re-run to supersede.
        // A failure here is not fatal to the interview — it costs the next
        // re-run its supersession, which shows up as duplicate answers rather
        // than a broken flow, so the interview proceeds and the miss is logged.
        let merged = profile_with_calibration_start(profile, &started_at.to_rfc3339());
        if let Err(e) = repos.profiles.upsert_profile(ctx.user_id, merged).await {
            warn!(error = %e, user_id = %ctx.user_id, "failed to record the calibration window");
        }

        // Snapshot recent load once, here, so every later turn quotes the same
        // figures. `None` means no cached activity — the interview then asks
        // for the athlete's typical week instead of quoting a baseline.
        let snapshot = recent_load_snapshot(
            repos.activity_cache.as_ref(),
            ctx.user_id,
            &ctx.tenant_id,
        )
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "activity cache unreadable; calibration will ask for the baseline");
            None
        });

        // The subject binding is what keeps a shared thread's walk the caller's
        // own; the audience records where they chose to run it, which is the
        // consent record and what filters room-unsafe topics.
        let audience = if ctx.is_direct_message {
            WalkAudience::Private
        } else {
            WalkAudience::Room
        };
        let state = OnboardingState::start(started_at.to_rfc3339(), GuidedFlow::Calibration)
            .with_snapshot(snapshot)
            .with_subject(user.clone())
            .with_audience(audience);
        let json = state.to_column().map_err(|e| {
            AppError::internal(format!("failed to serialize calibration state: {e}"))
        })?;

        // The returned `bool` reports whether a row actually matched: a tenant
        // mismatch updates nothing, and answering with the opener anyway would
        // start an interview that no turn ever runs in.
        // The conversation row lives under the conversation's own tenant — the
        // caller's in a DM, the channel tenant in a shared room — and the
        // update matches only there.
        let activated = repos
            .chat
            .set_conversation_onboarding_state(
                conversation_id,
                Some(&json),
                ctx.conversation_tenant_id,
            )
            .await?;
        if !activated {
            warn!(
                user_id = %ctx.user_id,
                conversation_id,
                "/calibrate matched no conversation row; interview not activated"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_CALIBRATE_START_FAILED,
                &ctx.locale,
                &[],
            )));
        }

        info!(
            user_id = %ctx.user_id,
            audience = ?audience,
            "calibration interview activated via /calibrate"
        );

        // The room opener says out loud what the athlete just chose: the
        // exchange is visible here, and it is theirs alone to answer.
        let opener_key = match audience {
            WalkAudience::Private => KEY_CALIBRATE_OPENER,
            WalkAudience::Room => KEY_CALIBRATE_OPENER_ROOM,
        };
        let msg = reg.render(opener_key, &ctx.locale, &[]);

        // Persist the opener as an assistant message, for the same two reasons
        // `/pillars` does: the coach otherwise receives the athlete's first
        // answer with no question attached and improvises a reply to a question
        // it cannot see, and with no history row that answer is message #1,
        // which arms the first-turn startup prefetch.
        let opener = AddMessageParams {
            tenant_id: ctx.conversation_tenant_id,
            conversation_id,
            user_id: &user,
            role: "assistant",
            content: &msg,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        };
        if let Err(e) = repos.chat.add_message(&opener).await {
            warn!(error = %e, "failed to persist /calibrate opener as assistant message");
        }

        Ok(CommandResponse::rich_text(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::{last_calibration_start, profile_with_calibration_start};
    use serde_json::json;

    #[test]
    fn a_first_calibration_has_no_window_to_supersede() {
        assert!(last_calibration_start(None).is_none());
        assert!(last_calibration_start(Some(&json!({}))).is_none());
        assert!(last_calibration_start(Some(&json!({ "nutrition": { "carbs": 60 } }))).is_none());
    }

    #[test]
    fn a_recorded_window_round_trips_through_the_profile() {
        let doc = profile_with_calibration_start(None, "2026-07-28T12:00:00+00:00");
        assert_eq!(
            last_calibration_start(Some(&doc)).map(|d| d.to_rfc3339()),
            Some("2026-07-28T12:00:00+00:00".to_owned()),
            "the window must be readable back"
        );
    }

    #[test]
    fn recording_the_window_preserves_the_rest_of_the_profile() {
        // `upsert_profile` replaces the whole document. Writing only the
        // calibration key would silently drop nutrition and equipment, which
        // compose_dossier reads straight out of this blob.
        let existing = json!({
            "nutrition": { "carbs_per_hour": 60 },
            "equipment": { "bike": "Tarmac" },
        });
        let merged = profile_with_calibration_start(Some(existing), "2026-07-28T12:00:00+00:00");
        assert_eq!(merged["nutrition"]["carbs_per_hour"], 60);
        assert_eq!(merged["equipment"]["bike"], "Tarmac");
        assert!(last_calibration_start(Some(&merged)).is_some());
    }

    #[test]
    fn a_second_calibration_overwrites_the_stored_window() {
        let first = profile_with_calibration_start(None, "2026-06-01T00:00:00+00:00");
        let second = profile_with_calibration_start(Some(first), "2026-07-28T12:00:00+00:00");
        assert_eq!(
            last_calibration_start(Some(&second)).map(|d| d.to_rfc3339()),
            Some("2026-07-28T12:00:00+00:00".to_owned()),
            "the next re-run must supersede the most recent interview, not the first"
        );
    }

    #[test]
    fn a_non_object_profile_does_not_lose_the_window() {
        // Defensive: a profile column holding a bare array or string is not
        // something this handler should propagate a panic over.
        let merged = profile_with_calibration_start(
            Some(json!(["unexpected"])),
            "2026-07-28T12:00:00+00:00",
        );
        assert!(last_calibration_start(Some(&merged)).is_some());
    }

    #[test]
    fn a_malformed_stored_timestamp_reads_as_no_window() {
        let doc = json!({ "calibration": { "last_started_at": "not a timestamp" } });
        assert!(
            last_calibration_start(Some(&doc)).is_none(),
            "an unparseable window must not be passed to the expiry as a bound"
        );
    }
}
