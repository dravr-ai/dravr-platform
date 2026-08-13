// ABOUTME: Identity-leak pattern vocabulary and its class taxonomy
// ABOUTME: Split out of narration/mod.rs so each file stays legible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
    pub(crate) const fn denial_is_legitimate(self) -> bool {
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
pub(super) struct IdentityPattern {
    pub(super) text: &'static str,
    pub(crate) class: IdentityPatternClass,
    pub(crate) locale: &'static str,
}

/// Shorthand constructor keeping the table below one-line-per-pattern.
pub(super) const fn ip(
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
pub(super) const IDENTITY_NARRATION_PATTERNS: &[IdentityPattern] = &[
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
