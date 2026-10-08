// ABOUTME: The messaging intake walk — profile type, then the seven PAR-Q+ questions, asked verbatim
// ABOUTME: Fixed topic list, strict answer parsing, and the durable step records both surfaces share

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Structured intake for athletes who arrive through a messaging channel.
//!
//! ## Why the platform asks these, and not the agent
//!
//! Web and mobile ask profile type and the PAR-Q+ on real wizard steps. A
//! messaging athlete reached neither: the pillar walk covers *who they are*
//! conversationally, but nothing covered *whether it is safe for them to train*
//! — the PAR-Q+ was built, wired to `GET/POST /api/me/parq`, and asked on two
//! surfaces out of three.
//!
//! The pillar walk is the wrong machinery to fix that with. It hands a topic to
//! the model and lets it ask in its own words, then infers the answer from the
//! reply. For a standardised pre-participation instrument the wording *is* the
//! instrument, and an inferred "no" on chest pain is a safety failure, not a
//! rounding error. So the platform renders these questions verbatim and parses
//! the answers strictly — the same posture `agent_choice` takes for a numeric
//! selection, and for the same reason.
//!
//! ## Getting out of the way
//!
//! A "Yes" never blocks anything; neither does silence. Each question is asked
//! at most [`MAX_ANSWER_ATTEMPTS`] times, counted from the delivered-probe
//! ledger, and an athlete who answers something else twice ends the intake
//! rather than being asked a third time. Insisting on a medical form in a
//! stranger's first conversation is worse than not screening them, and the
//! screen stays reachable afterwards.
//!
//! ## Sharing the answer with the other surfaces
//!
//! Completion is recorded as the same durable step rows the wizard writes —
//! `profile_type` and `parq` — so answering on any surface counts everywhere,
//! exactly as pillar coverage already does. That is also the only way to tell
//! "screened, all clear" from "never asked": a clean PAR-Q writes no facts at
//! all, since only a "Yes" raises a flag — a clean answer only ever removes
//! the flag an earlier "Yes" to the same question left.

use pierre_contremaitre::messaging_strings::{
    KEY_INTAKE_PARQ_CHEST_PAIN, KEY_INTAKE_PARQ_CHRONIC_CONDITION, KEY_INTAKE_PARQ_DIZZINESS,
    KEY_INTAKE_PARQ_HEART_CONDITION, KEY_INTAKE_PARQ_JOINT_PROBLEM, KEY_INTAKE_PARQ_MEDICATION,
    KEY_INTAKE_PARQ_SUPERVISED_ONLY, KEY_INTAKE_PERSONA,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantId, TopicSlug};
use pierre_database::repositories::{
    HarnessMemoryRepository, OnboardingStepRecord, TenantRepository, UserOnboardingRepository,
};
use uuid::Uuid;

use crate::parq::{persist_parq_flags, retire_parq_flags};

/// How many times one question may be put to the athlete.
///
/// The first delivery plus a single re-ask: enough to survive a crossed
/// message, short of an interrogation. Past it the intake stands aside.
pub const MAX_ANSWER_ATTEMPTS: usize = 2;

/// Step id for the profile-type answer, shared with the web wizard.
pub const STEP_PROFILE_TYPE: &str = "profile_type";
/// Step id for the PAR-Q screen, shared with the web wizard.
pub const STEP_PARQ: &str = "parq";
/// Step status meaning the athlete answered.
pub const STATUS_COMPLETE: &str = "complete";
/// Step status meaning the intake stood aside without an answer.
pub const STATUS_SKIPPED: &str = "skipped";
/// Step status meaning the step is not part of this person's journey.
///
/// Written for the athlete-only steps (`about_you`, `parq`) when someone says
/// they coach others and do not train themselves. Distinct from
/// [`STATUS_SKIPPED`] on purpose: a skipped screen was offered and declined, a
/// not-applicable one was never theirs to answer — and every surface reads it
/// to keep the athlete questions, and the pillar walk, away from that coach.
pub const STATUS_NOT_APPLICABLE: &str = "not_applicable";
/// Step id for the about-you answers, shared with the web wizard.
pub(crate) const STEP_ABOUT_YOU: &str = "about_you";

/// One question in the intake, in the order it is asked.
///
/// Profile type comes first for the same reason it is step 1 on the web: it is
/// the cheapest question in the set and it decides which voice the agent
/// answers in from that turn onward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntakeTopic {
    /// Athlete or agent — mirrors the wizard's profile-type step.
    Persona,
    /// PAR-Q+ Q1 — diagnosed heart condition.
    HeartCondition,
    /// PAR-Q+ Q2 — chest pain at rest or during activity.
    ChestPain,
    /// PAR-Q+ Q3 — dizziness or loss of consciousness.
    Dizziness,
    /// PAR-Q+ Q4 — another diagnosed chronic condition.
    ChronicCondition,
    /// PAR-Q+ Q5 — prescribed medication for a chronic condition.
    Medication,
    /// PAR-Q+ Q6 — bone, joint or soft-tissue problem.
    JointProblem,
    /// PAR-Q+ Q7 — advised to train only under medical supervision.
    SupervisedOnly,
}

/// Every intake topic, in the order they are asked.
pub const INTAKE_TOPICS: [IntakeTopic; 8] = [
    IntakeTopic::Persona,
    IntakeTopic::HeartCondition,
    IntakeTopic::ChestPain,
    IntakeTopic::Dizziness,
    IntakeTopic::ChronicCondition,
    IntakeTopic::Medication,
    IntakeTopic::JointProblem,
    IntakeTopic::SupervisedOnly,
];

impl IntakeTopic {
    /// The PAR-Q+ question id this topic screens, or `None` for profile type.
    ///
    /// These are the ids [`crate::parq::PARQ_QUESTION_IDS`] already uses, so a
    /// flag raised in chat is the same row as one raised through the API.
    #[must_use]
    pub const fn parq_id(self) -> Option<&'static str> {
        match self {
            Self::Persona => None,
            Self::HeartCondition => Some("heart_condition"),
            Self::ChestPain => Some("chest_pain"),
            Self::Dizziness => Some("dizziness"),
            Self::ChronicCondition => Some("chronic_condition"),
            Self::Medication => Some("medication"),
            Self::JointProblem => Some("joint_problem"),
            Self::SupervisedOnly => Some("supervised_only"),
        }
    }

    /// The slug recorded in the delivered-probe ledger.
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        let tail = self.parq_id().unwrap_or("persona");
        TopicSlug::new(format!("intake:{tail}"))
    }

    /// The messaging-strings key holding this question's text.
    ///
    /// Returns the registry's own constants rather than re-typing the key
    /// literals, so a renamed key is a compile error here instead of a question
    /// that silently renders as the empty string.
    #[must_use]
    pub const fn string_key(self) -> &'static str {
        match self {
            Self::Persona => KEY_INTAKE_PERSONA,
            Self::HeartCondition => KEY_INTAKE_PARQ_HEART_CONDITION,
            Self::ChestPain => KEY_INTAKE_PARQ_CHEST_PAIN,
            Self::Dizziness => KEY_INTAKE_PARQ_DIZZINESS,
            Self::ChronicCondition => KEY_INTAKE_PARQ_CHRONIC_CONDITION,
            Self::Medication => KEY_INTAKE_PARQ_MEDICATION,
            Self::JointProblem => KEY_INTAKE_PARQ_JOINT_PROBLEM,
            Self::SupervisedOnly => KEY_INTAKE_PARQ_SUPERVISED_ONLY,
        }
    }

    /// How many times this topic has already been put to the athlete.
    ///
    /// Counted from the ledger rather than stored: a re-ask pushes the slug
    /// again, so the ledger carries the attempt count for free and a resumed
    /// conversation cannot lose it.
    #[must_use]
    pub fn attempts(self, probed: &[TopicSlug]) -> usize {
        let slug = self.slug();
        probed.iter().filter(|s| **s == slug).count()
    }

    /// The next topic to ask, given the ledger.
    ///
    /// The first topic nobody has answered yet — which is the first one absent
    /// from the ledger, since a topic is recorded when it is *delivered*, and a
    /// delivered question is either answered or ends the intake.
    #[must_use]
    pub fn next(probed: &[TopicSlug]) -> Option<Self> {
        INTAKE_TOPICS
            .into_iter()
            .find(|topic| topic.attempts(probed) == 0)
    }

    /// The topic the athlete's current message is answering: the last one
    /// delivered.
    #[must_use]
    pub fn awaiting(probed: &[TopicSlug]) -> Option<Self> {
        probed
            .last()
            .and_then(|slug| INTAKE_TOPICS.into_iter().find(|t| t.slug() == *slug))
    }
}

/// What the athlete said they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaAnswer {
    /// Trains for themselves.
    Athlete,
    /// Coaches other people and does not train themselves.
    Coach,
    /// Coaches other people and trains too.
    CoachAndAthlete,
}

impl PersonaAnswer {
    /// Whether the athlete-only questions (PAR-Q+, about-you, the pillar walk)
    /// belong to this person's journey. Only a coach who does not train is
    /// spared them.
    #[must_use]
    pub const fn trains(self) -> bool {
        !matches!(self, Self::Coach)
    }
}

/// Affirmative tokens across the five compiled-in locales.
///
/// A fixed table rather than a locale lookup on purpose: an athlete whose
/// locale we resolved wrongly still gets their "yes" understood, and these
/// words do not drift. Accents are stripped before comparison, so `sí` and
/// `si`, `não` and `nao` both land here.
const YES_TOKENS: [&str; 8] = ["yes", "y", "oui", "o", "si", "ja", "j", "sim"];

/// Negative tokens across the five compiled-in locales.
const NO_TOKENS: [&str; 6] = ["no", "n", "non", "nein", "nao", "nee"];

/// Tokens naming someone who trains themselves.
const ATHLETE_TOKENS: [&str; 6] = ["athlete", "atleta", "sportler", "athletin", "moi", "me"];

/// Tokens naming someone who agents others.
const COACH_TOKENS: [&str; 6] = [
    "coach",
    "entraineur",
    "entrenador",
    "trainer",
    "treinador",
    "coaching",
];

/// Tokens naming someone who coaches others and trains too.
///
/// Compared against the whole normalised message, like the other tables, so
/// the two-word French answer is listed as it is typed.
const BOTH_TOKENS: [&str; 6] = ["both", "les deux", "deux", "ambos", "ambas", "beides"];

/// Lowercase, trim, and fold the accents the token tables do not carry.
///
/// Only the accented characters that actually appear in the answers we accept —
/// a general Unicode normaliser would be a dependency for six code points.
fn normalise(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .filter(|c| !matches!(c, '.' | '!' | ',' | ';' | ':'))
        .collect()
}

/// Parse a reply to a yes/no question.
///
/// Deliberately strict: the whole message must be the answer. "yes" and "1"
/// answer the question; "yes but only when I sprint" does not, because a
/// qualified answer to a medical screen is a conversation the agent should
/// have, not a flag the platform should raise on its own.
#[must_use]
pub fn parse_yes_no(text: &str) -> Option<bool> {
    let token = normalise(text);
    if token == "1" || YES_TOKENS.contains(&token.as_str()) {
        return Some(true);
    }
    if token == "2" || NO_TOKENS.contains(&token.as_str()) {
        return Some(false);
    }
    None
}

/// Parse a reply to the profile-type question.
///
/// Same strictness as [`parse_yes_no`], and the same numbering the question
/// presents: 1 is the athlete, 2 the coach who does not train, 3 both.
#[must_use]
pub fn parse_persona(text: &str) -> Option<PersonaAnswer> {
    let token = normalise(text);
    if token == "1" || ATHLETE_TOKENS.contains(&token.as_str()) {
        return Some(PersonaAnswer::Athlete);
    }
    if token == "2" || COACH_TOKENS.contains(&token.as_str()) {
        return Some(PersonaAnswer::Coach);
    }
    if token == "3" || BOTH_TOKENS.contains(&token.as_str()) {
        return Some(PersonaAnswer::CoachAndAthlete);
    }
    None
}

/// Persist a "Yes" to one PAR-Q question as an agent-visible medical flag.
///
/// A "No" raises nothing, which is what the API path does too — only a raised
/// flag is a fact — and retires the flag an earlier "Yes" left on that
/// question ([`record_parq_no`]). Whether the screen happened at all is
/// carried by the step row, not by the absence of flags.
///
/// # Errors
///
/// Returns the repository error if the fact upsert fails.
pub async fn record_parq_yes<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    topic: IntakeTopic,
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let Some(id) = topic.parq_id() else {
        return Ok(0);
    };
    persist_parq_flags(repo, tenant_id, user_id, &[id.to_owned()]).await
}

/// Record a "No" to one PAR-Q question: the flag an earlier "Yes" to it
/// raised is retired, exactly as a clean answer on the API path retires it.
///
/// Only that question's PAR-Q flag goes. A medical fact a coach tool raised,
/// and the flag on any question this answer does not address, stay on file.
/// Returns the number of flag rows retired — zero for an athlete with no flag
/// on the question, and for the profile-type topic.
///
/// # Errors
///
/// Returns the repository error if the fact delete fails.
pub async fn record_parq_no<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    topic: IntakeTopic,
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let Some(id) = topic.parq_id() else {
        return Ok(0);
    };
    retire_parq_flags(repo, tenant_id, user_id, &[id.to_owned()]).await
}

/// The role an answer records: whether this person coaches others
/// (`users.coaches_others`).
///
/// A role, never a style (carnet#827). Both coach answers record that the
/// person coaches others, which puts the group step in their onboarding and the
/// coach-facing agents in their catalogue; the athlete answer records that they
/// do not. None of them touches `coaching_persona`: the voice the agent uses in
/// the person's own conversation stays the style they had (`casual` by default)
/// or later pick under Settings. Mirrors the web step exactly.
#[must_use]
pub const fn coaches_others_for(answer: PersonaAnswer) -> bool {
    match answer {
        PersonaAnswer::Athlete => false,
        PersonaAnswer::Coach | PersonaAnswer::CoachAndAthlete => true,
    }
}

/// Record a finished (or abandoned) intake on the durable step rows.
///
/// Both surfaces read these, so an athlete screened in chat is not screened
/// again on the web, and vice versa.
///
/// # Errors
///
/// Returns the repository error if either step write fails.
pub async fn record_steps<R>(
    repo: &R,
    user_id: &str,
    tenant_id: Option<&str>,
    persona_status: &str,
    parq_status: &str,
) -> AppResult<()>
where
    R: UserOnboardingRepository + ?Sized,
{
    repo.set_onboarding_step(user_id, STEP_PROFILE_TYPE, persona_status, None, tenant_id)
        .await?;
    repo.set_onboarding_step(user_id, STEP_PARQ, parq_status, None, tenant_id)
        .await
}

/// Whether the intake still has something to ask this athlete.
///
/// False once either surface has recorded both steps — answered or skipped. A
/// skipped step counts: the athlete already declined once, and re-opening the
/// screen every time they start a conversation is how a safety feature becomes
/// something people learn to dismiss.
#[must_use]
pub fn is_outstanding(steps: &[OnboardingStepRecord]) -> bool {
    let recorded = |id: &str| steps.iter().any(|step| step.step_id == id);
    !(recorded(STEP_PROFILE_TYPE) && recorded(STEP_PARQ))
}

/// Whether this person said they coach others and do not train themselves.
///
/// Read from the durable step rows, which every surface writes: any
/// athlete-only step marked [`STATUS_NOT_APPLICABLE`] means the athlete
/// questions are not theirs. The chat intake marks `parq`; the wizard marks
/// both `about_you` and `parq`.
#[must_use]
pub fn athlete_steps_waived(steps: &[OnboardingStepRecord]) -> bool {
    steps
        .iter()
        .any(|step| waives_athlete_steps(&step.step_id, &step.status))
}

/// Whether writing `status` to `step_id` is the coach-only answer.
fn waives_athlete_steps(step_id: &str, status: &str) -> bool {
    (step_id == STEP_PARQ || step_id == STEP_ABOUT_YOU) && status == STATUS_NOT_APPLICABLE
}

/// Release the athlete's agent signup picked for a coach who does not train.
///
/// Called by every surface right after it writes a step, so the answer and
/// the selection never disagree: the starter is an athlete's, and this person
/// said they coach others and have no training of their own. Any other step
/// write is a no-op. The agent their groups answer with is the group's own,
/// untouched.
///
/// The selection is cleared rather than pointed at the roster agent: their
/// own thread resolves it from the answer at every turn
/// ([`crate::default_agent::own_thread_default`]), so an answer that changes
/// later — a reset, then "I train" — moves them back to an athlete's agent
/// with nothing left behind to undo.
///
/// The answer is the person's, like the step rows, while the selection is per
/// membership: every membership is cleared, not only the tenant the answer
/// arrived through.
///
/// # Errors
///
/// Returns the repository error if listing the memberships or clearing a
/// selection fails.
pub async fn release_coach_only_agent<T>(
    tenants: &T,
    user_id: Uuid,
    step_id: &str,
    status: &str,
) -> AppResult<()>
where
    T: TenantRepository + ?Sized,
{
    if !waives_athlete_steps(step_id, status) {
        return Ok(());
    }
    for tenant_id in tenants.list_membership_tenant_ids(user_id).await? {
        tenants.set_selected_agent(tenant_id, user_id, None).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(step_id: &str, status: &str) -> OnboardingStepRecord {
        OnboardingStepRecord {
            step_id: step_id.to_owned(),
            status: status.to_owned(),
            chosen_channel: None,
        }
    }

    #[test]
    fn both_coach_answers_record_the_role_and_the_athlete_answer_clears_it() {
        assert!(!coaches_others_for(PersonaAnswer::Athlete));
        assert!(coaches_others_for(PersonaAnswer::Coach));
        assert!(coaches_others_for(PersonaAnswer::CoachAndAthlete));
    }

    #[test]
    fn only_a_coach_who_does_not_train_is_spared_the_athlete_steps() {
        assert!(PersonaAnswer::Athlete.trains());
        assert!(PersonaAnswer::CoachAndAthlete.trains());
        assert!(!PersonaAnswer::Coach.trains());
    }

    #[test]
    fn either_athlete_step_marked_not_applicable_waives_them() {
        assert!(athlete_steps_waived(&[step(
            STEP_PARQ,
            STATUS_NOT_APPLICABLE
        )]));
        assert!(athlete_steps_waived(&[step(
            STEP_ABOUT_YOU,
            STATUS_NOT_APPLICABLE
        )]));
    }

    #[test]
    fn only_a_not_applicable_athlete_step_is_the_coach_only_answer() {
        assert!(waives_athlete_steps(STEP_PARQ, STATUS_NOT_APPLICABLE));
        assert!(waives_athlete_steps(STEP_ABOUT_YOU, STATUS_NOT_APPLICABLE));
        assert!(!waives_athlete_steps(STEP_PARQ, STATUS_COMPLETE));
        assert!(!waives_athlete_steps(STEP_PARQ, STATUS_SKIPPED));
        assert!(!waives_athlete_steps(
            STEP_PROFILE_TYPE,
            STATUS_NOT_APPLICABLE
        ));
    }

    #[test]
    fn a_skipped_screen_or_another_step_does_not_waive_them() {
        assert!(!athlete_steps_waived(&[step(STEP_PARQ, STATUS_SKIPPED)]));
        assert!(!athlete_steps_waived(&[step(
            STEP_PROFILE_TYPE,
            STATUS_NOT_APPLICABLE
        )]));
        assert!(!athlete_steps_waived(&[]));
    }
}
