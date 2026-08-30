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

/// Lowercase, separator-folded vocabulary that marks a sentence as a
/// **peer-access denial** — the coach saying it cannot read ANOTHER athlete's
/// data («je n'ai jamais eu accès à l'historique de Jean-Daniel … je n'ai
/// aucune donnée sur lui» — live incident 2026-08-30, a Telegram group where
/// the Guardian's own repair path manufactured that retraction).
///
/// Kept apart from [`CAPABILITY_FAILURE_PATTERNS`] on purpose: the own-access
/// register drives the outbound `ClaimedFailure` trigger on EVERY surface, and a
/// peer denial only means anything where a peer exists. The chat pipeline
/// consults this table only when the turn carries a group roster AND the reply
/// names a roster member, so a DM reply such as «je n'ai pas accès aux données
/// de fréquence cardiaque de cette sortie» never trips a verification fetch.
/// On REPLAY both registers apply: a consent state is re-derived live every
/// turn, so replaying yesterday's «I can't see his data» after he consented
/// teaches stale helplessness — the same adjudication as the account-state
/// denial above.
///
/// Precision philosophy: first-person subject + third-person object. English
/// possessive names («JD's data») cannot be matched statically, so the English
/// forms are pronoun-anchored (his/her/their, him/her/them); French carries the
/// «de <name>» forms because the object noun («données de», «activités de»,
/// «historique de») precedes the name there. Accented phrases carry
/// accent-stripped twins.
pub(super) const PEER_ACCESS_DENIAL_PATTERNS: &[&str] = &[
    // French — the incident register
    "je n'ai pas accès aux données de",
    "je n'ai pas acces aux donnees de",
    "je n'ai pas accès aux activités de",
    "je n'ai pas acces aux activites de",
    "je n'ai pas accès à l'historique de",
    "je n'ai pas acces a l'historique de",
    "je n'ai pas accès à ses données",
    "je n'ai pas acces a ses donnees",
    "je n'ai pas accès à ses activités",
    "je n'ai pas acces a ses activites",
    "je n'ai pas accès à son historique",
    "je n'ai pas acces a son historique",
    "je n'ai jamais eu accès à l'historique",
    "je n'ai jamais eu acces a l'historique",
    "je n'ai jamais eu accès aux données",
    "je n'ai jamais eu acces aux donnees",
    "je n'ai jamais eu accès aux activités",
    "je n'ai jamais eu acces aux activites",
    "je n'ai aucune donnée sur lui",
    "je n'ai aucune donnee sur lui",
    "je n'ai aucune donnée sur elle",
    "je n'ai aucune donnee sur elle",
    "je ne peux pas accéder aux données de",
    "je ne peux pas acceder aux donnees de",
    "je ne peux pas accéder à ses données",
    "je ne peux pas acceder a ses donnees",
    "je ne peux pas récupérer les activités de",
    "je ne peux pas recuperer les activites de",
    "je ne peux pas récupérer ses activités",
    "je ne peux pas recuperer ses activites",
    "je n'arrive pas à récupérer les activités de",
    "je n'arrive pas a recuperer les activites de",
    "je n'arrive pas à récupérer ses activités",
    "je n'arrive pas a recuperer ses activites",
    // English — pronoun-anchored (his/her/their data|activities|history;
    // no data on him/her/them)
    "i don't have access to his data",
    "i don't have access to his activities",
    "i don't have access to his history",
    "i do not have access to his data",
    "i do not have access to his activities",
    "i do not have access to his history",
    "i have no access to his data",
    "i have no access to his activities",
    "i've never had access to his data",
    "i've never had access to his activities",
    "i've never had access to his history",
    "i have never had access to his data",
    "i have never had access to his activities",
    "i have never had access to his history",
    "i can't access his data",
    "i can't access his activities",
    "i cannot access his data",
    "i cannot access his activities",
    "i don't have access to her data",
    "i don't have access to her activities",
    "i don't have access to her history",
    "i do not have access to her data",
    "i do not have access to her activities",
    "i do not have access to her history",
    "i have no access to her data",
    "i have no access to her activities",
    "i've never had access to her data",
    "i've never had access to her activities",
    "i've never had access to her history",
    "i have never had access to her data",
    "i have never had access to her activities",
    "i have never had access to her history",
    "i can't access her data",
    "i can't access her activities",
    "i cannot access her data",
    "i cannot access her activities",
    "i don't have access to their data",
    "i don't have access to their activities",
    "i don't have access to their history",
    "i do not have access to their data",
    "i do not have access to their activities",
    "i do not have access to their history",
    "i have no access to their data",
    "i have no access to their activities",
    "i've never had access to their data",
    "i've never had access to their activities",
    "i've never had access to their history",
    "i have never had access to their data",
    "i have never had access to their activities",
    "i have never had access to their history",
    "i can't access their data",
    "i can't access their activities",
    "i cannot access their data",
    "i cannot access their activities",
    "i have no data on him",
    "i don't have any data on him",
    "i do not have any data on him",
    "i have no data on her",
    "i don't have any data on her",
    "i do not have any data on her",
    "i have no data on them",
    "i don't have any data on them",
    "i do not have any data on them",
    // Spanish — «sus» reads as his/her/their (and formal your); either way a
    // replayed «no tengo acceso a sus datos» teaches stale helplessness.
    "no tengo acceso a sus datos",
    "no tengo acceso a sus actividades",
    "no tengo acceso a los datos de",
    "no tengo acceso a las actividades de",
    "no tengo acceso al historial de",
    "no puedo acceder a sus datos",
    "no puedo acceder a los datos de",
    "no tengo datos sobre él",
    "no tengo datos sobre el",
    "no tengo datos sobre ella",
    "nunca he tenido acceso a sus datos",
    "nunca he tenido acceso al historial de",
    // German — seine/ihre (his/her/their); «dein» stays in the own register so
    // «ich habe keinen Zugriff auf dein Garmin-Passwort» keeps passing.
    "ich habe keinen zugriff auf seine daten",
    "ich habe keinen zugriff auf ihre daten",
    "ich habe keinen zugriff auf seine aktivitäten",
    "ich habe keinen zugriff auf seine aktivitaten",
    "ich habe keinen zugriff auf ihre aktivitäten",
    "ich habe keinen zugriff auf ihre aktivitaten",
    "ich habe keinen zugriff auf die daten von",
    "ich habe keinen zugriff auf die aktivitäten von",
    "ich habe keinen zugriff auf die aktivitaten von",
    "ich habe keine daten über ihn",
    "ich habe keine daten ueber ihn",
    "ich habe keine daten über sie",
    "ich habe keine daten ueber sie",
    "ich kann nicht auf seine daten zugreifen",
    "ich kann nicht auf ihre daten zugreifen",
    "ich hatte nie zugriff auf seine daten",
    "ich hatte nie zugriff auf ihre daten",
    // Portuguese — dele/dela (his/her) plus the «de <name>» form
    "não tenho acesso aos dados dele",
    "nao tenho acesso aos dados dele",
    "não tenho acesso aos dados dela",
    "nao tenho acesso aos dados dela",
    "não tenho acesso às atividades dele",
    "nao tenho acesso as atividades dele",
    "não tenho acesso às atividades dela",
    "nao tenho acesso as atividades dela",
    "não tenho acesso aos dados de",
    "nao tenho acesso aos dados de",
    "não tenho dados sobre ele",
    "nao tenho dados sobre ele",
    "não tenho dados sobre ela",
    "nao tenho dados sobre ela",
    "nunca tive acesso aos dados dele",
    "nunca tive acesso aos dados dela",
    "nunca tive acesso aos dados de",
    "não consigo aceder aos dados dele",
    "nao consigo aceder aos dados dele",
    "não consigo acessar os dados dele",
    "nao consigo acessar os dados dele",
];

/// Separator-folded copy of [`PEER_ACCESS_DENIAL_PATTERNS`], built once.
pub(super) static FOLDED_PEER_DENIAL: LazyLock<Vec<String>> = LazyLock::new(|| {
    PEER_ACCESS_DENIAL_PATTERNS
        .iter()
        .map(|p| fold_separators(p))
        .collect()
});

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
