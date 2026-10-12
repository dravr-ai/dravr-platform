// ABOUTME: Scores blind grades against the key — silent-miss and false-positive rates with Wilson intervals, Cohen's kappa
// ABOUTME: Joins graders' CSV sheets to the operator's key; a disagreement waits for adjudication instead of being guessed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Scoring a blind grade (carnet#854)
//!
//! Each grader returns their sheet with every claim marked `true`, `false` or
//! `not_checkable` against the athlete's data. [`score`] joins those grades to
//! the [`GradingKey`] and reports, over the items with a settled grade:
//!
//! - **silent-miss rate** — of the extracted claims graded false, the share
//!   the checker did not flag. The number #854 exists for, with its condition:
//!   the sample is drawn from `claim_verdicts`, so a false sentence the
//!   extractor never turned into a claim is never in it, and this rate reads
//!   below the rate over everything the agents said. Read it beside the
//!   seeded run's not-extracted share (`claim_catch_rate`).
//! - **false-omission rate** — of the checkable claims the checker did not
//!   flag, the share graded false: what an athlete reading an unflagged claim
//!   is exposed to.
//! - **false-positive rate** — of the flagged claims, the share graded true,
//!   with `not_checkable` in the denominator the way #455 counts `unsure`, so
//!   the two rates compare.
//! - each per layer, every rate with its 95% Wilson interval, and Cohen's kappa
//!   for every pair of graders over the items both graded.
//!
//! An item's grade is settled by an adjudication grade when one exists, else
//! by its graders agreeing. Graders who disagree leave it disputed: it is
//! listed and kept out of every rate rather than settled by a rule that would
//! quietly favour one grader.
//!
//! The rates are unweighted because the sample is self-weighting
//! ([`crate::blind_grading`] allocates proportionally) — over the eligible
//! pool, which leaves out every verdict on a reply that read first-party-only
//! provider data. The report carries that [`PoolCoverage`] and prints it with
//! the rates, so the number never travels without what it covers.
//!
//! [`score`]: crate::blind_grading_score::score
//! [`GradingKey`]: crate::blind_grading::GradingKey
//! [`PoolCoverage`]: crate::blind_grading::PoolCoverage

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use pierre_core::errors::{AppError, AppResult};
use pierre_memory::claims::VerdictLayer;
use serde::{Deserialize, Serialize};

use crate::blind_grading::{GradingKey, KeyItem, PoolCoverage};
use crate::grading_csv::parse;
use crate::grading_stats::{cohens_kappa, Proportion};

/// A grader's call on one claim, against the athlete's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// The claim is true of this athlete.
    True,
    /// The claim is false of this athlete.
    False,
    /// The athlete's data cannot settle the claim either way.
    NotCheckable,
}

impl Grade {
    /// Parse a grade cell: `true`, `false` or `not_checkable`, case-insensitive,
    /// with a space or hyphen accepted for the underscore.
    #[must_use]
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
            "true" => Some(Self::True),
            "false" => Some(Self::False),
            "not_checkable" => Some(Self::NotCheckable),
            _ => None,
        }
    }

    const fn checkable(self) -> bool {
        matches!(self, Self::True | Self::False)
    }
}

/// One grader's grades, by item id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraderGrades {
    /// The grader's name, as the operator gave it.
    pub grader: String,
    /// Their grade per item id; an item left blank is absent.
    pub grades: BTreeMap<String, Grade>,
}

/// Read a graded sheet.
///
/// Columns are found by header name (`item_id`, `grade`), so a grader may
/// reorder or add columns. A blank grade is an item not graded yet and is left
/// out.
///
/// # Errors
///
/// An invalid-input error when the sheet has no header or lacks either column,
/// when a grade cell holds anything but the three grades (naming the row), or
/// when one item is graded twice with different grades.
pub fn read_grades(grader: &str, csv: &str) -> AppResult<GraderGrades> {
    let rows = parse(csv)?;
    let header = rows
        .first()
        .ok_or_else(|| AppError::invalid_input(format!("{grader}: the sheet is empty")))?;
    let column = |name: &str| {
        header
            .iter()
            .position(|h| h.trim() == name)
            .ok_or_else(|| AppError::invalid_input(format!("{grader}: no `{name}` column")))
    };
    let (id_col, grade_col) = (column("item_id")?, column("grade")?);

    let mut by_item = BTreeMap::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        let item = row.get(id_col).map_or("", |s| s.trim());
        let cell = row.get(grade_col).map_or("", |s| s.trim());
        if item.is_empty() || cell.is_empty() {
            continue;
        }
        let grade = Grade::parse(cell).ok_or_else(|| {
            AppError::invalid_input(format!(
                "{grader}: row {} grades {item} `{cell}`; use true, false or not_checkable",
                index + 1
            ))
        })?;
        if let Some(previous) = by_item.insert(item.to_owned(), grade) {
            if previous != grade {
                return Err(AppError::invalid_input(format!(
                    "{grader}: {item} is graded twice, differently"
                )));
            }
        }
    }
    Ok(GraderGrades {
        grader: grader.to_owned(),
        grades: by_item,
    })
}

/// The rates for one layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayerScore {
    /// The layer, in its stable form.
    pub layer: String,
    /// Items this layer decided whose grade is settled.
    pub graded: usize,
    /// Of this layer's extracted claims graded false, the share it did not
    /// flag.
    pub silent_miss: Proportion,
    /// Of this layer's flags, the share on claims graded true.
    pub false_positive: Proportion,
}

/// Agreement between two graders over the items both graded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraderAgreement {
    /// The first grader, in name order.
    pub first: String,
    /// The second grader.
    pub second: String,
    /// Items both graded, and how many they graded the same.
    pub agreement: Proportion,
    /// Cohen's kappa over those items; `None` where it is undefined.
    pub kappa: Option<f64>,
}

/// The scored grade.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GradingReport {
    /// Tenant the sample was drawn from.
    pub tenant_id: String,
    /// Window the sample covers, in days.
    pub window_days: i64,
    /// What the pool the sample was drawn from covers of the window: every
    /// rate below holds for that pool only.
    pub coverage: PoolCoverage,
    /// Items in the sample.
    pub items: usize,
    /// Items whose grade is settled.
    pub graded: usize,
    /// Items nobody graded.
    pub ungraded: usize,
    /// Items the graders disagree on and nobody adjudicated, by item id.
    pub disputed: Vec<String>,
    /// Settled items the athlete's data could settle (graded true or false).
    pub checkable: Proportion,
    /// Of the extracted claims graded false, the share the checker did not
    /// flag. Conditional on extraction: a false sentence the extractor never
    /// turned into a claim is never sampled, so this reads below the rate over
    /// everything the agents said.
    pub silent_miss: Proportion,
    /// Of the checkable extracted claims the checker did not flag, the share
    /// graded false.
    pub false_omission: Proportion,
    /// Of the flagged claims, the share graded true (`not_checkable` counted
    /// in the denominator, as #455 counts `unsure`).
    pub false_positive: Proportion,
    /// The same rates per layer, for every layer with a settled item.
    pub by_layer: Vec<LayerScore>,
    /// Agreement for every pair of graders who share an item.
    pub agreement: Vec<GraderAgreement>,
}

/// How an item's grade came out.
enum Settled {
    Graded(Grade),
    Disputed,
    Ungraded,
}

fn settle(item: &str, graders: &[GraderGrades], adjudication: Option<&GraderGrades>) -> Settled {
    if let Some(grade) = adjudication.and_then(|a| a.grades.get(item)) {
        return Settled::Graded(*grade);
    }
    let given: BTreeSet<Grade> = graders
        .iter()
        .filter_map(|g| g.grades.get(item).copied())
        .collect();
    let mut distinct = given.into_iter();
    match (distinct.next(), distinct.next()) {
        (None, _) => Settled::Ungraded,
        (Some(grade), None) => Settled::Graded(grade),
        (Some(_), Some(_)) => Settled::Disputed,
    }
}

/// Every grade must name an item the key holds, and grader names must be
/// distinct: a stray id is a sheet edited by hand, and a repeated name would
/// fold two people into one.
fn check_graders(
    key: &GradingKey,
    graders: &[GraderGrades],
    adjudication: Option<&GraderGrades>,
) -> AppResult<()> {
    let ids: BTreeSet<&str> = key.items.iter().map(|i| i.item_id.as_str()).collect();
    let mut names = BTreeSet::new();
    for sheet in graders.iter().chain(adjudication) {
        if let Some(stray) = sheet.grades.keys().find(|id| !ids.contains(id.as_str())) {
            return Err(AppError::invalid_input(format!(
                "{} grades {stray}, which the key does not hold",
                sheet.grader
            )));
        }
    }
    for sheet in graders {
        if !names.insert(sheet.grader.as_str()) {
            return Err(AppError::invalid_input(format!(
                "two grade sheets are both named {}",
                sheet.grader
            )));
        }
    }
    Ok(())
}

/// The three rates over a set of settled items.
struct Rates {
    silent_miss: Proportion,
    false_omission: Proportion,
    false_positive: Proportion,
}

fn rates(settled: &[(&KeyItem, Grade)]) -> Rates {
    let count = |pred: &dyn Fn(&KeyItem, Grade) -> bool| {
        settled
            .iter()
            .filter(|(item, grade)| pred(item, *grade))
            .count() as u64
    };
    let missed = count(&|i, g| g == Grade::False && !i.flagged);
    Rates {
        silent_miss: Proportion::new(missed, count(&|_, g| g == Grade::False)),
        false_omission: Proportion::new(missed, count(&|i, g| !i.flagged && g.checkable())),
        false_positive: Proportion::new(
            count(&|i, g| i.flagged && g == Grade::True),
            count(&|i, _| i.flagged),
        ),
    }
}

fn agreement(first: &GraderGrades, second: &GraderGrades) -> Option<GraderAgreement> {
    let pairs: Vec<(Grade, Grade)> = first
        .grades
        .iter()
        .filter_map(|(item, a)| second.grades.get(item).map(|b| (*a, *b)))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    let same = pairs.iter().filter(|(a, b)| a == b).count() as u64;
    Some(GraderAgreement {
        first: first.grader.clone(),
        second: second.grader.clone(),
        agreement: Proportion::new(same, pairs.len() as u64),
        kappa: cohens_kappa(&pairs),
    })
}

/// Join the graders' sheets (and an optional adjudication sheet) to the key and
/// compute the report.
///
/// # Errors
///
/// An invalid-input error when a sheet grades an item the key does not hold,
/// or two sheets carry the same grader name.
pub fn score(
    key: &GradingKey,
    graders: &[GraderGrades],
    adjudication: Option<&GraderGrades>,
) -> AppResult<GradingReport> {
    check_graders(key, graders, adjudication)?;

    let mut settled: Vec<(&KeyItem, Grade)> = Vec::new();
    let mut disputed = Vec::new();
    let mut ungraded = 0;
    for item in &key.items {
        match settle(&item.item_id, graders, adjudication) {
            Settled::Graded(grade) => settled.push((item, grade)),
            Settled::Disputed => disputed.push(item.item_id.clone()),
            Settled::Ungraded => ungraded += 1,
        }
    }

    let overall = rates(&settled);
    let by_layer = VerdictLayer::ALL
        .iter()
        .filter_map(|layer| {
            let mine: Vec<(&KeyItem, Grade)> = settled
                .iter()
                .filter(|(item, _)| item.layer == *layer)
                .copied()
                .collect();
            (!mine.is_empty()).then(|| {
                let r = rates(&mine);
                LayerScore {
                    layer: layer.as_str().to_owned(),
                    graded: mine.len(),
                    silent_miss: r.silent_miss,
                    false_positive: r.false_positive,
                }
            })
        })
        .collect();

    let mut sorted: Vec<&GraderGrades> = graders.iter().collect();
    sorted.sort_by(|a, b| a.grader.cmp(&b.grader));
    let pairs = sorted
        .iter()
        .enumerate()
        .flat_map(|(i, a)| sorted.iter().skip(i + 1).map(move |b| (*a, *b)))
        .filter_map(|(a, b)| agreement(a, b))
        .collect();

    let checkable = settled.iter().filter(|(_, g)| g.checkable()).count() as u64;
    Ok(GradingReport {
        tenant_id: key.tenant_id.clone(),
        window_days: key.window_days,
        coverage: key.coverage(),
        items: key.items.len(),
        graded: settled.len(),
        ungraded,
        disputed,
        checkable: Proportion::new(checkable, settled.len() as u64),
        silent_miss: overall.silent_miss,
        false_omission: overall.false_omission,
        false_positive: overall.false_positive,
        by_layer,
        agreement: pairs,
    })
}

impl GradingReport {
    /// The report as plain text for a terminal or a vault note.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Blind grade — tenant {}, {} items over {} days",
            self.tenant_id, self.items, self.window_days
        );
        let _ = writeln!(
            out,
            "settled {} · ungraded {} · disputed {}",
            self.graded,
            self.ungraded,
            self.disputed.len()
        );
        out.push_str(&self.coverage.render());
        let lines = [
            ("checkable", &self.checkable),
            ("silent-miss rate (extracted claims)", &self.silent_miss),
            (
                "false-omission rate (extracted claims)",
                &self.false_omission,
            ),
            ("false-positive rate", &self.false_positive),
        ];
        for (name, p) in lines {
            let _ = writeln!(out, "{name:<39} {p}");
        }
        let _ = writeln!(
            out,
            "A false sentence the extractor never turned into a claim is not in this sample; \
             read the silent-miss rate beside the seeded run's not-extracted share (claim_catch_rate)."
        );
        for layer in &self.by_layer {
            let _ = writeln!(
                out,
                "  {:<14} n={:<4} silent-miss {:<28} false-positive {}",
                layer.layer, layer.graded, layer.silent_miss, layer.false_positive
            );
        }
        for pair in &self.agreement {
            let kappa = pair
                .kappa
                .map_or_else(|| "undefined".to_owned(), |k| format!("{k:.2}"));
            let _ = writeln!(
                out,
                "agreement {} vs {}: {} · kappa {kappa}",
                pair.first, pair.second, pair.agreement
            );
        }
        if !self.disputed.is_empty() {
            let _ = writeln!(out, "awaiting adjudication: {}", self.disputed.join(", "));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use chrono::{TimeZone, Utc};
    use pierre_memory::claims::{ClaimCategory, ClaimStatus, GradingWithheld};

    use super::*;
    use crate::blind_grading::flags;

    fn item(id: &str, layer: VerdictLayer, status: ClaimStatus) -> KeyItem {
        KeyItem {
            item_id: id.to_owned(),
            athlete: "A01".to_owned(),
            sheets: vec![1, 2],
            verdict_id: format!("v-{id}"),
            user_id: "u".to_owned(),
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claimed_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
            locale: "fr".to_owned(),
            claim_text: "claim".to_owned(),
            category: ClaimCategory::AthleteData,
            status,
            layer,
            flagged: flags(status),
        }
    }

    fn key(items: Vec<KeyItem>) -> GradingKey {
        GradingKey {
            tenant_id: "t".to_owned(),
            seed: 1,
            window_start: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            window_days: 30,
            in_window: i64::try_from(items.len()).unwrap(),
            withheld: GradingWithheld::default(),
            eligible: i64::try_from(items.len()).unwrap(),
            pool_read: items.len(),
            sample_size: items.len(),
            overlap: 0,
            strata: Vec::new(),
            items,
        }
    }

    fn grades(grader: &str, pairs: &[(&str, Grade)]) -> GraderGrades {
        GraderGrades {
            grader: grader.to_owned(),
            grades: pairs.iter().map(|(i, g)| ((*i).to_owned(), *g)).collect(),
        }
    }

    /// Ten items: four false claims of which the checker flagged one, three
    /// flags of which one landed on a true claim, one claim nobody could check.
    fn ten() -> (GradingKey, GraderGrades) {
        use ClaimStatus::{Contradicted, Supported, Unverifiable};
        use Grade::{False, NotCheckable, True};
        use VerdictLayer::{AthleteData, Evidence};
        let rows = [
            ("C01", AthleteData, Unverifiable, False),
            ("C02", AthleteData, Unverifiable, False),
            ("C03", AthleteData, Contradicted, False),
            ("C04", AthleteData, Contradicted, True),
            ("C05", AthleteData, Contradicted, NotCheckable),
            ("C06", Evidence, Supported, False),
            ("C07", Evidence, Supported, True),
            ("C08", Evidence, Supported, True),
            ("C09", Evidence, Supported, True),
            ("C10", Evidence, Unverifiable, NotCheckable),
        ];
        let key = key(rows.iter().map(|(id, l, s, _)| item(id, *l, *s)).collect());
        let pairs: Vec<(&str, Grade)> = rows.iter().map(|(id, _, _, g)| (*id, *g)).collect();
        (key, grades("ana", &pairs))
    }

    #[test]
    fn grades_parse_leniently_on_case_and_separator_only() {
        assert_eq!(Grade::parse(" TRUE "), Some(Grade::True));
        assert_eq!(Grade::parse("False"), Some(Grade::False));
        assert_eq!(Grade::parse("not checkable"), Some(Grade::NotCheckable));
        assert_eq!(Grade::parse("Not-Checkable"), Some(Grade::NotCheckable));
        assert_eq!(Grade::parse("yes"), None);
        assert_eq!(Grade::parse("maybe"), None);
    }

    #[test]
    fn a_sheet_is_read_by_header_and_blank_grades_are_skipped() {
        let csv = "note,grade,item_id\r\n\"ran 12, not 21\",false,C01\r\n,,C02\r\n,true,C03\r\n";
        let read = read_grades("ana", csv).unwrap();
        assert_eq!(read.grades.len(), 2);
        assert_eq!(read.grades["C01"], Grade::False);
        assert_eq!(read.grades["C03"], Grade::True);
    }

    #[test]
    fn a_bad_grade_or_a_missing_column_is_refused() {
        let err = read_grades("ana", "item_id,grade\nC01,probably\n").unwrap_err();
        assert!(err.to_string().contains("row 2"), "{err}");
        let err = read_grades("ana", "item_id,verdict\nC01,true\n").unwrap_err();
        assert!(err.to_string().contains("`grade`"), "{err}");
        let err = read_grades("ana", "item_id,grade\nC01,true\nC01,false\n").unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }

    #[test]
    fn the_rates_count_what_the_checker_let_through() {
        let (key, ana) = ten();
        let report = score(&key, &[ana], None).unwrap();
        assert_eq!(report.graded, 10);
        // False claims C01, C02, C03, C06; the checker flagged only C03.
        assert_eq!(
            (report.silent_miss.numerator, report.silent_miss.denominator),
            (3, 4)
        );
        // Unflagged and checkable: C01, C02, C06, C07, C08, C09 — three false.
        assert_eq!(
            (
                report.false_omission.numerator,
                report.false_omission.denominator
            ),
            (3, 6)
        );
        // Flags C03, C04, C05: one on a true claim, the not-checkable one in
        // the denominator as #455 counts `unsure`.
        assert_eq!(
            (
                report.false_positive.numerator,
                report.false_positive.denominator
            ),
            (1, 3)
        );
        assert_eq!(
            (report.checkable.numerator, report.checkable.denominator),
            (8, 10)
        );
    }

    #[test]
    fn the_rates_break_down_by_the_layer_that_decided() {
        let (key, ana) = ten();
        let report = score(&key, &[ana], None).unwrap();
        let layer = |name: &str| report.by_layer.iter().find(|l| l.layer == name).unwrap();
        let data = layer("athlete_data");
        assert_eq!(data.graded, 5);
        assert_eq!(
            (data.silent_miss.numerator, data.silent_miss.denominator),
            (2, 3)
        );
        let evidence = layer("evidence");
        assert_eq!(
            (
                evidence.silent_miss.numerator,
                evidence.silent_miss.denominator
            ),
            (1, 1)
        );
        assert_eq!(evidence.false_positive.denominator, 0);
        assert_eq!(evidence.false_positive.rate, None);
        assert_eq!(report.by_layer.len(), 2);
    }

    #[test]
    fn disagreement_waits_for_adjudication() {
        let (key, ana) = ten();
        let mut ben = ana.clone();
        ben.grader = "ben".to_owned();
        ben.grades.insert("C01".to_owned(), Grade::True);
        ben.grades.remove("C10");

        let report = score(&key, &[ana.clone(), ben.clone()], None).unwrap();
        assert_eq!(report.disputed, vec!["C01"]);
        assert_eq!(report.graded, 9);
        assert_eq!(report.silent_miss.denominator, 3);

        let adjudication = grades("adjudication", &[("C01", Grade::False)]);
        let report = score(&key, &[ana, ben], Some(&adjudication)).unwrap();
        assert!(report.disputed.is_empty());
        assert_eq!(report.silent_miss.denominator, 4);
    }

    #[test]
    fn agreement_is_measured_on_the_shared_items() {
        let (key, ana) = ten();
        let mut ben = ana.clone();
        ben.grader = "ben".to_owned();
        ben.grades.insert("C01".to_owned(), Grade::True);
        let report = score(&key, &[ben, ana], None).unwrap();
        let pair = &report.agreement[0];
        assert_eq!((pair.first.as_str(), pair.second.as_str()), ("ana", "ben"));
        assert_eq!(
            (pair.agreement.numerator, pair.agreement.denominator),
            (9, 10)
        );
        let kappa = pair.kappa.unwrap();
        assert!(kappa > 0.8 && kappa < 1.0, "{kappa}");
    }

    #[test]
    fn a_grade_for_an_item_the_key_lacks_is_refused() {
        let (key, _) = ten();
        let stray = grades("ana", &[("C99", Grade::True)]);
        let err = score(&key, &[stray], None).unwrap_err();
        assert!(err.to_string().contains("C99"), "{err}");
    }

    #[test]
    fn two_sheets_under_one_name_are_refused() {
        let (key, ana) = ten();
        let err = score(&key, &[ana.clone(), ana], None).unwrap_err();
        assert!(err.to_string().contains("both named"), "{err}");
    }

    /// The figures a rendered report prints after `label`, padding trimmed.
    fn figures<'a>(text: &'a str, label: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(label))
            .map_or("", str::trim_start)
    }

    #[test]
    fn an_ungraded_sample_reports_no_rates() {
        let (key, _) = ten();
        let report = score(&key, &[], None).unwrap();
        assert_eq!(report.ungraded, 10);
        assert_eq!(report.silent_miss.rate, None);
        let text = report.render();
        assert_eq!(
            figures(&text, "silent-miss rate (extracted claims)"),
            "0/0  —"
        );
    }

    #[test]
    fn the_rendered_report_names_every_rate() {
        let (key, ana) = ten();
        let text = score(&key, &[ana], None).unwrap().render();
        assert!(
            figures(&text, "silent-miss rate (extracted claims)").starts_with("3/4  75.0%"),
            "{text}"
        );
        assert!(
            figures(&text, "false-positive rate").starts_with("1/3  33.3%"),
            "{text}"
        );
        assert!(text.contains("athlete_data"), "{text}");
    }

    #[test]
    fn the_silent_miss_rate_never_prints_without_its_conditions() {
        let (mut key, ana) = ten();
        let text = score(&key, slice::from_ref(&ana), None).unwrap().render();
        // Conditional on extraction, always.
        assert!(text.contains("never turned into a claim"), "{text}");
        assert!(text.contains("not-extracted share"), "{text}");
        assert!(text.contains("pool: 10 eligible of 10"), "{text}");
        assert!(!text.contains("warning:"), "{text}");

        // A window the first-party-only rule mostly withheld says so.
        key.in_window = 40;
        key.withheld.first_party_only = 30;
        let report = score(&key, &[ana], None).unwrap();
        assert_eq!(report.coverage.withheld.first_party_only, 30);
        let text = report.render();
        assert!(
            text.contains("withheld 30 first-party-only (75.0% of the window)"),
            "{text}"
        );
        assert!(text.contains("warning: most of the window"), "{text}");
    }
}
