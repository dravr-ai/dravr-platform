// ABOUTME: Drives the messaging intake — asks profile type and the PAR-Q+ verbatim, parses answers strictly
// ABOUTME: Owns the turn without the model, so a standardised medical instrument reaches the athlete unaltered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The intake turn handler.
//!
//! Sits in the same band as [`super::coach_choice`] — after auth, before the
//! model — and for the same reason: an answer to a question the platform asked
//! is a selection, not conversation. Handing "2" to the coach as the first
//! thing an athlete ever says to it is how the coach proposal used to behave,
//! and it taught people the bot does not listen.
//!
//! The intake is DM-only. Its answers are medical, and in a shared room they
//! would be stamped under the channel tenant — a fact space disjoint from the
//! athlete's own dossier — while also being read aloud to everyone present.
//!
//! ## What an active intake costs the turn that opens it
//!
//! `guided_flow_is_active` reads the same `onboarding_state` column and does not
//! discriminate by flow, so the one coached turn that opens an intake runs with
//! the guided-flow tool withhold applied — the model cannot save a training plan
//! on it. That is deliberate rather than incidental: the athlete is one message
//! away from being asked whether a doctor has ever told them they have a heart
//! condition, and a plan written before that answer is exactly what the screen
//! exists to inform. Every later intake turn skips the model entirely, so the
//! withhold costs one turn, not the walk.
//!
//! ## Standing aside
//!
//! Every question is asked at most [`MAX_ANSWER_ATTEMPTS`] times. Past that the
//! intake retires itself silently and the athlete's message goes to the coach
//! as ordinary conversation: someone who answers a yes/no question with a
//! sentence is trying to talk, and a third repetition of a medical form is a
//! worse outcome than an unscreened athlete. The steps are recorded as skipped
//! so the screen does not reopen on their next message.

use pierre_contremaitre::messaging_strings::{
    KEY_INTAKE_COMPLETE_CLEAR, KEY_INTAKE_COMPLETE_FLAGGED, KEY_INTAKE_OPENER,
    KEY_INTAKE_PARQ_INTRO, KEY_INTAKE_RETRY, KEY_INTAKE_YESNO_HINT,
};
use pierre_core::models::messaging::{ChannelType, OutgoingMessage};
use pierre_core::models::{GuidedFlow, OnboardingState, TenantId, TopicSlug};
use pierre_database::repositories::{ChatRepository, HarnessMemoryRepository};
use pierre_memory::PredicateCode;
use pierre_memory::{FactKind, FactSource};
use pierre_services::intake::{
    is_outstanding, parse_persona, parse_yes_no, persona_to_store, record_parq_yes, record_steps,
    IntakeTopic, PersonaAnswer, MAX_ANSWER_ATTEMPTS, STATUS_COMPLETE, STATUS_SKIPPED,
};
use tracing::{info, warn};
use uuid::Uuid;

use super::session::maybe_start_pillar_walk;
use crate::mcp::resources::ServerContext;
use pierre_services::messaging_broadcast::{proactive_rich_text, proactive_text};

/// How many onboarding-sourced facts to scan when counting raised flags.
///
/// The walk raises at most seven, and the athlete's onboarding facts are a
/// small set by construction — this bound exists so a corrupted dossier cannot
/// turn the wrap-up into an unbounded read.
const FACT_SCAN_LIMIT: i64 = 200;

/// Everything [`try_handle_intake`] needs to run one intake turn.
///
/// A struct rather than positional parameters, for the reason
/// [`super::coach_choice::CoachChoiceParams`] gives: the `&str`-ish fields are
/// trivially swappable at a call site, and this one writes medical facts.
pub(super) struct IntakeParams<'a> {
    /// Server context: repositories and the strings registry.
    pub resources: &'a ServerContext,
    /// Tenant that owns the athlete's facts.
    pub tenant_id: TenantId,
    /// Conversation carrying the intake state.
    pub conversation_id: &'a str,
    /// Channel the reply goes back out on.
    pub channel_type: ChannelType,
    /// Channel-side sender id, the reply's recipient.
    pub sender_id: &'a str,
    /// Pierre user answering.
    pub user_id: Uuid,
    /// Locale for every string rendered here.
    pub locale: &'a str,
    /// The athlete's inbound message text.
    pub text: &'a str,
    /// Whether this arrived in a 1:1 conversation.
    pub is_direct_message: bool,
}

/// What the intake did with an inbound message.
///
/// Distinguishes "nothing to do here" from "a question is outstanding and this
/// was not its answer", because the two fall through to different amounts of the
/// ingress pipeline: an outstanding question suppresses the coach-choice band,
/// where a bare digit would otherwise be read as picking a coach.
#[derive(Debug)]
pub(super) enum IntakeOutcome {
    /// No intake is awaiting an answer — no flow, a group, or not yet opened.
    Idle,
    /// A question is outstanding and this message did not answer it. The turn
    /// belongs to the coach; the re-ask rides behind its reply.
    Unanswered,
    /// The message answered the outstanding question, and this is the reply it
    /// earned — the next question, or the completion notice.
    Answered(Box<OutgoingMessage>),
}

impl IntakeOutcome {
    /// Whether a question is outstanding and this message did not answer it.
    pub(super) const fn awaiting(&self) -> bool {
        matches!(self, Self::Unanswered)
    }

    /// The reply this outcome earned, if it earned one.
    pub(super) fn into_reply(self) -> Option<OutgoingMessage> {
        match self {
            Self::Answered(reply) => Some(*reply),
            Self::Idle | Self::Unanswered => None,
        }
    }
}

/// Handle this turn if the conversation is mid-intake.
///
/// Only a message that PARSES as an answer takes the turn. A message that does
/// not is [`IntakeOutcome::Unanswered`]: the athlete asked something, and being
/// mid-form is no reason to refuse to answer it. On 2026-08-28 an athlete asked
/// whether tomorrow's climb suited his week and got "Désolé — j'ai besoin du
/// chiffre seul" instead; the question was swallowed and never answered. The
/// re-ask now rides behind the coach's reply, the way the opener already does.
///
/// Deliberately no test of what the message says. A keyword gate for "is this a
/// coaching question" is the 61-term list 71dd378de deleted, and it failed the
/// way substring routing always fails. The signal is structural: the parser
/// either claimed the message or it did not.
pub(super) async fn try_handle_intake(params: IntakeParams<'_>) -> IntakeOutcome {
    let IntakeParams {
        resources,
        tenant_id,
        conversation_id,
        channel_type,
        sender_id,
        user_id,
        locale,
        text,
        is_direct_message,
    } = params;

    if !is_direct_message {
        return IntakeOutcome::Idle;
    }

    let chat: &dyn ChatRepository = resources.common.repos.chat.as_ref();
    let user_id_str = user_id.to_string();
    let Ok(Some(conv)) = chat
        .get_conversation(conversation_id, &user_id_str, tenant_id)
        .await
    else {
        return IntakeOutcome::Idle;
    };

    let raw_state = conv.onboarding_state.clone();
    let Some(state) = OnboardingState::from_column(raw_state.as_deref()) else {
        return IntakeOutcome::Idle;
    };
    if state.flow != GuidedFlow::Intake {
        return IntakeOutcome::Idle;
    }

    // The question this message answers is the last one delivered. An empty
    // ledger means the first question has not gone out yet, and this message is
    // not answering anything — the coach takes the turn, and
    // [`try_build_pending_question`] appends the opener behind its reply.
    let Some(awaiting) = IntakeTopic::awaiting(&state.probed) else {
        return IntakeOutcome::Idle;
    };

    let Some(answer) = interpret(awaiting, text) else {
        // Not an answer. The coach takes the turn; the bottom-of-turn hook
        // re-derives the outstanding question from the ledger and re-asks
        // behind the reply, or retires the walk once the budget is spent.
        return IntakeOutcome::Unanswered;
    };

    persist_answer(resources, tenant_id, &user_id_str, awaiting, answer).await;

    let reply = match IntakeTopic::next(&state.probed) {
        Some(next) => {
            deliver(DeliverArgs {
                resources,
                tenant_id,
                conversation_id,
                channel_type,
                sender_id,
                locale,
                state,
                raw_state,
                topic: next,
                is_retry: false,
            })
            .await
        }
        None => {
            complete(CompleteArgs {
                resources,
                tenant_id,
                conversation_id,
                channel_type,
                sender_id,
                user_id: &user_id_str,
                locale,
                raw_state,
            })
            .await
        }
    };

    reply.map_or(IntakeOutcome::Idle, |m| {
        IntakeOutcome::Answered(Box::new(m))
    })
}

/// What an athlete's reply resolved to, in whichever question asked it.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// Profile type: `true` when they coach other people.
    Coaches(bool),
    /// PAR-Q+: `true` for a "yes", which raises a flag.
    Parq(bool),
}

/// Parse the reply against the question that is outstanding.
fn interpret(topic: IntakeTopic, text: &str) -> Option<Answer> {
    if topic == IntakeTopic::Persona {
        return parse_persona(text).map(|p| Answer::Coaches(p == PersonaAnswer::Coach));
    }
    parse_yes_no(text).map(Answer::Parq)
}

/// Write the answer where its surface's equivalent writes it.
///
/// Best-effort: a failed write costs one answer, never the walk. The athlete is
/// mid-conversation and re-asking the question they just answered reads as the
/// bot not listening, which is the failure this whole path exists to avoid.
async fn persist_answer(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: &str,
    topic: IntakeTopic,
    answer: Answer,
) {
    match answer {
        Answer::Coaches(true) => persist_coach_persona(resources, user_id).await,
        Answer::Parq(true) => persist_parq_flag(resources, tenant_id, user_id, topic).await,
        // Neither writes anything. "I'm an athlete" has no user-row value to
        // store — `coaching_persona` has no athlete variant, Casual *is* the
        // default — and a clean PAR-Q answer raises no flag by definition. That
        // the athlete answered at all is carried by the step row.
        Answer::Coaches(false) | Answer::Parq(false) => {}
    }
}

/// Mark the athlete as someone who coaches other people.
///
/// The one profile-type answer with a user-row write, mirroring the web step.
async fn persist_coach_persona(resources: &ServerContext, user_id: &str) {
    let Ok(uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    let Some(persona) = persona_to_store(PersonaAnswer::Coach) else {
        return;
    };
    if let Err(e) = resources
        .common
        .repos
        .users
        .set_coaching_persona(uuid, persona)
        .await
    {
        warn!(error = %e, "intake: failed to persist the coach persona");
    }
}

/// Raise the coach-visible medical flag for one "yes".
async fn persist_parq_flag(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: &str,
    topic: IntakeTopic,
) {
    let memory: &dyn HarnessMemoryRepository = resources.common.repos.memory.as_ref();
    match record_parq_yes(memory, tenant_id, user_id, topic).await {
        Ok(raised) => info!(
            parq_question = topic.parq_id().unwrap_or_default(),
            raised, "intake: PAR-Q flag raised from a chat answer"
        ),
        Err(e) => warn!(error = %e, "intake: failed to persist a PAR-Q flag"),
    }
}

/// Arguments for delivering one question.
struct DeliverArgs<'a> {
    resources: &'a ServerContext,
    tenant_id: TenantId,
    conversation_id: &'a str,
    channel_type: ChannelType,
    sender_id: &'a str,
    locale: &'a str,
    state: OnboardingState,
    raw_state: Option<String>,
    topic: IntakeTopic,
    /// Whether this delivery is a re-ask, which prefixes the question.
    is_retry: bool,
}

/// Record a question as delivered and return it as the turn's reply.
///
/// The ledger is written before the message goes out. A delivered question that
/// was never recorded would be asked again on the next turn, and the athlete
/// would answer the same question twice; a recorded question that failed to
/// send costs one question out of eight.
async fn deliver(args: DeliverArgs<'_>) -> Option<OutgoingMessage> {
    let DeliverArgs {
        resources,
        tenant_id,
        conversation_id,
        channel_type,
        sender_id,
        locale,
        mut state,
        raw_state,
        topic,
        is_retry,
    } = args;

    state.probed.push(topic.slug());
    // A question that could not be recorded is a question that must not be
    // asked. The ledger IS the attempt counter — every re-ask is bounded by
    // `MAX_ANSWER_ATTEMPTS` counted out of `probed` — so sending after a failed
    // compare-and-set would ask without charging, and an athlete who never
    // answers would be asked forever.
    if !write_state(resources, conversation_id, tenant_id, raw_state, &state).await {
        return None;
    }

    let body = render_question(resources, locale, topic, is_retry);
    Some(proactive_rich_text(
        channel_type,
        sender_id.to_owned(),
        &body,
    ))
}

/// Compose the message for one question, with the framing its position earns.
fn render_question(
    resources: &ServerContext,
    locale: &str,
    topic: IntakeTopic,
    is_retry: bool,
) -> String {
    let reg = &resources.mcp.messaging_strings_registry;
    let question = reg.get(topic.string_key(), locale);

    let block = match topic {
        // The opener rides with the first question rather than as a message of
        // its own: two notifications to ask one thing is how a chat assistant
        // becomes something people mute.
        IntakeTopic::Persona => format!("{}\n\n{question}", reg.get(KEY_INTAKE_OPENER, locale)),
        // The PAR-Q+ set opens with why we are asking seven medical questions.
        IntakeTopic::HeartCondition => format!(
            "{}\n\n{question}\n\n{}",
            reg.get(KEY_INTAKE_PARQ_INTRO, locale),
            reg.get(KEY_INTAKE_YESNO_HINT, locale)
        ),
        _ => format!("{question}\n\n{}", reg.get(KEY_INTAKE_YESNO_HINT, locale)),
    };

    if is_retry {
        return reg.render(KEY_INTAKE_RETRY, locale, &[&block]);
    }
    block
}

/// Whether the walk got past the profile-type question.
fn persona_answered(probed: &[TopicSlug]) -> bool {
    probed
        .iter()
        .any(|slug| *slug != IntakeTopic::Persona.slug())
}

/// Arguments for closing out a completed intake.
struct CompleteArgs<'a> {
    resources: &'a ServerContext,
    tenant_id: TenantId,
    conversation_id: &'a str,
    channel_type: ChannelType,
    sender_id: &'a str,
    user_id: &'a str,
    locale: &'a str,
    raw_state: Option<String>,
}

/// Every question answered: record both steps and send the wrap-up.
async fn complete(args: CompleteArgs<'_>) -> Option<OutgoingMessage> {
    let CompleteArgs {
        resources,
        tenant_id,
        conversation_id,
        channel_type,
        sender_id,
        user_id,
        locale,
        raw_state,
    } = args;

    let raised = count_medical_flags(resources, tenant_id, user_id).await;
    finish(FinishArgs {
        resources,
        tenant_id,
        conversation_id,
        user_id,
        raw_state,
        persona_status: STATUS_COMPLETE,
        parq_status: STATUS_COMPLETE,
    })
    .await;

    let reg = &resources.mcp.messaging_strings_registry;
    let body = if raised == 0 {
        reg.get(KEY_INTAKE_COMPLETE_CLEAR, locale)
    } else {
        reg.render(KEY_INTAKE_COMPLETE_FLAGGED, locale, &[&raised.to_string()])
    };

    info!(
        flags_raised = raised,
        "intake: complete — profile type and PAR-Q recorded for a messaging athlete"
    );
    Some(proactive_text(channel_type, sender_id.to_owned(), body))
}

/// How many medical flags this athlete's onboarding has raised.
///
/// Read back from the facts rather than counted in flow state: the facts are
/// what the coach will actually see, so a wrap-up that disagrees with them
/// would be reporting on a write that did not land.
async fn count_medical_flags(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: &str,
) -> usize {
    let memory: &dyn HarnessMemoryRepository = resources.common.repos.memory.as_ref();
    memory
        .list_user_facts_by_source(tenant_id, user_id, FactSource::Onboarding, FACT_SCAN_LIMIT)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|f| f.kind == FactKind::Medical && f.predicate_code == PredicateCode::ParqYes)
        .count()
}

/// Arguments for retiring the flow and recording its steps.
struct FinishArgs<'a> {
    resources: &'a ServerContext,
    tenant_id: TenantId,
    conversation_id: &'a str,
    user_id: &'a str,
    raw_state: Option<String>,
    persona_status: &'a str,
    parq_status: &'a str,
}

/// Record the durable steps and clear the intake off the conversation.
///
/// The steps are written first: they are what both surfaces read to decide
/// whether to ask again, so a cleared flow with no step rows would re-open the
/// intake on the athlete's next message.
async fn finish(args: FinishArgs<'_>) {
    let FinishArgs {
        resources,
        tenant_id,
        conversation_id,
        user_id,
        raw_state,
        persona_status,
        parq_status,
    } = args;

    if let Err(e) = record_steps(
        resources.common.repos.user_onboarding.as_ref(),
        user_id,
        Some(tenant_id.to_string().as_str()),
        persona_status,
        parq_status,
    )
    .await
    {
        warn!(error = %e, "intake: failed to record the onboarding steps");
    }

    let chat: &dyn ChatRepository = resources.common.repos.chat.as_ref();
    if let Err(e) = chat
        .compare_and_set_conversation_onboarding_state(
            conversation_id,
            raw_state.as_deref(),
            None,
            tenant_id,
        )
        .await
    {
        warn!(error = %e, "intake: failed to clear the intake state");
        // The column still holds the intake, so handing the conversation to the
        // pillar walk now would have it overwrite a flow that never retired.
        return;
    }

    // Hand off to the pillar walk the intake displaced. A messaging channel
    // holds ONE long-lived conversation per athlete, so "it starts on the next
    // conversation" would mean "only after /reset" — the walk would effectively
    // never run for anyone who arrived through a channel. The walk makes its own
    // decision from dossier coverage; an athlete who already has context is left
    // alone, exactly as at conversation creation.
    maybe_start_pillar_walk(resources, tenant_id, user_id, conversation_id).await;
}

/// Persist the ledger, refusing to clobber a state written under this turn.
///
/// Compare-and-set for the reason the repository documents: the intake is
/// handled synchronously in the webhook path while an earlier LLM turn may
/// still be holding a snapshot of this column, and a blind write would replace
/// whatever that turn wrote back.
async fn write_state(
    resources: &ServerContext,
    conversation_id: &str,
    tenant_id: TenantId,
    raw_state: Option<String>,
    state: &OnboardingState,
) -> bool {
    let Ok(json) = state.to_column() else {
        warn!("intake: could not serialize the intake state; the question will be re-asked");
        return false;
    };
    let chat: &dyn ChatRepository = resources.common.repos.chat.as_ref();
    match chat
        .compare_and_set_conversation_onboarding_state(
            conversation_id,
            raw_state.as_deref(),
            Some(&json),
            tenant_id,
        )
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            warn!(
                conversation_id,
                "intake: a newer state owns the column; leaving it alone"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "intake: failed to record the delivered question");
            false
        }
    }
}

/// Everything [`try_build_pending_question`] needs to open an intake.
pub(super) struct PendingQuestionParams<'a> {
    /// Server context: repositories and the strings registry.
    pub resources: &'a ServerContext,
    /// Tenant that owns the conversation.
    pub tenant_id: TenantId,
    /// Conversation the intake was activated on.
    pub conversation_id: &'a str,
    /// Channel the question goes out on.
    pub channel_type: ChannelType,
    /// Channel-side recipient.
    pub sender_id: &'a str,
    /// Pierre user being asked.
    pub user_id: Uuid,
    /// Locale for the rendered question.
    pub locale: &'a str,
    /// Whether this is a 1:1 conversation.
    pub is_direct_message: bool,
}

/// Ask whatever intake question is outstanding, behind the turn just served.
///
/// Rides *behind* a served turn rather than replacing it, the way the connect
/// card and the coach proposal already do. An athlete who said "hey, can you
/// help me train for a marathon?" should be answered before being handed a
/// form; hijacking that message to ask about heart conditions is how a helpful
/// bot becomes an intake desk.
///
/// Owns the whole decision, derived from the ledger alone:
/// - no intake on the conversation, or a group → `None`
/// - `probed` empty → the intake has never opened → ask the opener
/// - `probed` non-empty → a coached turn ran while a question was outstanding,
///   which is only reachable because [`try_handle_intake`] declined it, and
///   mid-intake it declines exactly one thing: a message that did not parse as
///   an answer → re-ask it, or retire the walk once its budget is spent.
///
/// That last inference is why no "was it an answer?" signal has to be threaded
/// from the webhook into the dispatch task. The state says it. A turn that
/// failed, was quota-denied, or produced no reply returns before this hook and
/// leaves the ledger untouched, so the next served turn reaches the same
/// conclusion.
pub(super) async fn try_build_pending_question(
    params: PendingQuestionParams<'_>,
) -> Option<OutgoingMessage> {
    let PendingQuestionParams {
        resources,
        tenant_id,
        conversation_id,
        channel_type,
        sender_id,
        user_id,
        locale,
        is_direct_message,
    } = params;

    if !is_direct_message {
        return None;
    }

    let chat: &dyn ChatRepository = resources.common.repos.chat.as_ref();
    let conv = chat
        .get_conversation(conversation_id, &user_id.to_string(), tenant_id)
        .await
        .ok()
        .flatten()?;

    let raw_state = conv.onboarding_state.clone();
    let state = OnboardingState::from_column(raw_state.as_deref())?;
    if state.flow != GuidedFlow::Intake {
        return None;
    }

    let Some(awaiting) = IntakeTopic::awaiting(&state.probed) else {
        // Nothing asked yet: open with the first question.
        return deliver(DeliverArgs {
            resources,
            tenant_id,
            conversation_id,
            channel_type,
            sender_id,
            locale,
            state,
            raw_state,
            topic: IntakeTopic::Persona,
            is_retry: false,
        })
        .await;
    };

    if awaiting.attempts(&state.probed) < MAX_ANSWER_ATTEMPTS {
        return deliver(DeliverArgs {
            resources,
            tenant_id,
            conversation_id,
            channel_type,
            sender_id,
            locale,
            state,
            raw_state,
            topic: awaiting,
            is_retry: true,
        })
        .await;
    }

    info!(
        topic = awaiting.slug().as_str(),
        "intake: standing aside after an unanswered question — the coach keeps the turn"
    );
    // Profile type is recorded as answered when the walk got past it: topics are
    // strictly sequential, so a delivered PAR-Q question proves the persona
    // question was answered.
    let persona_status = if persona_answered(&state.probed) {
        STATUS_COMPLETE
    } else {
        STATUS_SKIPPED
    };
    finish(FinishArgs {
        resources,
        tenant_id,
        conversation_id,
        user_id: &user_id.to_string(),
        raw_state,
        persona_status,
        parq_status: STATUS_SKIPPED,
    })
    .await;
    None
}

/// Start an intake on a conversation when the athlete still owes both steps.
///
/// Mirrors [`super::session::maybe_start_pillar_walk`]'s posture: best-effort,
/// and silent when there is nothing to ask. Ordering matters — the intake runs
/// before the pillar walk, matching the web wizard, where profile type and the
/// PAR-Q sit ahead of everything the coach reasons from.
pub(super) async fn maybe_start_intake(
    resources: &ServerContext,
    user_id_str: &str,
    conversation_id: &str,
    tenant_id: TenantId,
) -> bool {
    let steps = match resources
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(user_id_str)
        .await
    {
        Ok(steps) => steps,
        Err(e) => {
            warn!(error = %e, "intake: could not read the onboarding steps; not starting");
            return false;
        }
    };
    if !is_outstanding(&steps) {
        return false;
    }
    activate(resources, conversation_id, tenant_id).await
}

/// Write the fresh intake state onto the conversation.
///
/// Split from the decision above so each half reads as one thing: whether the
/// athlete is owed an intake, and whether the row took it.
async fn activate(resources: &ServerContext, conversation_id: &str, tenant_id: TenantId) -> bool {
    let json = OnboardingState::start_now_column(GuidedFlow::Intake);
    match resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(conversation_id, Some(&json), tenant_id)
        .await
    {
        Ok(true) => {
            info!(
                conversation_id,
                "intake started for a messaging athlete who has answered neither step"
            );
            true
        }
        Ok(false) => {
            warn!(
                conversation_id,
                "intake activation matched no conversation row"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "intake: failed to activate");
            false
        }
    }
}
