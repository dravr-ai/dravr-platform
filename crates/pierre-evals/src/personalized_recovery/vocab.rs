// ABOUTME: Five-locale vocabulary the overnight-recovery probes read — metric keywords, change words, units and fillers
// ABOUTME: Data, not logic: each table lists the words one locale uses for a concept the sentence reader looks for
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// HRV, in the five shipped locales.
pub(super) const HRV_KEYWORDS: &[&str] = &[
    "hrv",
    "heart rate variability",
    "rmssd",
    // fr / es / pt
    "vfc",
    "variabilité de la fréquence cardiaque",
    "variabilité cardiaque",
    "variabilidad de la frecuencia cardíaca",
    "variabilidade da frequência cardíaca",
    // de
    "herzfrequenzvariabilität",
    "herzratenvariabilität",
    "hfv",
];

/// Resting heart rate, in the five shipped locales.
pub(super) const RESTING_HR_KEYWORDS: &[&str] = &[
    "resting heart rate",
    "resting hr",
    "rhr",
    // fr
    "fc au repos",
    "fc de repos",
    "fréquence cardiaque au repos",
    "fréquence cardiaque de repos",
    // es
    "fc en reposo",
    "frecuencia cardíaca en reposo",
    "pulso en reposo",
    // de
    "ruhepuls",
    "ruheherzfrequenz",
    // pt
    "fc em repouso",
    "fc de repouso",
    "frequência cardíaca em repouso",
    "frequência cardíaca de repouso",
];

/// Phrases that make a sentence a statement about the athlete's sleep: a past
/// sleep verb, a possessive before sleep or nights, or a habitual second
/// person. "Sleep 8 hours" and "8 hours of sleep" are advice, not one of them.
pub(super) const SLEEP_CLAIMS: &[&str] = &[
    "slept",
    "your sleep",
    "your nights",
    "you sleep",
    "you ve been sleeping",
    "you have been sleeping",
    "you re sleeping",
    "you are sleeping",
    // fr
    "dormi",
    "ton sommeil",
    "votre sommeil",
    "tes nuits",
    "vos nuits",
    "tu dors",
    "vous dormez",
    // es
    "dormiste",
    "dormido",
    "durmió",
    "duermes",
    "tu sueño",
    "tus noches",
    // de
    "geschlafen",
    "du schläfst",
    "dein schlaf",
    "deine schlafdauer",
    "deine nächte",
    // pt
    "dormiu",
    "dormes",
    "você dorme",
    "teu sono",
    "seu sono",
    "tuas noites",
    "suas noites",
];

/// "You averaged 7 h" is about sleep only when the sentence names sleep.
pub(super) const AVERAGING_CLAIMS: &[&str] = &[
    "you averaged",
    "you ve averaged",
    "you have averaged",
    "you ve been averaging",
    "you re averaging",
    "you are averaging",
    "you average",
];

/// Sleep as a noun, for [`AVERAGING_CLAIMS`].
pub(super) const SLEEP_NOUNS: &[&str] = &["sleep", "sommeil", "sueño", "schlaf", "sono"];

/// A sleep stage, a nap or time awake: not the night's total.
pub(super) const SLEEP_EXCLUSIONS: &[&str] = &[
    "deep",
    "rem",
    "light",
    "nap",
    "naps",
    "napped",
    "awake",
    // fr
    "profond",
    "paradoxal",
    "léger",
    "sieste",
    "éveillé",
    // es / pt
    "profundo",
    "ligero",
    "siesta",
    "despierto",
    "leve",
    "sesta",
    "acordado",
    // de
    "tiefschlaf",
    "leichtschlaf",
    "nickerchen",
    "wach",
];

/// Words addressing the athlete, including the second-person verb forms that
/// carry no pronoun ("dormiste", "duermes").
pub(super) const SECOND_PERSON: &[&str] = &[
    "you",
    "your",
    "yours",
    "yourself",
    // fr
    "tu",
    "ta",
    "ton",
    "tes",
    "te",
    "toi",
    "vous",
    "votre",
    "vos",
    // es
    "tú",
    "tus",
    "ti",
    "usted",
    "dormiste",
    "duermes",
    // de. A bare "du" is left out: in French it is "of the", and would make
    // a generality ("la VFC du coureur") read as addressed to the athlete.
    // The verb forms that follow it carry the second person instead.
    "hast",
    "bist",
    "hattest",
    "warst",
    "dein",
    "deine",
    "deinen",
    "deinem",
    "deiner",
    "deines",
    "dir",
    "dich",
    "schläfst",
    // pt
    "teu",
    "tua",
    "teus",
    "tuas",
    "você",
    "voce",
    "vocês",
    "seu",
    "sua",
    "seus",
    "suas",
    "dormes",
];

/// Past-tense changes: a sentence reporting one is about what happened.
pub(super) const PAST_CHANGE: &[&str] = &[
    "dropped",
    "fell",
    "fallen",
    "rose",
    "risen",
    "increased",
    "decreased",
    "declined",
    "dipped",
    "climbed",
    "jumped",
    // fr
    "baissé",
    "baissée",
    "chuté",
    "chutée",
    "diminué",
    "diminuée",
    "augmenté",
    "augmentée",
    "remonté",
    "remontée",
    "grimpé",
    "descendu",
    "descendue",
    // es
    "bajó",
    "cayó",
    "subió",
    "aumentó",
    "disminuyó",
    "descendió",
    // de
    "gesunken",
    "gefallen",
    "gestiegen",
    "angestiegen",
    "zurückgegangen",
    "abgenommen",
    "zugenommen",
    // pt
    "caiu",
    "baixou",
    "subiu",
    "aumentou",
    "diminuiu",
    "desceu",
];

/// Words of a downward change, besides [`PAST_CHANGE`]'s.
pub(super) const DOWN_WORDS: &[&str] = &[
    "dropped",
    "drop",
    "drops",
    "fell",
    "fallen",
    "down",
    "decreased",
    "decrease",
    "declined",
    "decline",
    "dipped",
    "lower",
    "below",
    "under",
    "less",
    "fewer",
    // fr
    "baissé",
    "baissée",
    "baisse",
    "chuté",
    "chutée",
    "chute",
    "diminué",
    "diminuée",
    "diminution",
    "descendu",
    "descendue",
    "sous",
    "dessous",
    "inférieur",
    "inférieure",
    "basse",
    "moins",
    // es
    "bajó",
    "bajada",
    "baja",
    "bajo",
    "cayó",
    "caída",
    "descendió",
    "descenso",
    "disminuyó",
    "disminución",
    "debajo",
    "inferior",
    "menos",
    // de
    "gesunken",
    "gefallen",
    "zurückgegangen",
    "abgenommen",
    "rückgang",
    "niedriger",
    "unter",
    "unterhalb",
    "weniger",
    "tiefer",
    // pt
    "caiu",
    "baixou",
    "baixa",
    "baixo",
    "diminuiu",
    "diminuição",
    "desceu",
    "queda",
    "abaixo",
];

/// Words of an upward change, besides [`PAST_CHANGE`]'s.
pub(super) const UP_WORDS: &[&str] = &[
    "rose",
    "risen",
    "rise",
    "up",
    "increased",
    "increase",
    "climbed",
    "jumped",
    "higher",
    "above",
    "more",
    // fr
    "augmenté",
    "augmentée",
    "augmentation",
    "hausse",
    "remonté",
    "remontée",
    "grimpé",
    "dessus",
    "supérieur",
    "supérieure",
    "haute",
    "élevée",
    "plus",
    // es
    "subió",
    "subida",
    "aumentó",
    "aumento",
    "encima",
    "superior",
    "alta",
    "alto",
    "más",
    // de
    "gestiegen",
    "angestiegen",
    "anstieg",
    "zugenommen",
    "höher",
    "über",
    "oberhalb",
    "mehr",
    // pt. "mais" is left out: in French it is "but", and would cancel
    // the direction of a French change ("ta VFC a baissé de 15 %, mais…").
    // Portuguese "mais alta" still reads up through "alta".
    "subiu",
    "aumentou",
    "acima",
];

/// Multi-word downward changes.
pub(super) const DOWN_PHRASES: &[&str] = &["en baisse", "a la baja", "por debajo", "em queda"];

/// Multi-word upward changes.
pub(super) const UP_PHRASES: &[&str] = &["en hausse", "al alza", "por encima", "em alta"];

/// Every word that makes a figure next to it a change: [`DOWN_WORDS`] and
/// [`UP_WORDS`] together.
pub(super) const CHANGE_WORDS: &[&str] = &[
    "drop",
    "drops",
    "down",
    "decrease",
    "decline",
    "lower",
    "below",
    "under",
    "less",
    "fewer",
    "rise",
    "up",
    "increase",
    "higher",
    "above",
    "more",
    "least",
    "most",
    "extra",
    // fr
    "baisse",
    "chute",
    "diminution",
    "sous",
    "dessous",
    "inférieur",
    "inférieure",
    "moins",
    "augmentation",
    "hausse",
    "dessus",
    "supérieur",
    "supérieure",
    "plus",
    // es
    "bajada",
    "caída",
    "descenso",
    "disminución",
    "debajo",
    "inferior",
    "menos",
    "subida",
    "aumento",
    "encima",
    "superior",
    "más",
    // de
    "rückgang",
    "niedriger",
    "unter",
    "unterhalb",
    "weniger",
    "tiefer",
    "anstieg",
    "höher",
    "über",
    "oberhalb",
    "mehr",
    "mindestens",
    "höchstens",
    // pt
    "queda",
    "abaixo",
    "acima",
    "mais",
];

/// Words after a figure that make it a difference from something else.
pub(super) const COMPARED_WORDS: &[&str] = &[
    "below",
    "above",
    "lower",
    "higher",
    "less",
    "more",
    "fewer",
    "under",
    // fr
    "sous",
    "dessous",
    "dessus",
    "moins",
    "plus",
    "inférieur",
    "inférieure",
    "supérieur",
    "supérieure",
    // es
    "menos",
    "más",
    "debajo",
    "encima",
    "inferior",
    "superior",
    // de
    "weniger",
    "mehr",
    "niedriger",
    "höher",
    "unter",
    "über",
    "unterhalb",
    "oberhalb",
    // pt
    "abaixo",
    "acima",
    "mais",
];

/// Words that may sit between a figure and the comparison that follows it
/// ("5 ms de moins", "por debajo").
pub(super) const FILLERS: &[&str] = &["de", "en", "por", "au"];

/// Words before a figure that make it a difference or a bound: "by 5", "from
/// 70", "more than 8", "over 8".
pub(super) const DIFFERENCE_WORDS: &[&str] = &[
    "by", "from", "since", "than", "over", "um", "als", "depuis", "desde", "que",
];

/// Words that link a metric to its value: "is 62", "est de 48", "liegt bei 48".
pub(super) const LINK_WORDS: &[&str] = &[
    "is",
    "was",
    "at",
    "of",
    "around",
    "about",
    "approximately",
    "roughly",
    "nearly",
    "almost",
    "averaged",
    "averages",
    "average",
    // fr
    "de",
    "est",
    "était",
    "environ",
    "autour",
    "vers",
    "moyenne",
    // es
    "es",
    "fue",
    "era",
    "en",
    "unos",
    "alrededor",
    "cerca",
    "promedio",
    "aproximadamente",
    // de
    "bei",
    "ist",
    "war",
    "von",
    "etwa",
    "rund",
    "ca",
    "circa",
    "durchschnittlich",
    // pt
    "é",
    "foi",
    "em",
    "média",
    "uns",
];

/// Words that land a change on its new value: "dropped to 45".
pub(super) const TO_WORDS: &[&str] = &["to", "à", "a", "auf", "para"];

/// Words joining hours to minutes: "7 hours and 30 minutes".
pub(super) const CONNECTIVES: &[&str] = &["and", "et", "y", "und", "e"];

/// A half hour after an hour figure: "6 horas y media".
pub(super) const HALF_HOUR: &[&str] = &[
    "and a half",
    "et demie",
    "y media",
    "e meia",
    "und eine halbe",
];

/// Advice, a hypothesis or a goal: a sentence carrying one is not a statement
/// about the record.
pub(super) const ADVICE: &[&str] = &[
    "should",
    "must",
    "need",
    "needs",
    "aim",
    "target",
    "try",
    "ideally",
    "recommend",
    "suggest",
    "if",
    "make sure",
    "goal",
    "let s",
    // fr
    "devrais",
    "devriez",
    "dois",
    "doit",
    "devez",
    "faut",
    "besoin",
    "vise",
    "visez",
    "viser",
    "essaie",
    "essayez",
    "idéalement",
    "objectif",
    "cible",
    "si",
    "recommande",
    "conseille",
    // es
    "deberías",
    "debería",
    "debes",
    "necesitas",
    "necesita",
    "intenta",
    "procura",
    "idealmente",
    "objetivo",
    "recomiendo",
    // de
    "solltest",
    "sollte",
    "sollten",
    "musst",
    "muss",
    "brauchst",
    "versuche",
    "versuch",
    "idealerweise",
    "ziel",
    "wenn",
    "falls",
    "empfehle",
    // pt
    "deverias",
    "deveria",
    "deves",
    "precisas",
    "precisa",
    "tenta",
    "tente",
    "procure",
    "recomendo",
];

/// Negations: a sentence denying a figure ("you didn't sleep 8 hours") says
/// nothing about which figure the nights hold.
///
/// English (the stems "didn't" and its kind split into), then French,
/// Spanish, German and Portuguese. A bare "no" is left out: Portuguese writes
/// it for "in the".
pub(super) const NEGATIONS: &[&str] = &[
    "not", "never", "nor", "cannot", "didn", "doesn", "don", "isn", "wasn", "weren", "aren",
    "haven", "hasn", "hadn", "pas", "jamais", "aucun", "aucune", "nunca", "tampoco", "ni",
    "ningún", "ninguna", "nicht", "nie", "niemals", "kein", "keine", "keinen", "keiner", "não",
    "nem", "nenhum", "nenhuma",
];

/// Spans longer than the week the readings cover.
pub(super) const LONGER_PERIODS: &[&str] = &[
    "weeks",
    "fortnight",
    "month",
    "months",
    "year",
    "years",
    // fr
    "semaines",
    "quinzaine",
    "mois",
    "année",
    "années",
    "ans",
    // es. A bare "mes" is French for "my": the Spanish month is matched
    // by the phrases below.
    "semanas",
    "quincena",
    "meses",
    "año",
    "años",
    // de
    "wochen",
    "monat",
    "monate",
    "monaten",
    "monats",
    "jahr",
    "jahre",
    "jahren",
    // pt
    "quinzena",
    "mês",
    "ano",
    "anos",
];

/// The Spanish month, which [`LONGER_PERIODS`] cannot hold as a bare word.
pub(super) const LONGER_PERIOD_PHRASES: &[&str] =
    &["este mes", "el mes", "un mes", "último mes", "del mes"];

/// Days or nights, whose count says how long a span is.
pub(super) const DAY_WORDS: &[&str] = &[
    "days", "nights", "jours", "nuits", "días", "dias", "noches", "noites", "tage", "tagen",
    "nächte", "nächten",
];

/// The days of the week. A sentence naming one may be about that day's
/// night ("your HRV dropped 18% on Tuesday"), which neither last night nor a
/// mean stands for.
///
/// Portuguese writes a weekday as an ordinal ("segunda", "quarta"), which
/// also reads "second" or "fourth": a sentence using one as an ordinal is
/// left unread rather than scored against the wrong night.
pub(super) const WEEKDAYS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    // fr
    "lundi",
    "mardi",
    "mercredi",
    "jeudi",
    "vendredi",
    "samedi",
    "dimanche",
    // es
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
    "domingo",
    // de
    "montag",
    "dienstag",
    "mittwoch",
    "donnerstag",
    "freitag",
    "samstag",
    "sonnabend",
    "sonntag",
    // pt
    "segunda",
    "terça",
    "quarta",
    "quinta",
    "sexta",
];

/// Phrases naming a night before the most recent one.
pub(super) const EARLIER_NIGHT_PHRASES: &[&str] = &[
    "ago",
    "night before last",
    "day before yesterday",
    // fr
    "avant hier",
    "avant dernière nuit",
    // es / pt
    "atrás",
    "anteayer",
    "anteanoche",
    "antes de ayer",
    "anteontem",
    "antes de ontem",
    "penúltima noche",
    "penúltima noite",
    // de
    "vorgestern",
    "vorletzte nacht",
];

/// Words counting back from today when a day, night or week follows them:
/// "il y a trois jours", "hace dos noches", "vor drei Tagen", "há dois dias".
pub(super) const COUNT_BACK: &[&str] = &["il y a", "hace", "vor", "há"];

/// A day, night or week, after a [`COUNT_BACK`] word and its count.
pub(super) const COUNTED_BACK: &[&str] = &[
    "jour", "jours", "nuit", "nuits", "semaine", "día", "días", "noche", "noches", "semana", "tag",
    "tagen", "nacht", "nächten", "woche", "dia", "dias", "noite", "noites",
];

/// Phrases tying a figure to the most recent night.
pub(super) const LAST_NIGHT: &[&str] = &[
    "last night",
    "overnight",
    "this morning",
    "today",
    // fr
    "cette nuit",
    "la nuit dernière",
    "ce matin",
    "aujourd hui",
    // es
    "anoche",
    "esta mañana",
    "hoy",
    "la noche pasada",
    // de
    "letzte nacht",
    "heute nacht",
    "vergangene nacht",
    "heute morgen",
    "heute",
    // pt
    "ontem à noite",
    "esta noite",
    "esta manhã",
    "hoje",
    "última noite",
    "noite passada",
];

/// Phrases tying a figure to an average over several nights.
pub(super) const AVERAGE: &[&str] = &[
    "average",
    "averaged",
    "averaging",
    "avg",
    "mean",
    "baseline",
    "usual",
    "typical",
    "typically",
    "per night",
    "a night",
    "each night",
    "every night",
    // fr
    "moyenne",
    "habituel",
    "habituelle",
    "référence",
    "par nuit",
    "chaque nuit",
    // es
    "promedio",
    "habitual",
    "por noche",
    "cada noche",
    "línea base",
    // de
    "durchschnitt",
    "durchschnittlich",
    "schnitt",
    "üblich",
    "pro nacht",
    "jede nacht",
    "basiswert",
    // pt
    "média",
    "por noite",
    "cada noite",
    "linha de base",
    "normalmente",
];

/// Phrases tying a figure to the week: its mean, or one of its nights.
pub(super) const WEEK: &[&str] = &[
    "week",
    "weekly",
    "nights",
    // fr
    "semaine",
    "nuits",
    // es
    "semana",
    "noches",
    // de
    "woche",
    "wöchentlich",
    "nächte",
    // pt
    "noites",
];

/// "15 percent", "15 Prozent".
pub(super) const PERCENT_WORDS: &[&str] = &["percent", "pourcent", "prozent"];

pub(super) const MILLIS_WORDS: &[&str] = &[
    "ms",
    "msec",
    "millisecond",
    "milliseconds",
    "milliseconde",
    "millisecondes",
    "milisegundo",
    "milisegundos",
    "millisekunden",
    "milissegundos",
];

pub(super) const BPM_WORDS: &[&str] = &[
    "bpm",
    "beats",
    "beat",
    "battements",
    "pulsations",
    "bat",
    "lpm",
    "ppm",
    "pulsaciones",
    "latidos",
    "schläge",
    "schlägen",
    "batimentos",
];

pub(super) const HOUR_WORDS: &[&str] = &[
    "h", "hr", "hrs", "hour", "hours", "heure", "heures", "hora", "horas", "stunde", "stunden",
    "std",
];

pub(super) const MINUTE_WORDS: &[&str] = &[
    "min", "mins", "minute", "minutes", "mn", "minuto", "minutos", "minuten",
];

/// A count of days, nights, weeks or months: English, French, Spanish and
/// Portuguese, then German.
pub(super) const PERIOD_WORDS: &[&str] = &[
    "day", "days", "night", "nights", "week", "weeks", "month", "months", "jour", "jours", "nuit",
    "nuits", "semaine", "semaines", "mois", "día", "días", "dia", "dias", "noche", "noches",
    "noite", "noites", "semana", "semanas", "mes", "meses", "mês", "tag", "tage", "tagen", "nacht",
    "nächte", "nächten", "woche", "wochen", "monat", "monate",
];

pub(super) const OTHER_UNIT_WORDS: &[&str] = &[
    "km", "m", "mi", "w", "watt", "watts", "vatios", "kg", "kcal", "cal", "g", "x", "times",
    "fois", "veces", "mal", "vezes", "rpm", "spm", "points", "pts", "puntos", "punkte", "pontos",
];
