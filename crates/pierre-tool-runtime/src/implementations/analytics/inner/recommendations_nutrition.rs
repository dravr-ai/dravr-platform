// ABOUTME: The nutrition arm of generate_recommendations — macros, meals and the session they are for
// ABOUTME: Split from recommendations.rs to keep that file under the size ceiling; same module, same callers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_config::constants::time_constants;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::implementations::analytics::recommendations_output::{
    ActivitySummary, MacronutrientTargets, MealSuggestion, RecommendationsResult,
};
use crate::implementations::nutrition_gate::{
    athlete_tenants, medical_flag_on_file, FiguresWithheld,
};

use super::recommendations::base_recommendations;

/// Protein for the meal after a session, grams per kg of body weight: "about
/// 0.3 g/kg after key sessions" (Thomas, Erdman & Burke 2016). Kerksick et al.
/// 2017 give 0.25-0.40 g/kg, or 20-40 g, taken from immediately to 2 h after.
const RECOVERY_PROTEIN_G_PER_KG: f64 = 0.3;

/// Speedy refuelling when fewer than 8 h separate two fuel-demanding sessions:
/// 1-1.2 g/kg/h of carbohydrate for the first 4 h (Thomas, Erdman & Burke 2016;
/// Kerksick et al. 2017 name about 1.2 g/kg/h).
const REFUEL_CARBS_G_PER_KG_PER_H_MIN: f64 = 1.0;
/// The high end of that range.
const REFUEL_CARBS_G_PER_KG_PER_H_MAX: f64 = 1.2;

/// What the nutrition arm needs to know about the athlete beyond their sessions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NutritionAthlete {
    /// Stored body weight in kilograms, when one is on file. Scales the
    /// per-kilogram doses into grams.
    pub weight_kg: Option<f64>,
    /// Set when a medical flag is on file: the arm then answers without any
    /// amount, because the athlete's clinician sets them.
    pub figures_withheld: Option<FiguresWithheld>,
}

/// Read what [`generate_nutrition_recommendations`] needs about the athlete:
/// the medical-flag gate first, then the stored body weight.
///
/// # Errors
///
/// Returns the repository error when the tenants, the flag or the profile
/// cannot be read. A flag that cannot be read never reads as "no flag".
pub async fn load_nutrition_athlete(
    repos: &RepositoryRegistry,
    tenant_id: Option<TenantId>,
    user_id: Uuid,
) -> AppResult<NutritionAthlete> {
    let tenants = athlete_tenants(repos, tenant_id, user_id).await?;
    if medical_flag_on_file(repos, &tenants, user_id).await? {
        return Ok(NutritionAthlete {
            weight_kg: None,
            figures_withheld: Some(FiguresWithheld::medical_flag()),
        });
    }
    for tenant in tenants {
        let weight = repos
            .user_physiological_profile
            .get_user_physiological_profile(tenant, user_id)
            .await?
            // A profile written from first-party-only data is withheld from an
            // external caller (carnet#769).
            .filter(|profile| ai_scope::admit_derived(profile.transport_policy))
            .and_then(|profile| profile.weight)
            .filter(|kg| kg.is_finite() && *kg > 0.0);
        if weight.is_some() {
            return Ok(NutritionAthlete {
                weight_kg: weight,
                figures_withheld: None,
            });
        }
    }
    Ok(NutritionAthlete::default())
}

/// Calculate activity nutrition metrics (duration, calories, intensity)
fn calculate_nutrition_metrics(activity: &Activity) -> (f64, f64, &'static str) {
    use time_constants;

    let duration_hours = f64::from(
        u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
    ) / time_constants::SECONDS_PER_HOUR_F64;

    let calories_burned = f64::from(activity.calories().unwrap_or_else(|| {
        let duration_mins = u32::try_from(activity.duration_seconds() / 60).unwrap_or(u32::MAX);
        duration_mins * 10
    }));

    let intensity = activity.average_heart_rate().map_or(
        if duration_hours > 1.5 {
            "moderate"
        } else {
            "low"
        },
        |avg_hr| {
            let avg_hr_f64 = f64::from(avg_hr);
            if avg_hr_f64 > 160.0 {
                "high"
            } else if avg_hr_f64 > 130.0 {
                "moderate"
            } else {
                "low"
            }
        },
    );

    (duration_hours, calories_burned, intensity)
}

/// The recovery-window targets at a stored body weight.
fn macronutrient_targets(weight_kg: f64) -> MacronutrientTargets {
    MacronutrientTargets {
        protein_g: (weight_kg * RECOVERY_PROTEIN_G_PER_KG).round(),
        protein_g_per_kg: RECOVERY_PROTEIN_G_PER_KG,
        refuel_carbohydrates_g_per_h_min: (weight_kg * REFUEL_CARBS_G_PER_KG_PER_H_MIN).round(),
        refuel_carbohydrates_g_per_h_max: (weight_kg * REFUEL_CARBS_G_PER_KG_PER_H_MAX).round(),
        body_weight_kg: weight_kg,
    }
}

/// The amounts to take after the session, in the athlete's terms: grams at a
/// stored weight, the per-kilogram dose otherwise.
fn recovery_recommendations(targets: Option<&MacronutrientTargets>) -> Vec<String> {
    let (protein, refuel) = targets.map_or_else(
        || {
            (
                "Within about 2 hours of the session: about 0.25-0.3 g of protein per kg of body weight (roughly 20-40 g), in a meal or snack that also carries carbohydrate".to_owned(),
                "If your next hard or long session is less than 8 hours away, refuel with 1-1.2 g of carbohydrate per kg of body weight per hour for the first 4 hours; otherwise your regular meals refill glycogen".to_owned(),
            )
        },
        |t| {
            (
                format!(
                    "Within about 2 hours of the session: about {:.0} g of protein (0.3 g per kg of body weight), in a meal or snack that also carries carbohydrate",
                    t.protein_g
                ),
                format!(
                    "If your next hard or long session is less than 8 hours away, refuel with 1-1.2 g of carbohydrate per kg per hour for the first 4 hours (about {:.0}-{:.0} g per hour); otherwise your regular meals refill glycogen",
                    t.refuel_carbohydrates_g_per_h_min, t.refuel_carbohydrates_g_per_h_max
                ),
            )
        },
    );
    vec![
        protein,
        refuel,
        // Replace 125-150% of the fluid actually lost (Thomas, Erdman & Burke
        // 2016). No sweat loss is measured here, so no volume is prescribed:
        // the scale weight before and after the session is the measure.
        "Rehydrate with 1.25-1.5 L of water or electrolyte drink for every kg of body weight lost in the session (weigh before and after); sodium from food or drink helps you keep it"
            .to_owned(),
    ]
}

/// What a flagged athlete is told instead: foods and timing, no amount.
const WITHHELD_RECOVERY_GUIDANCE: &[&str] = &[
    "Within about 2 hours of the session: a meal or snack with a protein source and carbohydrate-rich food",
    "If your next hard or long session is less than 8 hours away, start refuelling with carbohydrate-rich food soon after this one",
    "Rehydrate by drinking to thirst, within any fluid limit your clinician has set",
    "Your clinician sets your protein, carbohydrate and fluid amounts; ask what they have advised",
];

/// Build meal suggestions based on workout intensity. Every option sits in
/// the same window: protein from immediately to 2 h after the session
/// stimulates muscle protein synthesis robustly (Kerksick et al. 2017), and
/// the urgency of a narrower one is not supported (Aragon & Schoenfeld 2013).
fn build_meal_suggestions(intensity: &str) -> Vec<MealSuggestion> {
    let mut suggestions = vec![
        MealSuggestion {
            option: "Quick Recovery Shake".to_owned(),
            description: "Protein shake with banana and honey".to_owned(),
            protein_g: 25,
            carbs_g: 50,
            timing: "Within 2 hours — the quickest option".to_owned(),
        },
        MealSuggestion {
            option: "Greek Yogurt Bowl".to_owned(),
            description: "200g Greek yogurt with granola, berries, and honey".to_owned(),
            protein_g: 20,
            carbs_g: 60,
            timing: "Within 2 hours".to_owned(),
        },
        MealSuggestion {
            option: "Recovery Meal".to_owned(),
            description: "Grilled chicken with sweet potato and vegetables".to_owned(),
            protein_g: 35,
            carbs_g: 50,
            timing: "Within 2 hours".to_owned(),
        },
    ];

    if intensity == "high" {
        suggestions.push(MealSuggestion {
            option: "Endurance Option".to_owned(),
            description: "Pasta with lean meat sauce and mixed salad".to_owned(),
            protein_g: 30,
            carbs_g: 80,
            timing: "Within 2 hours".to_owned(),
        });
    }

    suggestions
}

/// The `nutrition` mode of `generate_recommendations`, for the athlete's most
/// recent session.
///
/// Public so integration tests can assert the content of what the athlete is
/// told; its production caller is the tool handler.
#[must_use]
pub fn generate_nutrition_recommendations(
    activity: &Activity,
    athlete: &NutritionAthlete,
) -> RecommendationsResult {
    let (duration_hours, calories_burned, intensity) = calculate_nutrition_metrics(activity);

    let mut key_insights = vec![
        format!(
            "Activity burned approximately {:.0} calories",
            calories_burned
        ),
        format!("Workout intensity: {intensity} - adjust nutrition accordingly"),
    ];

    if duration_hours > 1.5 {
        key_insights
            .push("Extended duration activity - prioritize carbohydrate replenishment".to_owned());
    }

    let (recommendations, meal_suggestions, macronutrient_targets) =
        if athlete.figures_withheld.is_some() {
            (
                WITHHELD_RECOVERY_GUIDANCE
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
                Vec::new(),
                None,
            )
        } else {
            let targets = athlete.weight_kg.map(macronutrient_targets);
            let mut recommendations = recovery_recommendations(targets.as_ref());
            if intensity == "high" || duration_hours > 1.0 {
                recommendations.push(
                    "After a long or hard session, make that a complete meal rather than a snack"
                        .to_owned(),
                );
            }
            (recommendations, build_meal_suggestions(intensity), targets)
        };

    RecommendationsResult {
        recovery_window: Some(
            "Recovery window: about 0-2 hours after the session; total daily intake matters more than the exact timing"
                .to_owned(),
        ),
        key_insights,
        meal_suggestions,
        macronutrient_targets,
        figures_withheld: athlete.figures_withheld.clone(),
        activity_summary: Some(ActivitySummary {
            name: display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS),
            // The serde spelling, not the Debug one: `SportType` renames its
            // variants, so `{:?}` would put a different word on the wire than
            // every other surface uses for the same sport.
            sport: sport_name(activity),
            duration_minutes: activity.duration_seconds() / 60,
            distance_km: activity.distance_meters().map(|d| (d / 1000.0).round()),
            calories: calories_burned.round(),
        }),
        ..base_recommendations(
            "nutrition",
            if intensity == "high" {
                "high"
            } else {
                "medium"
            },
            format!(
                "Based on {:.1} hour {intensity} intensity {} with {:.0} calories burned",
                duration_hours,
                sport_name(activity),
                calories_burned
            ),
            recommendations,
        )
    }
}

/// The serde name of an activity's sport, which is what every other surface
/// reports. `{:?}` gives the Rust variant, and `SportType` renames.
fn sport_name(activity: &Activity) -> String {
    serde_json::to_value(activity.sport_type())
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", activity.sport_type()))
}
