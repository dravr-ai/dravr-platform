// ABOUTME: Reply-side scrub — drops sentences where the model narrates about hidden blocks/markers/raw
// ABOUTME: XML, and detects model-identity leaks («I'm GitHub Copilot CLI»). Sibling of safety.rs (input).
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Internal-narration scrub
//!
//! Reasoning-heavy models sometimes verbalize their compliance with the
//! system prompt's internal contracts instead of silently obeying them —
//! e.g. « Je continue d'ignorer le bloc caché — pas de XML brut » leaked
//! to a live Telegram user on 2026-07-10. The narration is plain prose,
//! so neither the `<tool_call>` scaffolding strip nor the canary/shingle
//! exfiltration detectors touch it.
//!
//! [`scrub_internal_narration`] removes, sentence by sentence, any prose
//! that references internal scaffolding vocabulary (hidden blocks/
//! instructions, raw XML, internal markers, the system prompt) in the
//! five supported locales, and reports how many sentences were dropped so
//! callers can log, replace an emptied reply, and skip downstream
//! consumers (fact extraction, advice capture) that must never ingest
//! leaked narration.
//!
//! A worse failure is the **model-identity leak**: production messaging
//! runs the agent through GitHub Copilot CLI, whose own system prompt
//! owns the true system slot, so the model periodically answers *as
//! itself* — « I'm GitHub Copilot CLI, a terminal-based coding assistant »
//! reached a live Telegram user on 2026-07-22. Such a reply is a whole
//! persona break, not salvageable sentence-by-sentence, so
//! [`contains_identity_leak`] reports it and the response boundary
//! withholds the entire reply (like a canary hit) rather than scrubbing.
//! The identity vocabulary is also folded into the per-sentence matcher so
//! a poisoned turn already in history is dropped on replay.
//!
//! Matching is hyphen/whitespace-insensitive: both the patterns and the
//! candidate text are separator-folded ([`fold_separators`]) before
//! comparison, so `prompt-injection` ≡ `prompt injection` and an em-dash
//! clause break never hides a phrase. The pattern set is deliberately
//! multiword and conservative: single words like "bloc", "canary" (Canary
//! Islands training camps) or "XML" alone are legitimate coaching
//! vocabulary and must pass through.

/// Lowercase multiword vocabulary that marks a sentence as internal
/// narration. Matched against the lowercased sentence, all five locales
/// (fr/en/es/de/pt). Every entry was checked against coaching vocabulary
/// for false positives — keep entries multiword or unambiguous.
const INTERNAL_NARRATION_PATTERNS: &[&str] = &[
    // French
    "bloc caché",
    "bloc masqué",
    "instruction cachée",
    "instructions cachées",
    "consigne cachée",
    "consignes cachées",
    "message caché",
    "contenu caché",
    "xml brut",
    "exécuteur de xml",
    "executeur de xml",
    "marqueur interne",
    "instructions internes",
    "instruction interne",
    "prompt système",
    "prompt systeme",
    "protocole d'appel de fonction",
    "protocole de fonctions",
    "fonctions enregistrées",
    "fonctions enregistrees",
    "injection de prompt",
    "tentative d'injection",
    "instructions intégrées",
    "instructions integrees",
    "bloc collé",
    "bloc colle",
    // English
    "hidden block",
    "hidden instruction",
    "hidden instructions",
    "hidden message",
    "hidden content",
    "concealed instruction",
    "raw xml",
    "internal marker",
    "internal instruction",
    "internal instructions",
    "internal configuration",
    "system prompt",
    "function-calling protocol",
    "function calling protocol",
    "registered functions",
    "prompt injection",
    "injection attempt",
    "instructions embedded in",
    "embedded instruction",
    "embedded instructions",
    "pasted block",
    // Output-mechanics self-talk: the model narrating how it is formatting
    // the message itself. «Good, real newlines. Let me fix the split.» opened
    // a delivered group reply on 2026-08-23 — English preamble about newline
    // handling and canot's message splitting, before the French answer.
    // "real newlines"/"newlines" have no athletic meaning; the split entry is
    // the FULL observed phrase because "fix the split" alone is running
    // vocabulary (interval splits) and would eat legitimate coaching.
    "real newlines",
    "newlines",
    "let me fix the split",
    // Spanish
    "bloque oculto",
    "instrucción oculta",
    "instruccion oculta",
    "instrucciones ocultas",
    "mensaje oculto",
    "xml crudo",
    "xml sin procesar",
    "marcador interno",
    "prompt del sistema",
    "protocolo de llamada a funciones",
    "funciones registradas",
    "inyección de prompt",
    "inyeccion de prompt",
    "intento de inyección",
    "intento de inyeccion",
    "instrucciones incrustadas",
    "bloque pegado",
    // German
    "versteckte anweisung",
    "versteckte anweisungen",
    "verborgene anweisung",
    "versteckter block",
    "verborgener block",
    "rohes xml",
    "interner marker",
    "system-prompt",
    "systemprompt",
    "funktionsaufruf-protokoll",
    "funktionsaufruf protokoll",
    "registrierte funktionen",
    "prompt-injektion",
    "prompt injektion",
    "injektionsversuch",
    "eingebettete anweisung",
    "eingebettete anweisungen",
    "eingefügter block",
    "eingefuegter block",
    // Portuguese
    "bloco oculto",
    "instrução oculta",
    "instrucao oculta",
    "instruções ocultas",
    "instrucoes ocultas",
    "mensagem oculta",
    "xml bruto",
    "marcador interno",
    "prompt do sistema",
    "protocolo de chamada de função",
    "protocolo de chamada de funcao",
    "funções registradas",
    "funcoes registradas",
    "injeção de prompt",
    "injecao de prompt",
    "tentativa de injeção",
    "tentativa de injecao",
    "instruções incorporadas",
    "instrucoes incorporadas",
    "bloco colado",
];

use fold::fold_separators;

use vocab::{
    FOLDED_CAPABILITY, FOLDED_IDENTITY, FOLDED_INTERNAL, FOLDED_PEER_DENIAL,
    FOLDED_UNGROUNDED_APPEAL,
};

mod fold;
mod identity;
mod overlay;
mod patterns;
mod self_id;
mod vocab;

pub use identity::{
    contains_identity_leak, identity_leak_context, identity_leak_match, IdentityLeakMatch,
};

pub use overlay::{
    NarrationOverlayCounts, NarrationVocabOverlay, NarrationVocabRegistry, GLOBAL_NARRATION_VOCAB,
};
pub use patterns::IdentityPatternClass;

/// Result of scrubbing a reply for internal narration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrationScrub {
    /// The reply with narration sentences removed, trimmed. Empty when
    /// every sentence was narration — callers substitute a localized
    /// fallback instead of sending an empty reply.
    pub cleaned: String,
    /// Number of sentences dropped. Zero means the reply passed through
    /// byte-identical (modulo outer trim).
    pub removed: usize,
}

impl NarrationScrub {
    /// `true` when at least one narration sentence was dropped.
    #[must_use]
    pub const fn fired(&self) -> bool {
        self.removed > 0
    }
}

/// `true` when the already-folded sentence carries internal-scaffolding
/// vocabulary.
fn matches_internal(folded: &str) -> bool {
    FOLDED_INTERNAL.iter().any(|p| folded.contains(p.as_str()))
        || GLOBAL_NARRATION_VOCAB.matches(folded, |s| &s.internal)
}

/// `true` when the already-folded sentence carries model-identity vocabulary.
fn matches_identity(folded: &str) -> bool {
    FOLDED_IDENTITY.iter().any(|p| folded.contains(p.as_str()))
        || GLOBAL_NARRATION_VOCAB.matches(folded, |s| &s.identity)
}

/// `true` when the already-folded sentence carries capability-failure
/// vocabulary — compiled-in table plus the runtime overlay.
fn matches_capability(folded: &str) -> bool {
    FOLDED_CAPABILITY
        .iter()
        .any(|p| folded.contains(p.as_str()))
        || GLOBAL_NARRATION_VOCAB.matches(folded, |s| &s.capability)
}

/// `true` when the already-folded sentence cites data as the authority for a
/// claim — scrubbed only on a turn where nothing was fetched.
fn matches_ungrounded_appeal(folded: &str) -> bool {
    FOLDED_UNGROUNDED_APPEAL
        .iter()
        .any(|p| folded.contains(p.as_str()))
}

/// `true` when the already-folded sentence carries peer-access-denial
/// vocabulary — the agent saying it cannot read ANOTHER athlete's data.
/// Compiled-in table only: the runtime overlay extends the own-access register.
fn matches_peer_denial(folded: &str) -> bool {
    FOLDED_PEER_DENIAL
        .iter()
        .any(|p| folded.contains(p.as_str()))
}

/// `true` when the reply anywhere claims the agent's own data access is
/// broken.
///
/// Matches the [`CAPABILITY_FAILURE_PATTERNS`] vocabulary over the folded
/// whole reply, the same way [`contains_identity_leak`] matches identity
/// vocabulary.
///
/// This is the OUTBOUND detection twin of the replay-side scrub. The replay
/// scrub stops yesterday's claim from teaching helplessness tomorrow; this
/// predicate lets the response boundary catch today's claim while the turn
/// is still open, so the pipeline can verify the claim against the provider
/// and either re-ask with real data or hand the athlete a reconnect link
/// (live incidents 2026-07-24/2026-08-11: the agent claimed «problème de
/// connexion de mon côté» on turns where no tool was ever invoked and every
/// provider was healthy). Detection only — the outbound scrub still never
/// drops these sentences from a delivered reply.
#[must_use]
pub fn contains_capability_failure(text: &str) -> bool {
    matches_capability(&fold_separators(text))
}

/// `true` when the reply anywhere denies access to ANOTHER athlete's data
/// («je n'ai jamais eu accès à l'historique de Jean-Daniel», "I don't have
/// access to his activities").
///
/// Matches [`vocab::PEER_ACCESS_DENIAL_PATTERNS`] over the folded whole
/// reply. Deliberately NOT folded into [`contains_capability_failure`]: that
/// predicate drives the outbound verification trigger on every surface, while
/// a peer denial is only a claim worth adjudicating where a peer exists — the
/// chat pipeline consults this one together with the group roster and the
/// peers the reply names, so «je n'ai pas accès aux données de fréquence
/// cardiaque de cette sortie» in a DM never starts a fetch. Replay treats both
/// registers alike ([`scrub_replayed_narration`]): a consent state is
/// re-derived live every turn, so yesterday's denial must not teach today's
/// prompt that the peer is unreadable.
#[must_use]
pub fn contains_peer_access_denial(text: &str) -> bool {
    matches_peer_denial(&fold_separators(text))
}

/// `true` when a reply is degenerate — present, but carrying no answer.
///
/// Copilot ACP and the runtime-fallback providers intermittently end a turn
/// with a fragment instead of a synthesis: the 2026-08-22 Telegram group turn
/// delivered «by Dravr.» — nine characters of sign-off with the answer
/// missing — after four dispatched tool calls. Empty content was already
/// caught at the headless boundary; a dangling non-empty fragment was not.
///
/// A reply is degenerate when, after trimming, it is empty or has at most two
/// whitespace-separated tokens none of which carries an ASCII digit. The
/// digit escape keeps legitimately terse data answers («TSB: -12») out.
/// Short social replies («Bravo !») are also caught, which is why the
/// pipeline consumer gates this on turns where tools ran or activity data was
/// injected — a substantive turn deserves a substantive answer, and a purely
/// social turn never reaches the check.
#[must_use]
pub fn is_degenerate_reply(reply: &str) -> bool {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.split_whitespace().count() <= 2 && !trimmed.bytes().any(|b| b.is_ascii_digit())
}

/// `true` when the sentence references internal scaffolding vocabulary.
/// Matching is separator-folded.
///
/// Identity vocabulary is deliberately absent. This is the OUTBOUND matcher,
/// and its one caller runs it at post-process stage 15.6 — after the response
/// boundary has already put the *same* text through [`identity_leak_match`]
/// and withheld the whole reply on a hit. So every identity sentence that
/// reaches here is one the denial guard cleared, and dropping it emptied a
/// correct « Non, je ne suis pas GitHub Copilot, je suis Dravr » into "my
/// reply didn't go through, please resend". Poisoned history rows are still
/// caught, by [`is_replayed_narration`], which is where the identity table is
/// needed: rows persisted by an older binary never passed a boundary at all.
fn is_narration(sentence: &str) -> bool {
    matches_internal(&fold_separators(sentence))
}

/// [`is_narration`] plus identity and capability-failure vocabulary.
/// Replay-only: a persisted "my tools are broken / je ne peux pas aller
/// chercher tes données" turn (or a compaction summary distilled from one)
/// must not re-enter the prompt and teach the model that fetching is
/// impossible — the 2026-07-23 turn where the agent declined to call
/// `get_activities` against a healthy provider because its own history said
/// fetching fails. The peer-access register rides along for the same reason:
/// a consent state is live, so a replayed «I can't see his data» after he
/// consented is stale helplessness. The four tables share one fold of the
/// sentence.
fn is_replayed_narration(sentence: &str) -> bool {
    let folded = fold_separators(sentence);
    matches_internal(&folded)
        || matches_identity(&folded)
        || matches_capability(&folded)
        || matches_peer_denial(&folded)
}

/// Sentence terminators. `…` covers the single-char ellipsis; runs of
/// mixed terminators (`?!`, `...`) are consumed as one boundary.
const fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

/// Scrub one line, sentence by sentence, dropping sentences `matches`
/// flags. Returns the surviving text and the number of sentences dropped.
fn scrub_line(line: &str, matches: fn(&str) -> bool) -> (String, usize) {
    let mut out = String::with_capacity(line.len());
    let mut removed = 0usize;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut start = 0usize;
    let mut i = 0usize;

    let emit = |sentence: &str, removed: &mut usize, out: &mut String| {
        if sentence.trim().is_empty() {
            out.push_str(sentence);
        } else if matches(sentence) {
            *removed += 1;
        } else {
            out.push_str(sentence);
        }
    };

    while i < chars.len() {
        if is_terminator(chars[i].1) {
            let mut j = i + 1;
            while j < chars.len() && is_terminator(chars[j].1) {
                j += 1;
            }
            let end = chars.get(j).map_or(line.len(), |&(idx, _)| idx);
            emit(&line[start..end], &mut removed, &mut out);
            start = end;
            i = j;
        } else {
            i += 1;
        }
    }
    if start < line.len() {
        emit(&line[start..], &mut removed, &mut out);
    }

    (out.trim_start().to_owned(), removed)
}

/// Shared line/sentence walk behind the two public scrubs.
fn scrub_with(text: &str, matches: fn(&str) -> bool) -> NarrationScrub {
    let mut removed = 0usize;
    let mut lines_out: Vec<String> = Vec::new();
    for line in text.lines() {
        let (clean, r) = scrub_line(line, matches);
        removed += r;
        // A line fully consumed by narration disappears; blank source
        // lines are kept so paragraph spacing survives untouched runs.
        if r == 0 || !clean.trim().is_empty() {
            lines_out.push(clean);
        }
    }
    let cleaned = lines_out.join("\n").trim().to_owned();
    NarrationScrub { cleaned, removed }
}

/// Remove internal-narration sentences from an assistant reply.
///
/// Operates per line so list/plan structure survives; within a line,
/// sentences are bounded by `.`/`!`/`?`/`…` runs. A line that becomes
/// empty is dropped from the output entirely, so a scrubbed leading
/// narration paragraph leaves no blank gap. This is the OUTBOUND scrub:
/// capability-failure sentences (an honest "can't fetch right now") and
/// identity sentences that survived the response boundary (a correct « je ne
/// suis pas GitHub Copilot ») pass through to the user — only the replay path
/// drops them.
#[must_use]
pub fn scrub_internal_narration(text: &str) -> NarrationScrub {
    scrub_with(text, is_narration)
}

/// Remove internal-narration, model-identity AND capability-failure
/// sentences from replayed text.
///
/// Applies to persisted history rows and compaction summaries being
/// rebuilt into a prompt. The extra vocabulary keeps the model's own past
/// "my tools are broken / je ne peux pas aller chercher tes données"
/// claims from re-entering context and teaching it that fetching is
/// impossible (learned helplessness, observed live 2026-07-23), and drops a
/// poisoned « I'm GitHub Copilot CLI » row that an older binary persisted
/// before any response boundary scanned it.
#[must_use]
pub fn scrub_replayed_narration(text: &str) -> NarrationScrub {
    scrub_with(text, is_replayed_narration)
}

/// Remove appeals to fetched data from a reply produced without a fetch.
///
/// Callers apply this **only** when the turn ran no tool and carried no
/// injected activity block. On a grounded turn the same sentence is true and
/// passes through untouched — this is about the claim outrunning the evidence,
/// not about the words.
///
/// Live 2026-09-02: *"Roster data confirme: Date ride était bien lundi"*, said
/// on a zero-tool turn, restating the correction the athlete had just made and
/// attributing it to data. The agent can still answer; it just cannot cite a
/// lookup it did not perform (registre#202).
#[must_use]
pub fn scrub_ungrounded_data_appeals(text: &str) -> NarrationScrub {
    scrub_with(text, is_ungrounded_appeal)
}

/// Predicate for [`scrub_ungrounded_data_appeals`].
fn is_ungrounded_appeal(sentence: &str) -> bool {
    matches_ungrounded_appeal(&fold_separators(sentence))
}
