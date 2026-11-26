// ABOUTME: The note a reply carries, once per athlete, when a science change moved numbers they had already seen
// ABOUTME: Due on the next turn whose tools show those numbers; claimed once per change, told ahead of the attribution (carnet#1718)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Science-change notes.
//!
//! When a science change moves numbers an athlete has already seen — how
//! training load is scored, how pace zones are cut — the agent says so once
//! per athlete, ever, the next time that topic comes up. The topic comes up
//! when a tool whose output shows those numbers ran in the turn
//! ([`ToolLoopResult::tools_called`]).
//!
//! The note is the platform's words: the catalogue's text in the reply's own
//! language, appended verbatim after a blank line, ahead of the Garmin
//! attribution so that line stays last. It is told only to the athlete
//! reading their own data — the turn's sender, on the subject's seat, in a
//! thread bound to no other athlete — whose account predates the change. A
//! coach reading an athlete is told nothing and marks nothing; an account
//! created after the change never saw the old numbers.
//!
//! Once is the once-claim the platform's own posts take
//! ([`PostPurpose::ScienceNote`], the change's id as its subject): a note is
//! appended only under a claim this turn took. [`HeldNotes::told_once`] marks
//! the claims once the reply is persisted. A turn that ends without persisting
//! its reply — withheld, stopped, a failed write, or dropped mid-turn —
//! releases them when its [`HeldNotes`] is dropped, so the next turn on the
//! topic tells the note.

use std::future::Future;
use std::mem;
use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_contremaitre::messaging_strings::{
    KEY_SCIENCE_CHANGE_FITNESS_SCORE, KEY_SCIENCE_CHANGE_PACE_ZONES,
    KEY_SCIENCE_CHANGE_TRAINING_LOAD,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationSubject, ConversationTurnId, PostPurpose, TenantId};
use pierre_core::uuid_utils;
use pierre_database::database::ConversationRecord;
use pierre_database::repositories::PlatformPostClaimRepository;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use tokio::runtime::Handle;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{reply_locale, source_attribution};
use crate::supervised::{self, TaskOwner};
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// Between the reply and a note, and between two notes.
const NOTE_SEPARATOR: &str = "\n\n";

/// How long a claim a turn took and has not marked is a note under way.
///
/// Past it, the turn holding it is presumed gone and the next turn on the
/// topic retakes it. A reply is persisted seconds after its claim, so the
/// quarter hour the platform's own posts are leased for is ample.
const NOTE_CLAIM_LEASE: Elapsed = Elapsed::minutes(15);

/// A science change that moved numbers an athlete may have seen before it.
struct ScienceChange {
    /// The subject its once-claim is stored under. Persisted on claim rows,
    /// so it never changes.
    id: &'static str,
    /// The catalogue key of the note's text.
    key: &'static str,
    /// When the change reached production, in epoch seconds. An account
    /// created at or after it never saw the numbers the change moved.
    effective_epoch_seconds: i64,
    /// The tools whose output shows the athlete the numbers the change moved.
    topic_tools: &'static [&'static str],
}

/// Every change owed a note, in the order the notes are appended.
const SCIENCE_CHANGES: &[ScienceChange] = &[
    // Sessions scored on normalized power, heart-rate reserve and threshold
    // pace (cageux 0.55), on main at 2026-11-21T22:03:59Z. Each tool's output
    // shows the athlete's CTL, ATL or form (TSB) computed from those scores.
    ScienceChange {
        id: "training_load_2026_11",
        key: KEY_SCIENCE_CHANGE_TRAINING_LOAD,
        effective_epoch_seconds: 1_795_298_639,
        topic_tools: &[
            "analyze_training_load",
            "get_training_history",
            "calculate_fitness_score",
            "calculate_recovery_score",
            "suggest_rest_day",
            "optimize_sleep_schedule",
            "generate_recommendations",
        ],
    },
    // Every pace-zone edge cut from Daniels' oxygen-cost curve (cageux 0.57),
    // on main at 2026-11-23T20:53:57Z.
    ScienceChange {
        id: "pace_zones_2026_11",
        key: KEY_SCIENCE_CHANGE_PACE_ZONES,
        effective_epoch_seconds: 1_795_467_237,
        topic_tools: &["calculate_personalized_zones"],
    },
    // The fitness score read on the athlete's heart-rate zones and sessions
    // per week, trended against the window before (cageux 0.60, carnet#1807),
    // on main at 2026-11-26T15:17:23Z.
    ScienceChange {
        id: "fitness_score_2026_11",
        key: KEY_SCIENCE_CHANGE_FITNESS_SCORE,
        effective_epoch_seconds: 1_795_706_243,
        topic_tools: &["calculate_fitness_score"],
    },
];

/// The athlete a note is told to, in their own tenant.
#[derive(Clone, Copy)]
struct Reader {
    tenant_id: TenantId,
    athlete: Uuid,
}

/// A once-claim a turn took for a note it appended.
struct HeldClaim {
    reader: Reader,
    note: &'static str,
    lease: String,
}

/// The notes a reply carries, each under the claim that makes it once.
///
/// [`Self::told_once`] marks them once the reply is persisted. Dropped
/// unmarked, it releases them on a detached task, so a reply that never
/// landed does not spend the athlete's note.
#[must_use = "dropping the notes releases their claims; mark them told once the reply is persisted"]
pub(crate) struct HeldNotes {
    /// The claim store, shared with the repository registry so a drop can
    /// release the claims on a task that outlives the turn.
    claims: Arc<dyn PlatformPostClaimRepository>,
    /// The turn the claims were taken for, named if their release fails.
    turn_id: ConversationTurnId,
    held: Vec<HeldClaim>,
}

/// `reply` with the note of each science change due on this turn appended, in
/// table order, and the claims that make each one once.
///
/// A change is due when a tool showing its numbers ran, the model answered,
/// the sender reads their own data, and their account predates the change. A
/// note the catalogue holds no text for is not told and takes no claim; nor is
/// one already told, or being told by another turn. An empty reply is
/// returned unchanged.
pub(crate) async fn tell_due(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    seat: TurnSeat,
    result: &ToolLoopResult,
    reply: String,
    locale: &str,
) -> (String, HeldNotes) {
    let mut notes = HeldNotes::new(Arc::clone(&ctx.repos.platform_post_claims), input.turn_id);
    let due = due_on(&result.tools_called);
    if due.is_empty() || !source_attribution::model_answered(result) || reply.trim().is_empty() {
        return (reply, notes);
    }
    let Ok(sender) = uuid_utils::parse_uuid(&input.user_id) else {
        return (reply, notes);
    };
    if !reads_own_data(seat, conv.subject(), sender) {
        debug!("science notes: the turn reads someone else's data; none told");
        return (reply, notes);
    }
    let created_at = match ctx.repos.users.get_global(sender).await {
        Ok(Some(user)) => user.created_at,
        Ok(None) => return (reply, notes),
        Err(e) => {
            warn!(error = %e, "science notes: the sender could not be read; none told");
            return (reply, notes);
        }
    };
    let note_locale = reply_locale::resolve_banner_locale(&reply, locale);
    let texts = with_text(
        due.into_iter()
            .filter(|change| saw_old_numbers(change, created_at)),
        |key| ctx.messaging_strings_registry.get(key, &note_locale),
    );
    // The athlete's own tenant: in a shared room the conversation's tenant is
    // the bot's.
    let reader = Reader {
        tenant_id: input.tool_tenant_id,
        athlete: sender,
    };
    let reply = notes.claim_and_append(reader, texts, reply).await;
    (reply, notes)
}

/// The changes whose numbers one of `tools_called` showed, in table order.
fn due_on(tools_called: &[String]) -> Vec<&'static ScienceChange> {
    SCIENCE_CHANGES
        .iter()
        .filter(|change| {
            change
                .topic_tools
                .iter()
                .any(|topic| tools_called.iter().any(|called| called == topic))
        })
        .collect()
}

/// Whether the turn's `sender` reads their own data: they hold the subject's
/// seat, in a thread bound to no athlete but them.
///
/// A coach's thread about an athlete is bound to that athlete. A room's coach
/// holds the coach's seat, where these tools are refused, and a headless loop
/// still lists the refused call among the calls it saw.
fn reads_own_data(seat: TurnSeat, subject: Option<ConversationSubject>, sender: Uuid) -> bool {
    seat.sender_is_subject() && subject.is_none_or(|subject| subject.athlete_user_id == sender)
}

/// Whether an account created at `created_at` saw the numbers `change` moved.
fn saw_old_numbers(change: &ScienceChange, created_at: Timestamp) -> bool {
    created_at.epoch_seconds() < change.effective_epoch_seconds
}

/// Each of `changes` with its note's text as `lookup` reads it by key. A
/// change whose text is empty is left out, so it is neither claimed nor told.
fn with_text(
    changes: impl IntoIterator<Item = &'static ScienceChange>,
    lookup: impl Fn(&str) -> String,
) -> Vec<(&'static ScienceChange, String)> {
    changes
        .into_iter()
        .map(|change| (change, lookup(change.key)))
        .filter(|(_, text)| !text.trim().is_empty())
        .collect()
}

impl HeldNotes {
    const fn new(
        claims: Arc<dyn PlatformPostClaimRepository>,
        turn_id: ConversationTurnId,
    ) -> Self {
        Self {
            claims,
            turn_id,
            held: Vec::new(),
        }
    }

    /// `reply` with each of `notes` appended that this call claims for
    /// `reader`, in order. A note already told, being told, or whose claim
    /// failed (logged) is left out.
    async fn claim_and_append(
        &mut self,
        reader: Reader,
        notes: Vec<(&'static ScienceChange, String)>,
        mut reply: String,
    ) -> String {
        for (change, text) in notes {
            let now = Timestamp::now();
            let claimed = self
                .claims
                .claim_platform_post_once(
                    reader.tenant_id,
                    reader.athlete,
                    PostPurpose::ScienceNote.as_str(),
                    change.id,
                    now - NOTE_CLAIM_LEASE,
                    now,
                )
                .await;
            match claimed {
                Ok(Some(lease)) => {
                    debug!(note = change.id, "science note told");
                    reply.push_str(NOTE_SEPARATOR);
                    reply.push_str(&text);
                    self.held.push(HeldClaim {
                        reader,
                        note: change.id,
                        lease,
                    });
                }
                Ok(None) => debug!(note = change.id, "science note already told or under way"),
                Err(e) => {
                    warn!(note = change.id, error = %e, "science note: the once-claim failed");
                }
            }
        }
        reply
    }

    /// The outcome of `persisted`, the write of the reply that carries the
    /// notes, with the notes marked told once it lands. A failed write drops
    /// them unmarked, which releases their claims.
    pub(crate) async fn told_once<T>(
        self,
        persisted: impl Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        let outcome = persisted.await;
        if outcome.is_ok() {
            self.mark_told().await;
        }
        outcome
    }

    /// Mark every note this reply carried as told, so its claim is never
    /// retaken. A mark that fails (logged) leaves the claim to lapse with its
    /// lease, and the note may then be told a second time.
    async fn mark_told(mut self) {
        for claim in mem::take(&mut self.held) {
            let marked = self
                .claims
                .mark_platform_post_posted(
                    claim.reader.tenant_id,
                    claim.reader.athlete,
                    PostPurpose::ScienceNote.as_str(),
                    claim.note,
                    &claim.lease,
                    Timestamp::now(),
                )
                .await;
            match marked {
                Ok(true) => {}
                Ok(false) => warn!(
                    note = claim.note,
                    "science note: its claim was retaken before it was marked; it may be told again"
                ),
                Err(e) => warn!(
                    note = claim.note,
                    error = %e,
                    "science note: marking the told note failed; its claim may be retaken"
                ),
            }
        }
    }
}

impl Drop for HeldNotes {
    fn drop(&mut self) {
        if self.held.is_empty() {
            return;
        }
        let held = mem::take(&mut self.held);
        let Ok(runtime) = Handle::try_current() else {
            warn!(
                notes = held.len(),
                "science notes: no runtime to release unmarked claims; they lapse with their lease"
            );
            return;
        };
        let claims = Arc::clone(&self.claims);
        let owner = TaskOwner::Turn(self.turn_id);
        supervised::spawn_supervised(&runtime, "science_note_release", owner, async move {
            for claim in held {
                release(claims.as_ref(), &claim).await;
            }
        });
    }
}

/// Release `claim`, whose note never reached a persisted reply, so the next
/// turn on the topic tells it.
async fn release(claims: &dyn PlatformPostClaimRepository, claim: &HeldClaim) {
    if let Err(e) = claims
        .release_platform_post(
            claim.reader.tenant_id,
            claim.reader.athlete,
            PostPurpose::ScienceNote.as_str(),
            claim.note,
            &claim.lease,
        )
        .await
    {
        warn!(
            note = claim.note,
            error = %e,
            "science note: releasing the claim of an untold note failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use dravr_tronc::dates::Timestamp;
    use pierre_core::errors::{AppError, AppResult};
    use pierre_core::models::{ConversationSubject, ConversationTurnId, TenantId};
    use pierre_database::repositories::PlatformPostClaimRepository;
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use tokio::task;
    use uuid::Uuid;

    use super::{
        due_on, reads_own_data, saw_old_numbers, with_text, HeldNotes, Reader, ScienceChange,
        SCIENCE_CHANGES,
    };

    fn change(id: &str) -> &'static ScienceChange {
        SCIENCE_CHANGES
            .iter()
            .find(|change| change.id == id)
            .unwrap_or_else(|| panic!("test bug: no science change {id}"))
    }

    fn training_load() -> &'static ScienceChange {
        change("training_load_2026_11")
    }

    fn pace_zones() -> &'static ScienceChange {
        change("pace_zones_2026_11")
    }

    fn fitness_score() -> &'static ScienceChange {
        change("fitness_score_2026_11")
    }

    fn called(tools: &[&str]) -> Vec<String> {
        tools.iter().map(|tool| (*tool).to_owned()).collect()
    }

    fn ids(changes: &[&ScienceChange]) -> Vec<&'static str> {
        changes.iter().map(|change| change.id).collect()
    }

    fn at(rfc3339: &str) -> Timestamp {
        Timestamp::parse_from_rfc3339(rfc3339).expect("test bug: an RFC 3339 instant")
    }

    #[test]
    fn a_tool_showing_the_numbers_makes_its_change_due() {
        assert_eq!(
            ids(&due_on(&called(&[
                "get_activities",
                "analyze_training_load"
            ]))),
            ["training_load_2026_11"],
            "the load tool makes the load change due; get_activities shows neither change's numbers"
        );
        assert_eq!(
            ids(&due_on(&called(&["suggest_rest_day"]))),
            ["training_load_2026_11"],
            "form (TSB) is among the numbers the load change moved"
        );
        assert_eq!(
            ids(&due_on(&called(&["calculate_personalized_zones"]))),
            ["pace_zones_2026_11"],
            "the zones tool makes the pace-zone change due"
        );
        assert_eq!(
            ids(&due_on(&called(&[
                "calculate_personalized_zones",
                "get_training_history",
                "get_training_history",
            ]))),
            ["training_load_2026_11", "pace_zones_2026_11"],
            "each change once, in table order, whatever the call order"
        );
        assert_eq!(
            ids(&due_on(&called(&["calculate_fitness_score"]))),
            ["training_load_2026_11", "fitness_score_2026_11"],
            "the fitness score shows the athlete's CTL beside a score the fitness change moved"
        );
    }

    #[test]
    fn a_turn_without_a_topic_tool_owes_no_note() {
        assert!(due_on(&[]).is_empty(), "no tool ran");
        assert!(
            due_on(&called(&[
                "get_activities",
                "get_roster_overview",
                "predict_performance"
            ]))
            .is_empty(),
            "none of these shows the athlete's own load or pace zones"
        );
    }

    #[test]
    fn only_an_account_older_than_the_change_saw_the_old_numbers() {
        let load = training_load();
        assert!(
            saw_old_numbers(load, at("2026-11-21T22:03:58Z")),
            "an account created a second before the load change saw its old numbers"
        );
        assert!(
            !saw_old_numbers(load, at("2026-11-21T22:03:59Z")),
            "an account created as the change landed only ever saw the new numbers"
        );
        let zones = pace_zones();
        assert!(
            saw_old_numbers(zones, at("2026-11-23T20:53:56Z")),
            "an account created a second before the pace-zone change saw its old zones"
        );
        assert!(
            !saw_old_numbers(zones, at("2026-11-23T20:53:57Z")),
            "an account created as the pace-zone change landed only ever saw the new zones"
        );
        assert!(
            saw_old_numbers(zones, at("2026-11-22T12:00:00Z"))
                && !saw_old_numbers(load, at("2026-11-22T12:00:00Z")),
            "an account created between the two changes saw only the old pace zones"
        );
        let fitness = fitness_score();
        assert!(
            saw_old_numbers(fitness, at("2026-11-26T15:17:22Z")),
            "an account created a second before the fitness change saw its old score"
        );
        assert!(
            !saw_old_numbers(fitness, at("2026-11-26T15:17:23Z")),
            "an account created as the fitness change landed only ever saw the new score"
        );
    }

    #[test]
    fn a_coach_reading_an_athlete_is_told_nothing() {
        let athlete = Uuid::new_v4();
        let coach = Uuid::new_v4();
        let thread = ConversationSubject {
            athlete_user_id: athlete,
            group_id: Uuid::new_v4(),
        };
        assert!(
            !reads_own_data(TurnSeat::AthleteThread, Some(thread), coach),
            "a coach's thread about an athlete"
        );
        assert!(
            !reads_own_data(TurnSeat::Coach, None, coach),
            "a room's coach, where a headless loop lists the refused call"
        );
        assert!(
            reads_own_data(TurnSeat::Subject, None, athlete),
            "the athlete in their own thread, or in a room they train in"
        );
    }

    /// A claim store that records what it was asked, holding `told` as
    /// already posted. A mock rather than the database so that a failed write
    /// and the detached release can be observed here; the integration suite
    /// `science_note_reply_test` drives the real store.
    #[derive(Default)]
    struct RecordingClaims {
        told: BTreeSet<String>,
        /// The tenant and athlete each call was keyed on, in call order.
        keyed_on: Mutex<Vec<(TenantId, Uuid)>>,
        claimed: Mutex<Vec<String>>,
        marked: Mutex<Vec<String>>,
        released: Mutex<Vec<String>>,
    }

    impl RecordingClaims {
        fn keyed_on(&self) -> Vec<(TenantId, Uuid)> {
            self.keyed_on.lock().unwrap().clone()
        }
        fn claimed(&self) -> Vec<String> {
            self.claimed.lock().unwrap().clone()
        }
        fn marked(&self) -> Vec<String> {
            self.marked.lock().unwrap().clone()
        }
        fn released(&self) -> Vec<String> {
            self.released.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PlatformPostClaimRepository for RecordingClaims {
        async fn claim_platform_post_once(
            &self,
            tenant_id: TenantId,
            user_id: Uuid,
            purpose: &str,
            subject: &str,
            _lease_expired_before: Timestamp,
            _claimed_at: Timestamp,
        ) -> AppResult<Option<String>> {
            assert_eq!(
                purpose, "science_note",
                "test bug: a science note's purpose"
            );
            self.keyed_on.lock().unwrap().push((tenant_id, user_id));
            self.claimed.lock().unwrap().push(subject.to_owned());
            Ok((!self.told.contains(subject)).then(|| format!("lease-{subject}")))
        }

        async fn mark_platform_post_posted(
            &self,
            tenant_id: TenantId,
            user_id: Uuid,
            _purpose: &str,
            subject: &str,
            lease_token: &str,
            _posted_at: Timestamp,
        ) -> AppResult<bool> {
            self.keyed_on.lock().unwrap().push((tenant_id, user_id));
            assert_eq!(
                lease_token,
                format!("lease-{subject}"),
                "the claim's own lease"
            );
            self.marked.lock().unwrap().push(subject.to_owned());
            Ok(true)
        }

        async fn claim_platform_post_since(
            &self,
            _tenant_id: TenantId,
            _user_id: Uuid,
            _purpose: &str,
            _subject: &str,
            _since: Timestamp,
            _claimed_at: Timestamp,
        ) -> AppResult<Option<String>> {
            panic!("a science note is owed once ever, never per period")
        }

        async fn release_platform_post(
            &self,
            tenant_id: TenantId,
            user_id: Uuid,
            _purpose: &str,
            subject: &str,
            lease_token: &str,
        ) -> AppResult<bool> {
            self.keyed_on.lock().unwrap().push((tenant_id, user_id));
            assert_eq!(
                lease_token,
                format!("lease-{subject}"),
                "the claim's own lease"
            );
            self.released.lock().unwrap().push(subject.to_owned());
            Ok(true)
        }

        async fn platform_post_posted_at(
            &self,
            _tenant_id: TenantId,
            _user_id: Uuid,
            _purpose: &str,
        ) -> AppResult<Option<Timestamp>> {
            panic!("test bug: no reader asks when a note was told")
        }

        async fn platform_posts_landed_since(
            &self,
            _tenant_id: TenantId,
            _user_id: Uuid,
            _since: Timestamp,
        ) -> AppResult<u32> {
            panic!("test bug: a science note rides a reply, so no daily cap counts it")
        }
    }

    fn reader() -> Reader {
        Reader {
            tenant_id: TenantId::generate(),
            athlete: Uuid::new_v4(),
        }
    }

    fn notes_over(claims: &Arc<RecordingClaims>) -> HeldNotes {
        let store: Arc<dyn PlatformPostClaimRepository> = claims.clone();
        HeldNotes::new(store, ConversationTurnId::new())
    }

    #[tokio::test]
    async fn a_note_with_no_text_takes_no_claim() {
        let claims = Arc::new(RecordingClaims::default());
        let mut notes = notes_over(&claims);
        let texts = with_text([training_load(), pace_zones()], |_| "  ".to_owned());
        let reply = notes
            .claim_and_append(reader(), texts, "Ta forme monte.".to_owned())
            .await;
        assert_eq!(reply, "Ta forme monte.", "nothing is appended");
        assert!(claims.claimed().is_empty(), "and nothing is claimed");
        notes.mark_told().await;
        assert!(claims.marked().is_empty(), "nor marked");
    }

    #[tokio::test]
    async fn each_due_note_is_claimed_and_appended_in_table_order() {
        let claims = Arc::new(RecordingClaims::default());
        let mut notes = notes_over(&claims);
        let texts = with_text([training_load(), pace_zones()], |key| format!("[{key}]"));
        let athlete = reader();
        let reply = notes
            .claim_and_append(athlete, texts, "Ta forme monte.".to_owned())
            .await;
        assert_eq!(
            reply,
            "Ta forme monte.\n\n[chat.science_change.training_load]\n\n\
             [chat.science_change.pace_zones]",
            "each note follows the reply after a blank line, in table order"
        );
        assert_eq!(
            claims.claimed(),
            ["training_load_2026_11", "pace_zones_2026_11"],
            "each appended note is claimed, in table order"
        );
        notes
            .told_once(async { Ok(()) })
            .await
            .expect("the reply's write landed");
        assert_eq!(
            claims.marked(),
            ["training_load_2026_11", "pace_zones_2026_11"],
            "each claim is marked once the reply's write landed"
        );
        assert_eq!(
            claims.keyed_on(),
            [(athlete.tenant_id, athlete.athlete); 4],
            "every claim and mark is keyed on the reader's own tenant and account"
        );
        assert!(
            claims.released().is_empty(),
            "a told note is never released"
        );
    }

    #[tokio::test]
    async fn a_note_already_told_is_not_told_again() {
        let claims = Arc::new(RecordingClaims {
            told: BTreeSet::from(["training_load_2026_11".to_owned()]),
            ..RecordingClaims::default()
        });
        let mut notes = notes_over(&claims);
        let texts = with_text([training_load(), pace_zones()], |key| format!("[{key}]"));
        let reply = notes
            .claim_and_append(reader(), texts, "Ta forme monte.".to_owned())
            .await;
        assert_eq!(
            reply, "Ta forme monte.\n\n[chat.science_change.pace_zones]",
            "only the note not yet told is appended"
        );
        notes.mark_told().await;
        assert_eq!(
            claims.marked(),
            ["pace_zones_2026_11"],
            "only the claim this turn took is marked"
        );
    }

    /// Notes holding the training-load claim for `athlete`, over `claims`.
    async fn holding_one(claims: &Arc<RecordingClaims>, athlete: Reader) -> HeldNotes {
        let mut notes = notes_over(claims);
        let texts = with_text([training_load()], |key| format!("[{key}]"));
        notes
            .claim_and_append(athlete, texts, "Ta forme monte.".to_owned())
            .await;
        notes
    }

    /// What `claims` released, once the detached release had its turn.
    async fn released_after_drop(claims: &RecordingClaims) -> Vec<String> {
        for _ in 0..10 {
            if !claims.released().is_empty() {
                break;
            }
            task::yield_now().await;
        }
        claims.released()
    }

    #[tokio::test]
    async fn a_turn_that_ends_before_its_reply_is_written_releases_its_notes() {
        let claims = Arc::new(RecordingClaims::default());
        let athlete = reader();
        drop(holding_one(&claims, athlete).await);
        assert_eq!(
            released_after_drop(&claims).await,
            ["training_load_2026_11"],
            "the claim of a reply never written is released"
        );
        assert!(claims.marked().is_empty(), "an untold note is not marked");
        assert_eq!(
            claims.keyed_on(),
            [(athlete.tenant_id, athlete.athlete); 2],
            "the release is keyed on the reader the claim was taken for"
        );
    }

    #[tokio::test]
    async fn a_failed_write_releases_its_notes() {
        let claims = Arc::new(RecordingClaims::default());
        let written = holding_one(&claims, reader())
            .await
            .told_once(async { Err::<(), _>(AppError::database("the write failed")) })
            .await;
        assert!(written.is_err(), "the write's failure is the caller's");
        assert_eq!(
            released_after_drop(&claims).await,
            ["training_load_2026_11"],
            "a failed write releases the claim"
        );
        assert!(
            claims.marked().is_empty(),
            "an unwritten note is not marked"
        );
    }
}
