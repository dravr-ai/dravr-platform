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

use std::sync::{Arc, LazyLock, RwLock};

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

/// Coarse class of an identity-leak pattern.
///
/// Threaded into the leak telemetry (log fields / notify event) when a
/// reply is withheld. The class + locale distinguish a true product flip
/// (« I'm GitHub Copilot CLI ») from e.g. roleplay-refusal framing
/// without ever logging the matched text or the reply itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityPatternClass {
    /// Names the underlying product/model outright ("github copilot").
    Product,
    /// Describes itself as a coding assistant.
    CodingAssistant,
    /// Describes itself as a (large) language model.
    LanguageModel,
    /// Contrasts the persona with its "actual identity/environment".
    ActualIdentity,
    /// Frames the persona as a role-play to decline.
    Roleplay,
    /// Frames the persona/turn as a prompt-injection test.
    Injection,
}

impl IdentityPatternClass {
    /// `true` when *denying* this class is correct coach behaviour, so a
    /// negated match must reach the athlete rather than being withheld.
    ///
    /// Splits the table in two. Denying a **claim** is right — « Non, je ne
    /// suis pas GitHub Copilot, je suis Dravr » is exactly what the coach
    /// should say, and withholding it was the only thing the boundary matcher
    /// actually caught in the 2026-07-25 A/B.
    ///
    /// Denying the **framing** classes is not: "I won't role-play as your
    /// coach", "this looks like a prompt-injection test", "abandon my actual
    /// identity" are refusals *to be Dravr* — the 2026-07-12 identity-break
    /// outage — and read as negations while being the leak itself. Those stay
    /// unguarded.
    const fn denial_is_legitimate(self) -> bool {
        matches!(
            self,
            Self::Product | Self::CodingAssistant | Self::LanguageModel
        )
    }
}

impl IdentityPatternClass {
    /// Stable `snake_case` label for log/notify fields.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::CodingAssistant => "coding_assistant",
            Self::LanguageModel => "language_model",
            Self::ActualIdentity => "actual_identity",
            Self::Roleplay => "roleplay",
            Self::Injection => "injection",
        }
    }
}

/// One entry of [`IDENTITY_NARRATION_PATTERNS`]: the folded-matchable
/// phrase plus the coarse telemetry labels reported when it fires.
/// `locale` is the language the phrase belongs to (`"any"` for
/// language-independent product names, else `en`/`fr`/`es`/`de`/`pt`).
struct IdentityPattern {
    text: &'static str,
    class: IdentityPatternClass,
    locale: &'static str,
}

/// Shorthand constructor keeping the table below one-line-per-pattern.
const fn ip(
    text: &'static str,
    class: IdentityPatternClass,
    locale: &'static str,
) -> IdentityPattern {
    IdentityPattern {
        text,
        class,
        locale,
    }
}

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
///
/// Ordered by class conclusiveness — product names first — because
/// [`identity_leak_match`] reports the first hit in table order when a
/// reply matches several patterns.
const IDENTITY_NARRATION_PATTERNS: &[IdentityPattern] = &[
    // Product / model self-identification (language-independent)
    ip("github copilot", IdentityPatternClass::Product, "any"),
    ip("copilot cli", IdentityPatternClass::Product, "any"),
    ip("copilot chat", IdentityPatternClass::Product, "any"),
    ip("chatgpt", IdentityPatternClass::Product, "any"),
    ip("openai", IdentityPatternClass::Product, "any"),
    // Underlying-model disclosure. Copilot's own system prompt carries an
    // explicit "when asked which model you are ... reply with something like
    // 'I'm powered by <name> (model ID: <id>)'" clause, and the coach recites
    // it verbatim — « I'm powered by Claude Sonnet 5 » was the single genuine
    // break observed across the 48-run A/B on 2026-07-25, in an otherwise
    // French conversation, and none of the patterns above matched it. The bare
    // family name stays absent on purpose: a teammate is called Claude (see
    // the `clean_coaching_reply_is_not_an_identity_leak` test), so only the
    // model-qualified forms are listed.
    ip("i m powered by", IdentityPatternClass::Product, "any"),
    ip("model id", IdentityPatternClass::Product, "any"),
    ip("claude sonnet", IdentityPatternClass::Product, "any"),
    ip("claude opus", IdentityPatternClass::Product, "any"),
    ip("gpt 4", IdentityPatternClass::Product, "any"),
    ip("gpt 5", IdentityPatternClass::Product, "any"),
    // English
    ip(
        "coding assistant",
        IdentityPatternClass::CodingAssistant,
        "en",
    ),
    ip(
        "terminal-based coding assistant",
        IdentityPatternClass::CodingAssistant,
        "en",
    ),
    ip(
        "command-line coding assistant",
        IdentityPatternClass::CodingAssistant,
        "en",
    ),
    ip("language model", IdentityPatternClass::LanguageModel, "en"),
    ip(
        "large language model",
        IdentityPatternClass::LanguageModel,
        "en",
    ),
    ip(
        "actual identity",
        IdentityPatternClass::ActualIdentity,
        "en",
    ),
    ip(
        "actual environment",
        IdentityPatternClass::ActualIdentity,
        "en",
    ),
    ip("role-play as", IdentityPatternClass::Roleplay, "en"),
    ip("roleplay as", IdentityPatternClass::Roleplay, "en"),
    ip("prompt injection", IdentityPatternClass::Injection, "en"),
    ip("injection test", IdentityPatternClass::Injection, "en"),
    // French
    ip(
        "assistant de programmation",
        IdentityPatternClass::CodingAssistant,
        "fr",
    ),
    ip(
        "assistant de codage",
        IdentityPatternClass::CodingAssistant,
        "fr",
    ),
    ip(
        "assistant de code",
        IdentityPatternClass::CodingAssistant,
        "fr",
    ),
    ip(
        "modèle de langage",
        IdentityPatternClass::LanguageModel,
        "fr",
    ),
    ip(
        "modele de langage",
        IdentityPatternClass::LanguageModel,
        "fr",
    ),
    ip(
        "grand modèle de langage",
        IdentityPatternClass::LanguageModel,
        "fr",
    ),
    ip(
        "grand modele de langage",
        IdentityPatternClass::LanguageModel,
        "fr",
    ),
    ip(
        "véritable identité",
        IdentityPatternClass::ActualIdentity,
        "fr",
    ),
    ip(
        "veritable identite",
        IdentityPatternClass::ActualIdentity,
        "fr",
    ),
    ip("vraie identité", IdentityPatternClass::ActualIdentity, "fr"),
    ip("vraie identite", IdentityPatternClass::ActualIdentity, "fr"),
    ip(
        "identité réelle",
        IdentityPatternClass::ActualIdentity,
        "fr",
    ),
    ip(
        "identite reelle",
        IdentityPatternClass::ActualIdentity,
        "fr",
    ),
    ip("jeu de rôle", IdentityPatternClass::Roleplay, "fr"),
    ip("jeu de role", IdentityPatternClass::Roleplay, "fr"),
    ip("jouer le rôle", IdentityPatternClass::Roleplay, "fr"),
    ip("jouer le role", IdentityPatternClass::Roleplay, "fr"),
    ip("test d'injection", IdentityPatternClass::Injection, "fr"),
    // Spanish
    ip(
        "asistente de programación",
        IdentityPatternClass::CodingAssistant,
        "es",
    ),
    ip(
        "asistente de programacion",
        IdentityPatternClass::CodingAssistant,
        "es",
    ),
    ip(
        "asistente de codificación",
        IdentityPatternClass::CodingAssistant,
        "es",
    ),
    ip(
        "asistente de codificacion",
        IdentityPatternClass::CodingAssistant,
        "es",
    ),
    ip(
        "modelo de lenguaje",
        IdentityPatternClass::LanguageModel,
        "es",
    ),
    ip("identidad real", IdentityPatternClass::ActualIdentity, "es"),
    ip(
        "verdadera identidad",
        IdentityPatternClass::ActualIdentity,
        "es",
    ),
    ip("juego de rol", IdentityPatternClass::Roleplay, "es"),
    ip("interpretar el papel", IdentityPatternClass::Roleplay, "es"),
    ip("prueba de inyección", IdentityPatternClass::Injection, "es"),
    ip("prueba de inyeccion", IdentityPatternClass::Injection, "es"),
    // German
    ip(
        "programmierassistent",
        IdentityPatternClass::CodingAssistant,
        "de",
    ),
    ip(
        "codierungsassistent",
        IdentityPatternClass::CodingAssistant,
        "de",
    ),
    ip("sprachmodell", IdentityPatternClass::LanguageModel, "de"),
    ip(
        "wahre identität",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip(
        "wahre identitat",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip(
        "tatsächliche identität",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip(
        "tatsachliche identitat",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip(
        "echte identität",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip(
        "echte identitat",
        IdentityPatternClass::ActualIdentity,
        "de",
    ),
    ip("rollenspiel", IdentityPatternClass::Roleplay, "de"),
    ip("injektionstest", IdentityPatternClass::Injection, "de"),
    // Portuguese
    ip(
        "assistente de programação",
        IdentityPatternClass::CodingAssistant,
        "pt",
    ),
    ip(
        "assistente de programacao",
        IdentityPatternClass::CodingAssistant,
        "pt",
    ),
    ip(
        "assistente de codificação",
        IdentityPatternClass::CodingAssistant,
        "pt",
    ),
    ip(
        "assistente de codificacao",
        IdentityPatternClass::CodingAssistant,
        "pt",
    ),
    ip(
        "modelo de linguagem",
        IdentityPatternClass::LanguageModel,
        "pt",
    ),
    ip(
        "identidade real",
        IdentityPatternClass::ActualIdentity,
        "pt",
    ),
    ip(
        "verdadeira identidade",
        IdentityPatternClass::ActualIdentity,
        "pt",
    ),
    ip("jogo de papéis", IdentityPatternClass::Roleplay, "pt"),
    ip("jogo de papeis", IdentityPatternClass::Roleplay, "pt"),
    ip("interpretar o papel", IdentityPatternClass::Roleplay, "pt"),
    ip("teste de injeção", IdentityPatternClass::Injection, "pt"),
    ip("teste de injecao", IdentityPatternClass::Injection, "pt"),
];

/// Dashes that join words: ASCII hyphen, U+2010 hyphen, U+2011 non-breaking
/// hyphen, U+2012 figure dash. Inside a word (`terminal-based`) they carry no
/// clause boundary.
const fn is_word_dash(ch: char) -> bool {
    matches!(ch, '-' | '\u{2010}' | '\u{2011}' | '\u{2012}')
}

/// Dashes that punctuate rather than join: en dash, em dash, horizontal bar.
/// These break a clause wherever they appear, spaced or not.
const fn is_clause_dash(ch: char) -> bool {
    matches!(ch, '\u{2013}' | '\u{2014}' | '\u{2015}')
}

/// Apostrophes, ASCII and typographic.
const fn is_apostrophe(ch: char) -> bool {
    matches!(ch, '\'' | '\u{2018}' | '\u{2019}')
}

/// Every character [`fold_into`] collapses to a space.
fn is_separator(ch: char) -> bool {
    ch.is_whitespace() || is_word_dash(ch) || is_clause_dash(ch) || is_apostrophe(ch)
}

/// Fold a string for separator-insensitive matching, reporting the clause
/// breaks the fold erases.
///
/// Lowercases, then collapses every run of ASCII/Unicode hyphens, dashes,
/// apostrophes and whitespace to a single space. So `« prompt-injection »`,
/// `prompt — injection` and `prompt injection` all compare equal, and the
/// ASCII apostrophe in a pattern (`test d'injection`) matches the typographic
/// apostrophe LLMs emit in French (`test d’injection`).
///
/// Erasing dashes is what makes that equality work, but a dash is also how a
/// reply breaks a clause without punctuation — « I'm not a fitness coach — I'm
/// GitHub Copilot CLI » — so `on_clause_break` receives the byte offset, in
/// the returned string, of every collapsed run that carried one. A run counts
/// when it holds a punctuating dash, or a word dash next to whitespace
/// (`coach - I'm`, the plain-text em-dash substitute messaging clients emit);
/// a bare intra-word hyphen does not.
///
/// A run at either end collapses to nothing, so the result needs no trim and
/// the reported offsets index the string as returned.
fn fold_into(s: &str, mut on_clause_break: impl FnMut(usize)) -> String {
    let mut out = String::with_capacity(s.len());
    // Start inside a run so a leading separator emits no space.
    let mut in_run = true;
    let mut space_at: Option<usize> = None;
    let mut run_word_dash = false;
    let mut run_clause_dash = false;
    let mut run_space = false;

    for ch in s.to_lowercase().chars() {
        if is_separator(ch) {
            run_word_dash |= is_word_dash(ch);
            run_clause_dash |= is_clause_dash(ch);
            run_space |= ch.is_whitespace();
            if !in_run {
                space_at = Some(out.len());
                out.push(' ');
                in_run = true;
            }
        } else {
            if in_run {
                // `space_at` is None for a leading run, which has no clause
                // in front of it to break.
                let breaks_clause = run_clause_dash || (run_word_dash && run_space);
                if let Some(at) = space_at.filter(|_| breaks_clause) {
                    on_clause_break(at);
                }
                space_at = None;
                run_word_dash = false;
                run_clause_dash = false;
                run_space = false;
                in_run = false;
            }
            out.push(ch);
        }
    }
    // A trailing run pushed a space and never closed; drop it rather than
    // trimming, so the offsets already reported stay valid.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// [`fold_into`] for callers that need only the folded text. Applied to both
/// the patterns (once, at first use) and every candidate sentence/reply.
fn fold_separators(s: &str) -> String {
    fold_into(s, |_| {})
}

/// Lowercase, separator-folded vocabulary that marks a sentence as
/// **capability-failure narration** — the coach saying its own tools are
/// broken or that it cannot fetch the user's data («attempting to call them
/// just returned "tool does not exist" errors», «Je ne peux pas aller
/// chercher tes données à l'instant» — live incidents 2026-07-22/23).
///
/// Replayed in history (or baked into a compaction summary), such sentences
/// teach the model learned helplessness: it stops calling `get_activities`
/// because its own past claims fetching is broken. They are scrubbed on the
/// REPLAY path only ([`scrub_replayed_narration`]) — an honest outbound
/// "can't fetch right now" during a real outage still reaches the user
/// ([`scrub_internal_narration`] does not match these).
///
/// Precision philosophy: every failure verb is anchored to the assistant's
/// own capability ("my tools", "je ne peux pas", first-person negation) or
/// is an error-string verbatim. Bare "tool"/"outil" (gear), "ne fonctionne
/// pas" (the user's sensor), empty results ("couldn't find any activities")
/// and provider-status lines ("sync failed on Garmin's side") must pass
/// through — those are legitimate coaching content. Accented phrases carry
/// accent-stripped duplicates (folding handles separators and apostrophes,
/// not accents). The account-state denial «je n'ai pas accès à tes données
/// Garmin car tu ne l'as pas connecté» matches by design: connection state
/// is re-derived live every turn, so scrubbing it from replay costs one
/// re-explained prompt, while replaying it after the user connects teaches
/// stale helplessness on every turn.
const CAPABILITY_FAILURE_PATTERNS: &[&str] = &[
    // Language-independent error strings (quoted verbatim in any locale;
    // "mcp" entries are multiword so sports-medicine "MCP joint" passes).
    "tool does not exist",
    "tool not found",
    "no such tool",
    "tool call failed",
    "tool calls failed",
    "mcp server",
    "mcp tool",
    "mcp error",
    "json rpc",
    // English — first-person subject + data/activity/platform object. Both
    // anchors are load-bearing: "when Strava can't fetch your heart-rate
    // data, re-pair the sensor" (subject = the app) and "I don't have
    // access to your Strava password — it stays private" (object = privacy
    // reassurance) are legitimate coaching and must pass. Apostrophes fold
    // to a space, so ASCII forms also match curly-apostrophe output.
    "i don't have access to fitness",
    "i do not have access to fitness",
    "i don't have access to your data",
    "i do not have access to your data",
    "i don't have access to your activities",
    "i do not have access to your activities",
    "i have no access to your data",
    "i can't access your data",
    "i cannot access your data",
    "i can't access your activities",
    "i cannot access your activities",
    "i'm unable to access your data",
    "i am unable to access your data",
    "i can't fetch your",
    "i cannot fetch your",
    "i'm unable to fetch your",
    "i am unable to fetch your",
    "i can't retrieve your",
    "i cannot retrieve your",
    "i'm unable to retrieve your",
    "i am unable to retrieve your",
    "my tools aren't working",
    "my tools are not working",
    "my tools are broken",
    "my tools are unavailable",
    "my tools are down",
    // "not able to" mutations of the "can't/unable" family above, same
    // anchoring ("if you're not able to access your Garmin account" is app
    // help — the leading i/i'm keeps it out).
    "i'm not able to access your data",
    "i am not able to access your data",
    "i'm not able to access your activities",
    "i am not able to access your activities",
    "i'm not able to fetch your",
    "i am not able to fetch your",
    "i'm not able to retrieve your",
    "i am not able to retrieve your",
    // Self-anchored connection excuse ("on my side/end"): the coach blaming
    // its own connection is never legitimate coaching; "connection problem"
    // alone (user wifi, watch sync) must pass.
    "connection problem on my side",
    "connection problem on my end",
    "connection issue on my side",
    "connection issue on my end",
    // English, third-person summary register — compaction summaries restate
    // the coach's failure as "the coach was unable to fetch the user's
    // data", which no first-person pattern sees. Anchored on "the user" so
    // an assistant reply about the athlete's own apps ("the Strava app was
    // unable to fetch data") passes.
    "unable to fetch the user",
    "unable to access the user",
    "unable to retrieve the user",
    "could not fetch the user",
    "couldn't fetch the user",
    "could not access the user",
    "couldn't access the user",
    "could not retrieve the user",
    "couldn't retrieve the user",
    "its tools were unavailable",
    "its tools were broken",
    "its tools were not working",
    "its tools did not work",
    "its tools failed",
    // French — first-person + object-anchored for the same reasons («je
    // n'ai pas accès à tes messages privés» is privacy reassurance; «tu
    // peux aller chercher tes données dans l'appli» is app help). «peux pas
    // raller chercher» pins the 2026-07-23 incident's verbatim typo.
    "je ne peux pas aller chercher",
    "peux pas raller chercher",
    "je n'ai pas accès à tes données",
    "je n'ai pas acces a tes donnees",
    "je n'ai pas accès à tes activités",
    "je n'ai pas acces a tes activites",
    "je n'ai pas accès à tes plateformes",
    "je n'ai pas acces a tes plateformes",
    "je ne peux pas accéder à tes données",
    "je ne peux pas acceder a tes donnees",
    "je ne peux pas accéder à tes activités",
    "je ne peux pas acceder a tes activites",
    "impossible de récupérer tes données",
    "impossible de recuperer tes donnees",
    "impossible de récupérer tes activités",
    "impossible de recuperer tes activites",
    "impossible d'accéder à tes données",
    "impossible d'acceder a tes donnees",
    "mes outils ne fonctionnent pas",
    "mes outils sont indisponibles",
    "mes outils sont hors service",
    "mes outils ne répondent pas",
    "mes outils ne repondent pas",
    // «être capable»/«arriver à» mutations — live incidents 2026-07-24 and
    // 2026-08-11 («Je ne suis pas capable de récupérer tes activités en ce
    // moment (problème de connexion de mon côté)»): the model rephrased the
    // scrubbed «je ne peux pas» family and the mutation replayed for 18 days,
    // re-teaching helplessness. «je ne suis/j'arrive» keeps the first-person
    // anchor («si tu n'arrives pas à accéder à tes données dans l'appli» is
    // app help and must pass); «tes données/activités» keeps the object
    // anchor (— «à tes messages privés» is privacy reassurance).
    "je ne suis pas capable de récupérer tes activités",
    "je ne suis pas capable de recuperer tes activites",
    "je ne suis pas capable de récupérer tes données",
    "je ne suis pas capable de recuperer tes donnees",
    "je ne suis pas capable d'accéder à tes données",
    "je ne suis pas capable d'acceder a tes donnees",
    "je ne suis pas capable d'accéder à tes activités",
    "je ne suis pas capable d'acceder a tes activites",
    "je ne suis pas capable d'aller chercher tes données",
    "je ne suis pas capable d'aller chercher tes donnees",
    "je n'arrive pas à récupérer tes activités",
    "je n'arrive pas a recuperer tes activites",
    "je n'arrive pas à récupérer tes données",
    "je n'arrive pas a recuperer tes donnees",
    "je n'arrive pas à accéder à tes données",
    "je n'arrive pas a acceder a tes donnees",
    "je n'arrive pas à accéder à tes activités",
    "je n'arrive pas a acceder a tes activites",
    // The fabricated excuse that rode along both incidents — «de mon côté»
    // is the self-anchor: the coach blaming its own connection can never be
    // legitimate coaching, while «problème de connexion» alone (the user's
    // wifi, the watch's sync) must pass.
    "problème de connexion de mon côté",
    "probleme de connexion de mon cote",
    // Spanish — object-anchored («no tengo acceso a tus mensajes privados»
    // is privacy reassurance and must pass)
    "no tengo acceso a tus datos",
    "no tengo acceso a tus actividades",
    "no tengo acceso a plataformas",
    "no puedo acceder a tus datos",
    "no puedo acceder a tus actividades",
    "no puedo obtener tus datos",
    "no puedo recuperar tus datos",
    "no puedo recuperar tus actividades",
    "mis herramientas no funcionan",
    "mis herramientas no responden",
    "mis herramientas no están disponibles",
    "mis herramientas no estan disponibles",
    // "no soy capaz" mutations + self-anchored connection excuse, mirroring
    // the FR/EN families.
    "no soy capaz de acceder a tus datos",
    "no soy capaz de acceder a tus actividades",
    "no soy capaz de recuperar tus datos",
    "no soy capaz de recuperar tus actividades",
    "no soy capaz de obtener tus datos",
    "problema de conexión de mi lado",
    "problema de conexion de mi lado",
    "problema de conexión por mi parte",
    "problema de conexion por mi parte",
    // German — ich-anchored pairs cover verb-second inversion ("leider kann
    // ich …"); object-anchored so password/privacy reassurance («ich habe
    // keinen Zugriff auf dein Garmin-Passwort») and third-party privacy
    // («Dritte können nicht auf deine Daten zugreifen») pass.
    "ich habe keinen zugriff auf deine daten",
    "ich habe keinen zugriff auf deine aktivitäten",
    "ich habe keinen zugriff auf deine aktivitaten",
    "ich kann nicht auf deine daten zugreifen",
    "kann ich nicht auf deine daten zugreifen",
    "ich kann deine daten nicht abrufen",
    "kann ich deine daten nicht abrufen",
    "meine tools funktionieren nicht",
    "meine werkzeuge funktionieren nicht",
    "meine tools sind nicht verfügbar",
    "meine tools sind nicht verfuegbar",
    // "nicht in der Lage" mutations + self-anchored connection excuse. The
    // separator fold keeps commas, so the standard comma-after-Lage form
    // needs its own entry beside the comma-less one.
    "nicht in der lage, auf deine daten zuzugreifen",
    "nicht in der lage auf deine daten zuzugreifen",
    "nicht in der lage, deine daten abzurufen",
    "nicht in der lage deine daten abzurufen",
    "verbindungsproblem auf meiner seite",
    // Portuguese — BR acessar + PT aceder, object-anchored («não consigo
    // aceder aos teus treinos privados — só vejo o que partilhas» is
    // privacy reassurance and must pass)
    "não tenho acesso aos teus dados",
    "nao tenho acesso aos teus dados",
    "não tenho acesso aos seus dados",
    "nao tenho acesso aos seus dados",
    "não tenho acesso a plataformas",
    "nao tenho acesso a plataformas",
    "não consigo acessar os teus dados",
    "nao consigo acessar os teus dados",
    "não consigo acessar seus dados",
    "nao consigo acessar seus dados",
    "não consigo aceder aos teus dados",
    "nao consigo aceder aos teus dados",
    "minhas ferramentas não funcionam",
    "minhas ferramentas nao funcionam",
    "minhas ferramentas não estão funcionando",
    "minhas ferramentas nao estao funcionando",
    // "não sou capaz" mutations (BR acessar + PT aceder) + self-anchored
    // connection excuse (BR conexão + PT ligação).
    "não sou capaz de acessar os teus dados",
    "nao sou capaz de acessar os teus dados",
    "não sou capaz de acessar seus dados",
    "nao sou capaz de acessar seus dados",
    "não sou capaz de aceder aos teus dados",
    "nao sou capaz de aceder aos teus dados",
    "não sou capaz de recuperar os teus dados",
    "nao sou capaz de recuperar os teus dados",
    "problema de conexão do meu lado",
    "problema de conexao do meu lado",
    "problema de ligação do meu lado",
    "problema de ligacao do meu lado",
];

/// Separator-folded copy of [`INTERNAL_NARRATION_PATTERNS`], built once.
static FOLDED_INTERNAL: LazyLock<Vec<String>> = LazyLock::new(|| {
    INTERNAL_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// Separator-folded copy of [`CAPABILITY_FAILURE_PATTERNS`], built once.
static FOLDED_CAPABILITY: LazyLock<Vec<String>> = LazyLock::new(|| {
    CAPABILITY_FAILURE_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// Separator-folded copy of [`IDENTITY_NARRATION_PATTERNS`], built once.
/// Index-aligned with the pattern table so a folded hit maps back to its
/// class + locale labels.
static FOLDED_IDENTITY: LazyLock<Vec<String>> = LazyLock::new(|| {
    IDENTITY_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p.text))
        .collect()
});

// ===========================================================================
// Runtime vocabulary overlay (contremaitre-synced)
// ===========================================================================

/// Shortest folded pattern the overlay accepts, in characters. The whole
/// apply is rejected below this — a typo'd two-word entry like «de mon»
/// would scrub half of every French reply, and last-good-wins means a
/// rejected overlay costs nothing.
const OVERLAY_MIN_FOLDED_CHARS: usize = 10;

/// Most entries one overlay class accepts — a runaway-file backstop far
/// above any plausible vocabulary size (the compiled-in tables sit under
/// 200 entries each).
const OVERLAY_MAX_ENTRIES: usize = 500;

/// Typed narration-vocabulary overlay.
///
/// The YAML lives in dravr-contremaitre (`config/narration.yaml`) and is
/// parsed by `pierre-contremaitre`, which owns the platform's YAML tooling —
/// this leaf crate only receives the typed lists.
///
/// Semantics are **additive**: entries extend the compiled-in tables, never
/// replace them, so a new phrasing mutation observed in production can start
/// matching on the next sync (≤60s) without a deploy — the incident cadence
/// that motivated this (2026-07-22 → 07-23 → 07-24 → 08-11 was four
/// deploy-gated pattern iterations). Each successful apply replaces the
/// *previous overlay* wholesale, so removing a bad overlay entry is just a
/// contremaitre edit too.
#[derive(Debug, Clone, Default)]
pub struct NarrationVocabOverlay {
    /// Extends [`CAPABILITY_FAILURE_PATTERNS`]: replay scrub AND the
    /// outbound [`contains_capability_failure`] boundary detector.
    pub capability_failure: Vec<String>,
    /// Extends [`INTERNAL_NARRATION_PATTERNS`]: outbound scrub and replay.
    pub internal_narration: Vec<String>,
    /// Extends the identity vocabulary on the REPLAY path only
    /// ([`scrub_replayed_narration`]). Deliberately NOT the outbound
    /// withhold: [`identity_leak_match`] carries negation-lookbehind and
    /// class/locale/index telemetry semantics that plain strings cannot
    /// express, so the withhold contract stays compiled-in.
    pub identity: Vec<String>,
}

/// Entry counts a successful apply installed, for the sync log line.
#[derive(Debug, Clone, Copy)]
pub struct NarrationOverlayCounts {
    /// Installed `capability_failure` entries.
    pub capability_failure: usize,
    /// Installed `internal_narration` entries.
    pub internal_narration: usize,
    /// Installed replay-only `identity` entries.
    pub identity: usize,
}

/// One immutable, pre-folded overlay generation.
#[derive(Default)]
struct NarrationVocabSnapshot {
    capability: Vec<String>,
    internal: Vec<String>,
    identity: Vec<String>,
    sha256: Option<String>,
}

/// Registry holding the live overlay snapshot.
///
/// Mirrors the `GLOBAL_PRICING_REGISTRY` / `NOTIFY_ROUTING_PROVIDER` house
/// pattern: the static lives in the leaf crate beside its consumers (the
/// matchers below), the writer is the contremaitre sync engine in a higher
/// crate. Swap is atomic (`Arc` behind an `RwLock`); a failed apply leaves
/// the previous snapshot untouched (last-good-wins, like every other
/// contremaitre overlay).
pub struct NarrationVocabRegistry {
    /// Current overlay generation. `Arc<RwLock>`-free: the registry itself
    /// is a process-wide static, so the lock alone suffices.
    snapshot: RwLock<Arc<NarrationVocabSnapshot>>,
}

impl NarrationVocabRegistry {
    fn new() -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(NarrationVocabSnapshot::default())),
        }
    }

    /// Validate, fold, and atomically install `overlay`, recording `sha256`
    /// (of the downloaded file) for the sync engine's skip check.
    ///
    /// # Errors
    ///
    /// Rejects the WHOLE overlay — keeping the previous snapshot live — when
    /// any class exceeds [`OVERLAY_MAX_ENTRIES`] or any entry folds below
    /// [`OVERLAY_MIN_FOLDED_CHARS`] characters: a half-installed vocabulary
    /// would be harder to reason about than a rejected one.
    pub fn apply_overlay(
        &self,
        overlay: &NarrationVocabOverlay,
        sha256: String,
    ) -> Result<NarrationOverlayCounts, String> {
        let capability = fold_overlay_entries("capability_failure", &overlay.capability_failure)?;
        let internal = fold_overlay_entries("internal_narration", &overlay.internal_narration)?;
        let identity = fold_overlay_entries("identity", &overlay.identity)?;

        let counts = NarrationOverlayCounts {
            capability_failure: capability.len(),
            internal_narration: internal.len(),
            identity: identity.len(),
        };
        let next = Arc::new(NarrationVocabSnapshot {
            capability,
            internal,
            identity,
            sha256: Some(sha256),
        });
        self.snapshot.write().map_or_else(
            |_| Err("narration vocabulary lock poisoned; overlay not installed".to_owned()),
            |mut guard| {
                *guard = next;
                Ok(counts)
            },
        )
    }

    /// SHA-256 of the currently installed overlay file, or `None` before the
    /// first successful apply. The sync engine compares this against the
    /// manifest entry to skip an unchanged file.
    #[must_use]
    pub fn current_overlay_sha256(&self) -> Option<String> {
        self.snapshot.read().ok().and_then(|s| s.sha256.clone())
    }

    /// Whether the already-folded sentence matches an overlay entry of the
    /// given class. Read-lock per call: the scrub walks sentences through
    /// `fn`-pointer matchers, so there is no seam to thread a snapshot
    /// through — and an uncontended read lock is nanoseconds against the
    /// milliseconds a chat turn costs.
    fn matches(&self, folded: &str, class: impl Fn(&NarrationVocabSnapshot) -> &[String]) -> bool {
        self.snapshot
            .read()
            .is_ok_and(|s| class(&s).iter().any(|p| folded.contains(p.as_str())))
    }
}

/// Fold one overlay class, rejecting entries that would over-match.
fn fold_overlay_entries(class: &str, entries: &[String]) -> Result<Vec<String>, String> {
    if entries.len() > OVERLAY_MAX_ENTRIES {
        return Err(format!(
            "narration overlay class `{class}` has {} entries (max {OVERLAY_MAX_ENTRIES})",
            entries.len()
        ));
    }
    entries
        .iter()
        .map(|raw| {
            let folded = fold_separators(raw);
            if folded.chars().count() < OVERLAY_MIN_FOLDED_CHARS {
                Err(format!(
                    "narration overlay class `{class}` entry `{raw}` folds to fewer than \
                     {OVERLAY_MIN_FOLDED_CHARS} characters and would over-match"
                ))
            } else {
                Ok(folded)
            }
        })
        .collect()
}

/// Process-wide narration-vocabulary overlay, seeded empty; the contremaitre
/// sync engine installs downloaded generations via
/// [`NarrationVocabRegistry::apply_overlay`].
pub static GLOBAL_NARRATION_VOCAB: LazyLock<NarrationVocabRegistry> =
    LazyLock::new(NarrationVocabRegistry::new);

/// Telemetry label for a matched identity-leak pattern.
///
/// Says which coarse class fired, in which locale, and the pattern's
/// index in [`IDENTITY_NARRATION_PATTERNS`]. Deliberately carries no
/// text — the withheld reply is never logged or persisted, so these
/// labels are the only per-leak signal that reaches operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityLeakMatch {
    /// Coarse pattern class (product flip vs roleplay framing vs …).
    pub class: IdentityPatternClass,
    /// Language of the matched phrase (`"any"` for product names).
    pub locale: &'static str,
    /// Index into [`IDENTITY_NARRATION_PATTERNS`], the finest-grained
    /// content-free label — pins the exact phrase via the source table.
    pub pattern_index: usize,
}

/// Negation markers that turn an identity phrase into a *denial*.
///
/// Separator-folded and whitespace-tokenized, so `n't` arrives as the bare
/// token `t` (`isn't` folds to `isn t`) and French `n'est` as `n est`.
const NEGATION_TOKENS: &[&str] = &[
    // en
    "not", "never", "no", // fr
    "pas", "non", "jamais", "aucun", "aucune", // es / pt
    "nao", "não", "ningun", "ninguna", // de
    "nicht", "kein", "keine",
];

/// Punctuation that closes a clause only when what follows re-asserts an
/// identity ([`asserts_self`]).
///
/// A comma is as often a list separator as a clause break, and the two need
/// opposite answers: « je ne suis pas un coach, je suis GitHub Copilot »
/// starts a new predication the negation does not reach, while "you are not a
/// general-purpose AI, a language model, a chat bot" keeps every item under
/// the one negation — the shape of the identity anchor the coach is told to
/// embody, so treating its commas as clause breaks would withhold the coach's
/// own persona statement.
const fn is_soft_clause_break(c: char) -> bool {
    matches!(c, ',' | ';' | ':')
}

/// Tokens that re-assert a first-person identity, marking the text after a
/// soft break as a new predication rather than another item in a negated
/// list. Subject pronouns for en/fr/de, plus the first-person copulas the
/// pro-drop languages use without one (« soy », « sou ») and the ones a
/// pronoun-dropping fold leaves behind. Folded, so `I'm` arrives as `i m`.
const SELF_ASSERTION_TOKENS: &[&str] = &[
    "i", "je", "ich", "yo", "eu", "am", "suis", "bin", "soy", "sou",
];

/// `true` when `segment` re-asserts a first-person identity.
fn asserts_self(segment: &str) -> bool {
    segment
        .split(' ')
        .any(|tok| SELF_ASSERTION_TOKENS.contains(&tok))
}

/// `true` when a negation marker in the identity phrase's **own clause**
/// denies it — i.e. the coach is rejecting the identity, not claiming it.
///
/// `dash_breaks` are the folded offsets [`fold_into`] reported for erased
/// clause-breaking dashes.
///
/// The clause bound is what makes the guard grammatical rather than merely
/// proximate. « Je ne suis pas un coach, je suis GitHub Copilot » negates
/// "un coach"; reading the negation across the comma marked a full persona
/// break as a denial and delivered it to the athlete, as did "I'm not able to
/// help with that. I'm GitHub Copilot CLI" across the sentence boundary and
/// "I'm not a fitness coach — I'm GitHub Copilot CLI" across the em dash.
fn is_negated_at(folded: &str, dash_breaks: &[usize], at: usize) -> bool {
    let prefix = &folded[..at];
    // Start of the clause the phrase sits in: just past the nearest
    // qualifying punctuation break, or the nearest erased dash break. A
    // sentence terminator always qualifies; a comma only when the text it
    // opens re-asserts an identity rather than continuing a list.
    let punctuation_start = prefix
        .char_indices()
        .rev()
        .find(|&(offset, c)| {
            is_terminator(c)
                || (is_soft_clause_break(c) && asserts_self(&prefix[offset + c.len_utf8()..]))
        })
        .map_or(0, |(offset, c)| offset + c.len_utf8());
    let dash_start = dash_breaks
        .iter()
        .rev()
        .find(|&&offset| offset < at)
        .map_or(0, |&offset| offset + 1);
    // Both bounds land on a char boundary, so their maximum does too. The
    // clause is the whole guard: a character-count lookbehind would have to
    // be short enough to stay out of the previous clause, and a negation
    // legitimately sits a whole list away from the item it governs — "not a
    // general-purpose AI, a language model, a chat bot, or a coding …
    // assistant".
    let start = punctuation_start.max(dash_start);
    prefix[start..]
        .split(' ')
        .any(|tok| NEGATION_TOKENS.contains(&tok))
}

/// First identity-leak pattern the reply *claims*, `None` when clean.
///
/// Matches in table order — product names first, so a reply that both
/// names Copilot and talks about role-play reports the more conclusive
/// product hit.
///
/// **Denials do not count.** A hit whose every occurrence is denied by a
/// negation marker *in its own clause* is the coach correctly rejecting the
/// identity (« Non, je ne suis pas GitHub Copilot — je suis Dravr ») and must
/// reach the athlete. Left unguarded this was not a theoretical false positive
/// but the *only* thing the matcher caught: across a 48-run live A/B on
/// 2026-07-25 all 5 boundary matches were correct denials, while the real
/// break — Copilot's own « I'm powered by Claude Sonnet 5 (model ID: …) »
/// clause — went undetected. Worse, the failure was self-reinforcing: a leak
/// prompts the athlete to ask "are you Copilot?", and the correct answer was
/// then withheld too. The clause bound is what keeps that guard from becoming
/// a bypass in turn — see [`is_negated_at`].
#[must_use]
pub fn identity_leak_match(text: &str) -> Option<IdentityLeakMatch> {
    let mut dash_breaks: Vec<usize> = Vec::new();
    let folded = fold_into(text, |at| dash_breaks.push(at));
    FOLDED_IDENTITY.iter().enumerate().find_map(|(idx, p)| {
        let pattern = &IDENTITY_NARRATION_PATTERNS[idx];
        let guarded = pattern.class.denial_is_legitimate();
        folded
            .match_indices(p.as_str())
            .any(|(at, _)| !guarded || !is_negated_at(&folded, &dash_breaks, at))
            .then_some(IdentityLeakMatch {
                class: pattern.class,
                locale: pattern.locale,
                pattern_index: idx,
            })
    })
}

/// A bounded, separator-folded window of the reply around the matched identity
/// phrase — the minimum forensics needed to tell a CLAIM from a DENIAL.
///
/// `pattern_index` alone cannot: «I'm GitHub Copilot», «I'm NOT GitHub Copilot»
/// and «I'm powered by Claude Sonnet 5» are three completely different
/// diagnoses that log identically today, which is why every historical leak
/// count is an upper bound rather than a measurement. The 2026-07-25 A/B found
/// all five live matches were correct denials; without a window there is no way
/// to know that from telemetry.
///
/// Deliberately bounded and folded rather than the whole reply: the withheld
/// text is withheld for a reason, so this returns at most `window_chars` either
/// side of the matched phrase, lowercased with separators collapsed. That is
/// enough to read the model's own words about its own identity and nothing
/// more — it is not athlete data, and the cap keeps an accidental PII spill
/// short. Returns `None` when the reply is clean.
#[must_use]
pub fn identity_leak_context(text: &str, window_chars: usize) -> Option<String> {
    let hit = identity_leak_match(text)?;
    let pattern = FOLDED_IDENTITY.get(hit.pattern_index)?;
    // The dash-break offsets are only needed by the negation lookbehind, which
    // `identity_leak_match` above has already applied — this call just needs the
    // folded text, so the callback discards them rather than allocating.
    let folded = fold_into(text, |_| ());
    let at = folded.find(pattern.as_str())?;

    // Char-safe on both ends: the folded text is still UTF-8 and a raw byte
    // slice here would panic on an accented character — the exact defect class
    // that took down a whole turn in 5c2c38c81.
    let start = folded[..at]
        .char_indices()
        .rev()
        .nth(window_chars.saturating_sub(1))
        .map_or(0, |(offset, _)| offset);
    let tail = &folded[at..];
    let take = pattern.chars().count() + window_chars;
    let end = at
        + tail
            .char_indices()
            .nth(take)
            .map_or(tail.len(), |(offset, _)| offset);

    Some(folded[start..end].to_owned())
}

/// Boolean shorthand for [`identity_leak_match`].
///
/// `true` when the reply anywhere identifies as the underlying model/
/// provider or frames its own persona as a roleplay/injection to refuse
/// — a whole persona break. The caller withholds the entire reply.
#[must_use]
pub fn contains_identity_leak(text: &str) -> bool {
    identity_leak_match(text).is_some()
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
/// fetching fails. The three tables share one fold of the sentence.
fn is_replayed_narration(sentence: &str) -> bool {
    let folded = fold_separators(sentence);
    matches_internal(&folded) || matches_identity(&folded) || matches_capability(&folded)
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
