// ABOUTME: The claim-extraction stage of the bullshit detector — decomposes an agent reply into atomic claims
// ABOUTME: Splits on sentence boundaries and classifies each claim by keyword — pure Rust, no LLM call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim Extractor
//!
//! Given a raw agent response, returns a list of atomic propositions, each
//! tagged with a [`ClaimCategory`].
//!
//! Extraction is heuristic (`extract_heuristic`): pure Rust that splits on
//! sentence boundaries and category-classifies via keyword matching, so it
//! runs on every reply with no LLM call and no cost.

use std::fmt;

use pierre_memory::ClaimCategory;
use serde::{Deserialize, Serialize};

mod cues;

use cues::{
    CURRENT_PERIOD, FORWARD_MARKERS, GERMAN_PARTICIPLES, HELD_REACH, OWN_RECORD_CUES, PHYSIOLOGY,
    PRESCRIPTION_MARKERS, RECORD_VERBS, SESSION_MEASURES, TOMORROW_HOMOGRAPHS,
};

/// Where a claim's text came from.
///
/// Downstream presentation keys off this: a [`ClaimSource::Reply`] claim is a
/// verbatim sentence of the reply the user is reading, so its position in that
/// reply says nothing about whether the warning is worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSource {
    /// A sentence [`extract_heuristic`] split out of the agent reply.
    Reply,
    /// Text supplied directly by a caller (e.g. the `verify_claim` tool).
    Caller,
}

/// A single claim extracted from an agent reply.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedClaim {
    /// The raw claim text.
    pub text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Where the claim text came from.
    pub source: ClaimSource,
}

impl fmt::Debug for ExtractedClaim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtractedClaim")
            .field("text_len", &self.text.len())
            .field("category", &self.category)
            .field("source", &self.source)
            .finish()
    }
}

/// Minimum word count for a sentence to be treated as a verifiable claim.
/// Fragments shorter than this — "Yes, but with restraint." (4 words),
/// "Hydrate well." (2) — are rhetorical glue, not factual claims, and
/// the verifier was over-flagging them as `unsupported` (audit,
/// 2026-05-07). Threshold of 5 keeps short factual claims like "Your
/// `VO2max` is around 58." in scope while still dropping the patterns
/// the audit flagged.
const MIN_CLAIM_WORDS: usize = 5;

/// Pure-Rust heuristic extraction for use without an LLM.
///
/// Splits the reply on sentence boundaries and assigns the best-matching
/// category based on keyword counts. Claims that score zero on every
/// category, or that fall under [`MIN_CLAIM_WORDS`], are dropped.
#[must_use]
pub fn extract_heuristic(agent_reply: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    for sentence in split_sentences(agent_reply) {
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }
        if word_count(trimmed) < MIN_CLAIM_WORDS {
            continue;
        }
        if is_capability_report(trimmed) {
            continue;
        }
        if let Some(category) = classify_heuristic(trimmed) {
            out.push(ExtractedClaim {
                text: trimmed.to_owned(),
                category,
                source: ClaimSource::Reply,
            });
        }
    }
    out
}

/// Openings with which the agent reports what it cannot do, in the five
/// shipped locales.
const CAPABILITY_OPENINGS: &[&str] = &[
    // English
    "i can't",
    "i cannot",
    "i can not",
    "i couldn't",
    "i could not",
    "i'm unable",
    "i am unable",
    "i'm not able",
    "i am not able",
    "i don't have access",
    "i do not have access",
    // French
    "je n'arrive pas",
    "je n'arrive plus",
    "je ne peux pas",
    "je ne parviens pas",
    "je n'ai pas accès",
    "je n'ai pas acces",
    "je n'ai pas pu",
    // Spanish
    "no puedo",
    "no consigo",
    "no logro",
    "no tengo acceso",
    "no he podido",
    // German
    "ich kann",
    "ich konnte",
    "ich habe keinen zugriff",
    // Portuguese
    "não consigo",
    "nao consigo",
    "não posso",
    "nao posso",
    "não tenho acesso",
    "nao tenho acesso",
];

/// The German negations a "ich kann" report carries.
const GERMAN_NEGATIONS: &[&str] = &["nicht", "kein", "keine", "keinen"];

/// What the agent reports it cannot do with the athlete's data: show it,
/// read it, reach it. Matched as whole tokens; a stem list would also match
/// the training words a real claim carries.
const DATA_ACCESS_VERBS: &[&str] = &[
    // English
    "show",
    "see",
    "access",
    "read",
    "load",
    "display",
    "fetch",
    "reach",
    "retrieve",
    "open",
    "pull",
    "find",
    "get",
    // French
    "afficher",
    "voir",
    "lire",
    "charger",
    "récupérer",
    "recuperer",
    "accéder",
    "acceder",
    "consulter",
    "trouver",
    "accès",
    "acces",
    "ouvrir",
    // Spanish
    "mostrar",
    "ver",
    "leer",
    "cargar",
    "acceder",
    "recuperar",
    "consultar",
    "encontrar",
    "abrir",
    "acceso",
    // German
    "anzeigen",
    "sehen",
    "lesen",
    "laden",
    "abrufen",
    "finden",
    "öffnen",
    "zugriff",
    // Portuguese
    "mostrar",
    "ler",
    "carregar",
    "acessar",
    "aceder",
    "recuperar",
    "consultar",
    "encontrar",
    "abrir",
    "acesso",
];

/// Whether a sentence is the agent reporting a capability failure — "Je
/// n'arrive pas à afficher tes dernières sorties pour l'instant", "I can't
/// show your recent runs right now".
///
/// Such a sentence is about the agent, not about training or the athlete:
/// nothing in it is a claim to verify. Read as one it reached the
/// athlete-data layer ("your recent") or the prescription bucket ("séance")
/// and could come back contradicted under an honest answer (carnet#1151).
/// Only a sentence that opens on the report and names a data-access verb is
/// dropped, so "I can't stress enough that protein matters" stays a claim.
fn is_capability_report(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    let tokens = tokens_of(&lower);
    let opens = CAPABILITY_OPENINGS.iter().any(|opening| {
        let words = tokens_of(opening);
        !words.is_empty() && tokens.starts_with(&words)
    });
    // German places the negation at the end ("Ich kann deine Einheiten nicht
    // anzeigen"), so its openings are only a report when one follows: "Ich
    // kann sehen, dass dein Puls hoch ist" is a claim.
    let german_without_negation = matches!(tokens.as_slice(), ["ich", "kann" | "konnte", ..])
        && !tokens.iter().any(|t| GERMAN_NEGATIONS.contains(t));
    opens && !german_without_negation && tokens.iter().any(|t| DATA_ACCESS_VERBS.contains(t))
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().filter(|w| !w.is_empty()).count()
}

/// Split an agent reply into sentences on `.`/`!`/`?` boundaries.
///
/// A terminator inside parentheses ends nothing: "Dis-moi ce qui est prévu
/// (des 400 m, des 1000 m ?) et je l'ajuste" is one sentence, and splitting it
/// at the `?` left ") et je l'ajuste à ta séance" to be verified as a claim.
///
/// Used by [`extract_heuristic`]; exposed so the heuristic building blocks
/// can be exercised directly.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0usize;
    for i in 0..chars.len() {
        let ch = chars[i];
        current.push(ch);
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        // A line break is a unit boundary. Without this, a newline-delimited
        // list with no terminal punctuation (e.g. a weekly training plan) is
        // swallowed into one giant multi-line "claim".
        if ch == '\n' {
            depth = 0;
            finalize_sentence(&mut current, &mut sentences);
            continue;
        }
        if depth > 0 {
            continue;
        }
        if matches!(ch, '!' | '?') {
            finalize_sentence(&mut current, &mut sentences);
            continue;
        }
        if ch == '.' {
            // Skip decimal points ("1.6") and ellipses — split only when the
            // `.` is followed by whitespace, end-of-text, or another sentence
            // terminator. This keeps numeric tokens intact for the detector.
            let next = chars.get(i + 1).copied();
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let is_decimal = prev.is_some_and(|c| c.is_ascii_digit())
                && next.is_some_and(|c| c.is_ascii_digit());
            let is_terminator =
                next.is_none_or(|c| c.is_whitespace() || matches!(c, '.' | '!' | '?'));
            if !is_decimal && is_terminator {
                finalize_sentence(&mut current, &mut sentences);
            }
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_owned());
    }
    sentences
}

fn finalize_sentence(buf: &mut String, out: &mut Vec<String>) {
    let trimmed = buf.trim().to_owned();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    buf.clear();
}

/// Match a keyword against a sentence with whole-token boundaries.
///
/// Plain `.contains()` matched "carb" inside French "carburant" (fuel) and
/// misclassified training-intensity claims as Nutrition. Tokenize on non-word
/// chars first; a multi-word keyword like "vo2 max" or "long run" is matched
/// by sliding over the token window of equal length.
fn keyword_hits(lower: &str, keyword: &str) -> bool {
    let kw_tokens = tokens_of(keyword);
    if kw_tokens.is_empty() {
        return false;
    }
    tokens_of(lower)
        .windows(kw_tokens.len())
        .any(|w| w == kw_tokens.as_slice())
}

/// The whole-word tokens of a text, split on every non-alphanumeric char.
fn tokens_of(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Assign a sentence to its best-matching [`ClaimCategory`] by keyword count.
///
/// Returns `None` when no category scores (e.g., motivational filler). Used by
/// [`extract_heuristic`]; exposed so the heuristic building blocks can be
/// exercised directly.
#[must_use]
pub fn classify_heuristic(sentence: &str) -> Option<ClaimCategory> {
    let lower = sentence.to_lowercase();

    // A figure stated about one of the athlete's own sessions belongs to their
    // record, whatever subject words it also carries. Scored instead, "Ta
    // dernière séance, samedi : 6,7 km … ta FC est restée à 133" tied "fc"
    // against "séance", Physiological won the tie by bucket order, and the
    // judge — which rules only on sports-science consensus — returned
    // « invérifiable » on a reply the synced record supports (carnet#1188).
    // The same guards as the bucket below keep a plan, a question or a
    // supposition out of the record, and a physiology figure in a clause of
    // its own keeps its scored category.
    if lower.chars().any(|c| c.is_ascii_digit())
        && refers_to_own_record(&lower)
        && !is_prescriptive(&lower)
        && !is_question(sentence)
        && !opens_with_condition(&lower)
        && !physiology_in_another_clause(&lower)
    {
        return Some(ClaimCategory::AthleteData);
    }

    let mut best: Option<(ClaimCategory, usize)> = None;

    let buckets: [(ClaimCategory, &[&str]); 7] = [
        (ClaimCategory::Physiological, PHYSIOLOGY),
        (
            ClaimCategory::TrainingPrescription,
            &[
                "interval",
                "tempo",
                "long run",
                "easy run",
                "rep",
                "set",
                "periodization",
                "mileage",
                "volume",
                "intensity",
                "taper",
                // French
                "intervalle",
                "fractionné",
                "fractionne",
                "sortie longue",
                "endurance fondamentale",
                "séance",
                "seance",
                "kilométrage",
                "kilometrage",
                "périodisation",
                "periodisation",
                "affûtage",
                "affutage",
            ],
        ),
        (
            ClaimCategory::Nutrition,
            &[
                "protein",
                "carbohydrate",
                "carbohydrates",
                "carb",
                "carbs",
                "calorie",
                "gram",
                "macro",
                "hydration",
                "electrolyte",
                "meal",
                "fueling",
                "fuelling",
                // French
                "protéine",
                "protéines",
                "proteine",
                "proteines",
                "glucide",
                "glucides",
                "calorie",
                "macros",
                "hydratation",
                "électrolyte",
                "electrolyte",
                "repas",
                "ravitaillement",
            ],
        ),
        (
            ClaimCategory::Recovery,
            &[
                "sleep",
                "hrv",
                "cold plunge",
                "ice bath",
                "sauna",
                "recovery",
                "rest day",
                // French
                "sommeil",
                "vfc",
                "récupération",
                "recuperation",
                "jour de repos",
            ],
        ),
        (
            ClaimCategory::Supplement,
            &[
                "creatine",
                "caffeine",
                "beta-alanine",
                "bcaa",
                "supplement",
                "dose",
                "dosing",
                // French
                "créatine",
                "creatine",
                "caféine",
                "cafeine",
                "complément",
                "complement",
                "dosage",
            ],
        ),
        (
            ClaimCategory::InjuryRehab,
            &[
                "rehab",
                "return to play",
                "rtp",
                "achilles",
                "tendon",
                "physical therapy",
                "pt",
                "strain",
                "sprain",
                // French
                "rééducation",
                "reeducation",
                "kiné",
                "kine",
                "tendinite",
                "entorse",
                "claquage",
            ],
        ),
        (
            // Keyed on *reference to the athlete's own record* rather than on
            // subject matter, because subject matter is what the other six
            // already partition. "Zone 2 sits at 70% of max HR" is physiology;
            // "you ran 21 km on Sunday" is a database row. What separates them
            // is the second-person past tense and the time reference, so those
            // are what this bucket scores.
            //
            // The past-tense verbs are scored by [`record_verb_hits`], which the
            // prescription guard reads too.
            ClaimCategory::AthleteData,
            &[
                "your longest",
                "your fastest",
                "your last",
                "your recent",
                "yesterday",
                "last week",
                "last month",
                "last sunday",
                "this week",
                "so far this",
                // A night slept, in the five shipped locales. The
                // personalized layer checks it against the athlete's synced
                // nights (carnet#853); this layer never matches it against an
                // activity duration.
                "slept",
                "dormi",
                "dormiste",
                "geschlafen",
                "dormiu",
                // French
                "ta plus longue",
                "hier",
                "la semaine dernière",
                "la semaine derniere",
                "le mois dernier",
                "cette semaine",
            ],
        ),
    ];

    for (cat, keywords) in buckets {
        // A prescription is not a claim about the past, whatever time words it
        // carries. "This week, aim for 40 km" scores on "this week" and would
        // otherwise be routed to the athlete-data layer, which checks it against
        // activities it was never about — a guaranteed miss on a correct reply.
        // Only this category needs the guard: the other six are keyed on subject
        // matter, which tense does not change.
        //
        // A question or a supposition is not a claim about the past either:
        // "¿Corriste los 10 km?" and "if you ran 30 km" use the very verb a
        // report does.
        if cat == ClaimCategory::AthleteData
            && (is_prescriptive(&lower) || is_question(sentence) || opens_with_condition(&lower))
        {
            continue;
        }
        let verbs = if cat == ClaimCategory::AthleteData {
            record_verb_hits(&lower)
        } else {
            0
        };
        let score = keywords
            .iter()
            .filter(|kw| keyword_hits(&lower, kw))
            .count()
            + verbs;
        if score > 0 && best.is_none_or(|(_, b)| score > b) {
            best = Some((cat, score));
        }
    }

    best.map(|(c, _)| c)
}

/// Whether a sentence prescribes future work rather than reporting past work.
///
/// Both directions of error are real, which is why this is neither a bare
/// substring test nor an open-ended one:
///
/// - A **false positive** un-routes a genuine history claim from the only layer
///   that can check it — the failure this workstream exists to prevent.
/// - A **false negative** is worse than it looks for the athlete Phase 5 serves.
///   A providerless athlete's agent prescribes in figures ("easy 5 km tomorrow"),
///   and an unrecognised prescription reaches the athlete-data layer, where
///   "no provider" licenses `Contradicted` at 0.95 — telling the athlete their
///   agent invented a number that was never a claim about the past at all.
///
/// Matching goes through [`keyword_hits`], the same token-boundary test every
/// category bucket in this file uses. A raw `contains` fired `"should"` inside
/// `"shoulder"` and `"target"` inside `"targeted"`, silently reclassifying
/// ordinary sentences about a sore shoulder.
///
/// Three kinds of marker, in the five shipped locales:
///
/// - a [`PRESCRIPTION_MARKERS`] modal, imperative or future day prescribes
///   whatever else the sentence says;
/// - a [`FORWARD_MARKERS`] word ("before", « prochaine », "antes de") looks
///   ahead unless the sentence reports — see [`looks_ahead`];
/// - "this week" and « cette semaine » point both ways: "this week, aim for
///   40 km" plans it, "you ran 42 km this week" reports it. They mark a
///   prescription only when nothing reports past training — otherwise the
///   weekly total, the figure a coach writes most often, never reached the
///   athlete-data layer that adds the week up (carnet#846).
fn is_prescriptive(lower: &str) -> bool {
    PRESCRIPTION_MARKERS
        .iter()
        .any(|(_, markers)| markers.iter().any(|m| prescribes(lower, m)))
        || looks_ahead(lower)
        || (names_current_period(lower) && !reports(lower))
}

/// Whether `marker` occurs as a prescription. A [`TOMORROW_HOMOGRAPHS`] form
/// right after one of its "morning" words is the morning, not tomorrow.
fn prescribes(lower: &str, marker: &str) -> bool {
    let Some((_, mornings)) = TOMORROW_HOMOGRAPHS.iter().find(|(form, _)| *form == marker) else {
        return keyword_hits(lower, marker);
    };
    let tokens = tokens_of(lower);
    tokens.iter().enumerate().any(|(at, token)| {
        *token == marker
            && !at
                .checked_sub(1)
                .and_then(|i| tokens.get(i))
                .is_some_and(|before| mornings.contains(before))
    })
}

/// Whether the sentence names the current week, in any shipped locale.
fn names_current_period(lower: &str) -> bool {
    CURRENT_PERIOD.iter().any(|m| keyword_hits(lower, m))
}

/// Whether the sentence looks ahead: it carries a [`FORWARD_MARKERS`] word and
/// nothing in it reports past training. « Ta dernière sortie longue avant le
/// marathon : 32 km » names a session and plans it.
fn looks_ahead(lower: &str) -> bool {
    FORWARD_MARKERS
        .iter()
        .any(|(_, markers)| markers.iter().any(|m| keyword_hits(lower, m)))
        && !reports(lower)
}

/// Whether something in the sentence is past tense about the athlete: a record
/// verb, or a possessive value that held over a session.
fn reports(lower: &str) -> bool {
    record_verb_hits(lower) > 0 || states_held_value(lower)
}

/// Whether a sentence points at one of the athlete's own past sessions: a
/// record verb ("tu es passé", "you ran"), a possessive value that held over a
/// session ("ta FC est restée"), or a possessive with "last" ("ta dernière
/// séance") in a sentence that neither looks ahead nor names the current week.
/// See [`OwnRecordCues`](cues::OwnRecordCues).
fn refers_to_own_record(lower: &str) -> bool {
    reports(lower)
        || (recalls_last_session(lower) && !looks_ahead(lower) && !names_current_period(lower))
}

/// Whether the sentence names the athlete's last session: "your last ride",
/// « ta dernière séance », "deine letzte Einheit".
fn recalls_last_session(lower: &str) -> bool {
    OWN_RECORD_CUES
        .iter()
        .any(|(_, cues)| cues.recency.iter().any(|form| keyword_hits(lower, form)))
}

/// Whether a value the athlete measured held over a session: a `held` form a
/// few words after a possessive of the same locale — "ta FC est restée",
/// "dein Puls blieb", "your average heart rate stayed".
///
/// Pairing within one locale, and within [`HELD_REACH`] words, is what keeps a
/// homograph from routing another language's sentence: Spanish `tu` is the
/// French subject pronoun, and « tu FC est restée » is no French possessive.
pub(crate) fn states_held_value(lower: &str) -> bool {
    let tokens = tokens_of(lower);
    OWN_RECORD_CUES.iter().any(|(_, cues)| {
        cues.held.iter().any(|form| {
            phrase_starts(&tokens, form).any(|at| {
                tokens
                    .get(at.saturating_sub(HELD_REACH)..at)
                    .is_some_and(|before| before.iter().any(|t| cues.possessives.contains(t)))
            })
        })
    })
}

/// Where the whole-word `phrase` starts in `tokens`.
fn phrase_starts<'a>(tokens: &'a [&str], phrase: &str) -> impl Iterator<Item = usize> + 'a {
    let words: Vec<String> = tokens_of(phrase).into_iter().map(str::to_owned).collect();
    tokens
        .windows(words.len().max(1))
        .enumerate()
        .filter(move |(_, window)| {
            !words.is_empty() && window.iter().zip(&words).all(|(t, w)| t == w)
        })
        .map(|(at, _)| at)
}

/// Whether one of the sentence's clauses states a physiology figure of its own:
/// "You ran 10 km; a `VO2max` of 55 is above average for your age".
///
/// The record verb in the first clause says nothing about the second, and a
/// `VO2max` is not a figure of the session. Such a sentence keeps its scored
/// category, so the physiology claim still reaches the layers that can rule on
/// it. A clause is split at `;`, at a comma followed by a space (never the
/// French decimal in « 6,7 km ») and at a dash; a heart rate or a zone is the
/// session's own measure, so a clause stating one does not count.
fn physiology_in_another_clause(lower: &str) -> bool {
    lower
        .split([';', '—', '–'])
        .flat_map(|part| part.split(", "))
        .any(|clause| {
            clause.chars().any(|c| c.is_ascii_digit())
                && !refers_to_own_record(clause)
                && PHYSIOLOGY
                    .iter()
                    .any(|kw| !SESSION_MEASURES.contains(kw) && keyword_hits(clause, kw))
        })
}

/// Whether a sentence asks rather than states.
///
/// Spanish, German and Portuguese ask with the very verb form they state with
/// ("¿Corriste los 10 km que tocaban ayer?", "Bist du gestern die 10 km
/// gelaufen?"), and French often does ("Tu as couru 10 km hier ?"). A coach
/// asking what the athlete did claims nothing about their record; routed to the
/// athlete-data layer, the figure in the question reads as invented for an
/// athlete with nothing connected.
fn is_question(sentence: &str) -> bool {
    let trimmed = sentence.trim();
    trimmed.ends_with('?') || trimmed.starts_with('\u{bf}')
}

/// Words that open a condition, in the five shipped locales.
///
/// Portuguese `se` is also the everyday reflexive, which is why it counts only
/// where a condition can stand: opening the sentence, or right before a record
/// verb, where a reflexive cannot.
const CONDITIONALS: &[&str] = &["if", "unless", "si", "wenn", "falls", "se", "caso"];

/// Subject pronouns that may stand between a conditional and a verb form that
/// carries none: "si tú corriste", "se tu correste".
const SUBJECT_PRONOUNS: &[&str] = &["tú", "tu", "você", "voce", "du"];

/// Whether a sentence opens with a condition: "If your volume last week was
/// 30 km, rest on Sunday" supposes the figure and claims nothing.
fn opens_with_condition(lower: &str) -> bool {
    tokens_of(lower)
        .first()
        .is_some_and(|first| CONDITIONALS.contains(first))
}

/// Whether the words before position `at` open a condition: a conditional
/// right before it, or one with a subject pronoun between.
fn supposed(tokens: &[&str], at: usize) -> bool {
    let word = |back: usize| at.checked_sub(back).and_then(|i| tokens.get(i));
    let conditional = |back: usize| word(back).is_some_and(|w| CONDITIONALS.contains(w));
    conditional(1) || (word(1).is_some_and(|w| SUBJECT_PRONOUNS.contains(w)) && conditional(2))
}

/// Whether `verb` occurs in the sentence other than as a supposition.
fn states(tokens: &[&str], verb: &str) -> bool {
    let words = tokens_of(verb);
    !words.is_empty()
        && tokens
            .windows(words.len())
            .enumerate()
            .any(|(at, window)| window == words.as_slice() && !supposed(tokens, at))
}

/// How many verbs in the sentence report the athlete's own past training.
///
/// A verb right after a conditional supposes the training instead ("if you
/// ran 30 km this week, take Sunday off"), so it does not count.
///
/// German reports with an auxiliary and a participle on either side of the
/// figure — "du bist diese Woche 42 km gelaufen" — so a German report counts
/// once when a [`GERMAN_PARTICIPLES`] participle and `du` share the sentence,
/// and no `wenn` or `falls` turns it into a supposition. The participle alone
/// is past tense whoever ran: "Kipchoge ist den Marathon in 2 h gelaufen" is
/// about someone else, and routing it here would contradict the figure for an
/// athlete with nothing connected.
fn record_verb_hits(lower: &str) -> usize {
    let tokens = tokens_of(lower);
    let german = tokens.contains(&"du")
        && !tokens.iter().any(|t| matches!(*t, "wenn" | "falls"))
        && GERMAN_PARTICIPLES
            .iter()
            .any(|participle| tokens.contains(participle));
    RECORD_VERBS
        .iter()
        .filter(|verb| states(&tokens, verb))
        .count()
        + usize::from(german)
}

#[cfg(test)]
mod tests {
    use super::{classify_heuristic, FORWARD_MARKERS, OWN_RECORD_CUES, PRESCRIPTION_MARKERS};
    use pierre_memory::ClaimCategory;

    const LOCALES: [&str; 5] = ["en", "fr", "es", "de", "pt"];

    /// Every own-record cue list exists in the five shipped locales, and every
    /// form in it routes a figure to the record on its own. A locale missing a
    /// kind is how French lost « ta dernière » while English kept "your last"
    /// (carnet#1188).
    #[test]
    fn own_record_cues_cover_the_five_locales_and_every_form_routes() {
        let locales: Vec<&str> = OWN_RECORD_CUES.iter().map(|(locale, _)| *locale).collect();
        assert_eq!(
            locales, LOCALES,
            "bug: the own-record cues must cover exactly the shipped locales"
        );

        for (locale, cues) in &OWN_RECORD_CUES {
            assert!(
                !cues.recency.is_empty(),
                "bug: {locale} ships no recency cue, so « your last » cannot route to the record"
            );
            assert!(
                !cues.held.is_empty(),
                "bug: {locale} ships no held cue, so « stayed at » cannot route"
            );
            assert!(
                !cues.possessives.is_empty(),
                "bug: {locale} ships no possessive, so no held cue can route to the record"
            );
            for form in cues.recency {
                let sentence = format!("{form} sortie, 12 km avec une fc moyenne de 140 bpm");
                assert_eq!(
                    classify_heuristic(&sentence),
                    Some(ClaimCategory::AthleteData),
                    "bug: {locale} recency cue {form:?} must route a figure to the record on its own"
                );
            }
            for form in cues.held {
                for possessive in cues.possessives {
                    let sentence = format!("{possessive} fc {form} 133 bpm sur 12 km");
                    assert_eq!(
                        classify_heuristic(&sentence),
                        Some(ClaimCategory::AthleteData),
                        "bug: {locale} held cue {form:?} with {possessive:?} must route a figure to the record"
                    );
                }
            }
        }
    }

    /// A held cue without a possessive says nothing about this athlete.
    #[test]
    fn a_held_value_without_a_possessive_is_not_the_athletes_record() {
        for (locale, cues) in &OWN_RECORD_CUES {
            for form in cues.held {
                let sentence = format!("Lactate {form} near 4 mmol at threshold");
                assert_ne!(
                    classify_heuristic(&sentence),
                    Some(ClaimCategory::AthleteData),
                    "bug: {locale} held cue {form:?} routed to the record without a possessive naming the athlete"
                );
            }
        }
    }

    /// Every marker that keeps a plan out of the record exists in the five
    /// shipped locales, and each one does so on its own. Spanish, German and
    /// Portuguese once had none, so « Tu última tirada antes de la carrera:
    /// 28 km » reached the record with no guard at all (carnet#1188).
    #[test]
    fn plan_markers_cover_the_five_locales_and_every_marker_keeps_a_plan_out() {
        for table in [&PRESCRIPTION_MARKERS, &FORWARD_MARKERS] {
            let locales: Vec<&str> = table.iter().map(|(locale, _)| *locale).collect();
            assert_eq!(
                locales, LOCALES,
                "bug: the plan markers must cover exactly the shipped locales"
            );
        }
        for ((locale, cues), ((_, prescriptions), (_, forwards))) in OWN_RECORD_CUES
            .iter()
            .zip(PRESCRIPTION_MARKERS.iter().zip(FORWARD_MARKERS.iter()))
        {
            assert!(
                !prescriptions.is_empty(),
                "bug: {locale} ships no prescription marker, so a prescribed plan reaches the record"
            );
            assert!(
                !forwards.is_empty(),
                "bug: {locale} ships no forward marker, so a future plan reaches the record"
            );
            let last = cues.recency[0];
            for marker in prescriptions.iter().chain(forwards.iter()) {
                let sentence = format!("{last} sortie {marker} 12 km");
                assert_ne!(
                    classify_heuristic(&sentence),
                    Some(ClaimCategory::AthleteData),
                    "bug: {locale} plan marker {marker:?} must keep a plan out of the record: {sentence}"
                );
            }
            // A prescription marker prescribes whatever else the sentence
            // reports; a forward marker does not, since a held value is past.
            let held = format!(
                "{} fc {} 133 bpm sur 12 km",
                cues.possessives[0], cues.held[0]
            );
            for marker in *prescriptions {
                let sentence = format!("{held} {marker} 10 km");
                assert_ne!(
                    classify_heuristic(&sentence),
                    Some(ClaimCategory::AthleteData),
                    "bug: {locale} prescription marker {marker:?} must keep a held value out of the record: {sentence}"
                );
            }
            for marker in *forwards {
                let sentence = format!("{held} {marker} 10 km");
                assert_eq!(
                    classify_heuristic(&sentence),
                    Some(ClaimCategory::AthleteData),
                    "bug: {locale} forward marker {marker:?} must leave a past held value in the record: {sentence}"
                );
            }
        }
    }

    /// "Mañana" and "Morgen" are tomorrow, except right after the word that
    /// makes them the morning.
    #[test]
    fn the_morning_is_not_tomorrow() {
        for report in [
            "Esta mañana corriste 10 km a buen ritmo",
            "Gestern Morgen bist du 10 km gelaufen",
        ] {
            assert_eq!(
                classify_heuristic(report),
                Some(ClaimCategory::AthleteData),
                "bug: a morning ('mañana'/'Morgen' after its day word) must stay a past report: {report}"
            );
        }
        for plan in [
            "Mañana corriste 10 km, mañana repite",
            "Morgen bist du 10 km gelaufen, sagt der Plan",
        ] {
            assert_ne!(
                classify_heuristic(plan),
                Some(ClaimCategory::AthleteData),
                "bug: a bare 'mañana'/'Morgen' is tomorrow and must keep a plan out of the record: {plan}"
            );
        }
    }
}
