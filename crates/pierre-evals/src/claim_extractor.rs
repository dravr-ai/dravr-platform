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

use pierre_memory::ClaimCategory;
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedClaim {
    /// The raw claim text.
    pub text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Where the claim text came from.
    pub source: ClaimSource,
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

fn word_count(s: &str) -> usize {
    s.split_whitespace().filter(|w| !w.is_empty()).count()
}

/// Split an agent reply into sentences on `.`/`!`/`?` boundaries.
///
/// Used by [`extract_heuristic`]; exposed so the heuristic building blocks
/// can be exercised directly.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        let ch = chars[i];
        current.push(ch);
        // A line break is a unit boundary. Without this, a newline-delimited
        // list with no terminal punctuation (e.g. a weekly training plan) is
        // swallowed into one giant multi-line "claim".
        if ch == '\n' {
            finalize_sentence(&mut current, &mut sentences);
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
    let mut best: Option<(ClaimCategory, usize)> = None;

    let buckets: [(ClaimCategory, &[&str]); 7] = [
        (
            ClaimCategory::Physiological,
            &[
                "vo2max",
                "vo2 max",
                "heart rate",
                "hr max",
                "hrmax",
                "lactate",
                "threshold",
                "substrate",
                "zone",
                "ftp",
                // Critical-power family in the five shipped locales. The
                // prime symbols are left out: tokenized, "W′" is the bare
                // "w" every wattage carries.
                "critical power",
                "critical speed",
                "w prime",
                "d prime",
                "puissance critique",
                "vitesse critique",
                "potencia crítica",
                "velocidad crítica",
                "kritische leistung",
                "kritische geschwindigkeit",
                "potência crítica",
                "velocidade crítica",
                // French
                "fc",
                "fc moy",
                "fc max",
                "fréquence cardiaque",
                "frequence cardiaque",
                "bpm",
                "seuil",
                // Resting heart rate, which the personalized layer checks
                // against the athlete's synced mornings (carnet#853). "Heart
                // rate" and "fc" already catch it in English and French.
                "resting hr",
                "fc en reposo",
                "frecuencia cardíaca en reposo",
                "ruhepuls",
                "ruheherzfrequenz",
                "fc em repouso",
                "frequência cardíaca em repouso",
                // Form, which the personalized layer checks as a share of the
                // athlete's own fitness and a band (carnet#850): the labels and
                // possessives a reply puts on it, the fitness it is a share of,
                // and the band only a form claim names.
                "tsb",
                "training stress balance",
                "ctl",
                "current form",
                "your form",
                "of your fitness",
                "deep fatigue",
                "forme actuelle",
                "ta forme",
                "votre forme",
                "indice de forme",
                "indice de fatigue",
                "condition physique",
                "fatigue profonde",
                "forma actual",
                "tu forma",
                "su forma",
                "condición física",
                "fatiga profunda",
                "aktuelle form",
                "deine form",
                "ihre form",
                "deiner fitness",
                "formwert",
                "tiefe ermüdung",
                "forma atual",
                "tua forma",
                "sua forma",
                "condição física",
                "fadiga profunda",
            ],
        ),
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
/// "This week" and "cette semaine" point both ways: "this week, aim for 40 km"
/// plans it, "you ran 42 km this week" reports it. They mark a prescription
/// only when no verb [`record_verb_hits`] reads reports past training — otherwise the
/// weekly total, the figure a coach writes most often, never reached the
/// athlete-data layer that adds the week up (carnet#846).
fn is_prescriptive(lower: &str) -> bool {
    const CURRENT_PERIOD: [&str; 2] = ["this week", "cette semaine"];
    const MARKERS: [&str; 24] = [
        // Modal / intent
        "aim for",
        "let's",
        "lets",
        "we'll",
        "you'll",
        "should",
        "try to",
        "plan to",
        "target",
        "recommend",
        "suggest",
        "go for",
        "start with",
        "focus on",
        // Future time
        "tomorrow",
        "next week",
        // French
        "vise",
        "on va",
        "tu devrais",
        "la semaine prochaine",
        "demain",
        "je te propose",
        "commence par",
        "essaie",
    ];
    MARKERS.iter().any(|m| keyword_hits(lower, m))
        || (CURRENT_PERIOD.iter().any(|m| keyword_hits(lower, m)) && record_verb_hits(lower) == 0)
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

/// Second-person past-tense verbs that report the athlete's own training, in
/// the five shipped locales.
///
/// The past tense is what separates a record from a plan: "you ran 42 km"
/// happened, "run 42 km" has not. That is why Spanish, German and Portuguese
/// enter the athlete-data bucket through these verbs alone, with no time words:
/// a time word ("esta semana") is as at home in a plan as in a report, and
/// those locales carry no prescription markers to tell the two apart.
///
/// Every form is second person, so a sentence about another athlete's result
/// is not read as one about this athlete. German is read separately, through
/// [`GERMAN_PARTICIPLES`].
const RECORD_VERBS: &[&str] = &[
    // English
    "you ran",
    "you rode",
    "you swam",
    "you covered",
    "you logged",
    "you completed",
    "you clocked",
    "you averaged",
    "you've run",
    "you have run",
    "you've ridden",
    "you've covered",
    "you've logged",
    // French
    "tu as couru",
    "tu as roulé",
    "tu as roule",
    "tu as nagé",
    "tu as nage",
    "tu as parcouru",
    "tu as cumulé",
    "tu as cumule",
    "tu as bouclé",
    "tu as boucle",
    "vous avez couru",
    "vous avez roulé",
    "vous avez roule",
    "vous avez parcouru",
    // Spanish
    "corriste",
    "rodaste",
    "pedaleaste",
    "nadaste",
    "recorriste",
    "acumulaste",
    "completaste",
    "has corrido",
    "has rodado",
    "has pedaleado",
    "has nadado",
    "has recorrido",
    "has acumulado",
    // Portuguese
    "correste",
    "pedalaste",
    "percorreste",
    "você correu",
    "voce correu",
    "você pedalou",
    "voce pedalou",
    "você nadou",
    "voce nadou",
    "você percorreu",
    "voce percorreu",
    "você acumulou",
    "voce acumulou",
];

/// German participles that report training, read with `du` by
/// [`record_verb_hits`].
const GERMAN_PARTICIPLES: &[&str] = &[
    "gelaufen",
    "gefahren",
    "geradelt",
    "geschwommen",
    "zurückgelegt",
    "absolviert",
];
