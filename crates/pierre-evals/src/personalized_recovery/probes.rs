// ABOUTME: Findings of the overnight-recovery probes — HRV, resting HR and sleep figures checked against the readings
// ABOUTME: Each finding is a stated value or change with the tolerance call on it; the parent module settles them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::sentence::{
    hrv_anchor, nearness, resting_hr_anchor, sleep_anchor, Direction, Sentence, Span, Unit,
};
use super::vocab::SLEEP_EXCLUSIONS;
use super::{
    across_views, assess_against, nights_across_views, Band, Finding, NightlyMetric, View,
    CHANGE_BAND, HRV_BAND_FRAC, RESTING_HR_BAND_BPM, SLEEP_BAND_HOURS,
};
use crate::personalized::{PersonalizedContext, ToleranceCall};

/// A metric stated as one number — HRV in ms, resting HR in bpm.
pub(super) struct ReadingProbe {
    /// The name the explanation gives the metric.
    label: &'static str,
    /// Where a sentence names the metric.
    anchor: fn(&Sentence<'_>) -> Option<Span>,
    /// The unit a figure may carry; a bare figure is accepted after a linking word.
    unit: Unit,
    /// How the explanation prints the unit.
    unit_label: &'static str,
    /// The accepted band around a reading.
    band: Band,
}

pub(super) const HRV_PROBE: ReadingProbe = ReadingProbe {
    label: "HRV",
    anchor: hrv_anchor,
    unit: Unit::Millis,
    unit_label: "ms",
    band: Band::Relative(HRV_BAND_FRAC),
};

pub(super) const RESTING_HR_PROBE: ReadingProbe = ReadingProbe {
    label: "resting HR",
    anchor: resting_hr_anchor,
    unit: Unit::Bpm,
    unit_label: "bpm",
    band: Band::Absolute(RESTING_HR_BAND_BPM),
};

/// What a sentence says about an HRV or resting-HR reading: its percent
/// change, its value, or both.
pub(super) fn reading_findings(
    sentence: &Sentence<'_>,
    views: &[View],
    probe: &ReadingProbe,
    ctx: &PersonalizedContext<'_>,
) -> Vec<Finding> {
    let Some(anchor) = (probe.anchor)(sentence).filter(|_| !views.is_empty()) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    if let Some(finding) = change_finding(sentence, anchor, views, probe.label, ctx) {
        findings.push(finding);
    }
    let value = sentence
        .figures_near(anchor)
        .filter_map(|i| {
            let value = sentence.number(i)?;
            let unit = sentence.unit_after(i);
            let explicit = unit == probe.unit;
            let end = if explicit { i + 2 } else { i + 1 };
            ((explicit || unit == Unit::Bare)
                && value > 0.0
                && sentence.is_reading_position(i, end, anchor, explicit))
            .then_some(((i, end), value))
        })
        .min_by_key(|&((i, _), _)| nearness(i, anchor));
    if let Some((figure, claimed)) = value {
        let fmt = |v: f64| format!("{v:.0} {}", probe.unit_label);
        let scope = sentence.scope_of(figure);
        if let Some(finding) = value_finding(
            ctx,
            &across_views(views, |metric| metric.references(scope)),
            &nights_across_views(views, scope, sentence.names_an_earlier_night()),
            claimed,
            probe.band,
            &fmt,
            probe.label,
        ) {
            findings.push(finding);
        }
    }
    findings
}

/// What a sentence says about the athlete's sleep: its duration, its percent
/// change, or both.
pub(super) fn sleep_findings(
    sentence: &Sentence<'_>,
    views: &[View],
    ctx: &PersonalizedContext<'_>,
) -> Vec<Finding> {
    let Some(anchor) = sleep_anchor(sentence).filter(|_| !views.is_empty()) else {
        return Vec::new();
    };
    if sentence.has_phrase(SLEEP_EXCLUSIONS) {
        return Vec::new();
    }
    let mut findings = Vec::new();
    if let Some(finding) = change_finding(sentence, anchor, views, "sleep", ctx) {
        findings.push(finding);
    }
    let duration = sentence
        .figures_near(anchor)
        .filter_map(|i| {
            let (hours, end) = sentence.duration_at(i)?;
            sentence
                .is_reading_position(i, end, anchor, true)
                .then_some(((i, end), hours))
        })
        .min_by_key(|&((i, _), _)| nearness(i, anchor));
    if let Some((figure, claimed)) = duration {
        let scope = sentence.scope_of(figure);
        if let Some(finding) = value_finding(
            ctx,
            &across_views(views, |metric| metric.references(scope)),
            &nights_across_views(views, scope, sentence.names_an_earlier_night()),
            claimed,
            Band::Absolute(SLEEP_BAND_HOURS),
            &fmt_hours,
            "sleep",
        ) {
            findings.push(finding);
        }
    }
    findings
}

/// Score a stated value against the readings its scope names.
///
/// A value no reading bears out is still not contradicted while it sits
/// within `nights`, the lowest-to-highest span of the week's nights its scope
/// or the sentence's earlier night leaves open: it may be about one of them
/// ("you slept 5 h 40 on Wednesday"), and those nights are not held one by
/// one.
fn value_finding(
    ctx: &PersonalizedContext<'_>,
    references: &[(f64, String)],
    nights: &[(f64, f64)],
    claimed: f64,
    band: Band,
    fmt: &dyn Fn(f64) -> String,
    label: &str,
) -> Option<Finding> {
    if references.is_empty() {
        return None;
    }
    let mut call = assess_against(
        ctx,
        claimed,
        references.iter().map(|&(reading, _)| band.around(reading)),
    );
    let within_a_night = nights.iter().any(|&(lowest, highest)| {
        let span = (band.around(lowest).0, band.around(highest).1);
        ctx.tolerance.assess(claimed, span) != ToleranceCall::Contradicted
    });
    if call == ToleranceCall::Contradicted && within_a_night {
        call = ToleranceCall::Indeterminate;
    }
    let yours = references
        .iter()
        .map(|(reading, name)| format!("{} {name}", fmt(*reading)))
        .collect::<Vec<_>>()
        .join(", ");
    Some(Finding {
        call,
        descr: format!("{label} {} vs your {yours}", fmt(claimed)),
    })
}

/// Score a stated percent change against the last night's change from each
/// mean. `None` when the sentence states no percent figure near the metric,
/// leaves its direction unclear, names an earlier night the change may be
/// about ("it dropped 18% on Tuesday, then bounced back"), or the readings
/// hold no change.
fn change_finding(
    sentence: &Sentence<'_>,
    anchor: Span,
    views: &[View],
    label: &str,
    ctx: &PersonalizedContext<'_>,
) -> Option<Finding> {
    if sentence.names_an_earlier_night() {
        return None;
    }
    let percent = sentence
        .figures_near(anchor)
        .filter(|&i| sentence.unit_after(i) == Unit::Percent)
        .min_by_key(|&i| nearness(i, anchor))
        .and_then(|i| sentence.number(i))?;
    let signed = match (percent < 0.0, sentence.direction()) {
        // A signed figure keeps its sign unless the words say the opposite.
        (true, Some(Direction::Up)) | (false, None) => return None,
        (true, _) | (false, Some(Direction::Up)) => percent,
        (false, Some(Direction::Down)) => -percent,
    };
    let ratios = across_views(views, NightlyMetric::change_ratios);
    if ratios.is_empty() {
        return None;
    }
    let call = assess_against(
        ctx,
        1.0 + signed / 100.0,
        ratios
            .iter()
            .map(|&(ratio, _)| (ratio - CHANGE_BAND, ratio + CHANGE_BAND)),
    );
    let yours = ratios
        .iter()
        .map(|(ratio, name)| format!("{:+.0}% against {name}", (ratio - 1.0) * 100.0))
        .collect::<Vec<_>>()
        .join(", ");
    Some(Finding {
        call,
        descr: format!("{label} change {signed:+.0}% vs your last night's {yours}"),
    })
}

/// Sleep hours as the explanation prints them: `6 h 30`, `8 h`.
fn fmt_hours(hours: f64) -> String {
    let minutes = (hours * 60.0).round().max(0.0) as u64;
    match (minutes / 60, minutes % 60) {
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02}"),
    }
}
