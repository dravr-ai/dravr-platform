// ABOUTME: The use-case starter catalogue: dravr-contremaitre's use_cases/catalogue.yaml, parsed and validated
// ABOUTME: Its predicates are a closed enum held to scripts/ci/use-case-predicates.txt, so an unknown one fails the load
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Use-case catalogue
//!
//! The starters an agent's welcome may offer, ranked from the athlete's state
//! (carnet#828). Each entry says what a tap runs — a slash command, or a
//! localized prompt sent as the athlete's own words — and which athlete-state
//! [`Predicate`]s must hold for it to be offered.
//!
//! The file is structure only. Its words are strings: `use_cases.<id>.label`
//! for the button and, for a prompt starter, `use_cases.<id>.prompt`
//! ([`UseCase::label_key`], [`UseCase::prompt_key`]).
//!
//! Unlike the training catalogue there is no runtime overlay: the structure is
//! compiled in from the pinned dravr-contremaitre, so a new entry or a reorder
//! ships with a deploy. Catalogue order is priority — the ranker keeps it
//! within each stage group.

use std::collections::HashSet;
use std::fmt;
use std::sync::LazyLock;

use pierre_core::models::AgentCategory;
use serde::Deserialize;
use tracing::error;

use super::errors::ContremaitreError;

/// The catalogue compiled in from the pinned dravr-contremaitre.
static PINNED: LazyLock<UseCaseCatalogue> = LazyLock::new(|| {
    UseCaseCatalogue::parse(dravr_contremaitre::USE_CASES_YAML).unwrap_or_else(|e| {
        error!(
            error = %e,
            "the pinned use-case catalogue was rejected; welcomes offer the agents' own examples"
        );
        UseCaseCatalogue::default()
    })
});

/// Telegram caps `callback_data` at 64 bytes; a starter's postback is
/// `uc:<position>:<id>` with a one-digit position.
const MAX_POSTBACK_BYTES: usize = 64;

/// One athlete-state condition a starter can require.
///
/// The vocabulary is [`Predicate::VOCABULARY`], which a unit test holds to
/// `scripts/ci/use-case-predicates.txt` — the list
/// `check-contremaitre-sync.sh` Check 9 checks the catalogue against before
/// contremaitre may push it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// A connected provider can take a pushed training plan.
    CalendarWritable,
    /// The athlete has told us nothing yet: no pillar covered, no physiology, no goals.
    DossierEmpty,
    /// The athlete has activities, or a connected provider nobody has read yet.
    HasActivities,
    /// A connected provider reports sleep or recovery.
    HasRecoverySource,
    /// No real provider is connected.
    NoProvider,
    /// No season plan is active.
    NoSeason,
    /// Every pillar is covered.
    PillarsDone,
    /// The active plan has a week covering today.
    PlanActive,
    /// A season plan is active.
    SeasonSet,
    /// At least this many distinct ISO weeks of activity in the last 28 days.
    WeeksOfData(u8),
}

impl Predicate {
    /// Every predicate name, and whether it takes a `>=N` threshold.
    pub const VOCABULARY: [(&'static str, bool); 10] = [
        ("calendar_writable", false),
        ("dossier_empty", false),
        ("has_activities", false),
        ("has_recovery_source", false),
        ("no_provider", false),
        ("no_season", false),
        ("pillars_done", false),
        ("plan_active", false),
        ("season_set", false),
        ("weeks_of_data", true),
    ];

    /// The catalogue name, without any threshold.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CalendarWritable => "calendar_writable",
            Self::DossierEmpty => "dossier_empty",
            Self::HasActivities => "has_activities",
            Self::HasRecoverySource => "has_recovery_source",
            Self::NoProvider => "no_provider",
            Self::NoSeason => "no_season",
            Self::PillarsDone => "pillars_done",
            Self::PlanActive => "plan_active",
            Self::SeasonSet => "season_set",
            Self::WeeksOfData(_) => "weeks_of_data",
        }
    }

    /// Parse one predicate as the catalogue writes it: `name` or `name>=N`.
    ///
    /// # Errors
    ///
    /// Returns a description of the fault when the name is not in the
    /// vocabulary, or the threshold is missing, unexpected or not a number.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (name, threshold) = match text.split_once(">=") {
            Some((name, threshold)) => (name.trim(), Some(threshold.trim())),
            None => (text.trim(), None),
        };
        let simple = match name {
            "calendar_writable" => Some(Self::CalendarWritable),
            "dossier_empty" => Some(Self::DossierEmpty),
            "has_activities" => Some(Self::HasActivities),
            "has_recovery_source" => Some(Self::HasRecoverySource),
            "no_provider" => Some(Self::NoProvider),
            "no_season" => Some(Self::NoSeason),
            "pillars_done" => Some(Self::PillarsDone),
            "plan_active" => Some(Self::PlanActive),
            "season_set" => Some(Self::SeasonSet),
            "weeks_of_data" => None,
            other => return Err(format!("unknown predicate '{other}'")),
        };
        match (simple, threshold) {
            (Some(predicate), None) => Ok(predicate),
            (Some(_), Some(_)) => Err(format!("'{text}': {name} takes no threshold")),
            (None, None) => Err(format!("'{text}': {name} takes a >=N threshold")),
            (None, Some(n)) => n
                .parse()
                .map(Self::WeeksOfData)
                .map_err(|_| format!("'{text}': the threshold is not a whole number")),
        }
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WeeksOfData(n) => write!(f, "{}>={n}", self.name()),
            other => f.write_str(other.name()),
        }
    }
}

/// One `requires:` entry: it holds when any of its predicates does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyOf(Vec<Predicate>);

impl AnyOf {
    /// Whether any alternative holds under `holds`.
    pub fn holds(&self, holds: impl Fn(Predicate) -> bool) -> bool {
        self.0.iter().any(|p| holds(*p))
    }
}

/// What tapping a starter does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseCaseRun {
    /// Send the localized `use_cases.<id>.prompt` as the athlete's message.
    Prompt,
    /// Run this slash command, arguments included (`/plan today`).
    Command(String),
}

/// The stage of an athlete's first days a starter is meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The account is under 24 hours old.
    FirstSession,
    /// The account is under 7 days old.
    FirstWeek,
    /// Any time.
    Any,
}

/// Whether a tap retires a starter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    /// Gone after the first tap: a setup step.
    Once,
    /// Stays until shown three times without a tap.
    Recurring,
}

/// Which agents' welcomes may offer a starter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Domains {
    /// A setup starter any agent may put in its first slot.
    Any,
    /// Only agents of these categories.
    Categories(Vec<AgentCategory>),
}

impl Domains {
    /// Whether an agent of `category` may offer the starter on topic.
    /// [`Domains::Any`] is the cross-domain slot, so it answers `false` here.
    #[must_use]
    pub fn includes(&self, category: AgentCategory) -> bool {
        match self {
            Self::Any => false,
            Self::Categories(categories) => categories.contains(&category),
        }
    }
}

/// Whether `id` has a catalogue id's shape.
///
/// Lowercase ASCII letters, digits and underscores, starting with a letter.
/// The catalogue holds its ids to it and a `uc:` postback parses only such an
/// id, so typed text never reads as a tap.
#[must_use]
pub fn is_use_case_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// One starter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCase {
    /// Stable `snake_case` id: stored per athlete, sent in the postback.
    pub id: String,
    /// What a tap does.
    pub run: UseCaseRun,
    /// Every entry must hold for the starter to be offered.
    pub requires: Vec<AnyOf>,
    /// The stage it is meant for.
    pub stage: Stage,
    /// Which agents may offer it.
    pub domains: Domains,
    /// Whether a tap retires it.
    pub repeat: Repeat,
}

impl UseCase {
    /// The string key of the button label.
    #[must_use]
    pub fn label_key(&self) -> String {
        format!("use_cases.{}.label", self.id)
    }

    /// The string key of the prompt a [`UseCaseRun::Prompt`] starter sends.
    #[must_use]
    pub fn prompt_key(&self) -> String {
        format!("use_cases.{}.prompt", self.id)
    }

    /// Whether every requirement holds under `holds`.
    pub fn eligible(&self, holds: impl Fn(Predicate) -> bool) -> bool {
        self.requires.iter().all(|any_of| any_of.holds(&holds))
    }
}

/// The parsed catalogue, in priority order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UseCaseCatalogue {
    entries: Vec<UseCase>,
}

impl UseCaseCatalogue {
    /// Parse and validate the catalogue text.
    ///
    /// # Errors
    ///
    /// Returns [`ContremaitreError::ManifestParse`] naming the entry when the
    /// text is not a list of well-formed entries, an id repeats or is not
    /// `snake_case`, a postback would exceed 64 bytes, `run` and `command`
    /// disagree, or a predicate or domain is unknown.
    pub fn parse(yaml: &str) -> Result<Self, ContremaitreError> {
        let raw: Vec<RawUseCase> = serde_yaml::from_str(yaml)
            .map_err(|e| ContremaitreError::ManifestParse(format!("use-case catalogue: {e}")))?;
        let mut seen = HashSet::new();
        let entries = raw
            .into_iter()
            .map(|entry| {
                if !seen.insert(entry.id.clone()) {
                    return Err(format!("the id '{}' repeats", entry.id));
                }
                entry.validate()
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(|e| ContremaitreError::ManifestParse(format!("use-case catalogue: {e}")))?;
        Ok(Self { entries })
    }

    /// The catalogue compiled in from the pinned dravr-contremaitre.
    ///
    /// Parsed once. A catalogue that fails to load is logged at `error!` and
    /// reads as empty, so welcomes fall back to the agents' own examples; the
    /// server's `use_case_catalogue_test` loads the pinned one, so that error
    /// cannot pass CI.
    #[must_use]
    pub fn pinned() -> &'static Self {
        &PINNED
    }

    /// Every starter, in catalogue (priority) order.
    #[must_use]
    pub fn entries(&self) -> &[UseCase] {
        &self.entries
    }

    /// One starter by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&UseCase> {
        self.entries.iter().find(|e| e.id == id)
    }
}

/// An entry as the file writes it, before validation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUseCase {
    id: String,
    run: RawRun,
    command: Option<String>,
    requires: Vec<String>,
    stage: Stage,
    domains: Vec<String>,
    repeat: Repeat,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawRun {
    Prompt,
    Command,
}

impl RawUseCase {
    fn validate(self) -> Result<UseCase, String> {
        let id = self.id;
        if !is_use_case_id(&id) {
            return Err(format!("the id '{id}' is not snake_case"));
        }
        if format!("uc:9:{id}").len() > MAX_POSTBACK_BYTES {
            return Err(format!(
                "{id}: its postback exceeds {MAX_POSTBACK_BYTES} bytes"
            ));
        }
        let run = match (self.run, self.command) {
            (RawRun::Prompt, None) => UseCaseRun::Prompt,
            (RawRun::Command, Some(command)) if command.starts_with('/') && command.len() > 1 => {
                UseCaseRun::Command(command)
            }
            (RawRun::Command, Some(command)) => {
                return Err(format!("{id}: '{command}' is not a slash command"))
            }
            (RawRun::Prompt, Some(_)) => {
                return Err(format!("{id}: a prompt starter has no command"))
            }
            (RawRun::Command, None) => {
                return Err(format!("{id}: `run: command` names no command"))
            }
        };
        let requires = self
            .requires
            .iter()
            .map(|entry| {
                entry
                    .split('|')
                    .map(Predicate::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map(AnyOf)
                    .map_err(|e| format!("{id}: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let domains = parse_domains(&self.domains).map_err(|e| format!("{id}: {e}"))?;
        Ok(UseCase {
            id,
            run,
            requires,
            stage: self.stage,
            domains,
            repeat: self.repeat,
        })
    }
}

fn parse_domains(names: &[String]) -> Result<Domains, String> {
    match names {
        [] => Err("`domains` names none".to_owned()),
        [only] if only == "any" => Ok(Domains::Any),
        _ => names
            .iter()
            .map(|name| {
                AgentCategory::ALL
                    .into_iter()
                    .find(|category| category.as_str() == name)
                    .ok_or_else(|| {
                        if name == "any" {
                            "`any` stands alone in `domains`".to_owned()
                        } else {
                            format!("unknown domain '{name}'")
                        }
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Domains::Categories),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    const ENTRY: &str = "
- id: recovered
  run: prompt
  command: null
  requires: [has_recovery_source | weeks_of_data>=2, no_season]
  stage: any
  domains: [recovery, training]
  repeat: recurring
- id: about_me
  run: command
  command: /pillars
  requires: []
  stage: first_session
  domains: [any]
  repeat: once
";

    fn rejects(yaml: &str, why: &str) {
        let err = UseCaseCatalogue::parse(yaml).expect_err("the catalogue should be rejected");
        assert!(err.to_string().contains(why), "{err} should say {why:?}");
    }

    #[test]
    fn the_vocabulary_is_check_nines_list() {
        // A parity check between two artefacts nothing at runtime joins: the
        // list Check 9 judges contremaitre's catalogue by before it is pushed,
        // and the enum this build parses that catalogue into. A predicate in
        // one and not the other passes upstream and fails the load here.
        let listed =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ci/use-case-predicates.txt");
        let text = fs::read_to_string(&listed).expect("use-case-predicates.txt");
        let from_file: BTreeMap<String, bool> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                line.strip_suffix(">=N")
                    .map_or_else(|| (line.to_owned(), false), |name| (name.to_owned(), true))
            })
            .collect();
        let vocabulary: BTreeMap<String, bool> = Predicate::VOCABULARY
            .iter()
            .map(|(name, threshold)| ((*name).to_owned(), *threshold))
            .collect();
        assert_eq!(vocabulary, from_file);
    }

    #[test]
    fn every_name_in_the_vocabulary_parses_back_to_itself() {
        for (name, threshold) in Predicate::VOCABULARY {
            let text = if threshold {
                format!("{name}>=3")
            } else {
                name.to_owned()
            };
            let predicate = Predicate::parse(&text).expect(name);
            assert_eq!(predicate.name(), name);
            assert_eq!(predicate.to_string(), text);
        }
    }

    #[test]
    fn a_catalogue_parses_in_order_with_its_alternatives() {
        let catalogue = UseCaseCatalogue::parse(ENTRY).expect("parses");
        let ids: Vec<&str> = catalogue.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["recovered", "about_me"]);

        let recovered = catalogue.get("recovered").expect("recovered");
        assert_eq!(recovered.run, UseCaseRun::Prompt);
        assert_eq!(
            recovered.requires[0].0,
            [Predicate::HasRecoverySource, Predicate::WeeksOfData(2)]
        );
        assert_eq!(recovered.requires[1].0, [Predicate::NoSeason]);
        assert!(recovered.domains.includes(AgentCategory::Recovery));
        assert!(!recovered.domains.includes(AgentCategory::Nutrition));
        assert_eq!(recovered.prompt_key(), "use_cases.recovered.prompt");

        let about_me = catalogue.get("about_me").expect("about_me");
        assert_eq!(about_me.run, UseCaseRun::Command("/pillars".to_owned()));
        assert_eq!(about_me.domains, Domains::Any);
        assert!(!about_me.domains.includes(AgentCategory::Training));
        assert_eq!(about_me.label_key(), "use_cases.about_me.label");
    }

    #[test]
    fn a_requirement_holds_when_any_alternative_does_and_all_must_hold() {
        let catalogue = UseCaseCatalogue::parse(ENTRY).expect("parses");
        let recovered = catalogue.get("recovered").expect("recovered");
        let enough_weeks =
            |p: Predicate| matches!(p, Predicate::WeeksOfData(2) | Predicate::NoSeason);
        assert!(recovered.eligible(enough_weeks));
        let season_set = |p: Predicate| matches!(p, Predicate::HasRecoverySource);
        assert!(!recovered.eligible(season_set));
        assert!(catalogue
            .get("about_me")
            .expect("about_me")
            .eligible(|_| false));
    }

    #[test]
    fn a_malformed_catalogue_is_rejected_with_its_reason() {
        rejects(
            &ENTRY.replace("no_season", "no_seasons"),
            "unknown predicate 'no_seasons'",
        );
        rejects(
            &ENTRY.replace("weeks_of_data>=2", "weeks_of_data"),
            "takes a >=N threshold",
        );
        rejects(
            &ENTRY.replace("no_season]", "no_season>=2]"),
            "takes no threshold",
        );
        rejects(
            &ENTRY.replace("[recovery, training]", "[recovery, any]"),
            "stands alone",
        );
        rejects(
            &ENTRY.replace("[recovery, training]", "[sleep]"),
            "unknown domain 'sleep'",
        );
        rejects(
            &ENTRY.replace("command: /pillars", "command: null"),
            "names no command",
        );
        rejects(
            &ENTRY.replace("command: null", "command: /status"),
            "has no command",
        );
        rejects(
            &ENTRY.replace("command: /pillars", "command: pillars"),
            "not a slash command",
        );
        rejects(&ENTRY.replace("id: about_me", "id: recovered"), "repeats");
        rejects(
            &ENTRY.replace("id: about_me", "id: About"),
            "not snake_case",
        );
        rejects(
            &ENTRY.replace("repeat: once", "repeat: twice"),
            "use-case catalogue",
        );
        rejects(
            &ENTRY.replace("repeat: once", "repeat: once\n  teaches: /pillars"),
            "teaches",
        );
        rejects(
            &ENTRY.replace("id: about_me", &format!("id: a{}", "b".repeat(60))),
            "exceeds 64 bytes",
        );
    }
}
