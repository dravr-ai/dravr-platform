// ABOUTME: Social data seeder for Pierre MCP Server social features testing
// ABOUTME: Generates friend connections, shared insights, reactions, and adapted insights
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Social data seeder for Pierre MCP Server.
//!
//! This binary populates the database with social demo data for testing
//! the Friends, Feed, and Adapt to My Training features.
//!
//! Usage:
//! ```bash
//! # Seed with default settings
//! pierre-cli seed social
//!
//! # Reset social data before seeding
//! pierre-cli seed social --reset
//! ```
//!
//! Prerequisites:
//! - Run `pierre-cli seed demo-data` first to create demo users

use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::seed_models::{
    SeedAdaptedInsight, SeedFriendConnection, SeedInsightReaction, SeedSharedInsight,
    SeedSocialSettings,
};
use pierre_database::RepositoryRegistry;
use rand::prelude::SliceRandom;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tracing::info;
use uuid::Uuid;

/// CLI arguments for the social data seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Reset social data before seeding
    #[arg(long)]
    pub reset: bool,
}

/// Shared insight content definitions
struct InsightContent {
    insight_type: &'static str,
    sport_type: Option<&'static str>,
    title: &'static str,
    content: &'static str,
    training_phase: Option<&'static str>,
}

/// Sample insights for achievements and milestones
fn get_achievement_insights() -> Vec<InsightContent> {
    vec![
        InsightContent {
            insight_type: "achievement", sport_type: Some("run"), title: "New Personal Best!",
            content: "Crushed my tempo run today! Coach noted that my aerobic base has really improved over the past month. Feeling strong heading into race season.",
            training_phase: Some("build"),
        },
        InsightContent {
            insight_type: "milestone", sport_type: Some("ride"), title: "1000km Cycling Milestone",
            content: "Hit 1000km on the bike this month! Coach says my endurance foundation is solid and it's time to add some intensity work.",
            training_phase: Some("base"),
        },
        InsightContent {
            insight_type: "achievement", sport_type: Some("strength"), title: "Strength Gains",
            content: "Deadlift PR today! Coach has been emphasizing strength work to complement my endurance training. Feeling the difference on hills.",
            training_phase: Some("build"),
        },
        InsightContent {
            insight_type: "milestone", sport_type: Some("ride"), title: "First Century Complete",
            content: "Completed my first 100-mile ride! Pacing strategy coach suggested worked perfectly. Finished strong with energy to spare.",
            training_phase: Some("peak"),
        },
        InsightContent {
            insight_type: "achievement", sport_type: Some("run"), title: "Race Day Success",
            content: "Negative split my half marathon! Coach's pacing plan was spot on. Started conservative and had so much left for the final miles.",
            training_phase: Some("peak"),
        },
        InsightContent {
            insight_type: "milestone", sport_type: Some("run"), title: "Sub-4 Hour Marathon",
            content: "Broke 4 hours in the marathon! Months of preparation came together perfectly. Trust the process and trust your coach.",
            training_phase: Some("peak"),
        },
    ]
}

/// Sample insights for training tips, recovery, and motivation
fn get_coaching_insights() -> Vec<InsightContent> {
    vec![
        InsightContent {
            insight_type: "training_tip", sport_type: Some("swim"), title: "Drill Focus Paying Off",
            content: "Been focusing on catch drills as coach suggested. Starting to feel more connected to the water. Efficiency improving!",
            training_phase: Some("base"),
        },
        InsightContent {
            insight_type: "recovery", sport_type: None, title: "Active Recovery Week",
            content: "Taking a planned recovery week. Coach reminded me that rest is when adaptation happens. Sleep quality has been great!",
            training_phase: Some("recovery"),
        },
        InsightContent {
            insight_type: "motivation", sport_type: Some("run"), title: "Consistency Is Key",
            content: "14 weeks of consistent training in the books! Coach pointed out that showing up every day matters more than any single workout.",
            training_phase: Some("build"),
        },
        InsightContent {
            insight_type: "training_tip", sport_type: Some("run"), title: "Heart Rate Zone Training",
            content: "Learning to stay in Zone 2 on easy runs was tough at first, but coach was right - my aerobic engine is so much stronger now.",
            training_phase: Some("base"),
        },
        InsightContent {
            insight_type: "motivation", sport_type: Some("swim"), title: "Open Water Confidence",
            content: "Did my first open water swim without anxiety! The visualization techniques coach taught really helped calm my nerves.",
            training_phase: Some("build"),
        },
        InsightContent {
            insight_type: "recovery", sport_type: None, title: "Sleep Quality Focus",
            content: "Been tracking sleep as coach suggested. Turns out my 5:30am workouts were hurting recovery. Shifted to evenings and feeling much better!",
            training_phase: Some("base"),
        },
        InsightContent {
            insight_type: "training_tip", sport_type: Some("ride"), title: "Cadence Work",
            content: "Finally comfortable at 90+ rpm on the bike. Those cadence drills coach programmed felt awkward at first but made a huge difference.",
            training_phase: Some("base"),
        },
        InsightContent {
            insight_type: "motivation", sport_type: None, title: "Community Support",
            content: "Love seeing everyone's progress on here! We're all on different journeys but pushing each other forward. Keep going!",
            training_phase: None,
        },
        InsightContent {
            insight_type: "recovery", sport_type: Some("run"), title: "Managing Minor Setback",
            content: "Dealing with some IT band tightness. Coach adjusted my plan with more mobility work and shorter runs. Smart training over tough training.",
            training_phase: Some("recovery"),
        },
    ]
}

/// Get all sample insights for seeding by combining achievement and coaching insights
fn get_sample_insights() -> Vec<InsightContent> {
    let mut insights = get_achievement_insights();
    insights.extend(get_coaching_insights());
    insights
}

/// Reaction types
const REACTION_TYPES: &[&str] = &["like", "celebrate", "inspire", "support"];

/// Adapted insight content templates
fn get_adaptation_templates() -> Vec<&'static str> {
    vec![
        "Interesting approach! For your current training phase, you might try something similar but with shorter intervals to match your fitness level.",
        "Love this! Given your focus on base building, you could adapt this by keeping the intensity lower but extending the duration.",
        "Great insight! Since you're training for a different distance, consider scaling the effort proportionally to your goal race.",
        "This resonates with my training too. For your recovery week, a lighter version of this approach could work well.",
        "Solid advice! With your higher weekly volume, you might need extra recovery time when incorporating this.",
    ]
}

/// Seed friend connections, shared insights, reactions, and adapted insights across demo users.
///
/// # Errors
///
/// Returns an error if fewer than five demo users exist, or if any repository operation fails.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    info!("=== Pierre MCP Server Social Data Seeder ===");
    let ctx = prepare_social_context(repos, args.reset).await?;
    run_social_pipeline(repos, &ctx).await?;
    info!("=== Seeding Complete ===");
    Ok(())
}

/// Mutable state assembled before the social seeding steps begin: the demo user pool and
/// an optional admin user id used to test admin-facing social features.
struct SocialContext {
    user_ids: Vec<Uuid>,
    admin_id: Option<Uuid>,
}

async fn prepare_social_context(
    repos: &RepositoryRegistry,
    reset: bool,
) -> AppResult<SocialContext> {
    let user_count = repos.seeder.seed_count_non_admin_users().await?;
    if user_count < 5 {
        return Err(AppError::config(format!(
            "Not enough demo users found ({user_count}). Run 'pierre-cli seed demo-data' first."
        )));
    }

    if reset {
        info!("Resetting social data...");
        repos.seeder.seed_reset_social_data().await?;
    }

    let user_ids = repos.seeder.seed_get_non_admin_user_ids().await?;
    let admin_id = repos.seeder.seed_get_admin_user().await?.map(|a| a.id);
    info!(
        "Found {} demo users (admin for testing: {:?})",
        user_ids.len(),
        admin_id
    );
    Ok(SocialContext { user_ids, admin_id })
}

async fn run_social_pipeline(repos: &RepositoryRegistry, ctx: &SocialContext) -> AppResult<()> {
    step_social_settings(repos, ctx).await?;
    step_friend_connections(repos, ctx).await?;
    step_shared_insights(repos, &ctx.user_ids).await?;
    step_reactions(repos, &ctx.user_ids).await?;
    step_adapted_insights(repos, ctx).await?;
    Ok(())
}

async fn step_social_settings(repos: &RepositoryRegistry, ctx: &SocialContext) -> AppResult<()> {
    let settings = seed_social_settings(repos, &ctx.user_ids).await?;
    let admin_settings = if let Some(id) = ctx.admin_id {
        seed_social_settings(repos, &[id]).await?
    } else {
        0
    };
    info!("Step 1: {settings} social settings (+ {admin_settings} for admin)");
    Ok(())
}

async fn step_friend_connections(repos: &RepositoryRegistry, ctx: &SocialContext) -> AppResult<()> {
    let friends = seed_friend_connections(repos, &ctx.user_ids).await?;
    let admin_friends = if let Some(id) = ctx.admin_id {
        seed_admin_friend_connections(repos, &id, &ctx.user_ids).await?
    } else {
        0
    };
    info!("Step 2: {friends} friend connections (+ {admin_friends} for admin)");
    Ok(())
}

async fn step_shared_insights(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<()> {
    let count = seed_shared_insights(repos, user_ids).await?;
    info!("Step 3: {count} shared insights");
    Ok(())
}

async fn step_reactions(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<()> {
    let count = seed_reactions(repos, user_ids).await?;
    info!("Step 4: {count} reactions");
    Ok(())
}

async fn step_adapted_insights(repos: &RepositoryRegistry, ctx: &SocialContext) -> AppResult<()> {
    let adapted = seed_adapted_insights(repos, &ctx.user_ids).await?;
    let admin_adapted = if let Some(id) = ctx.admin_id {
        seed_admin_adapted_insights(repos, &id).await?
    } else {
        0
    };
    info!("Step 5: {adapted} adapted insights (+ {admin_adapted} for admin)");
    Ok(())
}

/// Seed user social settings
async fn seed_social_settings(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let mut count: u32 = 0;

    for user_id in user_ids {
        let discoverable = rng.gen_bool(0.9);
        let visibility = if rng.gen_bool(0.7) {
            "friends_only"
        } else {
            "public"
        };
        let now = Utc::now();

        let settings = SeedSocialSettings {
            user_id: *user_id,
            discoverable,
            default_visibility: visibility.to_owned(),
            share_activity_types: r#"["run", "ride", "swim"]"#.to_owned(),
            created_at: now,
            updated_at: now,
        };

        if repos.seeder.seed_upsert_social_settings(&settings).await? {
            count += 1;
        }
    }

    Ok(count)
}

/// Seed friend connections between demo users
async fn seed_friend_connections(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let mut count: u32 = 0;

    for (i, initiator_id) in user_ids.iter().enumerate() {
        for offset in 1..=3 {
            let receiver_idx = (i + offset) % user_ids.len();
            if receiver_idx == i {
                continue;
            }

            let receiver_id = user_ids[receiver_idx];
            let days_ago: i64 = rng.gen_range(1..30);
            let created_at = Utc::now() - Duration::days(days_ago);

            // 80% accepted, 15% pending, 5% declined
            let status_roll: u8 = rng.gen_range(0..100);
            let (status, accepted_at) = match status_roll {
                0..=79 => {
                    let accept_time = Utc::now() - Duration::days(days_ago - 1);
                    ("accepted", Some(accept_time))
                }
                80..=94 => ("pending", None),
                _ => ("declined", None),
            };

            let conn = SeedFriendConnection {
                id: Uuid::new_v4(),
                initiator_id: *initiator_id,
                receiver_id,
                status: status.to_owned(),
                created_at,
                updated_at: created_at,
                accepted_at,
            };

            if repos
                .seeder
                .seed_insert_friend_connection_if_absent(&conn)
                .await?
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Seed friend connections between admin user and demo users
async fn seed_admin_friend_connections(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    user_ids: &[Uuid],
) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let mut count: u32 = 0;

    let friends_to_create = user_ids.len().min(8);

    for seed_user_id in user_ids.iter().take(friends_to_create) {
        let days_ago: i64 = rng.gen_range(1..15);
        let created_at = Utc::now() - Duration::days(days_ago);
        let accepted_at = Utc::now() - Duration::days(days_ago - 1);

        let conn = SeedFriendConnection {
            id: Uuid::new_v4(),
            initiator_id: *seed_user_id,
            receiver_id: *admin_id,
            status: "accepted".to_owned(),
            created_at,
            updated_at: created_at,
            accepted_at: Some(accepted_at),
        };

        if repos
            .seeder
            .seed_insert_friend_connection_if_absent(&conn)
            .await?
        {
            count += 1;
        }
    }

    Ok(count)
}

/// Seed shared insights from demo users
async fn seed_shared_insights(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let insights = get_sample_insights();
    let mut count: u32 = 0;

    for user_id in user_ids {
        let num_insights: u32 = rng.gen_range(1..=3);

        for _ in 0..num_insights {
            let insight = &insights[rng.gen_range(0..insights.len())];
            let days_ago: i64 = rng.gen_range(1..14);
            let created_at = Utc::now() - Duration::days(days_ago);
            let visibility = if rng.gen_bool(0.8) {
                "friends_only"
            } else {
                "public"
            };

            let shared = SeedSharedInsight {
                id: Uuid::new_v4(),
                user_id: *user_id,
                visibility: visibility.to_owned(),
                insight_type: insight.insight_type.to_owned(),
                sport_type: insight.sport_type.map(ToOwned::to_owned),
                content: insight.content.to_owned(),
                title: insight.title.to_owned(),
                training_phase: insight.training_phase.map(ToOwned::to_owned),
                created_at,
                updated_at: created_at,
            };

            if repos
                .seeder
                .seed_insert_shared_insight(&shared)
                .await
                .is_ok()
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Seed reactions on shared insights
async fn seed_reactions(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let mut count: u32 = 0;

    let insight_ids = repos.seeder.seed_get_shared_insight_ids().await?;

    for insight_id in &insight_ids {
        let react_probability: f64 = rng.gen_range(0.3..0.6);

        for user_id in user_ids {
            if !rng.gen_bool(react_probability) {
                continue;
            }

            let reaction = SeedInsightReaction {
                id: Uuid::new_v4(),
                insight_id: *insight_id,
                user_id: *user_id,
                reaction_type: REACTION_TYPES[rng.gen_range(0..REACTION_TYPES.len())].to_owned(),
                created_at: Utc::now(),
            };

            if repos
                .seeder
                .seed_insert_reaction_if_absent(&reaction)
                .await?
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Seed adapted insights
async fn seed_adapted_insights(repos: &RepositoryRegistry, user_ids: &[Uuid]) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let templates = get_adaptation_templates();
    let mut count: u32 = 0;

    let insights = repos.seeder.seed_get_shared_insights_with_authors().await?;

    for (insight_id, author_id) in &insights {
        let adapt_probability: f64 = rng.gen_range(0.1..0.25);

        for user_id in user_ids {
            if *user_id == *author_id {
                continue;
            }

            if !rng.gen_bool(adapt_probability) {
                continue;
            }

            let adapted = SeedAdaptedInsight {
                id: Uuid::new_v4(),
                source_insight_id: *insight_id,
                user_id: *user_id,
                adapted_content: templates[rng.gen_range(0..templates.len())].to_owned(),
                adaptation_context:
                    r#"{"training_phase": "base", "fitness_level": "intermediate"}"#.to_owned(),
                was_helpful: rng.gen_bool(0.8),
                created_at: Utc::now(),
            };

            if repos
                .seeder
                .seed_insert_adapted_insight_if_absent(&adapted)
                .await?
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Seed adapted insights for admin user from demo users' shared insights
async fn seed_admin_adapted_insights(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
) -> AppResult<u32> {
    let mut rng = StdRng::from_entropy();
    let templates = get_adaptation_templates();
    let mut count: u32 = 0;

    let insight_ids = repos
        .seeder
        .seed_get_shared_insights_not_by_user(*admin_id, 10)
        .await?;

    let num_to_adapt = rng.gen_range(3..=5).min(insight_ids.len());
    let mut indices: Vec<usize> = (0..insight_ids.len()).collect();
    indices.shuffle(&mut rng);

    for idx in indices.into_iter().take(num_to_adapt) {
        let insight_id = insight_ids[idx];
        let days_ago: i64 = rng.gen_range(1..7);

        let adapted = SeedAdaptedInsight {
            id: Uuid::new_v4(),
            source_insight_id: insight_id,
            user_id: *admin_id,
            adapted_content: templates[rng.gen_range(0..templates.len())].to_owned(),
            adaptation_context: r#"{"training_phase": "build", "fitness_level": "advanced"}"#
                .to_owned(),
            was_helpful: true,
            created_at: Utc::now() - Duration::days(days_ago),
        };

        if repos
            .seeder
            .seed_insert_adapted_insight_if_absent(&adapted)
            .await?
        {
            count += 1;
        }
    }

    Ok(count)
}
