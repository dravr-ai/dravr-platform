// ABOUTME: Builds the onboarding coach proposal's sport-profile view and LLM prompt text
// ABOUTME: Sport-mix shares, display labels, and the athlete/pillar context lines the re-rank reads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;

use pierre_config::coach_recommendations::CoachRecommendationConfig;
use pierre_core::models::{Dossier, Pillar, SportProfile};
use pierre_services::activity_sports::{sport_label, MessagingStringsRegistry};
use pierre_services::coaches::sport_code;

use super::types::{SportProfileSummary, SportShare};

/// Clamp + flatten untrusted fact text before it enters the re-rank LLM prompt.
fn sanitize_for_prompt(s: &str) -> String {
    s.chars()
        .take(120)
        .collect::<String>()
        .replace(['\n', '\r'], " ")
}

/// Build a short coach-matching context line from the user's onboarding facts.
///
/// Carries their North Star (sanitized, the most match-relevant signal) plus
/// which pillars they have shared context on (labels only). `None` when the
/// user has no non-stale pillar context yet — the proposal then matches on
/// sport-mix.
#[must_use]
pub fn pillar_context_prompt(dossier: &Dossier) -> Option<String> {
    let mut parts = Vec::new();

    let north_star: Vec<String> = dossier
        .north_star
        .iter()
        .filter(|f| !f.stale)
        .map(|f| sanitize_for_prompt(&f.object))
        .filter(|s| !s.is_empty())
        .collect();
    if !north_star.is_empty() {
        parts.push(format!(
            "North Star (the athlete's core motivations): {}",
            north_star.join("; ")
        ));
    }

    let covered: Vec<&str> = Pillar::ALL
        .iter()
        .filter(|p| {
            dossier
                .pillars
                .get(p)
                .is_some_and(|facts| facts.iter().any(|f| !f.stale))
        })
        .map(|p| p.display_label())
        .collect();
    if !covered.is_empty() {
        parts.push(format!("Has shared context on: {}", covered.join(", ")));
    }

    (!parts.is_empty()).then(|| parts.join(". "))
}

/// The user's sport profile in the two shapes the proposal needs: the
/// serializable [`SportProfileSummary`] for the response, and a short
/// human-readable `prompt_text` describing the athlete for the LLM re-rank.
pub(super) struct ProfileView {
    /// The wire shape. Its sports are serde **codes**; the clients own the
    /// five-locale label catalogue and translate them.
    pub(super) summary: SportProfileSummary,
    /// Prose for the LLM re-rank, sports spelled as the athlete's own words.
    pub(super) prompt_text: String,
    /// The primary sport as the athlete reads it, for the deterministic
    /// rationale this crate renders itself.
    pub(super) primary_sport: Option<String>,
}

/// Build the [`ProfileView`] from a (possibly absent) sport profile.
///
/// With activities present, produces the sport mix (sorted by count desc) and a
/// one-line athlete description. Otherwise returns a cold-start view describing
/// whether a provider is connected at all.
pub(super) fn build_profile_view(
    profile: Option<&SportProfile>,
    providers: &HashSet<String>,
    config: &CoachRecommendationConfig,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> ProfileView {
    match profile {
        Some(profile) if profile.total_activities > 0 => {
            let total = profile.total_activities.max(1);
            let mut sport_mix: Vec<SportShare> = profile
                .sport_counts
                .iter()
                .map(|(sport, &count)| SportShare {
                    sport: sport_code(sport),
                    count,
                    share: share_fraction(count, total),
                })
                .collect();
            sport_mix.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.sport.cmp(&b.sport)));

            // The wire carries the code, because the clients own the label
            // catalogue; the prose this crate renders reads the athlete's own
            // word for the sport through the shared vocabulary.
            let primary_code = profile.primary_sport().map(|s| sport_code(&s));
            let primary_label = primary_code
                .as_deref()
                .map(|code| sport_label(strings, code, locale));
            let mix_text = sport_mix
                .iter()
                .map(|s| {
                    format!(
                        "{} {:.0}%",
                        sport_label(strings, &s.sport, locale),
                        s.share * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let prompt_text = format!(
                "Trains primarily {primary}; recent sport mix over {days} days: {mix_text} \
                 ({total} activities).",
                primary = primary_label.as_deref().unwrap_or("various sports"),
                days = profile.window_days,
                total = profile.total_activities,
            );

            ProfileView {
                summary: SportProfileSummary {
                    has_profile: true,
                    primary_sport: primary_code,
                    total_activities: profile.total_activities,
                    window_days: profile.window_days,
                    sport_mix,
                },
                prompt_text,
                primary_sport: primary_label,
            }
        }
        _ => {
            let prompt_text = if providers.is_empty() {
                "New athlete: no fitness provider connected yet.".to_owned()
            } else {
                "New athlete: a provider is connected but no recent activities are available yet."
                    .to_owned()
            };
            ProfileView {
                summary: SportProfileSummary {
                    has_profile: false,
                    primary_sport: None,
                    total_activities: 0,
                    window_days: config.window_days,
                    sport_mix: Vec::new(),
                },
                prompt_text,
                primary_sport: None,
            }
        }
    }
}

/// `count / total` as an `f32` fraction without precision-loss casts, mirroring
/// the approach in [`SportProfile`].
fn share_fraction(count: u32, total: u32) -> f32 {
    let count = f32::from(u16::try_from(count).unwrap_or(u16::MAX));
    let total = f32::from(u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    count / total
}
