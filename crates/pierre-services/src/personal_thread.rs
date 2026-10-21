// ABOUTME: The athlete's one personal thread — designated once on the server, adopted from their newest 1:1 thread or forged
// ABOUTME: Home, "+", /reset and the coach's own posts land in it; a new topic is a divider row inside it, never a new thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Personal Home step 2 (carnet#840).
//!
//! Every athlete has one continuous conversation with Dravr per tenant. It is
//! named once, by the `chat_conversations.personal` flag, and everything that
//! needs "the athlete's thread" asks for it here: Home, the coach's own posts,
//! and "+". None of them picks or forges one of its own, so a post lands in
//! the thread Home shows.
//!
//! The first time it is asked, the server adopts the athlete's newest in-app
//! thread that is theirs alone and about no single activity. Older threads are
//! left as they are and stay readable. The designation is settled by a unique
//! index, so two first requests racing each other end on the same thread.
//!
//! Looking is not starting: [`find_personal_thread`] never forges, so Home
//! opens on its empty pane for an athlete who has not begun. Only something
//! that writes into the thread — "+", a tapped starter, the coach's own post —
//! forges one, through [`resolve_personal_thread`].
//!
//! The athlete's messaging DM writes into it too, through
//! [`resolve_dm_thread`], which adopts the thread the DM was already writing
//! into when that is the athlete's most recent one.
//!
//! Starting over inside it is [`open_topic`]: a divider row, after which the
//! model reads the thread from the divider on (see
//! [`DIVIDER_FINISH_REASON`]).

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    is_in_app_channel, AddMessageParams, ConversationRecord, MessageRecord, OnboardingState,
    TenantId, CHANNEL_TYPE_WEB, DIVIDER_FINISH_REASON,
};
use pierre_core::transport::TransportPolicy;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::RepositoryRegistry;
use pierre_llm::config::LlmChain;
use tracing::{info, warn};

use crate::agent_selection::AgentSelectionSource;
use crate::agents::require_bindable_agent;
use crate::conversation_forge::{
    forge_conversation, record_agent_usage, resolve_conversation_model, start_guided_flow,
    ForgeAgent, ForgeParams, SlotQuota,
};

/// The athlete's personal thread in `tenant_id`, when they have one.
///
/// The designated thread, else their newest in-app thread that is theirs
/// alone, adopted now. `None` when they have none — nothing is forged for a
/// look.
///
/// # Errors
///
/// Returns the repository error when the thread cannot be read or designated,
/// and an internal error when the designation reports a winner the next read
/// cannot find.
pub async fn find_personal_thread(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<Option<ConversationRecord>> {
    if let Some(conversation) = repos.chat.personal_conversation(user_id, tenant_id).await? {
        return Ok(Some(conversation));
    }
    if adopt_newest(repos, user_id, tenant_id).await? {
        return designated(repos, user_id, tenant_id).await.map(Some);
    }
    Ok(None)
}

/// How many of a thread's newest rows [`athlete_last_wrote_in_app`] reads to
/// find the athlete's latest words.
const ORIGIN_PEEK_ROWS: i64 = 20;

/// Whether the athlete's latest words in `conversation_id` were typed in the
/// app rather than on a messaging channel.
///
/// The personal thread is shared by the app and the athlete's DM
/// (carnet#840), so a notice owed to that thread answers where the athlete
/// last wrote: in the app when they asked there, on the DM when they asked on
/// it. A thread whose latest user row names no channel — written before rows
/// carried one — reads as not the app, which is how a DM's thread was
/// answered before.
///
/// # Errors
///
/// Returns the repository error when the thread cannot be read.
pub async fn athlete_last_wrote_in_app(
    repos: &RepositoryRegistry,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<bool> {
    let rows = repos
        .chat
        .get_recent_messages(
            conversation_id,
            user_id,
            tenant_id,
            ORIGIN_PEEK_ROWS,
            TransportPolicy::FirstPartyOnly,
        )
        .await?;
    Ok(rows
        .iter()
        .rev()
        .find(|row| row.role == "user")
        .and_then(|row| row.channel_type.as_deref())
        .is_some_and(is_in_app_channel))
}

/// How [`resolve_personal_thread`] forges the thread for an athlete with none.
#[derive(Debug, Clone, Copy)]
pub struct PersonalForge<'a> {
    /// What names the thread when no agent title does.
    pub title_fallback: &'a str,
    /// Whether it carries the guided walk for an athlete who has answered
    /// nothing yet — what a thread the platform opens on its own carries. A
    /// thread the athlete opened with "+" never did.
    pub guided_flow: bool,
    /// The chain the server read at boot; its head's model is the one a
    /// forged thread records.
    pub llm_chain: &'a LlmChain,
}

/// The athlete's personal thread, as [`resolve_personal_thread`] found it.
#[derive(Debug, Clone)]
pub struct PersonalThread {
    /// The conversation.
    pub conversation: ConversationRecord,
    /// Whether this call forged it, so it holds nothing yet.
    pub forged: bool,
}

/// The athlete's personal thread in `tenant_id`, forging one when
/// [`find_personal_thread`] finds none.
///
/// A forged thread is bound to the athlete's selected agent, or their own
/// thread's default, and carries the guided walk when `forge` says so. It holds
/// no slot of the `max_active_conversations` cap: it is the one thread every
/// athlete has, never one more they chose to open.
///
/// # Errors
///
/// Returns the repository error when the thread cannot be read, designated or
/// forged, and an internal error when the designation reports a winner the
/// next read cannot find.
pub async fn resolve_personal_thread(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
    forge: PersonalForge<'_>,
) -> AppResult<PersonalThread> {
    if let Some(conversation) = find_personal_thread(repos, user_id, tenant_id).await? {
        return Ok(PersonalThread {
            conversation,
            forged: false,
        });
    }
    forge_personal(repos, user_id, tenant_id, forge).await
}

/// The athlete's personal thread as their messaging DM reaches it: the DM
/// writes into the thread Home shows (carnet#840).
///
/// The designated thread when there is one. Otherwise the newest of the two
/// threads that are the athlete's alone — their newest in-app one and the one
/// the DM has been writing into, `dm_thread` — is adopted, so an athlete who
/// has only ever written on Telegram keeps their history rather than meeting
/// an empty thread. With neither, one is forged as for any other first write.
///
/// # Errors
///
/// Returns the repository error when a thread cannot be read, designated or
/// forged, and an internal error when a designation reports a winner the next
/// read cannot find.
pub async fn resolve_dm_thread(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
    dm_thread: Option<&str>,
    forge: PersonalForge<'_>,
) -> AppResult<PersonalThread> {
    let chat = repos.chat.as_ref();
    if let Some(conversation) = chat.personal_conversation(user_id, tenant_id).await? {
        return Ok(PersonalThread {
            conversation,
            forged: false,
        });
    }
    let in_app = chat
        .latest_personal_conversation(user_id, tenant_id)
        .await?;
    let dm = match dm_thread {
        Some(id) => chat
            .get_conversation(id, user_id, tenant_id)
            .await?
            .filter(|thread| is_own_thread(thread, user_id)),
        None => None,
    };
    let Some(candidate) = newest_of(in_app, dm) else {
        return forge_personal(repos, user_id, tenant_id, forge).await;
    };
    let adopted = chat
        .designate_personal_conversation(&candidate.id, user_id, tenant_id)
        .await?;
    info!(
        user_id,
        conversation_id = %candidate.id,
        adopted,
        "personal thread: adopted for the athlete's messaging DM"
    );
    // A refused designation is a racing request that designated first; the
    // winner is read either way.
    designated(repos, user_id, tenant_id)
        .await
        .map(|conversation| PersonalThread {
            conversation,
            forged: false,
        })
}

/// Whether `thread` is one the athlete may be given as their personal one:
/// theirs, outside any room, and about no other athlete.
fn is_own_thread(thread: &ConversationRecord, user_id: &str) -> bool {
    thread.user_id == user_id && thread.group_id.is_none() && thread.subject_user_id.is_none()
}

/// The more recently active of two candidate threads.
fn newest_of(
    a: Option<ConversationRecord>,
    b: Option<ConversationRecord>,
) -> Option<ConversationRecord> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if activity(&b) > activity(&a) { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// When a thread was last active, for ordering two of them; an unreadable
/// stamp orders first, so a readable one wins.
fn activity(thread: &ConversationRecord) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&thread.updated_at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Designate the athlete's newest in-app thread of their own, when they have
/// one. `true` once a thread is designated — this one, or one a racing
/// request designated first; `false` when they have none to adopt.
async fn adopt_newest(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<bool> {
    let chat = repos.chat.as_ref();
    let Some(candidate) = chat
        .latest_personal_conversation(user_id, tenant_id)
        .await?
    else {
        return Ok(false);
    };
    let adopted = chat
        .designate_personal_conversation(&candidate.id, user_id, tenant_id)
        .await?;
    info!(
        user_id,
        conversation_id = %candidate.id,
        adopted,
        "personal thread: adopted the athlete's newest own thread"
    );
    Ok(true)
}

/// Forge the athlete's personal thread, outside the cap, and designate it.
async fn forge_personal(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
    forge: PersonalForge<'_>,
) -> AppResult<PersonalThread> {
    let chat = repos.chat.as_ref();
    let model = resolve_conversation_model(None, forge.llm_chain)?;
    let forged_id = forge_conversation(
        repos,
        ForgeParams {
            user_id,
            tenant_id,
            title_fallback: forge.title_fallback,
            model: &model,
            agent: ForgeAgent::Selected,
            group_id: None,
            channel_type: CHANNEL_TYPE_WEB,
            selection_source: AgentSelectionSource::ChatConversation,
            guided_flow: forge.guided_flow,
            quota: SlotQuota::Exempt,
        },
    )
    .await?;
    if chat
        .designate_personal_conversation(&forged_id, user_id, tenant_id)
        .await?
    {
        info!(user_id, conversation_id = %forged_id, "personal thread: forged");
        return Ok(PersonalThread {
            conversation: designated(repos, user_id, tenant_id).await?,
            forged: true,
        });
    }
    // A racing request designated its own thread first. This one was forged
    // for nothing and nobody was sent to it, so it goes.
    if let Err(e) = chat
        .delete_conversation(&forged_id, user_id, tenant_id)
        .await
    {
        warn!(error = %e, conversation_id = %forged_id, "personal thread: removing the losing forge failed");
    }
    Ok(PersonalThread {
        conversation: designated(repos, user_id, tenant_id).await?,
        forged: false,
    })
}

/// Read the designated thread after a designation settled.
async fn designated(
    repos: &RepositoryRegistry,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<ConversationRecord> {
    repos
        .chat
        .personal_conversation(user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("personal thread designated but not found"))
}

/// How a new topic opens.
#[derive(Debug, Clone, Copy, Default)]
pub struct TopicOpening<'a> {
    /// The agent the topic is with, when the athlete chose one ("+" from the
    /// Store, or with a specialist picked). The thread is rebound to it, the
    /// same rebind `/agent add` makes; `None` keeps the thread's agent.
    pub agent_id: Option<&'a str>,
    /// Whether the guided walk may start, for an athlete who still owes it —
    /// what a fresh thread opened by `/reset` carries.
    pub guided_flow: bool,
}

/// How many of a thread's newest rows [`open_topic`] reads to tell whether
/// its current topic holds a turn yet. A topic with that many rows after its
/// divider has been spoken in whatever they are.
const TOPIC_PEEK_ROWS: i64 = 40;

/// The topic [`open_topic`] opened.
#[derive(Debug, Clone)]
pub struct OpenedTopic {
    /// The divider row; `None` when the current topic held no turn yet, so
    /// it already was the fresh start a divider would mark.
    pub divider: Option<MessageRecord>,
    /// Whether a guided walk was in progress and the topic ended it.
    pub interrupted_walk: bool,
}

/// Open a new topic in `thread`, the athlete's personal thread or a coach's
/// thread bound to one athlete — the two threads that are never left for a
/// fresh one (carnet#840, carnet#1070).
///
/// It writes the divider row the model reads from, rebinds the agent when the
/// topic names one, and ends a guided walk in progress — restarting it when
/// `opening` allows and the athlete still owes it, as a fresh thread would.
///
/// A current topic nobody has spoken in yet — a thread just forged, or "+"
/// pressed twice — gets no second divider: it already is a fresh start, and
/// a rule with nothing above it but another rule tells the athlete nothing.
///
/// # Errors
///
/// Returns an invalid-input error when `thread` is neither, and the
/// repository error when the divider or the rebind cannot be written. The
/// walk is best-effort, like the forge's: a topic that opened is not undone
/// because a walk could not restart.
pub async fn open_topic(
    repos: &RepositoryRegistry,
    thread: &ConversationRecord,
    user_id: &str,
    tenant_id: TenantId,
    opening: TopicOpening<'_>,
) -> AppResult<OpenedTopic> {
    if !thread.personal && thread.subject_user_id.is_none() {
        return Err(AppError::invalid_input(
            "A topic opens only inside the personal thread or a thread about one athlete",
        ));
    }
    let rebound = rebind_agent(repos, thread, user_id, tenant_id, opening.agent_id).await?;
    let divider = if topic_has_turns(repos, thread, user_id, tenant_id).await? {
        Some(write_divider(repos, thread, user_id, tenant_id).await?)
    } else {
        None
    };
    let interrupted_walk =
        restart_walk(repos, thread, user_id, tenant_id, opening.guided_flow).await;

    info!(
        user_id,
        conversation_id = %thread.id,
        divider_id = divider.as_ref().map(|row| row.id.as_str()),
        rebound,
        interrupted_walk,
        "personal thread: opened a new topic"
    );
    Ok(OpenedTopic {
        divider,
        interrupted_walk,
    })
}

/// Bind the thread to `agent_id` when it names an agent the thread is not
/// already bound to — the rebind `/agent add` makes, refused like it for an
/// agent the athlete cannot read or a coach-facing one, and in the personal
/// thread made the athlete's selection like it. Answers whether it did.
async fn rebind_agent(
    repos: &RepositoryRegistry,
    thread: &ConversationRecord,
    user_id: &str,
    tenant_id: TenantId,
    agent_id: Option<&str>,
) -> AppResult<bool> {
    let Some(agent_id) = agent_id.filter(|id| thread.agent_id.as_deref() != Some(id)) else {
        return Ok(false);
    };
    // Bound only to an agent the athlete may be given, as `/agent add` binds.
    let user = parse_uuid(user_id)?;
    require_bindable_agent(
        repos.agents.as_ref(),
        repos.users.as_ref(),
        agent_id,
        user,
        tenant_id,
    )
    .await?;
    repos
        .chat
        .set_conversation_agent_id(&thread.id, Some(agent_id), tenant_id)
        .await?;
    // In the personal thread the pick is the athlete's latest word on who
    // they talk to, as `/agent add` makes it: their messaging DM writes into
    // this thread and rebinds it to the selection on every turn (carnet#840).
    // A coach's thread about an athlete is work, never the coach's own pick.
    if thread.personal {
        repos
            .tenants
            .set_selected_agent(tenant_id, user, Some(agent_id))
            .await?;
    }
    record_agent_usage(
        repos,
        agent_id,
        user_id,
        tenant_id,
        AgentSelectionSource::ChatConversation,
    )
    .await;
    Ok(true)
}

/// Write the divider row that opens the new topic.
async fn write_divider(
    repos: &RepositoryRegistry,
    thread: &ConversationRecord,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<MessageRecord> {
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &thread.id,
            user_id,
            role: "system",
            content: "",
            token_count: None,
            finish_reason: Some(DIVIDER_FINISH_REASON),
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
}

/// End the walk the previous topic started, and start it again when
/// `guided_flow` allows and the athlete still owes it. The walk belongs to
/// the topic that started it; the new topic starts clean. Answers whether a
/// walk was in progress. Best-effort, like the forge's walk.
async fn restart_walk(
    repos: &RepositoryRegistry,
    thread: &ConversationRecord,
    user_id: &str,
    tenant_id: TenantId,
    guided_flow: bool,
) -> bool {
    let interrupted = OnboardingState::from_column(thread.onboarding_state.as_deref()).is_some();
    if interrupted {
        if let Err(e) = repos
            .chat
            .set_conversation_onboarding_state(&thread.id, None, tenant_id)
            .await
        {
            warn!(error = %e, conversation_id = %thread.id, "personal thread: ending the walk failed");
        }
    }
    if guided_flow {
        start_guided_flow(repos, tenant_id, user_id, &thread.id).await;
    }
    interrupted
}

/// Whether anything has been said in the thread's current topic: a row after
/// its newest divider that the athlete wrote, or that answered them. An
/// agent's welcome is its opening, not a turn, and a slash command's rows are
/// account state the model never reads — a topic holding only a `/reset`
/// exchange is still the fresh start its divider marked.
async fn topic_has_turns(
    repos: &RepositoryRegistry,
    thread: &ConversationRecord,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<bool> {
    let rows = repos
        .chat
        .get_recent_messages(
            &thread.id,
            user_id,
            tenant_id,
            TOPIC_PEEK_ROWS,
            TransportPolicy::FirstPartyOnly,
        )
        .await?;
    let topic_start = rows
        .iter()
        .rposition(MessageRecord::is_divider)
        .map_or(0, |i| i + 1);
    Ok(rows[topic_start..].iter().any(|row| {
        matches!(row.role.as_str(), "user" | "assistant")
            && !row.is_agent_welcome()
            && !row.is_command_turn()
    }))
}
