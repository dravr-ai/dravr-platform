// ABOUTME: Runs the seeded-fabrication corpus through the claim checker against the compiled-in evidence
// ABOUTME: Pins that every case is tallied, that the harness sees a catch, and that period totals are checked
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_evals::seeded_fabrications::{
    run, CaseOutcome, CatchReport, FabricationCorpus, FabricationKind,
};
use pierre_memory::{ClaimCategory, ClaimStatus, VerdictLayer};

/// The corpus is the data under test, so the test reads the fixture file the
/// binary reads.
const FIXTURE: &str = include_str!("../fixtures/seeded_fabrications.json");

async fn judge_less_run() -> (FabricationCorpus, CatchReport) {
    let corpus = FabricationCorpus::from_json(FIXTURE).unwrap();
    let evidence = EvidenceCorpus::compiled_in().unwrap();
    assert!(!evidence.is_empty(), "the compiled-in corpus parsed empty");
    let report = run(&corpus, &evidence, None).await.unwrap();
    (corpus, report)
}

#[tokio::test]
async fn every_seeded_case_runs_and_lands_in_one_tally() {
    let (corpus, report) = judge_less_run().await;
    assert!(corpus.cases.len() >= 30, "{} cases", corpus.cases.len());
    assert_eq!(report.cases.len(), corpus.cases.len());
    assert!(!report.judge);

    // Every kind the issue names is seeded at least five times.
    assert_eq!(report.by_kind.len(), FabricationKind::ALL.len());
    for kind in &report.by_kind {
        assert!(
            kind.tally.cases >= 5,
            "{:?}: {}",
            kind.kind,
            kind.tally.cases
        );
    }

    let overall = &report.overall;
    let per_kind: usize = report.by_kind.iter().map(|k| k.tally.cases).sum();
    assert_eq!(per_kind, overall.cases);
    assert_eq!(
        overall.caught + overall.judge_decides + overall.missed + overall.not_extracted,
        overall.cases
    );
    assert_eq!(overall.catch_rate.numerator, overall.caught as u64);
    assert_eq!(
        overall.catch_rate_ceiling.numerator,
        (overall.caught + overall.judge_decides) as u64
    );
}

/// The harness has to be able to see a catch, or a 0% would be
/// indistinguishable from a broken harness. A specific figure stated about an
/// athlete with nothing connected is the checker's one certain contradiction.
#[tokio::test]
async fn a_figure_stated_about_an_athlete_with_nothing_connected_is_caught() {
    let (_, report) = judge_less_run().await;
    for id in ["F27", "F31"] {
        let case = report.cases.iter().find(|c| c.id == id).unwrap();
        assert_eq!(case.outcome, CaseOutcome::Caught, "{case:#?}");
        assert!(
            case.verdicts.iter().any(|v| {
                v.layer == VerdictLayer::AthleteData && v.status == ClaimStatus::Contradicted
            }),
            "{case:#?}"
        );
    }
}

/// Every connected corpus athlete is fully synced, so the period check runs on
/// the corpus: a weekly or monthly total above what the synced record holds,
/// under every reading of its period, is contradicted by the athlete-data
/// layer, which names the total the record holds.
///
/// The other two weekly-total fabrications stay uncaught upstream of that
/// check. F09 also plans the rest of the week ("il te reste la sortie longue
/// de dimanche"), so the extractor files it as a prescription and it never
/// reaches the athlete-data layer; naming Sunday, it would be declined by the
/// period check anyway, since a figure tied to a day is that day's. The
/// extractor draws no claim at all from F12's climbing total.
#[tokio::test]
async fn a_period_total_above_the_synced_record_is_caught() {
    let (_, report) = judge_less_run().await;
    let case = |id: &str| report.cases.iter().find(|c| c.id == id).unwrap();

    for (id, held) in [
        ("F07", "53.7 km"),
        ("F08", "281 km"),
        ("F10", "303.1 km"),
        ("F11", "181.7 km"),
    ] {
        let case = case(id);
        assert_eq!(case.kind, FabricationKind::WrongWeeklyTotal);
        assert_eq!(case.outcome, CaseOutcome::Caught, "{case:#?}");
        assert!(
            case.verdicts.iter().any(|v| {
                v.layer == VerdictLayer::AthleteData
                    && v.status == ClaimStatus::Contradicted
                    && v.explanation.contains(&format!("the record holds {held}"))
            }),
            "{case:#?}"
        );
    }

    // The reasons stated above for the two left uncaught: when either
    // changes, so does that paragraph.
    let f09 = case("F09");
    assert!(
        f09.verdicts
            .iter()
            .all(|v| v.category != ClaimCategory::AthleteData),
        "{f09:#?}"
    );
    assert_eq!(case("F12").outcome, CaseOutcome::NotExtracted);

    // The kind's tally, and the report line rendering it, count the catches.
    let weekly = &report
        .by_kind
        .iter()
        .find(|k| k.kind == FabricationKind::WrongWeeklyTotal)
        .unwrap()
        .tally;
    assert!(weekly.caught >= 4, "{weekly:#?}");
    let text = report.render();
    let line = text
        .lines()
        .find(|l| l.starts_with(FabricationKind::WrongWeeklyTotal.as_str()))
        .unwrap();
    assert!(
        line.contains(&format!("caught {}/{}", weekly.caught, weekly.cases)),
        "{line}"
    );
}

#[tokio::test]
async fn each_case_carries_the_verdicts_on_its_false_sentence() {
    let (_, report) = judge_less_run().await;
    for case in &report.cases {
        if case.outcome == CaseOutcome::NotExtracted {
            assert!(case.verdicts.is_empty(), "{case:#?}");
        } else {
            assert!(!case.verdicts.is_empty(), "{case:#?}");
            for verdict in &case.verdicts {
                assert!(case
                    .fabricated
                    .contains(verdict.claim.trim_end_matches(['.', '!'])));
            }
        }
    }
}

#[tokio::test]
async fn the_rendered_report_has_a_line_per_case_and_per_kind() {
    let (corpus, report) = judge_less_run().await;
    let text = report.render();
    for case in &corpus.cases {
        assert!(text.contains(&case.id), "{} missing", case.id);
    }
    for kind in FabricationKind::ALL {
        assert!(text.contains(kind.as_str()), "{kind:?} missing");
    }
    assert!(text.contains("overall"));
}
