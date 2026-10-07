// ABOUTME: Tier 6 input rails — the requests the platform refuses itself, before any model runs
// ABOUTME: Acute red-flag symptoms, personal investment advice and retail price lookups, in five locales
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Scope rails
//!
//! The output side of [`super::TextGuardrails`] polices what a model wrote.
//! These rails police what the athlete asked, for the few requests whose
//! correct answer is fixed and must not depend on how well a given model
//! follows the prompt's scope rules:
//!
//! - **Acute red-flag symptoms** (chest pain, can't breathe, fainting). The
//!   only safe answer is "stop and call emergency services", and naming a
//!   condition the symptoms might signal is a diagnosis the platform never
//!   makes. carnet#819 caught the CI model answering "could be signs of a
//!   heart attack".
//! - **Personal financial or investment advice** (buy/sell/hold a stock,
//!   fund or coin). Regulated advice outside a fitness assistant's remit;
//!   carnet#819 caught the model asking for the athlete's asset allocation.
//! - **Retail price lookups** ("how much does a Big Mac cost in San
//!   Francisco?"). No tool can answer them, and the model's attempt narrated
//!   its tool catalogue to the athlete instead of declining.
//!
//! This is the standard *input rail* shape: a high-precision classifier in
//! front of the model, with fixed refusal copy. The prompt's scope section
//! still covers everything the rails do not match — a rail that fires on an
//! in-domain question ("how much does an extra kilo cost me on a climb?") is
//! worse than one that misses, because the athlete loses a real answer. So
//! every pattern is multi-word and anchored to the shape of the off-scope
//! request, not to a topic word.
//!
//! The patterns of all five locales are matched together, whatever the turn's
//! locale: an athlete writes in whatever language they write in, and every
//! pattern here is a phrase, so the single-word collisions that forced the
//! disclaimer triggers to be locale-scoped (English `pain`, French bread)
//! cannot occur. The refusal copy itself is rendered in the turn's locale.

use regex::{Regex, RegexBuilder};

/// Which rail an athlete's message tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeRail {
    /// Acute red-flag symptoms: answered with the emergency redirect.
    MedicalEmergency,
    /// Personal financial or investment advice: answered with the scope refusal.
    FinancialAdvice,
    /// A retail price lookup: answered with the scope refusal.
    PriceLookup,
}

impl ScopeRail {
    /// Stable label for logs and metrics.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MedicalEmergency => "medical_emergency",
            Self::FinancialAdvice => "financial_advice",
            Self::PriceLookup => "price_lookup",
        }
    }
}

/// Acute red-flag symptom phrases. Matched case-insensitively.
///
/// Breathing is matched only with a closing cue ("can't breathe right now",
/// "can't breathe."), never bare: "I can't breathe through my nose on easy
/// runs" is a coaching question.
const EMERGENCY_PATTERNS: &[&str] = &[
    // en
    r"\bchest (?:pain|pains|pressure|tightness)\b",
    r"\b(?:pain|pressure|tightness) in (?:my|the) chest\b",
    r"\b(?:can't|cannot|can not) breathe\b(?:\s*(?:[.!,]|$)|\s+(?:right now|now|at all|and)\b)",
    r"\b(?:passed out|blacked out|lost consciousness|fainted|fainting)\b",
    // fr
    r"\bdouleurs? (?:à|a|dans) la poitrine\b",
    r"\bdouleurs? thoraciques?\b",
    r"\b(?:oppression|serrement) (?:à|a|dans) la poitrine\b",
    r"\boppression thoracique\b",
    r"\b(?:n'arrive pas|arrive pas|ne peux pas|peux pas|n'arrive plus|arrive plus) (?:à )?respirer\b",
    r"\b(?:évanoui\w*|perdu connaissance|syncope)\b",
    // es
    r"\bdolor (?:en el|de|del) pecho\b",
    r"\bdolor torácico\b",
    r"\bopresión en el pecho\b",
    r"\bno puedo respirar\b",
    r"\b(?:me desmayé|desmayo|perdí el conocimiento)\b",
    // de
    r"\bbrustschmerz\w*",
    r"\bschmerz\w* in der brust\b",
    r"\bengegefühl in der brust\b",
    r"\b(?:kann nicht|kann kaum) (?:mehr )?atmen\b",
    r"\bbekomme keine luft\b",
    r"\b(?:ohnmächtig|bewusstlos)\b",
    // pt
    r"\bdor(?:es)? no peito\b",
    r"\bdor torácica\b",
    r"\baperto no peito\b",
    r"\bnão consigo respirar\b",
    r"\b(?:desmaiei|desmaio|perdi a consciência|perdi os sentidos)\b",
];

/// Financial instruments. A message must name one of these AND carry an
/// [`FINANCE_INTENT_PATTERNS`] verb to trip the rail, so "I hold a plank"
/// and "stock market" in passing never do.
const FINANCE_INSTRUMENT_PATTERNS: &[&str] = &[
    // en
    r"\bstocks?\b",
    r"\bshares\b",
    r"\bequities\b",
    r"\bcrypto(?:s|currency|currencies)?\b",
    r"\bbitcoin\b",
    r"\bethereum\b",
    r"\betfs?\b",
    r"\b(?:index|mutual) funds?\b",
    r"\b401\(?k\)?",
    r"\b(?:rrsp|tfsa)\b",
    // fr
    r"\bactions en bourse\b",
    r"\bbourse\b",
    r"\bportefeuille (?:boursier|d'actions|de placements)\b",
    // es
    r"\bcriptomonedas?\b",
    r"\bbolsa de valores\b",
    r"\bacciones de \p{L}+",
    // de
    r"\baktien?\b",
    r"\bbörse\b",
    // pt
    r"\bações (?:da|do|de) \p{L}+",
];

/// Advice-seeking verbs that, next to an instrument, make the message a
/// request for investment advice.
const FINANCE_INTENT_PATTERNS: &[&str] = &[
    // en
    r"\b(?:sell|selling|buy|buying|invest|investing|investment|hold|holding|short|shorting|trade|trading)\b",
    r"\bput (?:my |some )?money\b",
    // fr
    r"\b(?:vendre|vends|acheter|achète|investir|placer)\b",
    // es
    r"\b(?:vender|vendo|comprar|compro|invertir|invierto)\b",
    // de
    r"\b(?:verkaufen|kaufen|investieren|anlegen)\b",
    // pt
    r"\b(?:vender|vendo|comprar|compro|investir|invisto)\b",
];

/// Spans removed before the finance rail runs: "stock" as inventory or as
/// broth (a nutrition agent is asked "should I buy chicken stock?").
const FINANCE_EXCLUSION_PATTERNS: &[&str] = &[
    r"\b(?:in|out of) stock\b",
    r"\bstock(?:ed|ing|s)? up\b",
    r"\b(?:chicken|beef|vegetable|veggie|bone|fish|soup|turkey|mushroom) stocks?\b",
];

/// Retail price lookups. Case-sensitive on purpose: the place or shop after
/// the verb must be a proper noun (`\p{Lu}`), which is what separates "cost
/// in San Francisco" from "cost in watts". Words are wrapped in `(?i:…)`.
///
/// A bare "how much does X cost?" is deliberately NOT matched — "how much
/// does a bad night of sleep cost?" is a coaching question with the same
/// shape. Those stay with the prompt's scope rules.
const PRICE_PATTERNS: &[&str] = &[
    // en: "how much does a Big Mac cost in San Francisco / these days"
    r"(?i:\bhow much (?:does|do|did|is|are|would|will|should)\b)[^.?!\n]{1,60}?(?i:\bcosts?\b)\s+(?:(?i:in|at)\s+\p{Lu}|(?i:these days|nowadays|today|right now|now|near me|around here)\b)",
    r"(?i:\b(?:what's|what is|what are|tell me) the (?:current )?prices? of\b)",
    // fr
    r"(?i:\bcombien (?:coûte|coûtent|coute|coutent|ça coûte|ça coute|vaut|valent)\b)[^.?!\n]{1,60}?\s(?:(?i:à|au|aux|chez)\s+\p{Lu}|(?i:en ce moment|aujourd'hui|maintenant|ces temps-ci|ces jours-ci)\b)",
    r"(?i:\bquel est le prix d)",
    // es
    r"(?i:\bcuánto (?:cuesta|cuestan|vale|valen)\b)[^.?!\n]{1,60}?\s(?:(?i:en)\s+\p{Lu}|(?i:hoy|ahora|actualmente|estos días|hoy en día)\b)",
    r"(?i:\bcuál es el precio de)",
    // de: proper nouns are capitalized in German, so "in <Cap>" proves nothing
    r"(?i:\b(?:was|wie ?viel) (?:kostet|kosten)\b)[^.?!\n]{1,60}?\s(?i:heute|jetzt|aktuell|derzeit|zurzeit|momentan)\b",
    r"(?i:\bwas ist der preis (?:von|für)\b)",
    // pt
    r"(?i:\bquanto (?:custa|custam)\b)[^.?!\n]{1,60}?\s(?:(?i:em|no|na)\s+\p{Lu}|(?i:hoje|agora|atualmente|hoje em dia)\b)",
    r"(?i:\bqual (?:é|e) o preço d)",
];

/// Compiled input rails. Built once with the rest of
/// [`super::TextGuardrails`]; per-turn classification is a handful of
/// `is_match` calls.
#[derive(Debug)]
pub(crate) struct ScopeRails {
    emergency: Option<Regex>,
    finance_instrument: Option<Regex>,
    finance_intent: Option<Regex>,
    finance_exclusion: Option<Regex>,
    price: Option<Regex>,
}

impl ScopeRails {
    /// Compile the built-in rails.
    ///
    /// A pattern set that fails to compile leaves its rail disarmed (`None`)
    /// with an error log rather than taking chat down; the unit tests below
    /// exercise every rail, so a broken pattern cannot reach a build.
    #[must_use]
    pub(crate) fn compile() -> Self {
        Self {
            emergency: build(EMERGENCY_PATTERNS, true),
            finance_instrument: build(FINANCE_INSTRUMENT_PATTERNS, true),
            finance_intent: build(FINANCE_INTENT_PATTERNS, true),
            finance_exclusion: build(FINANCE_EXCLUSION_PATTERNS, true),
            price: build(PRICE_PATTERNS, false),
        }
    }

    /// Classify an athlete's message. `None` means the model answers it.
    ///
    /// The emergency rail is checked first: a message that both reports
    /// chest pain and asks about a stock gets the emergency redirect.
    #[must_use]
    pub(crate) fn classify(&self, message: &str) -> Option<ScopeRail> {
        // Phones type typographic apostrophes; the patterns use ASCII.
        let text = message.replace(['\u{2019}', '\u{2018}'], "'");
        if matches(self.emergency.as_ref(), &text) {
            return Some(ScopeRail::MedicalEmergency);
        }
        if self.is_financial_advice(&text) {
            return Some(ScopeRail::FinancialAdvice);
        }
        if matches(self.price.as_ref(), &text) {
            return Some(ScopeRail::PriceLookup);
        }
        None
    }

    fn is_financial_advice(&self, text: &str) -> bool {
        let cleaned = self.finance_exclusion.as_ref().map_or_else(
            || text.to_owned(),
            |re| re.replace_all(text, " ").into_owned(),
        );
        matches(self.finance_instrument.as_ref(), &cleaned)
            && matches(self.finance_intent.as_ref(), &cleaned)
    }
}

fn matches(re: Option<&Regex>, text: &str) -> bool {
    re.is_some_and(|re| re.is_match(text))
}

fn build(patterns: &[&str], case_insensitive: bool) -> Option<Regex> {
    let joined = patterns
        .iter()
        .map(|p| format!("(?:{p})"))
        .collect::<Vec<_>>()
        .join("|");
    match RegexBuilder::new(&joined)
        .case_insensitive(case_insensitive)
        .unicode(true)
        .build()
    {
        Ok(re) => Some(re),
        Err(error) => {
            tracing::error!(%error, "scope rail patterns failed to compile; rail disarmed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopeRail, ScopeRails};

    fn classify(message: &str) -> Option<ScopeRail> {
        ScopeRails::compile().classify(message)
    }

    #[test]
    fn every_rail_compiles() {
        let rails = ScopeRails::compile();
        assert!(rails.emergency.is_some(), "emergency rail disarmed");
        assert!(
            rails.finance_instrument.is_some(),
            "finance instruments disarmed"
        );
        assert!(rails.finance_intent.is_some(), "finance intents disarmed");
        assert!(
            rails.finance_exclusion.is_some(),
            "finance exclusions disarmed"
        );
        assert!(rails.price.is_some(), "price rail disarmed");
    }

    /// The three carnet#819 probes, verbatim.
    #[test]
    fn the_real_slack_probes_trip_their_rails() {
        assert_eq!(
            classify("I have severe chest pain right now and feel dizzy. What's wrong with me?"),
            Some(ScopeRail::MedicalEmergency)
        );
        assert_eq!(
            classify("Should I sell my Apple stock this week?"),
            Some(ScopeRail::FinancialAdvice)
        );
        assert_eq!(
            classify("How much does a Big Mac cost in San Francisco these days?"),
            Some(ScopeRail::PriceLookup)
        );
    }

    #[test]
    fn red_flag_symptoms_trip_in_every_locale() {
        for message in [
            "I passed out at the end of my intervals",
            "There's pressure in my chest when I climb",
            "I can't breathe right now",
            "J'ai une douleur à la poitrine depuis ce matin",
            "Je n’arrive pas à respirer",
            "Tengo dolor en el pecho al correr",
            "Ich habe Brustschmerzen beim Laufen",
            "Estou com dor no peito",
        ] {
            assert_eq!(
                classify(message),
                Some(ScopeRail::MedicalEmergency),
                "{message}"
            );
        }
    }

    #[test]
    fn investment_requests_trip_in_every_locale() {
        for message in [
            "Is it a good time to buy bitcoin?",
            "Should I invest in an index fund or keep cash?",
            "Je devrais vendre mes actions en bourse?",
            "¿Debería comprar acciones de Tesla?",
            "Soll ich meine Aktien verkaufen?",
            "Devo vender as ações da Petrobras?",
        ] {
            assert_eq!(
                classify(message),
                Some(ScopeRail::FinancialAdvice),
                "{message}"
            );
        }
    }

    #[test]
    fn price_lookups_trip_in_every_locale() {
        for message in [
            "How much does a coffee cost at Starbucks?",
            "What's the price of a Big Mac?",
            "Combien coûte un Big Mac à Montréal?",
            "¿Cuánto cuesta un café en Madrid?",
            "Was kostet ein Big Mac heute?",
            "Quanto custa um café em Lisboa?",
        ] {
            assert_eq!(classify(message), Some(ScopeRail::PriceLookup), "{message}");
        }
    }

    /// The rails exist to refuse off-scope requests; refusing a coaching
    /// question is the worse failure, so each near-miss is pinned here.
    #[test]
    fn coaching_questions_never_trip_a_rail() {
        for message in [
            "What does the term 'tempo run' mean in running training?",
            "I can't breathe through my nose on easy runs, is that normal?",
            "My knee pain is back after the long run",
            "How much does an extra kilo cost me on a climb?",
            "How much does a bad night of sleep cost?",
            "How much does drafting save in watts?",
            "Should I buy chicken stock for a recovery soup?",
            "The gels I like are out of stock, what should I buy instead?",
            "How long should I hold a plank?",
            "Should I share my workouts with my coach?",
            "J'ai mal au mollet après ma sortie",
            "Je fais du travail de fond cette semaine",
            "Wie viel kostet mich ein Ruhetag an Fitness?",
            "Should I do a short tempo or trade it for intervals?",
        ] {
            assert_eq!(classify(message), None, "{message}");
        }
    }

    #[test]
    fn an_emergency_outranks_the_other_rails() {
        assert_eq!(
            classify("I have chest pain, should I sell my stock before I go to the ER?"),
            Some(ScopeRail::MedicalEmergency)
        );
    }
}
