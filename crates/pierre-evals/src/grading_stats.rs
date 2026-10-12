// ABOUTME: Interval and agreement statistics for blind grading — Wilson score intervals and Cohen's kappa
// ABOUTME: Pure functions over counts, so a grading report can be recomputed from its own numbers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Grading statistics
//!
//! Two estimators the claim-checker measurement reports with (carnet#854):
//!
//! - [`Proportion`] — a rate with its 95% Wilson score interval. Wilson rather
//!   than the normal approximation because the rates this measures sit near 0
//!   or 1 on small denominators (a layer with nine false claims), where the
//!   normal interval runs outside `[0, 1]` and collapses to a point at 0/n.
//! - `cohens_kappa` — chance-corrected agreement between two graders over
//!   the items both graded.
//!
//! [`Proportion`]: crate::grading_stats::Proportion

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

/// The 0.975 quantile of the standard normal distribution: the `z` of a
/// two-sided 95% interval. A spec constant of the normal distribution.
pub(crate) const Z_95: f64 = 1.959_963_984_540_054;

/// A rate over a count, with its 95% Wilson score interval.
///
/// `rate` and `ci95` are `None` when the denominator is zero: an empty
/// denominator has no rate, and reporting `0.0` would read as a measured zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Proportion {
    /// Items counted as the event.
    pub numerator: u64,
    /// Items the rate is taken over.
    pub denominator: u64,
    /// `numerator / denominator`, when the denominator is not zero.
    pub rate: Option<f64>,
    /// The 95% Wilson score interval `(low, high)`, when the denominator is
    /// not zero.
    pub ci95: Option<(f64, f64)>,
}

impl Proportion {
    /// The rate of `numerator` events in `denominator` trials.
    ///
    /// A numerator above the denominator is a caller bug; it is clamped to the
    /// denominator so the interval stays inside `[0, 1]`.
    #[must_use]
    pub(crate) fn new(numerator: u64, denominator: u64) -> Self {
        let numerator = numerator.min(denominator);
        Self {
            numerator,
            denominator,
            rate: (denominator > 0).then(|| numerator as f64 / denominator as f64),
            ci95: wilson_interval(numerator, denominator, Z_95),
        }
    }
}

/// `9/31  29.0% [16.1–46.6]`, or `0/0  —` with nothing to divide. Padded to
/// the formatter's width, so a report can set it in a column.
impl fmt::Display for Proportion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match (self.rate, self.ci95) {
            (Some(rate), Some((lo, hi))) => format!(
                "{}/{}  {:.1}% [{:.1}–{:.1}]",
                self.numerator,
                self.denominator,
                rate * 100.0,
                lo * 100.0,
                hi * 100.0
            ),
            _ => format!("{}/{}  —", self.numerator, self.denominator),
        };
        f.pad(&text)
    }
}

/// The Wilson score interval for `successes` in `trials` at quantile `z`.
///
/// `None` when `trials` is zero. The bounds are clamped to `[0, 1]` against
/// floating-point overshoot at the extremes.
#[must_use]
pub(crate) fn wilson_interval(successes: u64, trials: u64, z: f64) -> Option<(f64, f64)> {
    if trials == 0 {
        return None;
    }
    let n = trials as f64;
    let p = successes.min(trials) as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denominator;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    Some(((centre - half).max(0.0), (centre + half).min(1.0)))
}

/// Cohen's kappa over paired labels from two graders.
///
/// Each pair is one item, as `(first grader's label, second grader's label)`.
/// Returns `None` when there are no pairs, or when chance agreement is total —
/// both graders used one and the same label for every item — where kappa is
/// `0 / 0` and has no honest value.
#[must_use]
pub(crate) fn cohens_kappa<T: Ord + Clone>(pairs: &[(T, T)]) -> Option<f64> {
    if pairs.is_empty() {
        return None;
    }
    let n = pairs.len() as f64;
    let agreed = pairs.iter().filter(|(a, b)| a == b).count() as f64;
    let observed = agreed / n;

    let mut first: BTreeMap<T, u64> = BTreeMap::new();
    let mut second: BTreeMap<T, u64> = BTreeMap::new();
    for (a, b) in pairs {
        *first.entry(a.clone()).or_default() += 1;
        *second.entry(b.clone()).or_default() += 1;
    }
    let expected: f64 = first
        .iter()
        .map(|(label, count)| {
            let other = second.get(label).copied().unwrap_or(0);
            (*count as f64 / n) * (other as f64 / n)
        })
        .sum();

    let headroom = 1.0 - expected;
    if headroom.abs() < f64::EPSILON {
        return None;
    }
    Some((observed - expected) / headroom)
}

#[cfg(test)]
mod tests {
    use std::iter::repeat_n;

    use super::*;

    /// Equal to four decimals — every reference value below is quoted to four.
    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 5e-5
    }

    #[test]
    fn wilson_matches_reference_values() {
        // Reference values from the closed form, quoted to four decimals.
        let (lo, hi) = wilson_interval(5, 10, Z_95).unwrap();
        assert!(close(lo, 0.2366) && close(hi, 0.7634), "{lo} {hi}");

        let (lo, hi) = wilson_interval(1, 20, Z_95).unwrap();
        assert!(close(lo, 0.0089) && close(hi, 0.2361), "{lo} {hi}");
    }

    #[test]
    fn wilson_stays_open_at_the_extremes() {
        // The reason Wilson is used: 0/10 still has an upper bound and 10/10 a
        // lower one, where the normal approximation collapses to a point.
        let (lo, hi) = wilson_interval(0, 10, Z_95).unwrap();
        assert!(close(lo, 0.0) && close(hi, 0.2775), "{lo} {hi}");
        let (lo, hi) = wilson_interval(10, 10, Z_95).unwrap();
        assert!(close(lo, 0.7225) && close(hi, 1.0), "{lo} {hi}");
    }

    #[test]
    fn wilson_narrows_as_the_sample_grows() {
        let (lo_small, hi_small) = wilson_interval(3, 10, Z_95).unwrap();
        let (lo_large, hi_large) = wilson_interval(300, 1000, Z_95).unwrap();
        assert!(hi_large - lo_large < hi_small - lo_small);
        assert!(lo_large < 0.3 && hi_large > 0.3);
    }

    #[test]
    fn an_empty_denominator_has_no_rate() {
        assert_eq!(wilson_interval(0, 0, Z_95), None);
        let p = Proportion::new(0, 0);
        assert_eq!(p.rate, None);
        assert_eq!(p.ci95, None);
    }

    #[test]
    fn proportion_carries_rate_and_interval() {
        let p = Proportion::new(3, 12);
        assert_eq!((p.numerator, p.denominator), (3, 12));
        assert!(close(p.rate.unwrap(), 0.25));
        let (lo, hi) = p.ci95.unwrap();
        assert!(lo < 0.25 && hi > 0.25);
    }

    #[test]
    fn a_proportion_prints_its_rate_and_interval_or_a_dash() {
        assert_eq!(
            Proportion::new(5, 10).to_string(),
            "5/10  50.0% [23.7–76.3]"
        );
        assert_eq!(Proportion::new(0, 0).to_string(), "0/0  —");
        // A column width pads the whole figure, as the report tables set it.
        assert_eq!(format!("{:<8}|", Proportion::new(0, 0)), "0/0  —  |");
    }

    #[test]
    fn a_numerator_above_its_denominator_is_clamped() {
        let p = Proportion::new(7, 5);
        assert_eq!(p.numerator, 5);
        assert!(close(p.rate.unwrap(), 1.0));
    }

    #[test]
    fn kappa_matches_the_textbook_two_by_two() {
        // 50 items: yes/yes 20, yes/no 5, no/yes 10, no/no 15. Observed
        // agreement 0.70, chance agreement 0.50, kappa 0.40.
        let mut pairs = Vec::new();
        pairs.extend(repeat_n(("yes", "yes"), 20));
        pairs.extend(repeat_n(("yes", "no"), 5));
        pairs.extend(repeat_n(("no", "yes"), 10));
        pairs.extend(repeat_n(("no", "no"), 15));
        assert!(close(cohens_kappa(&pairs).unwrap(), 0.4));
    }

    #[test]
    fn kappa_is_one_for_perfect_agreement_across_labels() {
        let pairs = [("true", "true"), ("false", "false"), ("na", "na")];
        assert!(close(cohens_kappa(&pairs).unwrap(), 1.0));
    }

    #[test]
    fn kappa_is_negative_when_graders_disagree_more_than_chance() {
        let pairs = [("true", "false"), ("false", "true")];
        assert!(cohens_kappa(&pairs).unwrap() < 0.0);
    }

    #[test]
    fn kappa_is_undefined_without_pairs_or_without_variation() {
        assert_eq!(cohens_kappa::<&str>(&[]), None);
        // Both graders said "true" every time: chance agreement is total.
        assert_eq!(cohens_kappa(&[("true", "true"), ("true", "true")]), None);
    }
}
