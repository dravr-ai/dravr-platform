// ABOUTME: A similarity score held as an integer per-mille, so thresholds compare exactly
// ABOUTME: Parses and prints as a decimal ("0.82"), and computes Jaccard and overlap ratios
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::str::FromStr;

/// Thousandths in a whole score.
const THOUSANDTHS: u16 = 1000;

/// Decimal places a score is written and parsed with: one per power of ten in
/// [`THOUSANDTHS`].
const DECIMALS: usize = 3;

/// A score between 0 and 1, kept in thousandths.
///
/// Integer thousandths keep threshold checks exact: `0.82` admits a pair at
/// exactly 0.820 on every platform, and no float rounding decides a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Similarity(u16);

impl Similarity {
    /// The highest score: identical sets.
    pub(crate) const ONE: Self = Self(THOUSANDTHS);

    /// `part / whole`, rounded down to a thousandth. An empty whole scores 0.
    #[must_use]
    pub(crate) fn ratio(part: usize, whole: usize) -> Self {
        let (part, whole) = (part as u64, whole as u64);
        if whole == 0 {
            return Self(0);
        }
        let thousandths = (part.min(whole) * u64::from(THOUSANDTHS)) / whole;
        Self(u16::try_from(thousandths).unwrap_or(u16::MAX)).min(Self::ONE)
    }

    /// The smallest count `n` with `n / total >= self`: how much of a set of
    /// `total` items must be shared for a pair to reach this score.
    #[must_use]
    pub(crate) fn min_count(self, total: usize) -> usize {
        let needed = (total as u64 * u64::from(self.0)).div_ceil(u64::from(THOUSANDTHS));
        usize::try_from(needed).unwrap_or(usize::MAX)
    }

    /// True when `part / whole` reaches this score, computed without rounding.
    #[must_use]
    pub(crate) fn admits(self, part: usize, whole: usize) -> bool {
        whole > 0 && (part as u64) * u64::from(THOUSANDTHS) >= (whole as u64) * u64::from(self.0)
    }
}

impl fmt::Display for Similarity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{:0width$}",
            self.0 / THOUSANDTHS,
            self.0 % THOUSANDTHS,
            width = DECIMALS
        )
    }
}

/// Why a score string was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a score is a decimal from 0 to 1 with at most three decimals, like 0.82; got {0:?}")]
pub struct ParseSimilarityError(String);

impl FromStr for Similarity {
    type Err = ParseSimilarityError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = || ParseSimilarityError(text.to_owned());
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        if fraction.len() > DECIMALS || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refuse());
        }
        let whole: u16 = whole.parse().map_err(|_| refuse())?;
        let padded = format!("{fraction:0<DECIMALS$}");
        let fraction: u16 = padded.parse().map_err(|_| refuse())?;
        let value = whole
            .checked_mul(THOUSANDTHS)
            .and_then(|w| w.checked_add(fraction))
            .ok_or_else(refuse)?;
        let score = Self(value);
        if score > Self::ONE {
            return Err(refuse());
        }
        Ok(score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_decimal_scores() {
        let score: Similarity = "0.82".parse().unwrap();
        assert_eq!(
            score.to_string(),
            "0.820",
            "a parsed score prints in thousandths"
        );
        assert_eq!(
            "1".parse::<Similarity>().unwrap(),
            Similarity::ONE,
            "1 is the top score"
        );
        assert_eq!(
            "0.5".parse::<Similarity>().unwrap().to_string(),
            "0.500",
            "short fractions pad"
        );
    }

    #[test]
    fn refuses_scores_outside_zero_to_one() {
        for bad in ["1.001", "2", "-0.5", "0.8125", "abc", "0.x"] {
            assert!(bad.parse::<Similarity>().is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn ratio_rounds_down_and_caps_at_one() {
        assert_eq!(
            Similarity::ratio(2, 3).to_string(),
            "0.666",
            "2/3 rounds down"
        );
        assert_eq!(
            Similarity::ratio(5, 5),
            Similarity::ONE,
            "equal sets score one"
        );
        assert_eq!(
            Similarity::ratio(3, 0).to_string(),
            "0.000",
            "an empty whole scores zero"
        );
    }

    #[test]
    fn admits_exactly_at_the_threshold() {
        let threshold: Similarity = "0.82".parse().unwrap();
        assert!(threshold.admits(82, 100), "82/100 reaches 0.82");
        assert!(!threshold.admits(81, 100), "81/100 falls short of 0.82");
        assert_eq!(
            threshold.min_count(100),
            82,
            "82 of 100 is the least that reaches 0.82"
        );
        assert_eq!(threshold.min_count(10), 9, "8.2 of 10 rounds up to 9");
    }
}
