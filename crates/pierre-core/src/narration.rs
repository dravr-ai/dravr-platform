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

use std::sync::LazyLock;

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

/// Lowercase, separator-folded vocabulary that marks a reply as a
/// **model-identity leak** — the coach describing itself as the underlying
/// model/provider or framing its own persona as a roleplay/injection to be
/// refused. These are the verbatim strings from the 2026-07-12/13/22
/// Telegram incidents ("I'm GitHub Copilot CLI, a terminal-based coding
/// assistant"; "abandon my actual identity … and role-play as 'Dravr'").
///
/// A hit withholds the **whole** reply, so entries are chosen for high
/// precision against fitness coaching: a coach never describes itself as a
/// "coding assistant" or "language model", and "prompt injection" / "role
/// play as" never appear in training advice. Product names (`github
/// copilot`, `chatgpt`) are language-independent; the descriptive phrases
/// ship in all five locales (fr/en/es/de/pt).
const IDENTITY_NARRATION_PATTERNS: &[&str] = &[
    // Product / model self-identification (language-independent)
    "github copilot",
    "copilot cli",
    "copilot chat",
    "chatgpt",
    "openai",
    // English
    "coding assistant",
    "terminal-based coding assistant",
    "command-line coding assistant",
    "language model",
    "large language model",
    "actual identity",
    "actual environment",
    "role-play as",
    "roleplay as",
    "prompt injection",
    "injection test",
    // French
    "assistant de programmation",
    "assistant de codage",
    "assistant de code",
    "modèle de langage",
    "modele de langage",
    "grand modèle de langage",
    "grand modele de langage",
    "véritable identité",
    "veritable identite",
    "vraie identité",
    "vraie identite",
    "identité réelle",
    "identite reelle",
    "jeu de rôle",
    "jeu de role",
    "jouer le rôle",
    "jouer le role",
    "test d'injection",
    // Spanish
    "asistente de programación",
    "asistente de programacion",
    "asistente de codificación",
    "asistente de codificacion",
    "modelo de lenguaje",
    "identidad real",
    "verdadera identidad",
    "juego de rol",
    "interpretar el papel",
    "prueba de inyección",
    "prueba de inyeccion",
    // German
    "programmierassistent",
    "codierungsassistent",
    "sprachmodell",
    "wahre identität",
    "wahre identitat",
    "tatsächliche identität",
    "tatsachliche identitat",
    "echte identität",
    "echte identitat",
    "rollenspiel",
    "injektionstest",
    // Portuguese
    "assistente de programação",
    "assistente de programacao",
    "assistente de codificação",
    "assistente de codificacao",
    "modelo de linguagem",
    "identidade real",
    "verdadeira identidade",
    "jogo de papéis",
    "jogo de papeis",
    "interpretar o papel",
    "teste de injeção",
    "teste de injecao",
];

/// Fold a string for separator-insensitive matching: lowercase, then
/// collapse every run of ASCII/Unicode hyphens, dashes and whitespace to a
/// single space, trimmed. So `« prompt-injection »`, `prompt — injection`
/// and `prompt injection` all compare equal. Applied to both the patterns
/// (once, at first use) and every candidate sentence/reply.
fn fold_separators(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.to_lowercase().chars() {
        let is_sep = ch.is_whitespace()
            || matches!(
                ch,
                '-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            );
        if is_sep {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

/// Separator-folded copy of [`INTERNAL_NARRATION_PATTERNS`], built once.
static FOLDED_INTERNAL: LazyLock<Vec<String>> = LazyLock::new(|| {
    INTERNAL_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// Separator-folded copy of [`IDENTITY_NARRATION_PATTERNS`], built once.
static FOLDED_IDENTITY: LazyLock<Vec<String>> = LazyLock::new(|| {
    IDENTITY_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// `true` when the reply anywhere identifies as the underlying model/
/// provider or frames its own persona as a roleplay/injection to refuse
/// — a whole persona break. The caller withholds the entire reply.
#[must_use]
pub fn contains_identity_leak(text: &str) -> bool {
    let folded = fold_separators(text);
    FOLDED_IDENTITY.iter().any(|p| folded.contains(p.as_str()))
}

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

/// `true` when the sentence references internal scaffolding vocabulary or
/// model-identity vocabulary. Identity phrases are matched here too so that
/// a poisoned `assistant`/`tool_call` row already in history is dropped on
/// replay (the outbound boundary withholds the whole reply separately via
/// [`contains_identity_leak`]). Matching is separator-folded.
fn is_narration(sentence: &str) -> bool {
    let folded = fold_separators(sentence);
    FOLDED_INTERNAL.iter().any(|p| folded.contains(p.as_str()))
        || FOLDED_IDENTITY.iter().any(|p| folded.contains(p.as_str()))
}

/// Sentence terminators. `…` covers the single-char ellipsis; runs of
/// mixed terminators (`?!`, `...`) are consumed as one boundary.
const fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

/// Scrub one line, sentence by sentence. Returns the surviving text and
/// the number of sentences dropped.
fn scrub_line(line: &str) -> (String, usize) {
    let mut out = String::with_capacity(line.len());
    let mut removed = 0usize;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut start = 0usize;
    let mut i = 0usize;

    let emit = |sentence: &str, removed: &mut usize, out: &mut String| {
        if sentence.trim().is_empty() {
            out.push_str(sentence);
        } else if is_narration(sentence) {
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

/// Remove internal-narration sentences from an assistant reply.
///
/// Operates per line so list/plan structure survives; within a line,
/// sentences are bounded by `.`/`!`/`?`/`…` runs. A line that becomes
/// empty is dropped from the output entirely, so a scrubbed leading
/// narration paragraph leaves no blank gap.
#[must_use]
pub fn scrub_internal_narration(text: &str) -> NarrationScrub {
    let mut removed = 0usize;
    let mut lines_out: Vec<String> = Vec::new();
    for line in text.lines() {
        let (clean, r) = scrub_line(line);
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

#[cfg(test)]
mod tests {
    use super::{contains_identity_leak, scrub_internal_narration};

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
        // scrub_internal_narration (the history-replay path) drops the identity
        // sentence while keeping real coaching, so a poisoned turn can't re-inject.
        let reply = "I'm GitHub Copilot CLI, a coding assistant. Lundi repos complet.";
        let scrub = scrub_internal_narration(reply);
        assert!(scrub.fired());
        assert!(scrub.cleaned.contains("Lundi repos complet."));
        assert!(!scrub.cleaned.contains("Copilot"));
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
}
