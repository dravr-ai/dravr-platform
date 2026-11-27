// ABOUTME: Seeds the account the connector-directory reviewers sign in with: a human coach with one group of three demo members
// ABOUTME: One member allowed the coach's assistant, one did not, one syncs only from Strava; the password comes from outside the source
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Directory reviewer seeder (carnet#1511, plan step 16, Q18).
//!
//! The reviewers of the Claude connector directory sign in to Dravr as a
//! human coach, so the listing they review is the coach's: the athlete tools
//! and the three coach tools, never an admin tool (JF, 2026-11-12). This
//! seeder creates that account on the environment it runs against, in its
//! own tenant:
//!
//! - **the coach**, a plain account, never an admin, holding coach access as
//!   an admin's grant (the one `pierre-cli user set --manages-roster` makes,
//!   which the `TrainingPeaks` reconciler never takes back), leading one
//!   active group with its agent;
//! - **three members** of that coach alone, each in their own tenant, every
//!   one sharing their training with the coach:
//!   - one who allowed the coach's assistant, with rides on Intervals.icu,
//!     whose terms let them reach an assistant outside Dravr: the roster
//!     overview names them with their training;
//!   - one who did not: the roster overview counts them, and a read by name
//!     through the `athlete` argument still reaches them;
//!   - one who allowed it and syncs only from Strava, whose data stays in
//!     Dravr: named, with their training withheld.
//!
//! The members' sessions are kept in the activity store the server reads,
//! which warms the cache with them, so the roster reads them at once. They
//! are keyed by calendar date over a rolling three-week window ending
//! yesterday, so the roster always reads recent training: a rerun drops the
//! sessions that fell out of the window and adds the days that entered it
//! (carnet#1511 call 29). No
//! provider token is written: a deployed build talks to the real providers,
//! and a stand-in token would read as a source that needs reconnecting.
//!
//! **The coach's password never lives in the source**, which is public. It is
//! read from `DIRECTORY_REVIEWER_PASSWORD` (or `--password`), held as a
//! [`SecretString`] that prints redacted, and is never printed or logged.
//! The members never sign in: each gets a random password nobody is told.
//!
//! It is idempotent and converges: a rerun creates only what is missing,
//! and a rerun the same day over a fully seeded account writes nothing, a
//! member who
//! deleted a seeded session included: that session stays deleted, and the
//! ones they kept are never rewritten. What a run that
//! failed part way left is completed by the next one: the members'
//! accounts, the coach's access, the group and its agent, the memberships,
//! the shares and opt-ins, the sessions. The group and its agent are held
//! under ids derived from the coach's account, so a rerun finds what an
//! earlier run wrote, and of two runs racing each other one writes them and
//! the other reads the winner's: they end on one group with one agent.
//!
//! What a rerun cannot converge it refuses in one read-only preflight,
//! before it writes the coach's access, the group or any member: the group
//! archived, coached or owned by another account, or in a workspace that is
//! not the coach's; an adopted coach account whose workspace another account
//! owns, or that already coaches or belongs to another group, or that would
//! reach, once the run is done, anyone but the members it makes (an athlete
//! it coaches, or a live member of the reviewer's group), whom the
//! reviewers would reach through the coach tools. Ten paths that open
//! reach, now or later, are refused on their existence: a coachless active
//! group the
//! account owns in any workspace it is a member of, other than the
//! reviewer's; a group invite the account holds, of either kind; an invite into
//! the reviewer's group (an open code of either kind, or any account's hold on
//! one of its codes); a private thread the account shares with another
//! account, as a participant in theirs or as the owner of one they were
//! added to; any other group the account coaches, owns or belongs to,
//! archived or active (an archive switched back on hands its coach and
//! members back; a group it owns another account coaches is someone else's
//! roster); a thread the account owns about an athlete it did not make,
//! which keeps their name and the coach's notes after the coaching ends;
//! the reviewer's group bound to a messaging chat it was created for, which
//! enrols whoever writes there; a room conversation of another group the
//! account keeps; a connection to a coaching platform, whose roster names
//! every athlete on it (a connection to any other provider reads the
//! account's own training); and a conversation the account owns whose rows
//! are about, or drew on, anyone it did not make. Also refused: a member who left the group, whom no join adds
//! back; an account or a workspace slug at a member's name that the
//! seeder did not create; a coach's or a member's workspace that is inactive
//! or has no owner; and an email, the coach's or a member's, that a deleted
//! account's live tombstone still holds, which the seeder never hands back
//! as an active account. Giving the reviewer's coachless group its coach back
//! is one write conditional on the group still being active with no coach:
//! a coach attached meanwhile is never overwritten, and the run stops,
//! unless the coach attached is this run's own (a racing run's write), which
//! converges. Every preflight refusal stands on every rerun for that coach,
//! masks the email it names, and names the remedy: seed another reviewer
//! with `--coach-email`; a refusal from the shared group seeder names its
//! own, seeding with another coach account. A rerun never changes a password: the coach account,
//! when it exists, must be active and already sign in with the password
//! passed, or the run is refused before anything is written. An account someone else made
//! at the coach email is adopted only by whoever passes its own password.
//!
//! **A member is the coach's own.** Its email and its tenant's slug derive
//! from the coach's account, so two coach emails never enrol the same
//! members. Anyone signed in may create a workspace with a slug of their
//! choosing, so the slug alone proves nothing; the proof is the account's
//! own workspace (`users.tenant_id`), which only a sign-up, the seeder or an
//! admin sets, and a sign-up never names it with a slug of its choosing. An
//! existing account at a member's email whose own workspace lacks that slug
//! is someone else's, as is a workspace holding the slug with no account at
//! the member's email, owned or not (its owner's account may be deleted):
//! the run is refused, unmodified.
//!
//! ```bash
//! DIRECTORY_REVIEWER_PASSWORD=… pierre-cli seed directory-reviewer
//! ```

use std::collections::BTreeSet;

use clap::builder::{StringValueParser, TypedValueParser};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::{INTERVALS_ICU, STRAVA};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::{CoachingGroup, GroupRole};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, TenantId, UserStatus,
};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use pierre_providers::registry::global_registry;
use pierre_services::live_activities::{LiveActivityStore, WriteOrigin};
use secrecy::{ExposeSecret, SecretString};
use tracing::info;
use uuid::Uuid;

use crate::accounts::{account, create_account, hash_password, Account, NewAccount};
use crate::coach_home::returning_coach;
use crate::coached_group::{
    add_member, group_agent, seed_coached_group_with_agent, GroupSpec, SeededGroupIds,
};

mod refusals;

/// The reviewer coach's group.
const GROUP_NAME: &str = "Directory Review Squad";

/// The plan every seeded account is on: one that enables every tool the
/// coach's listing carries.
const TIER: &str = "professional";

/// The shortest password the run accepts for the reviewer coach.
const MIN_PASSWORD_CHARS: usize = 16;

/// The longest password the run accepts, in bytes: bcrypt reads 72 bytes
/// including the NUL it appends, so a password of 72 bytes or more loses
/// that terminator and signs in with anything sharing its first 72 bytes.
/// 71 bytes and the terminator are read whole.
const MAX_PASSWORD_BYTES: usize = 71;

/// How the reviewer coach's members stand with the coach's assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// Allowed the coach's assistant; trains on a service whose data may
    /// reach it.
    Allowed,
    /// Shares with the coach and did not allow the coach's assistant.
    NotAllowed,
    /// Allowed the coach's assistant; syncs only from Strava, whose data
    /// stays in Dravr.
    StravaOnly,
}

/// The three members, in order: `(display name, standing)`. The first is
/// member 1 ([`member_key`]).
const MEMBERS: [(&str, Standing); 3] = [
    ("Ava Lambert", Standing::Allowed),
    ("Ben Okafor", Standing::NotAllowed),
    ("Cleo Strand", Standing::StravaOnly),
];

/// The domain of the members' emails, which belongs to no mailbox.
const MEMBER_EMAIL_DOMAIN: &str = "demo.pierre.dev";

/// The days the seeded training spans, ending yesterday: three weeks, so
/// the roster's recent week and its load always read some of it.
const WINDOW_DAYS: i64 = 21;

/// A member trains on every calendar day whose number since the epoch is a
/// multiple of this: an hour every third day, the same days on every run.
const SESSION_EVERY_DAYS: i64 = 3;

/// Milliseconds in a day.
const DAY_MS: i64 = 86_400_000;

/// When on its day a seeded session starts: 07:00 UTC.
const SESSION_START_MS: i64 = 7 * 3_600_000;

/// The most cached sessions of one source a rerun reads to find the seeded
/// ones to drop: a member account the seeder made holds only those.
const SEEDED_SESSIONS_READ: i64 = 1_000;

/// CLI arguments for the directory reviewer seeder.
#[derive(clap::Args, Debug)]
pub struct SeedArgs {
    /// The email the reviewers sign in with
    #[arg(long, default_value = "directory-reviewer@demo.pierre.dev")]
    pub coach_email: String,

    /// The password the reviewers sign in with. Required, and never printed.
    /// Prefer `DIRECTORY_REVIEWER_PASSWORD` over the flag: a flag value lands
    /// in the process argument list and the shell history
    #[arg(
        long,
        env = "DIRECTORY_REVIEWER_PASSWORD",
        hide_env_values = true,
        value_parser = StringValueParser::new().map(SecretString::from)
    )]
    pub password: SecretString,
}

/// Seed the reviewer coach, their group and its three members, the members'
/// sessions kept in the activity store `live`, which warms the cache with
/// them.
///
/// # Errors
///
/// Returns a configuration error when the password is shorter than
/// [`MIN_PASSWORD_CHARS`] or longer than [`MAX_PASSWORD_BYTES`], when the
/// adopted account or its group would reach anyone the run does not make
/// (each path the preflight refuses), when a member left the group, when the coach account exists and does not sign
/// in with the password passed, when an account the run would use already
/// exists as an admin, when an account or a workspace slug at a member's
/// name is someone else's, when a workspace the run acts in is inactive or
/// has no owner, or when a deleted account's live tombstone holds an email
/// the run would create an account at; the repository error from any read or write,
/// or the activity store's when it does not keep every session.
pub async fn run(
    args: SeedArgs,
    repos: &RepositoryRegistry,
    live: &dyn LiveActivityStore,
) -> AppResult<()> {
    password_fits(args.password.expose_secret())?;
    let coach = coach_account(repos, &args.coach_email, &args.password).await?;
    let spec = group_spec();
    let (ids, planned) = preflight(repos, &coach, &spec).await?;
    // The members never sign in: a random password, hashed once, told to no
    // one.
    let unusable = hash_password(&Uuid::new_v4().simple().to_string())?;
    let mut members = Vec::with_capacity(planned.len());
    for (plan, name, standing) in planned {
        members.push((
            member_account(repos, plan, name, &unusable).await?,
            standing,
        ));
    }
    if repos
        .users
        .manages_roster_admin_grant(coach.id)
        .await?
        .is_none()
    {
        repos
            .users
            .set_manages_roster_by_admin(coach.id, true, None)
            .await?;
    }
    let group = reviewer_group(repos, &coach, &spec, ids).await?;

    let now = Timestamp::now();
    for (member, standing) in members {
        join_group(repos, &group, &member, standing, now).await?;
        let provider = if standing == Standing::StravaOnly {
            STRAVA
        } else {
            INTERVALS_ICU
        };
        seed_sessions(repos, live, &member, provider, now).await?;
    }
    info!(
        "Seeded the directory reviewer {} leading {GROUP_NAME} with {} members",
        mask_email(&coach.email),
        MEMBERS.len()
    );
    Ok(())
}

/// The one read-only pass before any write but the coach's own account: the
/// ids of the coach's group and its agent, and a plan for each member.
///
/// Every state this run did not create and cannot converge is refused here,
/// with the coach's or the member's email masked and the remedy: the group
/// archived, coached by another account, or in a workspace that is not the
/// coach's ([`refuse_foreign_group`]); a member who left it, and an account
/// or a workspace slug at a member's name that is someone else's
/// ([`member_plan`]). Nothing past the coach's account is written before it
/// returns.
async fn preflight(
    repos: &RepositoryRegistry,
    coach: &Account,
    spec: &GroupSpec<'_>,
) -> AppResult<(SeededGroupIds, Vec<(MemberPlan, &'static str, Standing)>)> {
    let ids = SeededGroupIds::of(spec, coach, coach);
    refuse_foreign_group(repos, coach, ids.group).await?;
    let mut planned = Vec::with_capacity(MEMBERS.len());
    for (number, (name, standing)) in (1..).zip(MEMBERS) {
        planned.push((
            member_plan(repos, coach, number, ids.group).await?,
            name,
            standing,
        ));
    }
    refuse_reach_beyond_seeded(repos, coach, ids.group).await?;
    Ok((ids, planned))
}

/// Refuse unless the coach, once the run is done, reaches exactly the
/// members this seeder makes for them (carnet#1511 design call 14). Computed
/// once, before any write, over everything the run leaves the coach coaching
/// or belonging to:
///
/// - every group the coach coaches or belongs to now: any but the reviewer's
///   `group_id` is refused whole, since whoever joins it later is reached too
///   (a member reads the peers who share with the room);
/// - the group under `group_id` itself, coached or about to be given its
///   coach back (one a failed run or an admin left coachless): each of its
///   live members;
/// - every athlete the coach coaches.
///
/// Four paths open reach only later, so each is refused on its existence,
/// whoever is behind it now (anyone may join a group after the run):
///
/// - a coachless active group the coach owns other than `group_id`, in any
///   workspace the coach is a member of (their home one and every other):
///   the coach access the run grants lets its owner claim its coach seat
///   (`group_coach_claim`);
/// - a group invite the coach holds, a member's or the coach seat's, which
///   the app redeems on the account's next sign-in (`held_invite`);
/// - an invite into the group under `group_id` itself, whoever may redeem
///   it: a code still open (active, unexpired, under its use limit), of
///   either kind, or a hold any account took on one of its codes, open or
///   not. Whoever redeems it joins the group the reviewers read;
/// - a private thread the coach shares with another account, either way: one
///   another account owns that the coach was added to (whose rows are that
///   account's), or one the coach owns that another account was added to
///   (who keeps reading and posting beside the reviewers).
///
/// Each person reached but the coach must be one of the seeded members,
/// proven as [`member_plan`] proves them (it runs first, so an account at a
/// member's email it cannot prove is refused with its own reason). An
/// account that reached a real athlete would hand their training to every
/// directory reviewer through the coach tools.
///
/// More are refused on their existence (carnet#1511 calls 21 and 29):
///
/// - any other group the account coaches, owns or belongs to, archived or
///   active;
/// - a thread the account owns about an athlete it did not make;
/// - the group under `group_id` bound to a messaging chat it was created for;
/// - a room conversation the account owns or takes part in, bound to any
///   group but `group_id`: it stays listed to the account after it left the
///   group or stopped coaching it, with other members' words said to the
///   room;
/// - a token for a coach-roster provider (one whose descriptor declares
///   `COACH_ROSTER`): a roster read through it lists every athlete on that
///   roster, linked or not. A token for any other provider reads the
///   account's own training and is kept;
/// - a conversation the account owns, of any kind, holding a row about or
///   drawing on anyone but the account and the members it makes: the
///   account keeps reading those rows whatever it reaches now.
///
/// The reads see what the tools reach: active groups and live members, and
/// every group the account holds archived, which switching back on reaches.
async fn refuse_reach_beyond_seeded(
    repos: &RepositoryRegistry,
    coach: &Account,
    group_id: Uuid,
) -> AppResult<()> {
    let refuse = |whom: &str| {
        AppError::config(format!(
            "The account at {} {whom} this seeder did not make; it was not modified. \
             Seed another reviewer with --coach-email",
            mask_email(&coach.email)
        ))
    };
    let coached = repos.groups.list_groups_coached_by(coach.id).await?;
    if coached.iter().any(|group| group.id != group_id) {
        return Err(refuse("already coaches a group"));
    }
    let joined = repos.groups.list_groups_for_user(coach.id).await?;
    if joined.iter().any(|group| group.id != group_id) {
        return Err(refuse("already belongs to a group"));
    }
    if owns_another_coachless_group(repos, coach, group_id).await? {
        return Err(refuse("owns a coachless group"));
    }
    // The reads above see active groups alone, but an archive keeps its
    // coach and its live members and switching it back on hands them back:
    // a group the account coaches, owns or belongs to counts archived or not.
    if repos
        .groups
        .list_groups_held_by(coach.id)
        .await?
        .iter()
        .any(|group| *group != group_id)
    {
        return Err(refuse("holds another group, archived or owned"));
    }
    if repos
        .signup_codes
        .held_group_invite(coach.id)
        .await?
        .is_some()
    {
        return Err(refuse("holds an invite to a group"));
    }
    if group_invite_open(repos, group_id).await? {
        return Err(AppError::config(format!(
            "The group of the account at {} has an invite someone could still join it \
             through; it was not modified. Seed another reviewer with --coach-email",
            mask_email(&coach.email)
        )));
    }
    if repos.chat.shares_a_private_thread(coach.id).await? {
        return Err(refuse("shares a private thread with an account"));
    }
    // A room conversation is listed to its participants whatever their
    // membership now, and replays what the room's members said to it.
    if repos
        .chat
        .list_room_thread_groups(coach.id)
        .await?
        .iter()
        .any(|room| !Uuid::parse_str(room).is_ok_and(|room| room == group_id))
    {
        return Err(refuse("keeps a room conversation of another group"));
    }
    // A coaching platform's roster, read through the account's own
    // connection, names every athlete on it, linked to a group or not. A
    // connection to any other provider reads the account's own training.
    let roster_providers = global_registry().coach_roster_providers();
    if repos
        .oauth_tokens
        .list_token_providers(coach.id)
        .await?
        .iter()
        .any(|(_, provider)| roster_providers.contains(&provider.as_str()))
    {
        return Err(refuse("holds a coaching-platform connection"));
    }
    // A conversation the account owns keeps serving it what its rows
    // carry, whoever it reaches now: a reply about an athlete, or one that
    // drew on a member's data (carnet#1511 call 29).
    for person in repos
        .seeder
        .seed_conversation_read_members(coach.id)
        .await?
    {
        if person != coach.id && !seeded_member(repos, coach, person).await? {
            return Err(refuse("keeps a conversation about an athlete"));
        }
    }
    // A coach's thread about one athlete stays theirs after they stop
    // coaching them: titled with the athlete's name and holding the coach's
    // notes about them, it lists to the account whatever it coaches now.
    for athlete in repos.chat.list_subject_thread_athletes(coach.id).await? {
        if athlete != coach.id && !seeded_member(repos, coach, athlete).await? {
            return Err(refuse("holds a thread about an athlete"));
        }
    }
    let mut reached: BTreeSet<Uuid> = repos
        .groups
        .list_athletes_coached_by(coach.id)
        .await?
        .into_iter()
        .collect();
    let group = group_id.to_string();
    let existing = repos.groups.get_group(&group, coach.tenant).await?;
    // A group bound to a messaging chat it was created for (no web
    // connection) enrols every linked sender who writes there, with coach
    // sharing on: a way in with no invite and no hold.
    if existing.as_ref().is_some_and(|existing| {
        existing.channel_chat_id.is_some() && existing.chat_connection.is_none()
    }) {
        return Err(AppError::config(format!(
            "The group of the account at {} is bound to a messaging chat that enrols whoever \
             writes in it; it was not modified. Seed another reviewer with --coach-email",
            mask_email(&coach.email)
        )));
    }
    if existing.is_some() {
        reached.extend(
            repos
                .groups
                .list_members(&group)
                .await?
                .into_iter()
                .filter(|member| member.left_at.is_none())
                .map(|member| member.user_id),
        );
    }
    reached.remove(&coach.id);
    for person in reached {
        if !seeded_member(repos, coach, person).await? {
            return Err(refuse("would reach an athlete"));
        }
    }
    Ok(())
}

/// Whether `coach` owns an active coachless group other than `group_id` in
/// any workspace they are a member of, their home one included.
async fn owns_another_coachless_group(
    repos: &RepositoryRegistry,
    coach: &Account,
    group_id: Uuid,
) -> AppResult<bool> {
    let mut tenants = repos.tenants.list_membership_tenant_ids(coach.id).await?;
    if !tenants.contains(&coach.tenant) {
        tenants.push(coach.tenant);
    }
    for tenant in tenants {
        let owned = repos
            .groups
            .list_coachless_groups_owned_by(coach.id, tenant)
            .await?;
        if owned.iter().any(|group| group.id != group_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the group under `group_id` has an invite someone could still
/// join it through: a code that is active, unexpired and under its use
/// limit, of either kind, or a hold any account took on any of its codes
/// (counted open or not, lapsed or not, so a hold the app would still honour
/// is never missed). A group not created yet has none.
async fn group_invite_open(repos: &RepositoryRegistry, group_id: Uuid) -> AppResult<bool> {
    let now = Timestamp::now();
    let invites = repos.groups.list_invites(&group_id.to_string()).await?;
    let open_code = invites.iter().any(|invite| {
        invite.is_active
            && invite.expires_at.is_none_or(|expires| expires > now)
            && invite.max_uses.is_none_or(|limit| invite.use_count < limit)
    });
    if open_code {
        return Ok(true);
    }
    if invites.is_empty() {
        return Ok(false);
    }
    let tenants: BTreeSet<&str> = invites
        .iter()
        .map(|invite| invite.tenant_id.as_str())
        .collect();
    for tenant in tenants {
        let funnels = repos
            .signup_codes
            .group_invite_funnels(group_id, tenant, Timestamp::UNIX_EPOCH)
            .await?;
        if funnels.iter().any(|funnel| funnel.held_count > 0) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `athlete` is one of the members this seeder makes for `coach`:
/// an account at one of their emails whose own workspace carries its slug.
async fn seeded_member(
    repos: &RepositoryRegistry,
    coach: &Account,
    athlete: Uuid,
) -> AppResult<bool> {
    let Some(user) = repos.users.get_global(athlete).await? else {
        return Ok(false);
    };
    let Some(number) = (1..=MEMBERS.len()).find(|number| {
        user.email.eq_ignore_ascii_case(&format!(
            "{}@{MEMBER_EMAIL_DOMAIN}",
            member_key(coach, *number)
        ))
    }) else {
        return Ok(false);
    };
    let Some(tenant) = repos
        .seeder
        .seed_get_user_tenant(athlete)
        .await?
        .and_then(|tenant| TenantId::parse_str(&tenant).ok())
    else {
        return Ok(false);
    };
    Ok(refusals::workspace(repos, tenant)
        .await?
        .is_some_and(|workspace| workspace.slug == member_key(coach, number)))
}

/// Refuse the coach's group, when it exists under `group_id`, in a state a
/// rerun cannot converge: archived (a seeder never reactivates a group, and
/// filling an inactive one leaves the coach coaching nothing), coached by
/// another account (whose members the run would re-share with that coach),
/// owned by another account (whose group and agent the run would fill with
/// the reviewer's members), or in a workspace that is not the coach's
/// (where the coach cannot be given back). A group with no coach in the coach's own workspace is
/// completed: its coach is given back.
async fn refuse_foreign_group(
    repos: &RepositoryRegistry,
    coach: &Account,
    group_id: Uuid,
) -> AppResult<()> {
    let Some(group) = repos
        .groups
        .get_group(&group_id.to_string(), coach.tenant)
        .await?
    else {
        return Ok(());
    };
    let state = if !group.is_active {
        "is archived, and a seeder never reactivates a group"
    } else if group.coach_user_id.is_some_and(|other| other != coach.id) {
        "is coached by another account"
    } else if group.owner_id != coach.id {
        "is owned by another account"
    } else if group.tenant_id != coach.tenant.to_string() {
        "lives in another workspace than the coach's"
    } else {
        return Ok(());
    };
    Err(AppError::config(format!(
        "The {GROUP_NAME} of {} {state}; nothing was written. Seed another reviewer with \
         --coach-email",
        mask_email(&coach.email)
    )))
}

/// Refuse a password the reviewer coach could not sign in with exactly: one
/// shorter than [`MIN_PASSWORD_CHARS`], or longer than bcrypt reads
/// ([`MAX_PASSWORD_BYTES`]). Neither message repeats the password.
fn password_fits(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::config(format!(
            "The directory reviewer's password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AppError::config(format!(
            "The directory reviewer's password must be at most {MAX_PASSWORD_BYTES} bytes: \
             bcrypt ignores what follows"
        )));
    }
    Ok(())
}

/// The group the coach leads, under `ids`: created with its agent unless an
/// earlier or a racing run created them, and given its coach back if it was
/// taken off. A run that failed after creating the agent finds it again
/// under its id, so no second agent is ever made; a group that exists
/// already runs the agent made with it, so none is made for it.
async fn reviewer_group(
    repos: &RepositoryRegistry,
    coach: &Account,
    spec: &GroupSpec<'_>,
    ids: SeededGroupIds,
) -> AppResult<CoachingGroup> {
    let existing = repos
        .groups
        .get_group(&ids.group.to_string(), coach.tenant)
        .await?;
    if let Some(group) = existing
        .as_ref()
        .filter(|group| group.coach_user_id == Some(coach.id))
    {
        return Ok(group.clone());
    }
    returning_coach(repos, coach).await?;
    if existing.is_none() {
        group_agent(repos, spec, coach, ids).await?;
    }
    seed_coached_group_with_agent(repos, spec, ids, coach, coach, &[]).await
}

/// `member` in `group`, sharing their training with its coach, and allowed
/// the coach's assistant exactly when their `standing` says so: each written
/// only when it is not so already. A member who left the group was refused
/// before the run wrote anything ([`member_plan`]).
async fn join_group(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    member: &Account,
    standing: Standing,
    now: Timestamp,
) -> AppResult<()> {
    let group_id = group.id.to_string();
    if repos
        .groups
        .get_member(&group_id, member.id)
        .await?
        .is_none()
    {
        add_member(repos, group.id, member, GroupRole::Member, true).await?;
    }
    let joined = repos
        .groups
        .get_member(&group_id, member.id)
        .await?
        .ok_or_else(|| not_a_member(member))?;
    if !joined.coach_sharing_consent
        && !repos
            .groups
            .update_coach_sharing_consent(&group_id, member.id, true)
            .await?
    {
        return Err(not_a_member(member));
    }
    let allowed = standing != Standing::NotAllowed;
    if joined.coach_consents.assistant_read != allowed {
        repos
            .groups
            .set_coach_assistant_read(&group_id, member.id, allowed, now)
            .await?
            .ok_or_else(|| not_a_member(member))?;
    }
    Ok(())
}

fn not_a_member(member: &Account) -> AppError {
    AppError::internal(format!(
        "{} is no live member of {GROUP_NAME}",
        mask_email(&member.email)
    ))
}

fn group_spec() -> GroupSpec<'static> {
    GroupSpec {
        name: GROUP_NAME,
        description: "The squad the connector-directory reviewers coach",
        agent_description: "The agent of the directory reviewers' squad",
        agent_prompt: "You coach the athletes of this squad beside their human coach.",
        max_members: 10,
    }
}

/// The reviewer coach's account, created with `password` when a previous run
/// has not.
///
/// An account that already exists is used only when it is active, signs in
/// with `password` and works in a workspace of its own: a rerun never
/// changes a password or a status, never leaves the one passed silently
/// unused, and never writes the reviewer's group into someone else's
/// workspace ([`refusals::refuse_shared_workspace`]). Refused too when the account
/// exists as an admin: an admin is listed every tool, and the directory's
/// listing is a coach's. Every refusal comes before anything is written.
async fn coach_account(
    repos: &RepositoryRegistry,
    email: &str,
    password: &SecretString,
) -> AppResult<Account> {
    // Created by this run only when no account is there, even one another
    // run created a moment ago: that one is checked as any existing account.
    let absent = repos.seeder.seed_check_user_exists(email).await?.is_none();
    if absent {
        refusals::refuse_deleted_email(repos, email).await?;
    }
    let created = absent
        && create_account(
            repos,
            &NewAccount {
                email,
                display_name: "Directory Reviewer",
                password_hash: &hash_password(password.expose_secret())?,
                tier: TIER,
                status: "active",
                tenant_slug: None,
            },
        )
        .await?
        .is_some();
    if created {
        return reviewer_account(repos, email).await;
    }
    let coach = reviewer_account(repos, email).await?;
    refuse_admin(repos, &coach).await?;
    let stored = repos
        .users
        .get_global(coach.id)
        .await?
        .ok_or_else(|| AppError::config(format!("{} not found", mask_email(email))))?;
    // A suspended or pending account signs in with no password: a run that
    // adopted it would report a reviewer the directory cannot reach.
    if stored.user_status != UserStatus::Active {
        return Err(AppError::config(format!(
            "The account at {} is not active, so it cannot sign in; it was not modified. \
             Reactivate it, or choose another --coach-email",
            mask_email(email)
        )));
    }
    // A hash that cannot be read reads as another password.
    if !bcrypt::verify(password.expose_secret(), &stored.password_hash).unwrap_or(false) {
        return Err(AppError::config(format!(
            "An account already exists at {} and does not sign in with the password passed; it \
             was not modified. Pass that account's password, or choose another --coach-email",
            mask_email(email)
        )));
    }
    refusals::refuse_shared_workspace(repos, &coach).await?;
    Ok(coach)
}

/// What names member `number` of `coach`: the local part of their email and
/// their tenant's slug alike. It derives from the coach's account, so another
/// coach email never reaches the same members, and fits the 64 characters an
/// email's local part may hold.
fn member_key(coach: &Account, number: usize) -> String {
    format!("directory-reviewer-{}-member-{number}", coach.id.simple())
}

/// What a run does for member `number` of `coach`, decided before it writes
/// anything for the coach's access, the group or any member.
enum MemberPlan {
    /// The account the seeder created for this coach, already there.
    Existing(Account),
    /// No account yet, and the member's workspace slug is free: created by
    /// the run.
    Create {
        /// The member's email.
        email: String,
        /// Their email's local part and their workspace's slug.
        key: String,
    },
}

/// Member `number` of `coach`, read and proven without writing anything.
///
/// An account that already exists at the member's email is the member only
/// when it is the one the seeder created for this coach: its own workspace
/// carries the member's slug. Refused too when it is an admin, or when it
/// left the coach's group `group_id`, which no join undoes. With no account
/// there, the member's slug must be free: a workspace holding it is
/// someone else's, and creating the member would fail on it. The slug is
/// read before the account, so a racing run that creates the member
/// between the two reads is seen as the account it created.
async fn member_plan(
    repos: &RepositoryRegistry,
    coach: &Account,
    number: usize,
    group_id: Uuid,
) -> AppResult<MemberPlan> {
    let key = member_key(coach, number);
    let email = format!("{key}@{MEMBER_EMAIL_DOMAIN}");
    // Read from the workspaces themselves: one whose owner's account was
    // deleted lists under no owner, yet still holds the slug.
    let slug_held = repos.seeder.seed_slug_taken(&key).await?;
    if repos.seeder.seed_check_user_exists(&email).await?.is_none() {
        if slug_held {
            return Err(AppError::config(format!(
                "A workspace already holds the slug of {}, but no account at that email owns it: \
                 someone else made it, or the member this seeder made there was deleted; \
                 nothing was written. Seed another reviewer with --coach-email",
                mask_email(&email)
            )));
        }
        refusals::refuse_deleted_email(repos, &email).await?;
        return Ok(MemberPlan::Create { email, key });
    }
    let member = reviewer_account(repos, &email).await?;
    proven_member(repos, &member, &key).await?;
    if repos
        .seeder
        .seed_member_has_left(&member.tenant.to_string(), group_id, member.id)
        .await?
    {
        return Err(AppError::config(format!(
            "{} left {GROUP_NAME}, and a member who left a group is never added back; nothing \
             was written. Seed another reviewer with --coach-email",
            mask_email(&member.email)
        )));
    }
    Ok(MemberPlan::Existing(member))
}

/// The member `plan` names, created now when it planned to: each in their
/// own tenant, as a coached read refuses an athlete in several.
async fn member_account(
    repos: &RepositoryRegistry,
    plan: MemberPlan,
    name: &str,
    password_hash: &str,
) -> AppResult<Account> {
    let (email, key) = match plan {
        MemberPlan::Existing(member) => return Ok(member),
        MemberPlan::Create { email, key } => (email, key),
    };
    // A racing run that created the member first leaves this a no-op, and
    // the account read below is the one it created.
    create_account(
        repos,
        &NewAccount {
            email: &email,
            display_name: name,
            password_hash,
            tier: TIER,
            status: "active",
            tenant_slug: Some(&key),
        },
    )
    .await?;
    let member = reviewer_account(repos, &email).await?;
    proven_member(repos, &member, &key).await?;
    Ok(member)
}

/// Refuse `member` unless it is a plain account whose own workspace carries
/// `key`, the slug the seeder gives the member it creates.
async fn proven_member(repos: &RepositoryRegistry, member: &Account, key: &str) -> AppResult<()> {
    refuse_admin(repos, member).await?;
    if refusals::live_workspace(repos, member).await?.slug != key {
        return Err(AppError::config(format!(
            "An account already exists at {} that this seeder did not create for this coach; it \
             was not modified. Remove it, or choose another --coach-email",
            mask_email(&member.email)
        )));
    }
    Ok(())
}

/// [`account`], its refusal naming the remedy every refusal of this seeder
/// names.
async fn reviewer_account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
    account(repos, email).await.map_err(|e| {
        if e.code == ErrorCode::ConfigError {
            AppError::config(format!(
                "{}. Seed another reviewer with --coach-email",
                e.message
            ))
        } else {
            e
        }
    })
}

async fn refuse_admin(repos: &RepositoryRegistry, seeded: &Account) -> AppResult<()> {
    let is_admin = repos
        .users
        .get_global(seeded.id)
        .await?
        .is_some_and(|user| user.role.is_admin());
    if is_admin {
        return Err(AppError::config(format!(
            "{} is an admin, and the directory reviewer's accounts are plain accounts; it was \
             not modified. Seed another reviewer with --coach-email",
            mask_email(&seeded.email)
        )));
    }
    Ok(())
}

/// The member's rides on `provider`: the source connected, the sessions kept
/// in the activity store, which projects them into the cache.
///
/// The sessions are keyed by calendar date over a rolling window
/// ([`WINDOW_DAYS`] ending yesterday, carnet#1511 call 29), so the roster
/// tools, which read the present, always find recent training. A seeded
/// session that fell out of the window is dropped; within the window only
/// the sessions the cache does not serve are written: a rerun never
/// rewrites or re-dates one the member kept. One the member deleted is
/// offered again and refused by the store, which writes nothing, so a
/// member who deleted one has converged on the rest, and a rerun the same
/// day writes nothing for them.
async fn seed_sessions(
    repos: &RepositoryRegistry,
    live: &dyn LiveActivityStore,
    member: &Account,
    provider: &str,
    now: Timestamp,
) -> AppResult<()> {
    let connected = repos
        .provider_connections
        .get_for_user(member.id, Some(member.tenant))
        .await?
        .iter()
        .any(|connection| connection.provider == provider);
    if !connected {
        repos
            .provider_connections
            .register_connection(
                member.id,
                member.tenant,
                provider,
                &ConnectionType::Synthetic,
                Some(r#"{"source": "seed-directory-reviewer"}"#),
            )
            .await?;
    }
    let window: Vec<Activity> = session_days(now)
        .into_iter()
        .map(|day| ride(member, provider, day))
        .collect::<AppResult<_>>()?;
    let in_window: BTreeSet<&str> = window.iter().map(Activity::id).collect();
    let prefix = session_prefix(member);
    let epoch = Timestamp::UNIX_EPOCH;
    let cached = repos
        .activity_cache
        .get_cached_activities(
            member.id,
            &member.tenant,
            Some(provider),
            epoch,
            now + Elapsed::days(1),
            SEEDED_SESSIONS_READ,
        )
        .await?;
    let mut kept: BTreeSet<String> = BTreeSet::new();
    for session in cached {
        if !session.id().starts_with(&prefix) {
            continue;
        }
        if in_window.contains(session.id()) {
            kept.insert(session.id().to_owned());
        } else {
            // Out of the window: dropped without refusing it, as nothing
            // offers it again.
            live.discard(member.id, member.tenant, provider, session.id())
                .await?;
        }
    }
    let sessions: Vec<Activity> = window
        .into_iter()
        .filter(|session| !kept.contains(session.id()))
        .collect();
    if sessions.is_empty() {
        return Ok(());
    }
    let kept = live
        .ingest(
            member.id,
            member.tenant,
            provider,
            &sessions,
            now,
            WriteOrigin::Import,
        )
        .await?;
    // A session the member deleted is refused on every later write, by
    // design: a seeder never brings it back, so the member converges on the
    // sessions they kept. Any other shortfall would leave their history
    // short with nothing failing: the seed stops instead.
    let seeded = u64::try_from(sessions.len()).unwrap_or(u64::MAX);
    if kept.stored.saturating_add(kept.refused) != seeded {
        return Err(AppError::internal(format!(
            "the activity store kept {} of {} seeded sessions",
            kept.stored,
            sessions.len()
        )));
    }
    if kept.refused > 0 {
        info!(
            "{} of a directory reviewer member's seeded sessions were deleted and stay so",
            kept.refused
        );
    }
    Ok(())
}

/// The calendar days, counted from the epoch, the member trained on in the
/// window ending the day before `now`: every [`SESSION_EVERY_DAYS`]th day,
/// oldest first. The window spans a whole number of those, so it always
/// holds the same count.
fn session_days(now: Timestamp) -> Vec<i64> {
    let today = now.epoch_millis().div_euclid(DAY_MS);
    (today - WINDOW_DAYS..today)
        .filter(|day| day.rem_euclid(SESSION_EVERY_DAYS) == 0)
        .collect()
}

/// What every seeded session id of `member` starts with.
fn session_prefix(member: &Account) -> String {
    format!("directory-reviewer-{}-", member.id.simple())
}

/// An hour's ride at 30 km/h on calendar `day` (days since the epoch), its id
/// unique to the member and the date and the same on every run.
///
/// # Errors
///
/// Returns an internal error for a day no instant represents.
fn ride(member: &Account, provider: &str, day: i64) -> AppResult<Activity> {
    let start = Timestamp::from_epoch_millis(day * DAY_MS + SESSION_START_MS)
        .map_err(|e| AppError::internal(format!("day {day} is not an instant: {e}")))?;
    Ok(ActivityBuilder::new(
        format!("{}day{day}", session_prefix(member)),
        "Endurance ride".to_owned(),
        SportType::Ride,
        start,
        3_600,
        provider.to_owned(),
    )
    .distance_meters(30_000.0)
    .average_heart_rate_opt(Some(138))
    .build())
}

#[cfg(test)]
mod tests {
    use super::{
        global_registry, password_fits, INTERVALS_ICU, MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS,
    };

    #[test]
    fn a_password_must_fit_what_bcrypt_reads() {
        let longest = "p".repeat(MAX_PASSWORD_BYTES);
        assert!(
            password_fits(&longest).is_ok(),
            "71 bytes and bcrypt's terminator are read whole"
        );
        // bcrypt 0.16 appends a NUL before cutting at 72 bytes: a 72-byte
        // password loses it and signs in with any suffix.
        let at_bcrypts_limit = "p".repeat(72);
        assert!(
            password_fits(&at_bcrypts_limit).is_err(),
            "bug: a 72-byte password signs in with any suffix"
        );
        assert!(
            bcrypt::verify(
                format!("{at_bcrypts_limit}-tail"),
                &bcrypt::hash(&at_bcrypts_limit, 4).unwrap()
            )
            .unwrap(),
            "bcrypt ignores what follows a 72-byte password, which is why it is refused"
        );
        assert!(
            password_fits(&"p".repeat(MIN_PASSWORD_CHARS)).is_ok(),
            "the shortest allowed"
        );
        let too_long = format!("{longest}-tail");
        let refused = password_fits(&too_long).expect_err("bcrypt would ignore the tail");
        assert!(
            !refused.to_string().contains(&too_long) && !format!("{refused:?}").contains(&too_long),
            "never the password: {refused}"
        );
        // Multi-byte characters count as bytes: 25 four-byte characters are
        // 25 characters and 100 bytes.
        for unfit in ["\u{1F6B4}".repeat(25), "p".repeat(MIN_PASSWORD_CHARS - 1)] {
            assert!(password_fits(&unfit).is_err(), "refused: {unfit}");
        }
    }

    /// The preflight refuses a coaching platform's roster connection by the
    /// registry's list, which holds only the providers this binary compiles:
    /// a `pierre-cli` built on its own must still know the server's.
    #[test]
    fn the_preflight_knows_every_roster_provider_the_server_ships() {
        let roster = global_registry().coach_roster_providers();
        assert!(
            roster.contains(&INTERVALS_ICU),
            "bug: an Intervals.icu coach key passes the preflight: {roster:?}"
        );
    }
}
