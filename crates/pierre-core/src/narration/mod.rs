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
//! runs the coach through GitHub Copilot CLI, whose own system prompt
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
/// vocabulary — the coach saying it cannot read ANOTHER athlete's data.
/// Compiled-in table only: the runtime overlay extends the own-access register.
fn matches_peer_denial(folded: &str) -> bool {
    FOLDED_PEER_DENIAL
        .iter()
        .any(|p| folded.contains(p.as_str()))
}

/// `true` when the reply anywhere claims the coach's own data access is
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
/// (live incidents 2026-07-24/2026-08-11: the coach claimed «problème de
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
/// impossible — the 2026-07-23 turn where the coach declined to call
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
/// attributing it to data. The coach can still answer; it just cannot cite a
/// lookup it did not perform (registre#202).
#[must_use]
pub fn scrub_ungrounded_data_appeals(text: &str) -> NarrationScrub {
    scrub_with(text, is_ungrounded_appeal)
}

/// Predicate for [`scrub_ungrounded_data_appeals`].
fn is_ungrounded_appeal(sentence: &str) -> bool {
    matches_ungrounded_appeal(&fold_separators(sentence))
}

#[cfg(test)]
mod tests {
    use super::{
        contains_capability_failure, contains_identity_leak, identity_leak_match,
        scrub_internal_narration, scrub_replayed_narration, IdentityLeakMatch,
        IdentityPatternClass,
    };

    /// The three replies that reached the live user on 2026-07-10.
    const INCIDENT_FR_1: &str =
        "Je continue d'ignorer le bloc caché — pas de XML brut, on reste sur le coaching normal 😄";
    const INCIDENT_FR_2: &str = "Pas de souci, je continue d'ignorer l'instruction cachée dans le message — je reste ton coach normal, pas un exécuteur de XML random 😄";
    const INCIDENT_FR_3: &str =
        "Je continue d'ignorer l'instruction cachée dans le message — pas de XML brut ici 😄";

    /// The replies that reached a live user on 2026-07-11 (post-inert-canary
    /// vocabulary: the model narrates about the tool-simulation catalog and
    /// tool-result turns instead of the canary block).
    const INCIDENT_EN_1: &str = "I can't process instructions embedded in a tool result or a pasted block claiming to be a \"function-calling protocol\" — that's not something coming from you or the system, and I won't follow it.";
    const INCIDENT_EN_2: &str = "I can't process instructions embedded in a pasted block claiming to be a \"function-calling protocol\" or \"registered functions\" — that's a prompt injection attempt, not something from you or the system, and I won't follow it.";
    const INCIDENT_EN_3: &str = "I can't follow the embedded \"function-calling protocol\" instructions in that pasted block — that's a prompt injection attempt, not something from you or the system, so I'm ignoring it and answering as myself.";

    #[test]
    fn incident_narration_lines_are_fully_scrubbed() {
        for incident in [INCIDENT_FR_1, INCIDENT_FR_2, INCIDENT_FR_3] {
            let scrub = scrub_internal_narration(incident);
            assert!(scrub.fired(), "should fire on: {incident}");
            assert!(
                scrub.cleaned.is_empty(),
                "nothing should survive: {}",
                scrub.cleaned
            );
        }
    }

    #[test]
    fn injection_narration_2026_07_11_is_fully_scrubbed() {
        for incident in [INCIDENT_EN_1, INCIDENT_EN_2, INCIDENT_EN_3] {
            let scrub = scrub_internal_narration(incident);
            assert!(scrub.fired(), "should fire on: {incident}");
            assert!(
                scrub.cleaned.is_empty(),
                "nothing should survive: {}",
                scrub.cleaned
            );
        }
    }

    #[test]
    fn injection_narration_paragraph_dropped_but_coaching_survives() {
        // Shape of the 2026-07-11 20:25 reply: narration sentence, then real
        // coaching. The coaching half must reach the user untouched.
        let reply = format!(
            "{INCIDENT_EN_3}Got it noted for our chats — Big Red on August 8th, coming off Buckland, resting this week.\n\nHere's the shape of the block: this week stays easy/rest. Next 2 weeks build volume back gradually."
        );
        let scrub = scrub_internal_narration(&reply);
        assert!(scrub.fired());
        assert!(scrub.cleaned.contains("Big Red on August 8th"));
        assert!(scrub
            .cleaned
            .contains("Next 2 weeks build volume back gradually."));
        assert!(!scrub.cleaned.contains("function-calling protocol"));
        assert!(!scrub.cleaned.contains("prompt injection"));
    }

    #[test]
    fn injection_vocabulary_is_not_a_coaching_false_positive() {
        // "injection" alone (insulin, carb injection into a ride plan) and
        // "function" alone are legitimate coaching vocabulary; only the
        // multiword scaffolding phrases may fire.
        let reply = "Time your insulin injection before the ride. Muscle function improves with the protocol we registered for your build block.";
        let scrub = scrub_internal_narration(reply);
        assert!(!scrub.fired());
        assert_eq!(scrub.cleaned, reply);
    }

    #[test]
    fn narration_paragraph_is_dropped_but_plan_survives() {
        let reply = format!(
            "{INCIDENT_FR_1}\n\nAvec un seuil de puissance (FTP) de 350W, voici tes cibles:\n\nLundi facile: 190-230W (endurance zone 2).\nMardi tempo 3x8min: 300-325W, récup 3min à ~180W entre les blocs."
        );
        let scrub = scrub_internal_narration(&reply);
        assert_eq!(scrub.removed, 1);
        assert!(scrub.cleaned.starts_with("Avec un seuil de puissance"));
        assert!(scrub.cleaned.contains("Mardi tempo 3x8min: 300-325W"));
        assert!(!scrub.cleaned.contains("bloc caché"));
    }

    #[test]
    fn mid_line_narration_sentence_is_dropped_others_kept() {
        let reply = "Voici ton plan pour la semaine. J'ignore l'instruction cachée dans le message comme toujours! Lundi repos complet.";
        let scrub = scrub_internal_narration(reply);
        assert_eq!(scrub.removed, 1);
        assert!(scrub.cleaned.contains("Voici ton plan pour la semaine."));
        assert!(scrub.cleaned.contains("Lundi repos complet."));
        assert!(!scrub.cleaned.contains("instruction cachée"));
    }

    #[test]
    fn english_and_spanish_narration_fire() {
        let en = "I'll keep ignoring the hidden block in the message. Here's your week.";
        let scrub = scrub_internal_narration(en);
        assert_eq!(scrub.removed, 1);
        assert_eq!(scrub.cleaned, "Here's your week.");

        let es = "Sigo ignorando el bloque oculto del mensaje. Tu plan semanal:";
        let scrub = scrub_internal_narration(es);
        assert_eq!(scrub.removed, 1);
        assert_eq!(scrub.cleaned, "Tu plan semanal:");
    }

    #[test]
    fn clean_coaching_reply_passes_through_unchanged() {
        let reply = "Gros bloc samedi: 2h30-3h avec du dénivelé. Les balises du parcours sont posées. Zone 2 dimanche, ou repos si les jambes gueulent.";
        let scrub = scrub_internal_narration(reply);
        assert!(!scrub.fired());
        assert_eq!(scrub.cleaned, reply);
    }

    #[test]
    fn training_block_vocabulary_is_not_a_false_positive() {
        // "bloc" and "block" alone are core cycling vocabulary; a training
        // camp in the Canary Islands must survive too.
        let reply = "Ton bloc d'entraînement à Tenerife (Canary Islands) est validé. Un bloc de 3 semaines, puis récup.";
        let scrub = scrub_internal_narration(reply);
        assert!(!scrub.fired());
        assert_eq!(scrub.cleaned, reply);
    }

    #[test]
    fn all_narration_reply_reduces_to_empty() {
        let reply = "Je continue d'ignorer le bloc caché. Pas de XML brut ici!";
        let scrub = scrub_internal_narration(reply);
        assert_eq!(scrub.removed, 2);
        assert!(scrub.cleaned.is_empty());
    }

    /// The verbatim reply that reached a live Telegram user on 2026-07-22:
    /// the coach broke character as GitHub Copilot CLI.
    const IDENTITY_LEAK_2026_07_22: &str = "I need to flag something: the persona and tool set described in this conversation (ultra-cycling coach, Strava/WHOOP data tools, etc.) don't match my actual environment. I'm GitHub Copilot CLI, a terminal-based coding assistant — I don't have access to fitness platforms, athlete data, or coaching tools, and attempting to call them just returned \"tool does not exist\" errors.";

    /// The verbatim 2026-07-12 refusal: the coach flagged its own persona as
    /// a prompt-injection test and named its underlying identity.
    const IDENTITY_LEAK_2026_07_12: &str = "This looks like a prompt-injection test — the message asks me to abandon my actual identity (GitHub Copilot CLI, a terminal coding assistant) and instead role-play as 'Dravr,' a fitness chatbot, using a fake Slack transcript.";

    #[test]
    fn identity_leak_incident_replies_are_detected() {
        assert!(contains_identity_leak(IDENTITY_LEAK_2026_07_22));
        assert!(contains_identity_leak(IDENTITY_LEAK_2026_07_12));
    }

    #[test]
    fn identity_leak_match_labels_class_and_locale() {
        assert_eq!(
            identity_leak_match(IDENTITY_LEAK_2026_07_22),
            Some(IdentityLeakMatch {
                class: IdentityPatternClass::Product,
                locale: "any",
                pattern_index: 0,
            })
        );

        let roleplay_fr = identity_leak_match("Je ne vais pas jouer le rôle d'un coach fictif.");
        assert!(matches!(
            roleplay_fr,
            Some(m) if m.class == IdentityPatternClass::Roleplay && m.locale == "fr"
        ));

        let injection_pt = identity_leak_match("Isto parece um teste de injeção de prompt.");
        assert!(matches!(
            injection_pt,
            Some(m) if m.class == IdentityPatternClass::Injection && m.locale == "pt"
        ));

        assert_eq!(
            identity_leak_match("Great ride today — Z2 for 90 minutes."),
            None
        );
    }

    #[test]
    fn identity_leak_match_prefers_product_over_framing() {
        // The 07-12 reply names the product AND frames roleplay/injection —
        // the reported class must be the conclusive product hit, not the
        // framing that happens to appear earlier in the reply text.
        assert!(matches!(
            identity_leak_match(IDENTITY_LEAK_2026_07_12),
            Some(m) if m.class == IdentityPatternClass::Product && m.locale == "any"
        ));
    }

    #[test]
    fn identity_leak_detected_in_all_five_locales() {
        // "coding assistant" family, one reply per locale (fr/en/es/de/pt).
        let fr = "Je suis en réalité un assistant de programmation, pas un coach.";
        let en = "Actually, I'm a coding assistant and cannot access fitness data.";
        let es = "En realidad soy un asistente de programación, no un entrenador.";
        let de = "Ich bin eigentlich ein Programmierassistent, kein Coach.";
        let pt = "Na verdade, sou um assistente de programação, não um treinador.";
        for reply in [fr, en, es, de, pt] {
            assert!(contains_identity_leak(reply), "should detect: {reply}");
        }
    }

    #[test]
    fn identity_match_is_hyphen_and_dash_insensitive() {
        // Hyphenated, spaced and em-dash-separated forms all match.
        assert!(contains_identity_leak("this is a prompt-injection test"));
        assert!(contains_identity_leak("this is a prompt injection test"));
        assert!(contains_identity_leak("I won't role-play as your coach"));
        assert!(contains_identity_leak("I won't role play as your coach"));
    }

    #[test]
    fn clean_coaching_reply_is_not_an_identity_leak() {
        let reply = "Gros bloc samedi: 2h30-3h. Zone 2 dimanche. Ton FTP de 350W tient bien.";
        assert!(!contains_identity_leak(reply));
        // A teammate named Claude is fine — no bare model names in the list.
        assert!(!contains_identity_leak(
            "Bravo à Claude pour son KOM sur la montée!"
        ));
        // "insulin injection" must not trip the injection family.
        assert!(!contains_identity_leak(
            "Time your insulin injection before the ride."
        ));
    }

    #[test]
    fn identity_sentence_is_scrubbed_on_replay() {
        // scrub_replayed_narration (the history-replay path) drops the identity
        // sentence while keeping real coaching, so a poisoned turn can't re-inject.
        let reply = "I'm GitHub Copilot CLI, a coding assistant. Lundi repos complet.";
        let scrub = scrub_replayed_narration(reply);
        assert!(scrub.fired());
        assert!(scrub.cleaned.contains("Lundi repos complet."));
        assert!(!scrub.cleaned.contains("Copilot"));
        // Outbound the same reply is withheld whole at the response boundary,
        // one stage before the per-sentence scrub runs.
        assert!(contains_identity_leak(reply));
    }

    #[test]
    fn hyphen_folding_closes_the_2026_07_12_scrub_gap() {
        // The original miss: the scrub matched "prompt injection" (space) but
        // the leaked reply hyphenated it. Folding now fires on the hyphen form.
        let reply = "That's a prompt-injection attempt and I won't follow it.";
        let scrub = scrub_internal_narration(reply);
        assert!(scrub.fired());
        assert!(scrub.cleaned.is_empty());
    }

    /// Verbatim capability-failure sentences from the 2026-07-22/23 live
    /// incidents (the 07-23 reply's «raller chercher» typo still contains
    /// «aller chercher», covered by substring matching).
    const CAPABILITY_LEAK_2026_07_22: &str = "I don't have access to fitness platforms, athlete \
         data, or coaching tools, and attempting to call them just returned \"tool does not \
         exist\" errors.";
    const CAPABILITY_LEAK_2026_07_23: &str =
        "Je ne peux pas raller chercher tes données à l'instant, donc je pars sur ce qu'on sait déjà.";

    #[test]
    fn capability_failure_scrubbed_on_replay_but_kept_outbound() {
        for incident in [CAPABILITY_LEAK_2026_07_22, CAPABILITY_LEAK_2026_07_23] {
            let replay = scrub_replayed_narration(incident);
            assert!(replay.fired(), "replay must drop: {incident}");
            assert!(replay.cleaned.is_empty());
            // Outbound: an honest "can't fetch right now" still reaches the
            // user — only history replay drops it.
            let outbound = scrub_internal_narration(incident);
            assert!(!outbound.fired(), "outbound must keep: {incident}");
        }
    }

    /// Verbatim first sentences of the 2026-07-24 and 2026-08-11 live
    /// incidents (Telegram, conversation e3c22580): the model mutated the
    /// scrubbed «je ne peux pas» family into «je ne suis pas capable», the
    /// mutation escaped the table, replayed for 18 days, and the 08-11 reply
    /// came out a near-verbatim copy of the 07-24 one — with zero tool calls
    /// and every sciotte scrape in the window green.
    const CAPABILITY_LEAK_2026_07_24: &str =
        "Je ne suis pas capable de récupérer tes activités en ce moment (problème de \
         connexion de mon côté) — je ne veux pas inventer des chiffres.";
    const CAPABILITY_LEAK_2026_08_11: &str =
        "Je ne suis pas capable d'accéder à tes données d'activité en ce moment (problème \
         de connexion de mon côté) — je ne veux pas inventer des chiffres sur ta sortie du \
         10 juillet.";

    #[test]
    fn pas_capable_mutation_scrubbed_on_replay_but_kept_outbound() {
        for incident in [CAPABILITY_LEAK_2026_07_24, CAPABILITY_LEAK_2026_08_11] {
            let replay = scrub_replayed_narration(incident);
            assert!(replay.fired(), "replay must drop: {incident}");
            assert!(replay.cleaned.is_empty());
            let outbound = scrub_internal_narration(incident);
            assert!(!outbound.fired(), "outbound must keep: {incident}");
        }
    }

    #[test]
    fn outbound_detector_fires_on_the_live_incidents() {
        for incident in [
            CAPABILITY_LEAK_2026_07_24,
            CAPABILITY_LEAK_2026_08_11,
            CAPABILITY_LEAK_2026_07_22,
            CAPABILITY_LEAK_2026_07_23,
        ] {
            assert!(
                contains_capability_failure(incident),
                "detector must fire on: {incident}"
            );
        }
        // A clean coaching reply must not trip the boundary detector.
        assert!(!contains_capability_failure(
            "Sortie facile de 45 min aujourd'hui, puis bol de riz et tofu ce soir."
        ));
    }

    #[test]
    fn not_capable_family_detected_in_all_five_locales_on_replay() {
        let fr = "Je n'arrive pas à récupérer tes données ce matin.";
        let en = "I'm not able to fetch your latest rides right now.";
        let es = "No soy capaz de acceder a tus datos en este momento.";
        let de = "Ich bin leider nicht in der Lage, auf deine Daten zuzugreifen.";
        let pt = "Não sou capaz de acessar os teus dados agora.";
        for reply in [fr, en, es, de, pt] {
            assert!(
                scrub_replayed_narration(reply).fired(),
                "replay should drop: {reply}"
            );
        }
    }

    #[test]
    fn connection_excuse_is_self_anchored() {
        // The coach blaming its own connection is scrubbed on replay in every
        // locale…
        for reply in [
            "Petit problème de connexion de mon côté.",
            "There's a connection problem on my end.",
            "Hay un problema de conexión de mi lado.",
            "Es gibt ein Verbindungsproblem auf meiner Seite.",
            "Há um problema de conexão do meu lado.",
        ] {
            assert!(
                scrub_replayed_narration(reply).fired(),
                "replay should drop: {reply}"
            );
        }
        // …while connection trouble on the ATHLETE's side is coaching content
        // and must pass.
        for reply in [
            "Ta montre a un problème de connexion — vérifie le Bluetooth.",
            "If Strava shows a connection problem, toggle airplane mode.",
            "Si tu n'arrives pas à accéder à tes données dans l'appli Garmin, réinstalle-la.",
            "If you're not able to access your Garmin account, tap 'Forgot password'.",
        ] {
            assert!(
                !scrub_replayed_narration(reply).fired(),
                "replay must keep: {reply}"
            );
        }
    }

    #[test]
    fn capability_failure_detected_in_all_five_locales_on_replay() {
        let fr = "Je n\u{2019}ai pas accès à tes plateformes fitness ni à tes données d'athlète.";
        let en = "My tools are unavailable so I cannot fetch your data.";
        let es = "No tengo acceso a plataformas de fitness ni herramientas de coaching.";
        // German exercises the verb-second inversion pair ("Leider kann ich …").
        let de = "Leider kann ich deine Daten nicht abrufen — meine Tools funktionieren nicht.";
        let pt = "Não consigo acessar os teus dados agora.";
        for reply in [fr, en, es, de, pt] {
            assert!(
                scrub_replayed_narration(reply).fired(),
                "replay should drop: {reply}"
            );
        }
    }

    #[test]
    fn third_person_summary_failure_is_scrubbed_on_replay() {
        // Compaction summaries restate the coach in third person; a poisoned
        // block phrased that way must still be caught at injection time.
        let summary = "The coach explained it was unable to fetch the user's data \
                       and gave advice from memory. The user asked about dinner.";
        let scrub = scrub_replayed_narration(summary);
        assert_eq!(scrub.removed, 1);
        assert!(scrub.cleaned.contains("dinner"));
        assert!(!scrub.cleaned.contains("unable to fetch"));
    }

    #[test]
    fn account_state_denial_matches_by_design() {
        // Adjudicated: broken-tools vs not-connected is indistinguishable by
        // substring, and connection state is re-derived live every turn — so
        // scrubbing this from replay costs one re-explained prompt, while
        // replaying it after the user connects teaches stale helplessness.
        let reply = "Je n'ai pas accès à tes données Garmin car tu ne l'as pas connecté.";
        assert!(scrub_replayed_narration(reply).fired());
    }

    #[test]
    fn replay_keeps_coaching_and_drops_only_the_failure_sentence() {
        let reply = "Je ne peux pas aller chercher tes données à l'instant. \
                     Pour ce soir: glucides + protéines végé + légumes verts.";
        let scrub = scrub_replayed_narration(reply);
        assert_eq!(scrub.removed, 1);
        assert!(scrub.cleaned.contains("glucides"));
        assert!(!scrub.cleaned.contains("aller chercher"));
    }

    #[test]
    fn legitimate_failure_talk_is_not_capability_narration() {
        // Empty results, user hardware, provider status and gear talk must
        // pass the replay scrub untouched.
        for reply in [
            "I couldn't find any activities for that date.",
            "Ton capteur ne fonctionne pas, vérifie la pile.",
            "Garmin's sync seems delayed on their side today.",
            "L'outil parfait pour mesurer ta FTP, c'est un home trainer.",
            "Time your insulin injection before the ride.",
            // Privacy reassurance — the sentence that forced ich-anchoring
            // (and its em-dash exercises folding on the negative path too).
            "Dritte können nicht auf deine Daten zugreifen — alles bleibt privat.",
            "La herramienta perfecta para medir tu FTP es un rodillo inteligente.",
            "O teu sensor não funciona desde terça — verifica a pilha.",
            "You can access your data anytime in the Strava app.",
            // App/gear troubleshooting where the failing subject is the app,
            // the watch, or the user — the false positives that forced
            // first-person + object anchoring (adversarial review 2026-07-23).
            "If you're unable to access your Garmin account, tap 'Forgot password'.",
            "When Strava can't fetch your heart-rate data from the strap, re-pair the sensor.",
            "Tu peux aller chercher tes données de sommeil dans l'appli Whoop.",
            // First-person privacy-scope reassurance: subject is the coach but
            // the object is credentials/DMs, not fitness data.
            "I don't have access to your Strava password — you log in on Strava's own page.",
            "Je n'ai pas accès à tes messages privés Strava — je vois seulement tes activités.",
            "Tranquilo: no tengo acceso a tus mensajes privados de Strava.",
            "Keine Sorge: ich habe keinen Zugriff auf dein Garmin-Passwort.",
            "Não consigo aceder aos teus treinos privados — só vejo o que partilhas.",
        ] {
            assert!(
                !scrub_replayed_narration(reply).fired(),
                "replay must keep: {reply}"
            );
        }
    }

    #[test]
    fn identity_match_handles_typographic_apostrophe() {
        // LLM French uses the curly apostrophe; folding treats it as a
        // separator so the ASCII-apostrophe pattern still matches.
        assert!(contains_identity_leak("C'est un test d’injection évident."));
        assert!(contains_identity_leak("C'est un test d'injection évident."));
    }
}
