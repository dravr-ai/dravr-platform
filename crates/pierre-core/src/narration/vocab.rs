// ABOUTME: Capability-failure vocabulary plus the lazily folded lookup tables
// ABOUTME: Split out of narration/mod.rs so each file stays legible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::LazyLock;

use super::fold::fold_separators;
use super::patterns::IDENTITY_NARRATION_PATTERNS;
use super::INTERNAL_NARRATION_PATTERNS;

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
pub(super) const CAPABILITY_FAILURE_PATTERNS: &[&str] = &[
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
pub(super) static FOLDED_INTERNAL: LazyLock<Vec<String>> = LazyLock::new(|| {
    INTERNAL_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// Separator-folded copy of [`CAPABILITY_FAILURE_PATTERNS`], built once.
pub(super) static FOLDED_CAPABILITY: LazyLock<Vec<String>> = LazyLock::new(|| {
    CAPABILITY_FAILURE_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

/// Separator-folded copy of [`IDENTITY_NARRATION_PATTERNS`], built once.
/// Index-aligned with the pattern table so a folded hit maps back to its
/// class + locale labels.
pub(super) static FOLDED_IDENTITY: LazyLock<Vec<String>> = LazyLock::new(|| {
    IDENTITY_NARRATION_PATTERNS
        .iter()
        .map(|p| fold_separators(p.text))
        .collect()
});

// ===========================================================================
// Runtime vocabulary overlay (contremaitre-synced)
// ===========================================================================
