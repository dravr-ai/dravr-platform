// ABOUTME: The nutrition arm of generate_recommendations — macros, meals and the session they are for
// ABOUTME: Split from recommendations.rs to keep that file under the size ceiling; same module, same callers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_config::constants::time_constants;
use pierre_core::models::Activity;

use crate::implementations::analytics::recommendations_output::{
    ActivitySummary, MacronutrientTargets, MealSuggestion, RecommendationsResult,
};

use super::recommendations::base_recommendations;

/// Generate nutrition recommendations based on recent activity
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

/// Calculate macronutrient needs based on workout intensity and duration
fn calculate_macronutrient_needs(intensity: &str, duration_hours: f64) -> (f64, f64, f64) {
    let protein_g = if intensity == "high" || duration_hours > 1.5 {
        30.0 + (duration_hours * 5.0).min(20.0)
    } else {
        20.0 + (duration_hours * 5.0).min(15.0)
    };

    let carbs_g = duration_hours * 70.0;
    let hydration_ml = duration_hours * 750.0;

    (protein_g, carbs_g, hydration_ml)
}

/// Build meal suggestions based on workout intensity
fn build_meal_suggestions(intensity: &str) -> Vec<MealSuggestion> {
    let mut suggestions = vec![
        MealSuggestion {
            option: "Quick Recovery Shake".to_owned(),
            description: "Protein shake with banana and honey".to_owned(),
            protein_g: 25,
            carbs_g: 50,
            timing: "Immediate (0-15 min)".to_owned(),
        },
        MealSuggestion {
            option: "Greek Yogurt Bowl".to_owned(),
            description: "200g Greek yogurt with granola, berries, and honey".to_owned(),
            protein_g: 20,
            carbs_g: 60,
            timing: "Within 30 minutes".to_owned(),
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

pub(super) fn generate_nutrition_recommendations(activities: &[Activity]) -> RecommendationsResult {
    let most_recent = activities.iter().max_by_key(|a| a.start_date());

    if most_recent.is_none() {
        return base_recommendations(
            "nutrition",
            "medium",
            "No recent activity data available".to_owned(),
            vec![
                "Maintain balanced nutrition with adequate protein (1.6-2.2g/kg body weight)"
                    .to_owned(),
                "Stay hydrated throughout the day (2-3 liters water)".to_owned(),
                "Eat regular meals with complex carbohydrates, lean protein, and healthy fats"
                    .to_owned(),
            ],
        );
    }

    let Some(activity) = most_recent else {
        // Unreachable: the `is_none` arm above returns first. Kept as the
        // exhaustive arm, and now answering the declared shape rather than a
        // bare `recommendations` key that named neither mode nor priority.
        return base_recommendations(
            "nutrition",
            "medium",
            "No recent activity data available".to_owned(),
            vec!["No recent activities found for nutrition analysis".to_owned()],
        );
    };

    let (duration_hours, calories_burned, intensity) = calculate_nutrition_metrics(activity);
    let (protein_g, carbs_g, hydration_ml) =
        calculate_macronutrient_needs(intensity, duration_hours);

    let mut recommendations = vec![
        format!(
            "Within 30 minutes: Consume {:.0}g protein and {:.0}g carbohydrates for optimal recovery",
            protein_g,
            carbs_g * 0.5
        ),
        format!(
            "Rehydrate with {:.0}-{:.0}ml of water or electrolyte drink",
            hydration_ml,
            hydration_ml * 1.3
        ),
    ];

    if intensity == "high" || duration_hours > 1.0 {
        recommendations.push(
            "Follow up with a complete meal within 2 hours to fully replenish glycogen stores"
                .to_owned(),
        );
    }

    let meal_suggestions = build_meal_suggestions(intensity);

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

    RecommendationsResult {
        recovery_window: Some("Critical recovery period: 0-2 hours post-workout".to_owned()),
        key_insights,
        meal_suggestions,
        macronutrient_targets: Some(MacronutrientTargets {
            protein_g: protein_g.round(),
            carbohydrates_g: carbs_g.round(),
            hydration_ml: hydration_ml.round(),
        }),
        activity_summary: Some(ActivitySummary {
            name: activity.name().to_owned(),
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
