// ABOUTME: Strava athlete self-report mapping — Perceived Exertion as the API's 1–10 number or the slider's band label
// ABOUTME: Pure functions the Strava OAuth and sciotte conversions call so a Strava encoding never leaves this module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Strava athlete self-report
//!
//! Strava asks the athlete for one thing about how a session went: its
//! Perceived Exertion, a 1–10 slider running from "Easy" to "Max Effort"
//! (Strava support, "Perceived Exertion", article 15401767; the French page
//! names the ends "Facile" and "Effort max"). Strava records no feeling, so
//! nothing here produces a [`crate::models::Feel`].
//!
//! ## OAuth: the number
//!
//! `GET /activities/{id}` carries `perceived_exertion`, the slider's value, and
//! `null` when the athlete never set it. The field is not in Strava's API
//! reference: stravalib lists it, with `prefer_perceived_exertion`, among the
//! "undocumented attributes" of its `DetailedActivity` (`perceived_exertion:
//! int | None`, stravalib @ `1951f26e`, `src/stravalib/model.py`), and Strava's
//! own MCP server reports it as an integer on its per-activity performance read.
//! The list endpoint does not carry it (intervals.icu forum, "RPE fed from
//! Strava", 2021-02-27 and 2021-09-17), so only the detail read sees it.
//!
//! ## Scraped: the band label
//!
//! A scraped activity page shows the band the slider sits in rather than the
//! number. Strava's help names only the two ends; the four bands below are the
//! ones secondary sources describe the app labelling (sarakurth.com, "Rate of
//! Perceived Exertion: A Guide for Runners"; Runner's World UK, "How to use
//! RPE", 2020-02-01):
//!
//! | label | slider values | CR-10 point |
//! |-------|---------------|-------------|
//! | Easy (Facile) | 1–3 | 2 |
//! | Moderate | 4–6 | 5 |
//! | Hard | 7–9 | 8 |
//! | Max Effort (Effort max) | 10 | 10 |
//!
//! A band becomes its midpoint: the value nearest every rating the athlete
//! could have picked inside it, so the stored point is never more than one
//! step from what they chose. Max Effort is a single value and stays 10.
//! "Modéré" and "Difficile" are the plain French words for the middle bands;
//! no Strava page seen names them, so a French scrape that labels those bands
//! otherwise maps to `None` rather than to a guess. When the scrape carries the
//! slider's number instead, it is read as the rating directly.

/// Strava's `perceived_exertion` as a CR-10 rating, or `None` when it is not
/// a slider value inside 1–10.
///
/// The athlete's rating is kept whether or not they also ticked
/// `prefer_perceived_exertion`: that per-activity choice only decides whether
/// Strava's Relative Effort is computed from the rating instead of heart rate.
#[must_use]
pub(crate) fn rpe_from_strava(rating: f32) -> Option<f32> {
    (1.0..=10.0).contains(&rating).then_some(rating)
}

/// A scraped Strava Perceived Exertion — the slider's number, or the label of
/// the band it sits in — as a CR-10 rating. A band label becomes its midpoint
/// (see the module documentation); anything else is `None`.
#[cfg(feature = "provider-sciotte")]
#[must_use]
pub(crate) fn rpe_from_strava_label(label: &str) -> Option<f32> {
    let label = label.trim();
    if let Ok(rating) = label.parse::<f32>() {
        return rpe_from_strava(rating);
    }
    let words = label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    match words.as_str() {
        "easy" | "facile" => Some(2.0),
        "moderate" | "modéré" => Some(5.0),
        "hard" | "difficile" => Some(8.0),
        "max effort" | "effort max" => Some(10.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slider_value_is_the_rating() {
        assert_eq!(rpe_from_strava(1.0), Some(1.0));
        assert_eq!(rpe_from_strava(6.0), Some(6.0));
        assert_eq!(rpe_from_strava(10.0), Some(10.0));
    }

    #[test]
    fn a_value_off_the_slider_is_no_rating() {
        for rating in [0.0, -1.0, 10.5, 11.0, f32::NAN, f32::INFINITY] {
            assert_eq!(rpe_from_strava(rating), None, "{rating}");
        }
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn each_band_label_becomes_its_midpoint() {
        assert_eq!(rpe_from_strava_label("Easy"), Some(2.0));
        assert_eq!(rpe_from_strava_label("Moderate"), Some(5.0));
        assert_eq!(rpe_from_strava_label("Hard"), Some(8.0));
        assert_eq!(rpe_from_strava_label("Max Effort"), Some(10.0));
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn a_label_is_read_whatever_its_case_and_spacing() {
        assert_eq!(rpe_from_strava_label("  HARD\n"), Some(8.0));
        assert_eq!(rpe_from_strava_label("max   effort"), Some(10.0));
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn the_french_labels_map_like_the_english_ones() {
        assert_eq!(rpe_from_strava_label("Facile"), Some(2.0));
        assert_eq!(rpe_from_strava_label("Modéré"), Some(5.0));
        assert_eq!(rpe_from_strava_label("Difficile"), Some(8.0));
        assert_eq!(rpe_from_strava_label("Effort max"), Some(10.0));
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn a_scraped_number_is_read_directly() {
        assert_eq!(rpe_from_strava_label("7"), Some(7.0));
        assert_eq!(rpe_from_strava_label(" 4 "), Some(4.0));
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn an_unknown_label_or_an_out_of_range_number_is_none() {
        for label in ["", "  ", "All Out", "Very Hard", "Effort", "0", "11", "NaN"] {
            assert_eq!(rpe_from_strava_label(label), None, "{label:?}");
        }
    }
}
