// ABOUTME: Reply-side internal-narration scrub — removes sentences where the model narrates
// ABOUTME: about hidden blocks/markers/raw XML instead of coaching. Sibling of safety.rs (input side).
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
//! The pattern set is deliberately multiword and conservative: single
//! words like "bloc", "canary" (Canary Islands training camps) or "XML"
//! alone are legitimate coaching vocabulary and must pass through.

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

/// `true` when the sentence references internal scaffolding vocabulary.
fn is_narration(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    INTERNAL_NARRATION_PATTERNS
        .iter()
        .any(|p| lower.contains(p))
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
    use super::scrub_internal_narration;

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
}
