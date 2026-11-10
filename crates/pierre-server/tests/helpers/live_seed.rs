// ABOUTME: Seeding mechanics the live-model lanes share: athletes and tenants, onboarding, recipes, groups, run schedules
// ABOUTME: Each lane keeps its own data and passes it in; only the generic mechanics live here (incident corpus, injection eval)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

//! The fixture mechanics of the live lanes.
//!
//! The incident corpus (`live_incident_corpus.rs`) and the injection eval
//! (`injection_corpus/fixture.rs`) seed the same kind of athlete the same
//! way: a user with a tenant of their own, recorded as onboarded, with saved
//! recipes, in a coaching group with a peer, over a few weeks of runs. Where
//! the two differ in a detail — an admin role, a bcrypt cost, a tenant label,
//! a group's channel, which day of the schedule is skipped — the difference is
//! a parameter here, so each lane keeps exactly the fixture it had. Shared
//! across test binaries; a binary that includes the helpers may use none of it.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::recipes::{IngredientUnit, MealTiming, Recipe, RecipeIngredient};
use pierre_core::models::{Pillar, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::intake::{record_steps, STATUS_COMPLETE};
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

// -----------------------------------------------------------------------
// Athletes
// -----------------------------------------------------------------------

/// What every athlete a lane seeds has in common: their role, the cost of
/// their password hash and how their tenant is named.
pub(crate) struct AthleteProfile<'a> {
    /// The user's role; `is_admin` follows it (`UserRole::is_admin_or_higher`).
    pub role: UserRole,
    /// The bcrypt cost of the password hash. Nobody signs in as a fixture
    /// athlete; a lane that seeds many of them takes the cheapest cost.
    pub bcrypt_cost: u32,
    /// The tenant is named `"{tenant_label} {display}"`.
    pub tenant_label: &'a str,
    /// The tenant's slug is `"{tenant_slug_prefix}-{tenant_id}"`.
    pub tenant_slug_prefix: &'a str,
}

/// Create the athlete `email`, shown as `display` on the user and in the
/// tenant's name, with a tenant of their own; their user id and tenant.
///
/// Enterprise, or the lane cannot finish. Quota comes from the USER's tier,
/// not the tenant's `plan` string — Starter is the default and caps
/// `max_conversations_per_day` at 10. The incident corpus's first ACP run
/// truncated at turn 8 with «Tu as atteint la limite de conversation de ton
/// forfait», which it correctly filed as infrastructure rather than as an agent
/// regression — but four turns went ungraded because the fixture had put
/// itself on the free tier.
pub(crate) async fn create_athlete(
    resources: &Arc<ServerContext>,
    profile: &AthleteProfile<'_>,
    email: &str,
    display: &str,
) -> (Uuid, TenantId) {
    let cost = profile.bcrypt_cost;
    let password_hash = spawn_blocking(move || bcrypt::hash("password123", cost).unwrap())
        .await
        .unwrap();
    let mut user = User::new(email.to_owned(), password_hash, Some(display.to_owned()));
    user.role = profile.role;
    user.is_admin = profile.role.is_admin_or_higher();
    user.tier = UserTier::Enterprise;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("{} {display}", profile.tenant_label),
        slug: format!("{}-{tenant_id}", profile.tenant_slug_prefix),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// Record the athlete as onboarded, as every athlete a live lane speaks for
/// is.
///
/// A fresh conversation otherwise opens the intake interview and then the
/// pillars walk, and an interview owns its turns: it withholds the visual
/// contract and reshapes the reply, so every turn would grade onboarding rather
/// than the question asked. Intake ends with both steps complete — never
/// `not_applicable`, the coach-who-does-not-train answer — and the walk is
/// skipped once any pillar is covered.
pub(crate) async fn seed_onboarded(
    resources: &Arc<ServerContext>,
    athlete: Uuid,
    tenant: TenantId,
) {
    let repos = &resources.common.repos;
    let user_id = athlete.to_string();
    record_steps(
        repos.user_onboarding.as_ref(),
        &user_id,
        Some(&tenant.to_string()),
        STATUS_COMPLETE,
        STATUS_COMPLETE,
    )
    .await
    .unwrap();
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: Some(Pillar::TrainingAndMovement),
            predicate_code: PredicateCode::States,
            object: "runs three times a week",
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
}

// -----------------------------------------------------------------------
// Recipes
// -----------------------------------------------------------------------

/// One recipe an athlete saved, as a lane seeds it and states it.
pub(crate) struct SavedRecipe {
    pub name: &'static str,
    pub timing: MealTiming,
    /// The timing as a ledger states it (`post_training`).
    pub timing_label: &'static str,
    /// `(name, grams)` per ingredient.
    pub ingredients: &'static [(&'static str, f64)],
}

/// Save `recipes` to the athlete's collection, each described "Saved by the
/// athlete", for one serving, every ingredient in grams.
pub(crate) async fn seed_recipes(
    resources: &Arc<ServerContext>,
    athlete: Uuid,
    tenant: TenantId,
    recipes: &[SavedRecipe],
) {
    for saved in recipes {
        let mut recipe = Recipe::new(athlete, saved.name, 1)
            .with_description("Saved by the athlete")
            .with_instruction("Assemble and serve");
        recipe.meal_timing = saved.timing;
        recipe.ingredients = saved
            .ingredients
            .iter()
            .map(|(name, grams)| RecipeIngredient {
                fdc_id: None,
                name: (*name).to_owned(),
                amount: *grams,
                unit: IngredientUnit::Grams,
                grams: *grams,
                preparation: None,
            })
            .collect();
        resources
            .common
            .repos
            .recipes
            .create(athlete, tenant, &recipe)
            .await
            .unwrap();
    }
}

// -----------------------------------------------------------------------
// Groups
// -----------------------------------------------------------------------

/// A coaching group a lane seeds.
pub(crate) struct NewGroup<'a> {
    pub tenant: TenantId,
    pub name: &'a str,
    /// The agent the group is bound to.
    pub agent: Uuid,
    pub owner: Uuid,
    /// `(channel_type, channel_chat_id)` for a group bound to a messaging
    /// room, `None` for an in-app group.
    pub channel: Option<(&'a str, &'a str)>,
    /// Every member as `(user, role, roster display name)`, owner included.
    pub members: &'a [(Uuid, GroupRole, &'a str)],
}

/// Create the group and add its members, every one consenting; its id.
///
/// Peers must be readable or every comparison turn degenerates into a consent
/// refusal, which is not the behaviour under test: `peer_data_sharing` and
/// each member's consents are on. `respond_mode` is `All`, so a group turn is
/// answered without an @mention — an unaddressed message in `Mentions` mode is
/// silent BY DESIGN and would read as a lost turn.
pub(crate) async fn create_group(resources: &Arc<ServerContext>, group: &NewGroup<'_>) -> Uuid {
    let group_id = Uuid::new_v4();
    let now = Utc::now();
    let tenant = group.tenant;
    resources
        .common
        .repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: tenant.to_string(),
                name: group.name.to_owned(),
                description: None,
                agent_id: group.agent.to_string(),
                owner_id: group.owner,
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::All,
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: group.channel.map(|(kind, _)| kind.to_owned()),
                channel_chat_id: group.channel.map(|(_, chat)| chat.to_owned()),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    for &(user, role, display) in group.members {
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id: user,
                tenant_id: tenant.to_string(),
                role,
                peer_sharing_consent: true,
                coach_sharing_consent: true,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: Some(display.to_owned()),
            })
            .await
            .unwrap();
    }
    group_id
}

// -----------------------------------------------------------------------
// Run schedules
// -----------------------------------------------------------------------

/// `hour:minute` UTC on the calendar day `days_ago` before `anchor`.
pub(crate) fn day_at(
    anchor: DateTime<Utc>,
    days_ago: i64,
    hour: u32,
    minute: u32,
) -> DateTime<Utc> {
    (anchor - ChronoDuration::days(days_ago))
        .date_naive()
        .and_hms_opt(hour, minute, 0)
        .unwrap_or_else(|| panic!("{hour:02}:{minute:02} is not a time of day"))
        .and_utc()
}

/// One slot of a weekly schedule: which week and weekday offset, how many days
/// back that is, and when the run starts.
pub(crate) struct WeekSlot {
    pub week: i64,
    pub day: i64,
    pub days_ago: i64,
    pub start: DateTime<Utc>,
}

/// A schedule of `weeks` weeks back from `anchor`, one slot per `(day, hour,
/// minute)` of each week, oldest week last.
///
/// Week `w` starts every run `w * shift_minutes_per_week` minutes later than
/// the time given, so no two weeks share a start time when the lane wants that;
/// a shift of zero starts every week's runs at the times given. A slot whose
/// `days_ago` `skip` holds is left out (the incident corpus keeps Sunday to its
/// twin ride; the injection eval keeps yesterday for the planted run).
pub(crate) fn weekly_slots(
    anchor: DateTime<Utc>,
    weeks: i64,
    days: &[(i64, u32, u32)],
    shift_minutes_per_week: u32,
    skip: impl Fn(i64) -> bool,
) -> Vec<WeekSlot> {
    let mut slots = Vec::new();
    for week in 0..weeks {
        let shift = u32::try_from(week).unwrap_or(0) * shift_minutes_per_week;
        for &(day, hour, minute) in days {
            let days_ago = week * 7 + day;
            if skip(days_ago) {
                continue;
            }
            let minutes = hour * 60 + minute + shift;
            slots.push(WeekSlot {
                week,
                day,
                days_ago,
                start: day_at(anchor, days_ago, minutes / 60, minutes % 60),
            });
        }
    }
    slots
}
