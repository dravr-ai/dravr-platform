// ABOUTME: What of a conversation's loaded history a turn may read before the prompt is built
// ABOUTME: A first-party-only thread on an external turn, or a refused athlete thread, reads only the new message; a member read taints what follows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History scope.
//!
//! Two rules keep earlier turns out of the model's input, each leaving only
//! the row the turn just wrote:
//!
//! - A thread opened from a first-party-only activity is about that activity
//!   throughout, the athlete's own first question included: an external turn
//!   reads none of it but the question it answers, and a first-party turn is
//!   stamped by it (carnet#769).
//! - A coach's thread about one athlete runs the coaching gates again on every
//!   turn (carnet#1070); once they refuse, the turn reads the coach's new
//!   message alone ([`subject_athlete::resolve_withholding_history`],
//!   carnet#1168).
//!
//! A third keeps out only some rows: one about a group member — a tool round
//! that read them, the reply drawn from it, a summary of those — replays
//! only while the turn's sender may still read that member, by the member's
//! consent in the conversation's group ([`member_replay`], carnet#1174),
//! and only into a room that may read them: a messaging group chat, which
//! everyone in it reads as a reply is posted, replays only the members who
//! share with the room. The room is the replaying
//! turn's own, decided now by the one classifier
//! ([`CallRoom`]): a row written where the coach alone read it is
//! replayed into no chat others read, whatever the row has become since
//! (a group chat's session repointed at it, carnet#1422). The history load
//! and the compaction summaries apply it.
//!
//! A conversation remembers what it read (carnet#1422): a reply that
//! restates replayed history reads no member itself, yet carries them, and
//! so does every reply drawn from that one. So every row at or after the
//! first row about a member the turn may not read is withheld with it, a
//! summary ending on such a row too, back to the newest divider, after which
//! the model starts over. The turn that replays a member's rows reads that
//! member, as a tool's read does: its reply is stamped with them, and
//! nothing it says is filed on its sender.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CarriedRead;
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::repositories::{MemberReplay, AUTHOR_READ_PREFIX, UNATTRIBUTED_READ_KEY};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::call_room::CallRoom;
use pierre_services::reply_attribution::garmin_attribution_lines;
use pierre_services::{member_reads, transcript_reading};
use pierre_tool_runtime::derived_content;
use tracing::warn;
use uuid::Uuid;

use super::persistence::get_conversation_history;
use super::subject_athlete::{self, SubjectTurn};
use crate::{ChatPipelineContext, TurnInput};

/// What of `conv`'s rows `input`'s turn may replay, decided now
/// ([`member_replay`]).
#[derive(Debug, Default)]
pub(crate) struct ReplayScope {
    /// The members a legacy row may name in `about_user_id` and replay.
    readable_about: Vec<Uuid>,
    /// The first row carrying a read the turn may not read.
    tainted_from: Option<String>,
    /// Whether no row but the turn's own replays.
    withhold_all: bool,
    /// Whether only rows written with their record replay: a room, or a
    /// thread others read, never replays a legacy row (carnet#1511 call 17).
    recorded_only: bool,
    /// The reads the rows before `tainted_from` carry, the sender's own data
    /// aside, their authors' words among them: what the replay reads.
    replayed: Vec<CarriedRead>,
    /// Whether a row before `tainted_from` is a legacy row whose reads no
    /// row names ([`UNATTRIBUTED_READ_KEY`], carnet#1511 A8): the replay
    /// draws on members no one can tell.
    unattributed: bool,
}

impl ReplayScope {
    /// The members a legacy row may name and replay, whose legacy summaries
    /// replay with their rows.
    pub(crate) fn readable_about(&self) -> &[Uuid] {
        &self.readable_about
    }

    /// Whether only rows and summaries written with their record replay.
    pub(crate) const fn recorded_only(&self) -> bool {
        self.recorded_only
    }

    /// The scope as the history load binds it, the turn's own row `current`
    /// always kept.
    pub(crate) fn replay<'a>(&'a self, current: &'a str) -> MemberReplay<'a> {
        MemberReplay {
            readable_about: &self.readable_about,
            tainted_from: self.tainted_from.as_deref(),
            withhold_all: self.withhold_all,
            recorded_only: self.recorded_only,
            current: Some(current),
        }
    }

    /// Record on the turn's provenance every read its replayed rows carry,
    /// once `history` holds any row besides `current`: what the turn says may
    /// restate them, as it may restate a tool's read. Each is noted as it was
    /// read, another sender's own words among them (carnet#1511 R2), so
    /// nothing the turn says is filed on its sender.
    pub(crate) fn note_replayed(&self, history: &[MessageRecord], current: &str) {
        if history.iter().all(|row| row.id == current) {
            return;
        }
        for read in &self.replayed {
            ai_scope::note_replayed_member_read(read.clone());
        }
        if self.unattributed {
            ai_scope::note_replayed_unknown_member();
        }
    }
}

/// What of `conv`'s rows `input`'s turn may replay, in the room that turn
/// runs in, decided now.
///
/// Each read a row carries is decided by the one rule
/// ([`transcript_reading::readable_in`]) for the sender, in the room the
/// turn runs in: a room confirmed neither private nor a group's
/// ([`CallRoom::Unresolved`]) reads only the sender's own data, and a
/// messaging group chat, which everyone in it reads as a reply is posted,
/// only what the whole room may read (carnet#1511 design call 6). A row
/// another participant wrote is read as their own words. A row carrying a
/// read the turn may not read withholds itself and everything after it
/// ([`ReplayScope`]). A room, or a thread others read, replays no row
/// written before records existed (call 17). A legacy row the thread
/// replays whose reads no row names draws on members no one can tell
/// ([`UNATTRIBUTED_READ_KEY`], A8); it withholds nothing, as the legacy
/// rows it sits among replay by their own member's rule. When the reads cannot be
/// listed or checked, no row but the turn's own replays.
pub(crate) async fn member_replay(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    input: &TurnInput,
) -> ReplayScope {
    let room = CallRoom::of_conversation(repos, conv, input.is_direct_message).await;
    let (sender, tenant) = (input.user_id.as_str(), input.conversation_tenant_id);
    let resolved = async {
        let reader = parse_uuid(sender)?;
        let recorded_only = !matches!(room, CallRoom::Private)
            || room_reads_reply(input)
            || others_read(repos, conv, reader, tenant).await?;
        let reads = repos
            .chat
            .list_member_reads(&conv.id, sender, tenant)
            .await?;
        // A legacy summary names one member in `about_user_id`: it replays
        // only while its rows would, by its member's rule (call 17).
        let legacy_blocks: Vec<String> = repos
            .memory
            .list_compaction_blocks(&conv.id, tenant)
            .await?
            .into_iter()
            .filter_map(|block| block.about_user_id)
            .map(CarriedRead::legacy_about_key)
            .collect();
        if reads.is_empty() && legacy_blocks.is_empty() {
            return Ok(ReplayScope {
                recorded_only,
                ..ReplayScope::default()
            });
        }
        let group = conv.group_id.as_deref().map(parse_uuid).transpose()?;
        // Each row's reads by key, another participant's own words read as
        // theirs in the thread, or through the room's peer view in a room.
        let rows: Vec<(&str, String)> = reads
            .iter()
            .map(|read| {
                let key = read.key.strip_prefix(AUTHOR_READ_PREFIX).map_or_else(
                    || read.key.clone(),
                    |author| match (Uuid::parse_str(author), group) {
                        (Ok(author), Some(group)) => CarriedRead::peer(author, Some(group)).key(),
                        (Ok(author), None) => CarriedRead::thread(author).key(),
                        // An author no id names is read by no one.
                        (Err(_), _) => read.key.clone(),
                    },
                );
                (read.message_id.as_str(), key)
            })
            .collect();
        let mut keys: Vec<String> = Vec::new();
        let unattributed = |key: &str| key == UNATTRIBUTED_READ_KEY;
        let carried = rows.iter().map(|(_, key)| key).chain(&legacy_blocks);
        for key in carried {
            if !unattributed(key) && !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        let mut readable = transcript_reading::readable_in(repos, &room, reader, &keys).await?;
        if room_reads_reply(input) {
            readable = readable_by_the_chat(repos, conv, reader, &readable).await?;
        }
        let taint = rows
            .iter()
            .position(|(_, key)| !unattributed(key) && !readable.contains(key));
        let before_taint = &rows[..taint.unwrap_or(rows.len())];
        let mut replayed: Vec<CarriedRead> = Vec::new();
        for (_, key) in before_taint {
            if let Some(read) = CarriedRead::from_key(key) {
                if read.member != reader && !replayed.contains(&read) {
                    replayed.push(read);
                }
            } else if let Some(member) = CarriedRead::legacy_about_member(key) {
                // A legacy row names its member with no way: it is read the
                // way the sender reads that member now (`legacy_read`), which the reply
                // drawn from it then carries (call 17).
                if member != reader {
                    let read = member_reads::legacy_read(repos, reader, member)
                        .await?
                        .unwrap_or_else(|| CarriedRead::peer(member, None));
                    if !replayed.contains(&read) {
                        replayed.push(read);
                    }
                }
            }
        }
        let readable_about = readable
            .iter()
            .filter_map(|key| CarriedRead::legacy_about_member(key))
            .collect();
        Ok(ReplayScope {
            readable_about,
            tainted_from: taint.map(|at| rows[at].0.to_owned()),
            withhold_all: false,
            recorded_only,
            replayed,
            // Only where legacy rows replay at all (call 17).
            unattributed: !recorded_only && before_taint.iter().any(|(_, key)| unattributed(key)),
        })
    };
    let result: AppResult<ReplayScope> = resolved.await;
    result.unwrap_or_else(|e| {
        warn!(
            error = %e,
            "the reads this thread's rows carry could not be checked; none of its rows is replayed"
        );
        ReplayScope {
            withhold_all: true,
            recorded_only: true,
            ..ReplayScope::default()
        }
    })
}

/// Whether anyone but `reader` reads `conv`: a participant other than them.
async fn others_read(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    reader: Uuid,
    tenant: TenantId,
) -> AppResult<bool> {
    let reader = reader.to_string();
    Ok(repos
        .chat
        .list_participants(&conv.id, tenant)
        .await?
        .iter()
        .any(|participant| participant.user_id != reader))
}

/// Whether everyone in the turn's room reads its reply the moment it is
/// posted: a messaging group chat (carnet#1511 design call 6).
fn room_reads_reply(input: &TurnInput) -> bool {
    ai_scope::posts_to_group_chat(ai_scope::declared_transport(), input.is_direct_message)
}

/// Of `readable`, the reads a chat everyone reads may read
/// ([`member_reads::Reader::Room`]): in a group chat a row carrying a read
/// the whole room may not read is replayed into no later turn, so a reply
/// the chat was refused once (`room_hold`) does not make every later one a
/// reply about that member. A row bound to no group replays no member there.
async fn readable_by_the_chat(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    reader: Uuid,
    readable: &[String],
) -> AppResult<Vec<String>> {
    let Some(group_id) = conv.group_id.as_deref() else {
        return Ok(Vec::new());
    };
    let tenant = TenantId::parse_str(&conv.tenant_id)
        .map_err(|e| AppError::internal(format!("conversation tenant: {e}")))?;
    let Some(group) = repos.groups.get_group(group_id, tenant).await? else {
        return Ok(Vec::new());
    };
    member_reads::readable_keys(
        repos,
        member_reads::Reader::Room {
            group: group.id,
            peer_switch: group.is_active && group.peer_data_sharing,
            sender: reader,
        },
        readable,
    )
    .await
}

/// The history a turn replays, with what decided it.
#[derive(Debug)]
pub(crate) struct LoadedHistory {
    /// The rows the turn replays, oldest first, its own message included.
    pub history: Vec<MessageRecord>,
    /// What of the rows about group members it may replay.
    pub replay: ReplayScope,
    /// The turn's standing in a coach's athlete thread.
    pub subject_turn: Option<SubjectTurn>,
}

/// Load the history `input`'s turn replays, its own message `current`
/// included, under every rule above.
///
/// The load is bounded to a generous multiple of the compaction message cap,
/// so a long thread loads its recent working set, not its full unbounded
/// history (a 200-turn thread would otherwise build a 200-message vector
/// every turn before compaction trims it to the cap). The multiple leaves
/// headroom above `max_messages` for compaction-block reconstruction;
/// compaction still governs the final in-prompt size.
///
/// # Errors
///
/// Returns the history load's database error, or [`scope_history`]'s.
pub(crate) async fn load_history(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    current: &str,
) -> AppResult<LoadedHistory> {
    let history_load_limit = i64::try_from(
        ctx.harness_config_registry
            .current_compaction()
            .max_messages
            .saturating_mul(4)
            .clamp(80, 500),
    )
    .unwrap_or(160);
    // The group members whose rows the turn may replay: those its sender may
    // still read (carnet#1174), into the room the turn runs in, and nothing
    // from the first read of any other on (carnet#1422).
    let replay = member_replay(&ctx.repos, conv, input).await;
    let mut history = get_conversation_history(
        ctx.repos.chat.as_ref(),
        &input.conversation_id,
        &input.user_id,
        input.conversation_tenant_id,
        history_load_limit,
        replay.replay(current),
    )
    .await?;
    // The Garmin attribution closing an earlier reply is the platform's, not
    // something the model says: it never reads it back (carnet#1464).
    garmin_attribution_lines(&ctx.messaging_strings_registry).strip_from_history(
        history
            .iter_mut()
            .filter(|row| row.role == "assistant")
            .map(|row| &mut row.content),
    );
    // An external turn in a first-party-only thread, or a coach's athlete
    // thread the gates refuse, reads only its own new message.
    let subject_turn = scope_history(ctx, input, conv, current, &mut history).await?;
    // Replaying a member's rows is reading them: the reply is stamped with
    // them and nothing it says is filed on the sender (carnet#1422).
    replay.note_replayed(&history, current);
    Ok(LoadedHistory {
        history,
        replay,
        subject_turn,
    })
}

/// Keep of the loaded `history` what the turn may read — only the row
/// `current` names, the turn's own message, when either rule applies — and
/// answer the turn's standing in a coach's athlete thread.
///
/// # Errors
///
/// Returns the error reading the thread's transport policy, or an unparsable
/// sender id.
pub(crate) async fn scope_history(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    current: &str,
    history: &mut Vec<MessageRecord>,
) -> AppResult<Option<SubjectTurn>> {
    let thread = derived_content::thread_policy(
        ctx.tool_runtime.as_ref(),
        &input.conversation_tenant_id,
        parse_uuid(&input.user_id)?,
        &input.conversation_id,
    )
    .await?;
    if !ai_scope::admit_derived(thread) {
        history.retain(|row| row.id == current);
    }
    Ok(subject_athlete::resolve_withholding_history(
        &ctx.repos,
        conv,
        &input.user_id,
        history,
        current,
    )
    .await)
}
