// ABOUTME: The words the claim extractor routes on, in the five shipped locales
// ABOUTME: Own-record cues, record verbs, prescription and look-ahead markers, physiology terms

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The vocabulary [`super::classify_heuristic`] reads. Every entry is a
//! lowercase phrase of whole words, matched token by token, so "should" never
//! matches inside "shoulder".
//!
//! The lists that decide whether a sentence is about the athlete's own record
//! are tables keyed by locale, so a locale missing a kind shows up in the
//! parity tests instead of in production — French once lost « ta dernière »
//! while English kept "your last" (carnet#1188).

/// The words with which a reply points at one of the athlete's own past
/// sessions, in one locale.
///
/// Three kinds, because each is a reference to the record only in its own
/// shape:
///
/// - `recency` — a possessive with "last": "ta dernière séance", "your latest
///   ride". It names a session but carries no tense, so it counts only when
///   nothing in the sentence looks ahead (see [`FORWARD_MARKERS`]): « ta
///   dernière sortie longue avant le marathon » is a plan.
/// - `held` — a measured value that stayed somewhere over a session: "est
///   restée à 133", "blieb bei 133". Past, but a bare "stayed at" is about
///   anything, so it counts only a few words after one of the locale's
///   `possessives`.
/// - `possessives` — the second-person possessives `held` is read with.
///
/// A weekday is deliberately not a cue. Every weekly plan names one —
/// "Samedi : 18 km" — and for an athlete with nothing connected the record
/// layer contradicts any figure it is handed, so a plan routed there would be
/// called invented.
#[derive(Debug)]
pub(super) struct OwnRecordCues {
    pub(super) recency: &'static [&'static str],
    pub(super) held: &'static [&'static str],
    pub(super) possessives: &'static [&'static str],
}

/// [`OwnRecordCues`] for the five shipped locales, each beside the locale its
/// forms are written in.
///
/// Read across every locale at once, like every other list in this file: the
/// extractor reads a reply without its locale. A homograph is not harmless
/// here. Routed to the record, a figure about an athlete with nothing
/// connected is called invented, so a form that is an ordinary word in another
/// language manufactures a contradiction. That is why a `held` form pairs only
/// with a possessive of its own locale standing right before it, and why
/// English "sat at" is not a form: it is also "Sat at 7:00", a plan.
pub(super) const OWN_RECORD_CUES: [(&str, OwnRecordCues); 5] = [
    (
        "en",
        OwnRecordCues {
            recency: &[
                "your last",
                "your latest",
                "your most recent",
                "your previous",
            ],
            held: &["stayed", "remained", "held at"],
            possessives: &["your"],
        },
    ),
    (
        "fr",
        OwnRecordCues {
            recency: &[
                "ta dernière",
                "ta derniere",
                "ton dernier",
                "tes dernières",
                "tes dernieres",
                "tes derniers",
                "ta précédente",
                "ta precedente",
                "ton précédent",
                "ton precedent",
            ],
            held: &[
                "est restée",
                "est restee",
                "est resté",
                "est reste",
                "sont restées",
                "sont restees",
                "sont restés",
                "sont restes",
            ],
            possessives: &["ta", "ton", "tes", "votre", "vos"],
        },
    ),
    (
        "es",
        OwnRecordCues {
            recency: &[
                "tu última",
                "tu ultima",
                "tu último",
                "tu ultimo",
                "tus últimas",
                "tus ultimas",
                "tus últimos",
                "tus ultimos",
                "tu anterior",
            ],
            held: &["se mantuvo", "se quedó", "se quedo", "estuvo"],
            possessives: &["tu", "tus"],
        },
    ),
    (
        "de",
        OwnRecordCues {
            recency: &[
                "deine letzte",
                "dein letzter",
                "dein letztes",
                "deinen letzten",
                "deiner letzten",
                "deinem letzten",
                "deine letzten",
            ],
            held: &["blieb", "lag bei", "ist geblieben"],
            possessives: &["dein", "deine", "deinen", "deiner", "deinem", "deines"],
        },
    ),
    (
        "pt",
        OwnRecordCues {
            recency: &[
                "tua última",
                "tua ultima",
                "teu último",
                "teu ultimo",
                "tuas últimas",
                "tuas ultimas",
                "teus últimos",
                "teus ultimos",
                "sua última",
                "sua ultima",
                "seu último",
                "seu ultimo",
            ],
            held: &["ficou", "manteve-se", "se manteve", "esteve"],
            possessives: &["teu", "tua", "teus", "tuas", "seu", "sua", "seus", "suas"],
        },
    ),
];

/// How many words may stand between a possessive and the `held` form it goes
/// with: "ta FC est restée" (one), "your average heart rate stayed" (three).
pub(super) const HELD_REACH: usize = 4;

/// Words that put a sentence in the future rather than the past, in the five
/// shipped locales: what comes before an event, what is next, what will be.
///
/// They mark a plan only where nothing reports: a record verb or a held value
/// keeps the sentence a report ("you ran 30 km before the race"). A recency cue
/// does not, because "your last long run before the race is 30 km on Sunday"
/// is next week's session.
pub(super) const FORWARD_MARKERS: [(&str, &[&str]); 5] = [
    (
        "en",
        &[
            "before",
            "next",
            "upcoming",
            "will",
            "going to",
            "planned",
            "scheduled",
        ],
    ),
    (
        "fr",
        &[
            "avant",
            "prochaine",
            "prochain",
            "prochaines",
            "prochains",
            "à venir",
            "sera",
            "seront",
            "feras",
            "fera",
            "tu vas",
            "prévue",
            "prévu",
            "prevue",
            "prevu",
        ],
    ),
    (
        "es",
        &[
            "antes de",
            "antes del",
            "próxima",
            "proxima",
            "próximo",
            "proximo",
            "será",
            "serán",
            "harás",
            "haras",
            "vas a",
            "prevista",
            "previsto",
        ],
    ),
    (
        "de",
        &[
            "vor dem",
            "vor der",
            "vor deinem",
            "vor deiner",
            "nächste",
            "nächsten",
            "nächster",
            "nächstes",
            "wird",
            "wirst",
            "geplant",
            "geplante",
        ],
    ),
    (
        "pt",
        &[
            "antes da", "antes do", "antes de", "próxima", "proxima", "próximo", "proximo", "será",
            "serão", "vais", "farás", "prevista", "previsto",
        ],
    ),
];

/// Words that prescribe work — a modal, an intent, an imperative, a future
/// day — in the five shipped locales. Any one of them keeps a sentence out of
/// the athlete's record, whatever verb it also carries.
///
/// "mañana" and "morgen" also mean "morning"; [`TOMORROW_HOMOGRAPHS`] says
/// when they do.
pub(super) const PRESCRIPTION_MARKERS: [(&str, &[&str]); 5] = [
    (
        "en",
        &[
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
            "make sure",
            "tomorrow",
            "next week",
        ],
    ),
    (
        "fr",
        &[
            "vise",
            "on va",
            "tu devrais",
            "je te propose",
            "commence par",
            "essaie",
            "fais",
            "garde",
            "prévois",
            "prevois",
            "demain",
            "la semaine prochaine",
        ],
    ),
    (
        "es",
        &[
            "apunta a",
            "vamos a",
            "deberías",
            "deberias",
            "intenta",
            "trata de",
            "te recomiendo",
            "te sugiero",
            "te propongo",
            "empieza con",
            "empieza por",
            "céntrate en",
            "centrate en",
            "haz",
            "mantén",
            "manten",
            "mañana",
            "manana",
            "la próxima semana",
            "la proxima semana",
            "la semana que viene",
        ],
    ),
    (
        "de",
        &[
            "lass uns",
            "wir werden",
            "solltest",
            "versuche",
            "versuch",
            "ich empfehle",
            "ich schlage",
            "beginne mit",
            "starte mit",
            "konzentriere dich",
            "achte auf",
            "mach",
            "halte",
            "morgen",
            "nächste woche",
            "nächsten woche",
        ],
    ),
    (
        "pt",
        &[
            "vamos",
            "deverias",
            "deves",
            "tenta",
            "tente",
            "recomendo",
            "sugiro",
            "proponho",
            "começa com",
            "comeca com",
            "comece com",
            "foca em",
            "faz",
            "faça",
            "mantém",
            "mantenha",
            "amanhã",
            "amanha",
            "próxima semana",
            "proxima semana",
            "semana que vem",
        ],
    ),
];

/// Markers that are also the word for "morning", and the words that, right
/// before one, make it the morning: "esta mañana corriste 10 km", "gestern
/// Morgen bist du 10 km gelaufen" report; "mañana, 10 km" plans.
pub(super) const TOMORROW_HOMOGRAPHS: [(&str, &[&str]); 3] = [
    ("mañana", &["la", "esta", "una", "cada", "media"]),
    ("manana", &["la", "esta", "una", "cada", "media"]),
    (
        "morgen",
        &[
            "heute",
            "gestern",
            "vorgestern",
            "am",
            "jeden",
            "diesen",
            "den",
            "einen",
            "guten",
        ],
    ),
];

/// Phrases naming the current week. A plan and a report both carry them, so
/// they mark a plan only when nothing in the sentence reports.
pub(super) const CURRENT_PERIOD: [&str; 5] = [
    "this week",
    "cette semaine",
    "esta semana",
    "diese woche",
    "dieser woche",
];

/// Second-person past-tense verbs that report the athlete's own training, in
/// the five shipped locales.
///
/// The past tense is what separates a record from a plan: "you ran 42 km"
/// happened, "run 42 km" has not. Spanish, German and Portuguese enter the
/// athlete-data bucket through these verbs and the own-record cues, with no
/// time words: a time word ("esta semana") is as at home in a plan as in a
/// report.
///
/// Every form is second person, so a sentence about another athlete's result
/// is not read as one about this athlete. German is read separately, through
/// [`GERMAN_PARTICIPLES`].
pub(super) const RECORD_VERBS: &[&str] = &[
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
    // The être auxiliary: « tu es passé à 36 km » reports the session as
    // surely as « tu as couru » (carnet#1188).
    "tu es passé",
    "tu es passée",
    "tu es passe",
    "tu es passee",
    "tu es monté",
    "tu es montée",
    "tu es monte",
    "tu es montee",
    "tu es allé",
    "tu es allée",
    "tu es alle",
    "tu es allee",
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
/// [`super::record_verb_hits`].
pub(super) const GERMAN_PARTICIPLES: &[&str] = &[
    "gelaufen",
    "gefahren",
    "geradelt",
    "geschwommen",
    "zurückgelegt",
    "absolviert",
];

/// The physiology bucket's keywords.
pub(super) const PHYSIOLOGY: &[&str] = &[
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
    // Critical-power family in the five shipped locales. The prime symbols
    // are left out: tokenized, "W′" is the bare "w" every wattage carries.
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
    // Resting heart rate, which the personalized layer checks against the
    // athlete's synced mornings (carnet#853). "Heart rate" and "fc" already
    // catch it in English and French.
    "resting hr",
    "fc en reposo",
    "frecuencia cardíaca en reposo",
    "ruhepuls",
    "ruheherzfrequenz",
    "fc em repouso",
    "frequência cardíaca em repouso",
    // Form, which the personalized layer checks as a share of the athlete's
    // own fitness and a band (carnet#850): the labels and possessives a reply
    // puts on it, the fitness it is a share of, and the band only a form
    // claim names.
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
];

/// The physiology keywords a single session measures: its heart rate and the
/// zone it was ridden in. Stated in the clause of an own-session reference,
/// they are that session's figures; any other physiology term in a clause of
/// its own is a claim about the athlete's physiology, not about the session.
pub(super) const SESSION_MEASURES: &[&str] = &[
    "heart rate",
    "fc",
    "fc moy",
    "fréquence cardiaque",
    "frequence cardiaque",
    "bpm",
    "zone",
];
