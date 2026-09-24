// ABOUTME: Seeds a group whose human coach proposed a TrainingPeaks link to one member, for the member-confirm flows
// ABOUTME: The coach's TrainingPeaks session is a dev stand-in; the member confirms or declines it in Group info
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `TrainingPeaks` delegated-connection seeder.
//!
//! A group's human coach reads a member's `TrainingPeaks` workouts through the
//! coach's own `TrainingPeaks` account once the member confirms the coach's
//! link. This seeder leaves one such link waiting for the member's answer, so
//! the web and mobile member-confirm flows (the mobile Maestro flow
//! `chat/10-trainingpeaks-link-confirm.yaml`) have something to answer:
//!
//! - the coach (`alice@acme.com` by default) is granted `manages_roster`, holds
//!   a stand-in `sciotte_trainingpeaks` session recorded as a coach account,
//!   and has accepted the current `TrainingPeaks` notice;
//! - a group named [`GROUP_NAME`], owned by `bob@startup.io`, is coached by
//!   the coach and has the member (`mobiletest@pierre.dev`) in it, with the
//!   member's and the coach's group threads filed;
//! - one proposed link names roster athlete [`ATHLETE_ID`] ([`ATHLETE_NAME`]).
//!
//! The session is a stand-in: nothing reads the coach's real `TrainingPeaks`
//! through it, so a read after the member confirms fails at the scraper and is
//! logged, as any dead session is. Run it after `seed demo-data`, which
//! creates the three accounts. It is idempotent: a coach who already coaches
//! a group of that name is left as they are.
//!
//! ```bash
//! pierre-cli seed trainingpeaks-delegation
//! ```

use chrono::Utc;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::{SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection, ProviderAccountRole,
    TenantId, UserOAuthToken,
};
use pierre_database::RepositoryRegistry;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

/// The seeded group's name, which the member's thread is titled after.
pub const GROUP_NAME: &str = "TrainingPeaks Squad";

/// The roster athlete the coach proposed.
pub const ATHLETE_ID: &str = "900001";

/// That athlete's name on the coach's `TrainingPeaks` roster.
pub const ATHLETE_NAME: &str = "Alex Athlete";

/// CLI arguments for the `TrainingPeaks` delegated-connection seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// The group's human coach, who proposes the link
    #[arg(long, default_value = "alice@acme.com")]
    pub coach_email: String,

    /// The member the link names, who confirms or declines it
    #[arg(long, default_value = "mobiletest@pierre.dev")]
    pub member_email: String,

    /// The group's owner
    #[arg(long, default_value = "bob@startup.io")]
    pub owner_email: String,

    /// The model the group threads are created with. `pierre-cli` fills it,
    /// when not given, with the model every new conversation gets from the
    /// configured LLM provider
    #[arg(long)]
    pub model: Option<String>,
}

/// One seeded account and the tenant it acts in.
struct Account {
    id: Uuid,
    tenant: TenantId,
}

/// Seed the coach, the group and the proposed link.
///
/// # Errors
///
/// Returns a configuration error when no model was resolved, an error if any
/// of the three accounts is missing or has no tenant, or if a repository write
/// fails.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    let model = args.model.ok_or_else(|| {
        AppError::config(
            "No model for the seeded group threads: pass --model, or configure the LLM \
             provider the server creates conversations with",
        )
    })?;
    let coach = account(repos, &args.coach_email).await?;
    let member = account(repos, &args.member_email).await?;
    let owner = account(repos, &args.owner_email).await?;
    if coaches_the_group(repos, &coach, &args.coach_email).await? {
        return Ok(());
    }

    seed_coach_account(repos, &coach).await?;
    let group = seed_group(repos, &owner, &coach, &member).await?;
    seed_member_link(repos, &group, &coach, &member, &model).await?;
    info!(
        "Seeded {GROUP_NAME}: {} proposed {ATHLETE_NAME} ({ATHLETE_ID}) to {}",
        args.coach_email, args.member_email
    );
    Ok(())
}

/// Whether the coach already coaches the seeded group, which makes a rerun a
/// no-op.
async fn coaches_the_group(
    repos: &RepositoryRegistry,
    coach: &Account,
    coach_email: &str,
) -> AppResult<bool> {
    let coached = repos.groups.list_groups_coached_by(coach.id).await?;
    let already = coached.iter().any(|group| group.name == GROUP_NAME);
    if already {
        info!("{coach_email} already coaches {GROUP_NAME}; nothing to seed");
    }
    Ok(already)
}

/// The member's and the coach's group threads, and the coach's link waiting
/// for the member.
///
/// A coach who redeems a coach invite gets a group thread, and that thread is
/// how they reach Group info and its roster section; the seeded coach is
/// attached directly, so it is filed here the same way.
async fn seed_member_link(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    coach: &Account,
    member: &Account,
    model: &str,
) -> AppResult<()> {
    for account in [member, coach] {
        repos
            .chat
            .create_conversation(
                &account.id.to_string(),
                account.tenant,
                GROUP_NAME,
                model,
                Some(&group.agent_id),
                Some(&group.id.to_string()),
            )
            .await?;
    }
    let link = DelegatedConnection::propose(
        SCIOTTE_TRAININGPEAKS.to_owned(),
        group.id,
        coach.id,
        coach.tenant,
        member.id,
        ATHLETE_ID.to_owned(),
        Some(ATHLETE_NAME.to_owned()),
    );
    repos
        .delegated_connections
        .propose(&link)
        .await?
        .map(|_| ())
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "A live TrainingPeaks link already holds athlete {ATHLETE_ID} or the member"
            ))
        })
}

/// The account behind `email`, with its tenant.
async fn account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
    let user = repos
        .seeder
        .seed_find_user_by_email(email)
        .await?
        .ok_or_else(|| {
            AppError::config(format!("User {email} not found; run seed demo-data first"))
        })?;
    let tenant = repos
        .seeder
        .seed_get_user_tenant(user.id)
        .await?
        .ok_or_else(|| AppError::config(format!("User {email} has no tenant_id")))?;
    let tenant = Uuid::parse_str(&tenant)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID for {email}: {e}")))?;
    Ok(Account {
        id: user.id,
        tenant: TenantId::from_uuid(tenant),
    })
}

/// The coach's own `TrainingPeaks`: a stand-in session recorded as a coach
/// account, the roster grant, and the current notice accepted.
async fn seed_coach_account(repos: &RepositoryRegistry, coach: &Account) -> AppResult<()> {
    let now = Utc::now();
    let session = json!({
        "session_id": format!("seed-coach-{}", coach.id.as_simple()),
        "cookies": [],
        "created_at": now.to_rfc3339(),
        "expires_at": null,
    });
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: coach.id,
            tenant_id: coach.tenant.to_string(),
            provider: SCIOTTE_TRAININGPEAKS.to_owned(),
            access_token: session.to_string(),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await?;
    repos
        .provider_connections
        .register_connection(
            coach.id,
            coach.tenant,
            SCIOTTE_TRAININGPEAKS,
            &ConnectionType::Manual,
            None,
        )
        .await?;
    repos
        .provider_connections
        .set_account_role(
            coach.id,
            coach.tenant,
            SCIOTTE_TRAININGPEAKS,
            ProviderAccountRole::Coach,
        )
        .await?;
    repos.users.set_manages_roster(coach.id, true).await?;
    let current = provider_terms_version(SCIOTTE_TRAININGPEAKS)
        .ok_or_else(|| AppError::internal("TrainingPeaks carries no exposure notice version"))?;
    repos
        .users
        .record_provider_terms(coach.id, SCIOTTE_TRAININGPEAKS, current)
        .await
}

/// The group: owned by `owner`, coached by `coach`, with the owner and the
/// member in it.
async fn seed_group(
    repos: &RepositoryRegistry,
    owner: &Account,
    coach: &Account,
    member: &Account,
) -> AppResult<CoachingGroup> {
    let agent_id = repos
        .agents
        .create(
            owner.id,
            owner.tenant,
            &CreateAgentRequest {
                title: format!("{GROUP_NAME} Agent"),
                description: Some(
                    "The agent of the seeded TrainingPeaks coaching group".to_owned(),
                ),
                system_prompt:
                    "You coach an endurance group whose human coach plans on TrainingPeaks."
                        .to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await?
        .id
        .to_string();

    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: GROUP_NAME.to_owned(),
                description: Some("Planned on TrainingPeaks by the group's coach".to_owned()),
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await?;
    for (account, role) in [(owner, GroupRole::Owner), (member, GroupRole::Member)] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: account.id,
                tenant_id: account.tenant.to_string(),
                role,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await?;
    }
    Ok(CoachingGroup {
        coach_user_id: Some(coach.id),
        ..group
    })
}
