// ABOUTME: Batch fitness snapshot fetcher for group coaching context
// ABOUTME: Fetches CTL, ATL, TSB, and weekly volume for group members in parallel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group fitness snapshot service
//!
//! Provides batch fetching of fitness metrics (CTL, ATL, TSB, weekly volume)
//! for all members of a coaching group. Used by the group context injection
//! system to give the AI agent data-driven group advice.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Days, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use dravr_cageux::algorithms::training_load::{DailyTrainingLoad, TrainingLoadAlgorithm};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::training_load::TrainingLoadCalculator;
use futures_util::future::join_all;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::constants::time_constants::DAYS_PER_WEEK;
use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::models::groups::{
    FormMorning, MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity,
};
use pierre_core::models::{Activity, ProviderConnection, TenantId};
use pierre_core::models::{FormBand, FormReading};
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_fitness_compute::route_track::RouteTrack;
use pierre_fitness_compute::AthleteInputs;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::provider_notice::under_ai_consent;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::activity_fetch::connection_grant_beside_session;
use crate::activity_fetch::write_through::{write_through_activity_cache, WriteThrough};
use crate::group_activity_cache::fetch_member_activities;
use crate::implementations::stored_physiology::member_athlete_inputs;
use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};

/// Lookback (days of the athlete's calendar, their today included) for the
/// `recent_activities` roster list rendered into the group context. One week
/// is what agents ask about ("this week", "the weekend", "yesterday"); longer
/// windows blow the token budget.
const RECENT_ACTIVITIES_LOOKBACK_DAYS: i64 = 7;

/// Cap on the number of `recent_activities` rendered per member.
/// Protects the prompt budget against athletes who log many short
/// sessions (WHOOP can emit 5+ entries on a hard day).
const RECENT_ACTIVITIES_LIMIT: usize = 12;

/// Number of days of activity history to fetch for training load calculation.
///
/// CTL is a 42-day exponential average that cageux seeds at the history's
/// mean daily TSS over its first 42 days, so 60 days read a steady member's
/// load exactly; from a zero start they would hold only `1 - (41/42)^60`,
/// 76%, of it and band a daily rider as deep fatigue. A member whose load
/// changed within the fetched window is still read through the 1/N daily
/// update from that seed.
const TRAINING_LOAD_LOOKBACK_DAYS: i64 = 60;

// ══════════════════════════════════════════════════════════════
// Activity fetch across every provider
// ══════════════════════════════════════════════════════════════

/// Fetch a member's recent activities from every connected provider in
/// parallel, write each provider's answer through to the cache, and merge
/// every recording of one workout into one session ([`merge_duplicates`],
/// the one duplicate-workout merge): every provider's activities contribute
/// to the training load, giving a complete picture of the athlete's
/// workload.
///
/// `activity_limit` caps each provider's read — the caller passes
/// `ServerConfig::activity_fetch_limit`, so one env variable
/// (`ACTIVITY_FETCH_LIMIT`) governs every activity-fetching path.
pub(crate) async fn fetch_and_merge_all_providers(
    auth_service: &AuthService,
    providers: &[String],
    user_id: Uuid,
    tenant_id: TenantId,
    activity_limit: usize,
) -> Vec<Activity> {
    let tenant_id_str = tenant_id.to_string();
    let now = Utc::now();
    let lookback_start = (now - Duration::days(TRAINING_LOAD_LOOKBACK_DAYS)).timestamp();
    let params = ActivityQueryParams {
        limit: Some(activity_limit),
        offset: None,
        before: None,
        after: Some(lookback_start),
    };

    // Fetch from all providers in parallel
    let futures: Vec<_> = providers
        .iter()
        .map(|provider_name| {
            let tid = tenant_id_str.clone();
            let p = params.clone();
            let pname = provider_name.clone();
            async move {
                let result = try_fetch_from_provider(auth_service, &pname, user_id, &tid, &p).await;
                (pname, result)
            }
        })
        .collect();

    // Fetched and written through on the providers' full answers — the
    // cache is the athlete's. Each provider's rows are then filtered for a
    // model BEFORE the merge: a merge fills one recording's gaps from
    // another's, so filtering after it would let a withheld recording's
    // values ride in on a permitted one (carnet#723).
    let results = ai_scope::unfiltered(join_all(futures)).await;
    let policies = auth_service.runtime().provider_registry();

    // Merge all successful results
    let mut all_activities = Vec::new();
    let mut provider_count = 0u32;
    for (provider_name, maybe_fetched) in results {
        if let Some(fetched) = maybe_fetched {
            info!(
                user_id = %user_id,
                provider = %provider_name,
                count = fetched.activities.len(),
                head_complete = fetched.head_complete,
                "Snapshot: fetched from provider"
            );
            persist_fetched(
                auth_service,
                (user_id, tenant_id),
                &provider_name,
                &fetched,
                &params,
            )
            .await;
            all_activities.extend(ai_scope::filter_activities(
                policies.as_ref(),
                fetched.activities,
            ));
            provider_count += 1;
        }
    }

    if all_activities.is_empty() {
        return Vec::new();
    }

    // Merge every recording of one workout into one session
    let before_dedup = all_activities.len();
    let (merged, _) = merge_duplicates(all_activities, &DedupConfig::from_env());

    info!(
        user_id = %user_id,
        providers_used = provider_count,
        before_dedup,
        after_dedup = merged.len(),
        "Snapshot: merged activities from all providers"
    );

    merged
}

/// Write one provider's fetched activities through to the cache, keyed by
/// the connection's provider name, so later chat turns serve them from the
/// cache instead of re-fetching (stale-while-revalidate).
///
/// A capture missing its head is served but never persisted: the upsert and
/// the fetch mark would stamp it fresh, and the days it lacks would read as a
/// quiet week — the same gate `fetch_provider_head` applies (carnet#149,
/// carnet#151).
async fn persist_fetched(
    auth_service: &AuthService,
    (user_id, tenant_id): (Uuid, TenantId),
    provider_name: &str,
    fetched: &FetchedActivities,
    params: &ActivityQueryParams,
) {
    if fetched.head_complete {
        write_through_activity_cache(
            auth_service,
            user_id,
            tenant_id,
            provider_name,
            &fetched.activities,
            WriteThrough {
                read: params,
                announces_sessions: true,
            },
        )
        .await;
    } else {
        warn!(
            user_id = %user_id,
            provider = %provider_name,
            count = fetched.activities.len(),
            "Snapshot: capture is missing the list head; served without write-through"
        );
    }
}

/// Fetch fitness snapshots for a batch of group members in parallel.
///
/// For each user, creates an authenticated provider, fetches recent activities,
/// and computes training load metrics. Users without connected providers or
/// activity data receive snapshots with `None` metric fields.
///
/// Errors are handled per-user: a failed provider connection or activity fetch
/// does not prevent other members' snapshots from being returned.
///
/// Each member's reads run as reads of their data, so an attribution their
/// sessions owe is owed on them alone (carnet#1464).
pub async fn fetch_member_snapshots(
    runtime: &Arc<dyn ToolRuntime>,
    user_ids: &[Uuid],
    tenant_id: TenantId,
) -> Vec<MemberFitnessSnapshot> {
    let members: Vec<(Uuid, TenantId)> = user_ids.iter().map(|&id| (id, tenant_id)).collect();
    fetch_member_snapshots_in(runtime, &members).await
}

/// [`fetch_member_snapshots`] with each member's own fallback tenant.
///
/// For a view where a coach's read of a member resolved their one home
/// tenant ([`crate::coach_group_view`]), which their data is read under, as
/// the coach's other reads of them are. In `members`' order.
pub async fn fetch_member_snapshots_in(
    runtime: &Arc<dyn ToolRuntime>,
    members: &[(Uuid, TenantId)],
) -> Vec<MemberFitnessSnapshot> {
    if members.is_empty() {
        return Vec::new();
    }

    let futures: Vec<_> = members
        .iter()
        .map(|&(user_id, tenant_id)| {
            let runtime = Arc::clone(runtime);
            async move {
                ai_scope::reading_about(
                    user_id,
                    fetch_single_member_snapshot(&runtime, user_id, tenant_id),
                )
                .await
            }
        })
        .collect();

    join_all(futures).await
}

// The member display-name rule lives in `pierre_services::athlete_display_name`,
// which every build compiles: the snapshot roster, the peer-fetch tool and the
// plan tools' `athlete=` resolution all render a member by that one function.
use pierre_services::athlete_display_name::fetch_athlete_identity;

/// The training load on `day` of the athlete's calendar: CTL and ATL at its
/// end, form (`tsb` over `form_ctl`) from the end of the day before, a
/// session after it not counted. `None` when the calculation fails.
fn load_on(
    algorithm: &TrainingLoadAlgorithm,
    daily_tss: &BTreeMap<NaiveDate, f64>,
    day: NaiveDate,
) -> Option<DailyTrainingLoad> {
    match algorithm.daily_series(daily_tss, day, day) {
        Ok(mut series) => series.pop(),
        Err(e) => {
            debug!(error = ?e, "Training load calculation failed");
            None
        }
    }
}

/// Mornings, today included, the deep-fatigue run is counted back over at
/// most. A compute bound, not a threshold: each morning is one more load
/// calculation, and only while the run lasts.
const DEEP_FATIGUE_SCAN_MORNINGS: u32 = 21;

/// The consecutive mornings ending `as_of` whose form fell in
/// [`FormBand::DeepFatigue`], `as_of` first; empty when `today` is not in it.
///
/// `today` is the load already computed for `as_of`; each earlier morning is
/// the same calculation on the same days one day earlier (a session after
/// that day is not counted), so the run and the snapshot's own band never
/// disagree about today. Counting is all this does: the band is cageux's.
fn deep_fatigue_mornings(
    algorithm: &TrainingLoadAlgorithm,
    daily_tss: &BTreeMap<NaiveDate, f64>,
    as_of: NaiveDate,
    today: Option<&DailyTrainingLoad>,
) -> Vec<FormMorning> {
    let deep = |load: &DailyTrainingLoad| {
        let reading = FormReading::from_daily_load(load);
        (reading.band == FormBand::DeepFatigue)
            .then_some(reading.form_pct)
            .flatten()
    };
    let Some(form_pct) = today.and_then(deep) else {
        return Vec::new();
    };
    let mut run = vec![FormMorning {
        days_ago: 0,
        form_pct,
    }];
    for days_ago in 1..DEEP_FATIGUE_SCAN_MORNINGS {
        let Some(day) = as_of.checked_sub_days(Days::new(u64::from(days_ago))) else {
            break;
        };
        match load_on(algorithm, daily_tss, day).as_ref().and_then(deep) {
            Some(form_pct) => run.push(FormMorning { days_ago, form_pct }),
            None => break,
        }
    }
    run
}

/// Aggregated current-week metrics plus the prior week's volume trend input.
///
/// `previous_week_volume_km` is `None` when no activity falls in the
/// 7-to-14-day window — the group trend aggregation treats a missing prior
/// week as "no trend signal" rather than a zero-volume week.
#[derive(Debug)]
pub struct WeeklyMetrics {
    /// Number of activities in the trailing 7-day window.
    pub activity_count: i32,
    /// Distance (km) summed across the trailing 7-day window.
    pub volume_km: f64,
    /// Total active duration (seconds) across the trailing 7-day window.
    pub duration_seconds: u64,
    /// Distance (km) for the 7-to-14-day window, or `None` when that window
    /// holds no activities. Feeds the group-level weekly trend.
    pub previous_week_volume_km: Option<f64>,
    /// Days on the athlete's calendar from the one their most recent session
    /// started on to their today, or `None` when there are none.
    pub days_since_last: Option<i32>,
}

/// Compute weekly metrics from activities in the past 7 days, plus the prior
/// week's volume for trend detection.
///
/// `duration_seconds` is the sum of `Activity::duration_seconds()` across the
/// week — preserved separately from `volume_km` so HR/duration-only sources
/// (WHOOP, indoor trainers) surface as training volume even with no GPS.
/// `previous_week_volume_km` covers the 7-to-14-day window and feeds the
/// group-level weekly trend; the 60-day lookback already fetched that data.
///
/// Every window counts days of the athlete's calendar in `zone`, the one the
/// roster lists each session's day on (carnet#1640). This week is their today
/// and the six days before it, the week before is the seven days before
/// that, and `days_since_last` is one for a session yesterday where they
/// live at any hour: never 24-hour spans back from `now`, which move a
/// session between weeks at the hour of day it started.
#[must_use]
pub fn compute_weekly_metrics(
    activities: &[Activity],
    now: DateTime<Utc>,
    zone: Tz,
) -> WeeklyMetrics {
    let today = clock_date(now, zone);
    let days_back = |a: &Activity| days_before(today, a.start_date(), zone);
    // Days in one week, splitting the current week from the previous one.
    let week = i64::from(DAYS_PER_WEEK);
    let weekly_activities: Vec<_> = activities.iter().filter(|a| days_back(a) < week).collect();

    let weekly_activity_count = i32::try_from(weekly_activities.len()).unwrap_or(i32::MAX);

    let weekly_volume_km = weekly_activities
        .iter()
        .filter_map(|a| a.distance_meters())
        .sum::<f64>()
        / METERS_PER_KM;

    let weekly_duration_seconds: u64 = weekly_activities.iter().map(|a| a.duration_seconds()).sum();

    let previous_week: Vec<_> = activities
        .iter()
        .filter(|a| (week..2 * week).contains(&days_back(a)))
        .collect();
    let previous_week_volume_km = if previous_week.is_empty() {
        None
    } else {
        Some(
            previous_week
                .iter()
                .filter_map(|a| a.distance_meters())
                .sum::<f64>()
                / METERS_PER_KM,
        )
    };

    let days_since_last = activities
        .iter()
        .map(Activity::start_date)
        .max()
        .map(|last| i32::try_from(days_before(today, last, zone)).unwrap_or(i32::MAX));

    WeeklyMetrics {
        activity_count: weekly_activity_count,
        volume_km: weekly_volume_km,
        duration_seconds: weekly_duration_seconds,
        previous_week_volume_km,
        days_since_last,
    }
}

/// The days on the athlete's calendar in `zone` from the one a session
/// `started` on to `today`: zero for today, one for yesterday, negative for a
/// session dated after today.
fn days_before(today: NaiveDate, started: DateTime<Utc>, zone: Tz) -> i64 {
    (today - local_date(started, zone)).num_days()
}

/// One provider's answer to the snapshot fetch, with whether its capture
/// reached the list head. Only a complete capture may be written through.
struct FetchedActivities {
    activities: Vec<Activity>,
    head_complete: bool,
}

/// Try authenticating with one provider and fetching its activities.
///
/// Returns `Some` on success, `None` if auth or fetch fails.
async fn try_fetch_from_provider(
    auth_service: &AuthService,
    provider_name: &str,
    user_id: Uuid,
    tenant_id_str: &str,
    params: &ActivityQueryParams,
) -> Option<FetchedActivities> {
    let provider = auth_service
        .create_authenticated_provider(provider_name, user_id, Some(tenant_id_str))
        .await
        .map_err(|e| {
            info!(user_id = %user_id, provider = %provider_name, error = ?e, "Snapshot: auth failed");
        })
        .ok()?;

    let activities = provider
        .get_activities_with_params(params)
        .await
        .map_err(|e| {
            // WARN, not INFO: a failed live fetch is why the stale-while-revalidate
            // cache stops advancing. Logging it at INFO hid a Chrome profile-lock
            // collision that froze a user's activity data for a full day.
            warn!(user_id = %user_id, provider = %provider_name, error = %e, "Snapshot: fetch failed");
        })
        .ok()
        .filter(|a| !a.is_empty())?;

    debug!(user_id = %user_id, provider = %provider_name, count = activities.len(), "Snapshot: fetched");
    Some(FetchedActivities {
        activities,
        head_complete: provider.head_complete(),
    })
}

/// Collect the distinct tenants in which a member actually holds a provider
/// connection, falling back to `fallback_tenant_id` only when they hold none.
///
/// A member's OAuth token + `provider_connections` row are pinned to the tenant
/// that was active when they connected the provider (their earliest-joined
/// tenant — see `oauth_flow::get_user_and_tenant`), which is NOT necessarily the
/// group-host/requester tenant. Enumerating the member's own connection tenants
/// (from a cross-tenant lookup) and fetching each under its OWN tenant makes the
/// group snapshot resolve identically to the member's 1-1 chat, instead of
/// guessing a single tenant relative to the requester and silently reading
/// nothing when the guess misses.
fn member_connection_tenants(
    user_id: Uuid,
    connections: &[ProviderConnection],
    fallback_tenant_id: TenantId,
) -> Vec<TenantId> {
    let mut tenants: Vec<TenantId> = Vec::new();
    for conn in connections {
        match TenantId::parse_str(&conn.tenant_id) {
            Ok(t) if !tenants.contains(&t) => tenants.push(t),
            Ok(_) => {}
            Err(e) => info!(
                user_id = %user_id,
                tenant = %conn.tenant_id,
                error = %e,
                "Snapshot: skipping connection with unparseable tenant id"
            ),
        }
    }
    if tenants.is_empty() {
        tenants.push(fallback_tenant_id);
    }
    tenants
}

/// Fetch + merge a member's activities across every tenant where they hold a
/// provider connection.
///
/// Each tenant is fetched via [`fetch_member_activities`] (its own
/// stale-while-revalidate cache read + write-through), then results are merged
/// and deduplicated. The same provider connected under two tenants can surface
/// a workout twice, so a final dedup runs across the merged set even though each
/// per-tenant fetch already dedups within itself.
/// The second element is true when ANY tenant's rows were served stale — one
/// unrefreshed leg is enough to make the merged training load untrustworthy.
async fn fetch_member_activities_across_tenants(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenants: &[TenantId],
) -> (Vec<Activity>, bool) {
    let mut all = Vec::new();
    let mut any_stale = false;
    for &tenant_id in tenants {
        let (activities, served_stale) = fetch_member_activities(runtime, user_id, tenant_id).await;
        all.extend(activities);
        any_stale = any_stale || served_stale;
    }
    if tenants.len() > 1 {
        return (merge_duplicates(all, &DedupConfig::from_env()).0, any_stale);
    }
    (all, any_stale)
}

/// Fetch a single member's fitness snapshot.
///
/// Resolves the member's connections across all their tenants, fetches and
/// merges activities under each connection's own tenant, and computes training
/// load against the thresholds the member saved with `set_physiology`. Returns
/// a snapshot with `None` metrics if no provider is connected, if the fetch
/// fails, or if those thresholds cannot be read.
async fn fetch_single_member_snapshot(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    fallback_tenant_id: TenantId,
) -> MemberFitnessSnapshot {
    let now = Utc::now();
    let data = runtime.data();
    let identity = fetch_athlete_identity(data.repos(), user_id).await;
    let display_name = identity.display_name.clone();
    let timezone = identity.timezone.clone();

    // Cross-tenant lookup: a member's connections live under their own tenant,
    // which may differ from the requester/group-host tenant. See
    // [`member_connection_tenants`] for why guessing a single tenant fails.
    let connections = data
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_else(|e| {
            info!(
                user_id = %user_id,
                error = %e,
                "Snapshot: cross-tenant connection lookup failed; treating member as having no connections"
            );
            Vec::new()
        });

    let tenants = member_connection_tenants(user_id, &connections, fallback_tenant_id);
    // The member's data reaches the requester's model under the member's own
    // AI consents, not the requester's (carnet#726).
    let (activities, served_stale) = under_ai_consent(
        data.repos(),
        fallback_tenant_id.as_uuid(),
        user_id,
        fetch_member_activities_across_tenants(runtime, user_id, &tenants),
    )
    .await;
    // Emit one log line per fetched activity so an admin can verify
    // what reached the training-load calc when a snapshot looks stale
    // (e.g. "Phil rode 250km yesterday but ATL=19" — either the ride
    // isn't on the provider yet, or its TSS was skipped for missing
    // fields). debug! to keep the noise off INFO in normal operation.
    for a in &activities {
        debug!(
            user_id = %user_id,
            activity_id = %a.id(),
            name = %a.name(),
            sport = ?a.sport_type(),
            start = %a.start_date(),
            duration_s = a.duration_seconds(),
            distance_m = ?a.distance_meters(),
            avg_hr = ?a.average_heart_rate(),
            "Snapshot: activity included in training-load input"
        );
    }
    let mut snapshot = if activities.is_empty() {
        empty_snapshot(user_id, display_name, timezone, now)
    } else {
        // The thresholds the member saved with set_physiology, looked for in
        // the tenants their sessions came from. A profile that cannot be read
        // leaves the load unread rather than scoring their sessions as though
        // they had saved nothing.
        let athlete = member_athlete_inputs(runtime, &tenants, user_id)
            .await
            .inspect_err(|e| {
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "Snapshot: reading the member's saved thresholds failed; training load left unread"
                );
            })
            .ok();
        build_snapshot_from_activities(
            user_id,
            display_name,
            timezone,
            &activities,
            now,
            athlete.as_ref(),
            &runtime.cageux_config().algorithms,
        )
    };

    // Surface any provider whose connection died non-recoverably so the group agent can
    // name it ("Phil's WHOOP needs reconnecting") instead of treating the dead source as
    // merely quiet. Drawn from the same cross-tenant connection set so a dead provider is
    // named regardless of which tenant it lives in (a tenant-scoped re-query here was the
    // bug: it read nothing when the member's connection lived in a non-host tenant).
    // Named as the athlete knows it: a row stored under a mirror backend
    // (`sciotte_garmin`) is their Garmin, signed in with the username and
    // password they gave Dravr, so both halves of a pair name it once.
    // A grant behind a session that is not revoked is the session's
    // fallback, so its flag asks nothing of the athlete (carnet#1494): the
    // session's own flag names the provider.
    let mut needs_reauth: Vec<String> = Vec::new();
    let flagged = connections.iter().filter(|c| c.status.requires_reauth());
    for connection in flagged {
        if !connection_grant_beside_session(runtime, user_id, connection).await {
            needs_reauth.push(backend_resolver::user_facing_name(&connection.provider).to_owned());
        }
    }
    needs_reauth.sort_unstable();
    needs_reauth.dedup();
    snapshot.needs_reauth_providers = needs_reauth;
    snapshot.served_stale = served_stale;
    snapshot
}

/// Compute all fitness metrics from activities and assemble the snapshot.
///
/// `athlete` is the member's saved thresholds; `None` when they could not be
/// read, and the snapshot then carries no training load.
fn build_snapshot_from_activities(
    user_id: Uuid,
    display_name: String,
    timezone: Option<String>,
    activities: &[Activity],
    now: DateTime<Utc>,
    athlete: Option<&AthleteInputs>,
    algorithm_config: &AlgorithmConfig,
) -> MemberFitnessSnapshot {
    // As of the athlete's own today, in the zone the roster lists their days
    // in, so `computed_at` read on their clock names the day the form reading
    // describes and the deep-fatigue mornings are counted back from the day
    // the roster dates the first of them by (carnet#1640).
    //
    // The daily split is cageux's, the one `get_training_history` reads: each
    // session scored against the member's saved thresholds and counted on
    // the day it started where they live.
    let zone = resolve_zone(timezone.as_deref());
    let calculator = TrainingLoadCalculator::as_of_today(algorithm_config.clone(), zone, now);
    let today = calculator.as_of();
    let algorithm = calculator.algorithm();
    let daily = athlete.map(|athlete| athlete.daily_tss(calculator, activities));
    // Diagnoses a member with N workouts landing at an unexpectedly low
    // CTL/ATL (e.g. WHOOP entries with no distance or HR); cageux logs each
    // session it could not score.
    if let Some(daily) = &daily {
        info!(
            user_id = %user_id,
            total_activities = activities.len(),
            days_with_load = daily.len(),
            total_tss = daily.values().sum::<f64>(),
            "Snapshot TSS aggregate"
        );
    }
    let load = daily
        .as_ref()
        .and_then(|daily| load_on(&algorithm, daily, today));
    let deep_fatigue_mornings = daily.as_ref().map_or_else(Vec::new, |daily| {
        deep_fatigue_mornings(&algorithm, daily, today, load.as_ref())
    });
    let weekly = compute_weekly_metrics(activities, now, zone);
    let primary_sport = determine_primary_sport(activities);
    let overtraining_risk = assess_overtraining_risk(load.as_ref());
    let last_activity_per_provider = compute_last_activity_per_provider(activities);
    let recent_activities = compute_recent_activities(activities, now, zone);

    MemberFitnessSnapshot {
        user_id,
        display_name,
        ctl: load.as_ref().map(|l| l.ctl),
        atl: load.as_ref().map(|l| l.atl),
        tsb: load.as_ref().map(|l| l.tsb),
        form_ctl: load.as_ref().map(|l| l.form_ctl),
        weekly_volume_km: weekly.volume_km,
        previous_week_volume_km: weekly.previous_week_volume_km,
        weekly_activity_count: weekly.activity_count,
        weekly_duration_seconds: weekly.duration_seconds,
        primary_sport,
        vdot: None,
        overtraining_risk,
        days_since_last_activity: weekly.days_since_last,
        last_activity_per_provider,
        recent_activities,
        // Populated by the caller (fetch_single_member_snapshot), which holds the repos
        // needed to read connection status. Activity data alone can't tell needs_reauth.
        needs_reauth_providers: Vec::new(),
        // Also populated by the caller — freshness is a property of how the
        // activities were fetched, which this pure computation never sees.
        served_stale: false,
        timezone,
        deep_fatigue_mornings,
        computed_at: now,
    }
}

/// Build the compact per-activity list rendered into the group context.
///
/// Returns at most `RECENT_ACTIVITIES_LIMIT` activities from the last
/// `RECENT_ACTIVITIES_LOOKBACK_DAYS` days of the athlete's calendar in
/// `zone`, their today included, newest first: the days this week's volume
/// counts. The cap is applied after the day filter so a single heavy week
/// doesn't push older context into the prompt.
fn compute_recent_activities(
    activities: &[Activity],
    now: DateTime<Utc>,
    zone: Tz,
) -> Vec<RosterActivity> {
    let today = clock_date(now, zone);
    let first_day = today - Days::new(RECENT_ACTIVITIES_LOOKBACK_DAYS.unsigned_abs() - 1);
    let total_input = activities.len();
    let mut filtered: Vec<&Activity> = activities
        .iter()
        .filter(|a| days_before(today, a.start_date(), zone) < RECENT_ACTIVITIES_LOOKBACK_DAYS)
        .collect();
    let after_filter = filtered.len();
    filtered.sort_by_key(|a| Reverse(a.start_date()));
    let result: Vec<RosterActivity> = filtered
        .into_iter()
        .take(RECENT_ACTIVITIES_LIMIT)
        .map(|a| {
            // Where the trimmed overview picks the route up, never the raw
            // start a provider recorded, which is usually the member's door
            // (carnet#1592). A row with no overview that survives the trim
            // shares no coordinates at all.
            let shared_start: Option<(f64, f64)> = a
                .summary_polyline()
                .and_then(RouteTrack::trimmed_overview)
                .and_then(|track| track.first_point());
            RosterActivity {
                start: a.start_date(),
                sport: format!("{:?}", a.sport_type()),
                distance_km: a.distance_meters().map(|m| m / METERS_PER_KM),
                duration_seconds: a.duration_seconds(),
                name: display_line(a.name(), ACTIVITY_NAME_MAX_CHARS),
                city: a.city().map(str::to_owned),
                start_latitude: shared_start.map(|(latitude, _)| latitude),
                start_longitude: shared_start.map(|(_, longitude)| longitude),
                elevation_gain_m: a.elevation_gain(),
            }
        })
        .collect();
    info!(
        total_input,
        after_filter,
        emitted = result.len(),
        first_day = %first_day,
        "Snapshot: recent_activities computed"
    );
    result
}

/// Group activities by provider and keep the most-recent `start_date` per provider.
///
/// Gives the LLM a per-source freshness signal so it can say "Strava
/// stale 33 days, WHOOP current today" instead of pasting a single
/// `days_since_last_activity` over a multi-provider member and inventing
/// "your Strava is not synced" defenses. Keyed by the service the athlete
/// connected ([`backend_resolver::activity_origin`]), never a backend.
fn compute_last_activity_per_provider(activities: &[Activity]) -> HashMap<String, DateTime<Utc>> {
    let mut latest: HashMap<String, DateTime<Utc>> = HashMap::new();
    for activity in activities {
        let provider =
            backend_resolver::activity_origin(activity.provider(), activity.source()).to_owned();
        let start = activity.start_date();
        latest
            .entry(provider)
            .and_modify(|existing| {
                if start > *existing {
                    *existing = start;
                }
            })
            .or_insert(start);
    }
    latest
}

/// Create an empty snapshot for a user with no activity data.
fn empty_snapshot(
    user_id: Uuid,
    display_name: String,
    timezone: Option<String>,
    computed_at: DateTime<Utc>,
) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id,
        display_name,
        ctl: None,
        atl: None,
        tsb: None,
        form_ctl: None,
        weekly_volume_km: 0.0,
        previous_week_volume_km: None,
        weekly_activity_count: 0,
        weekly_duration_seconds: 0,
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: None,
        last_activity_per_provider: HashMap::new(),
        recent_activities: Vec::new(),
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        timezone,
        deep_fatigue_mornings: Vec::new(),
        computed_at,
    }
}

/// Determine the primary sport type from a list of activities.
///
/// Returns the most frequently occurring sport type, or `None` if empty.
fn determine_primary_sport(activities: &[Activity]) -> Option<String> {
    if activities.is_empty() {
        return None;
    }

    let mut counts = HashMap::new();
    for activity in activities {
        *counts.entry(activity.sport_type().clone()).or_insert(0u32) += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(sport, _)| format!("{sport:?}"))
}

/// Assess overtraining risk from the athlete's form band:
/// [`FormBand::DeepFatigue`] → High, [`FormBand::HeavyBlock`] → Moderate,
/// otherwise Low.
///
/// The ATL/CTL ratio is deliberately absent: because form is the previous
/// day's CTL minus ATL, `DeepFatigue` (form below -30% of that CTL) *is*
/// `atl > 1.3 * ctl` at the end of that day, so a ratio test alongside the band
/// is the same inequality counted twice — it read as a second corroborating
/// signal while adding no information. One axis, stated once.
///
/// An athlete with no chronic base bands as [`FormBand::InsufficientHistory`]
/// and is reported Low: at a near-zero CTL a single session swings the ratio
/// wildly, so there is no honest risk claim to make, and inventing one is how
/// beginners collected critical flags for an ordinary hard week.
fn assess_overtraining_risk(load: Option<&DailyTrainingLoad>) -> OvertrainingRiskLevel {
    match load.map_or(FormBand::InsufficientHistory, FormBand::from_daily_load) {
        FormBand::DeepFatigue => OvertrainingRiskLevel::High,
        FormBand::HeavyBlock => OvertrainingRiskLevel::Moderate,
        _ => OvertrainingRiskLevel::Low,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_fitness_compute::compute_training_history;

    use super::*;

    /// A run `days` before `now` lasting `seconds`.
    fn run(id: &str, now: DateTime<Utc>, days: i64, seconds: u64) -> Activity {
        ActivityBuilder::new(
            id,
            "Course",
            SportType::Run,
            now - Duration::days(days),
            seconds,
            "strava",
        )
        .distance_meters(6_100.0)
        .build()
    }

    #[test]
    fn roster_durations_keep_the_seconds() {
        // 53:59 truncated to whole minutes reads as 53, about 10 s/km slow on
        // 6.1 km (carnet#1015).
        let now = Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap();
        let activities = [run("a", now, 1, 3_239), run("b", now, 2, 1_801)];

        let recent = compute_recent_activities(&activities, now, Tz::UTC);
        let durations: Vec<u64> = recent.iter().map(|a| a.duration_seconds).collect();
        assert_eq!(durations, [3_239, 1_801]);

        let weekly = compute_weekly_metrics(&activities, now, Tz::UTC);
        assert_eq!(weekly.duration_seconds, 5_040);
    }

    #[test]
    fn freshness_is_keyed_by_the_service_never_the_backend() {
        let now = Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap();
        let scraped = ActivityBuilder::new(
            "g",
            "Ride",
            SportType::Ride,
            now - Duration::days(3),
            3_600,
            "sciotte",
        )
        .source("garmin")
        .build();
        let latest = compute_last_activity_per_provider(&[run("a", now, 1, 1_800), scraped]);
        let mut keys: Vec<&str> = latest.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["garmin", "strava"]);
    }

    const KIRITIMATI: &str = "Pacific/Kiritimati";
    const WEST_OF_THE_DATE_LINE: &str = "Etc/GMT+12";

    /// Sixty daily runs at 12:30 on the athlete's own calendar in `zone`,
    /// ending on their yesterday as of `now`; the last six long and hard.
    fn hard_week_ending_yesterday(now: DateTime<Utc>, zone: Tz) -> Vec<Activity> {
        let today = clock_date(now, zone);
        (1..=60_u64)
            .map(|back| {
                let (seconds, heart_rate) = if back <= 6 {
                    (14_400, 178)
                } else {
                    (1_800, 120)
                };
                let start = (today - Days::new(back))
                    .and_hms_opt(12, 30, 0)
                    .unwrap()
                    .and_local_timezone(zone)
                    .unwrap()
                    .with_timezone(&Utc);
                ActivityBuilder::new(
                    format!("run-{back}"),
                    "Course",
                    SportType::Run,
                    start,
                    seconds,
                    "strava",
                )
                .distance_meters(6_000.0)
                .average_heart_rate(heart_rate)
                .build()
            })
            .collect()
    }

    /// Ada's saved heart-rate thresholds, which score every fixture run.
    fn ada_thresholds() -> AthleteInputs {
        AthleteInputs {
            lthr: Some(161.5),
            max_hr: Some(190.0),
            resting_hr: Some(50.0),
            ..AthleteInputs::default()
        }
    }

    fn snapshot_at(now: DateTime<Utc>, zone: &str) -> MemberFitnessSnapshot {
        let activities = hard_week_ending_yesterday(now, zone.parse().unwrap());
        let athlete = ada_thresholds();
        build_snapshot_from_activities(
            Uuid::nil(),
            "Ada".to_owned(),
            Some(zone.to_owned()),
            &activities,
            now,
            Some(&athlete),
            &AlgorithmConfig::default(),
        )
    }

    /// The instants where UTC's day and the athlete's differ: east of UTC
    /// before UTC midnight (Kiritimati is already on the next day), west of
    /// it after (the date line's west is still on the day before), with the
    /// hours where the two agree beside them. Read on UTC's day, the fixture
    /// counts six mornings at the first two, seven at the third, six at the
    /// fourth and five at the last.
    fn readings() -> [(DateTime<Utc>, &'static str); 5] {
        [
            (
                Utc.with_ymd_and_hms(2026, 11, 17, 22, 0, 0).unwrap(),
                KIRITIMATI,
            ),
            (
                Utc.with_ymd_and_hms(2026, 11, 17, 11, 0, 0).unwrap(),
                KIRITIMATI,
            ),
            (
                Utc.with_ymd_and_hms(2026, 11, 17, 5, 0, 0).unwrap(),
                KIRITIMATI,
            ),
            (
                Utc.with_ymd_and_hms(2026, 11, 18, 2, 0, 0).unwrap(),
                WEST_OF_THE_DATE_LINE,
            ),
            (
                Utc.with_ymd_and_hms(2026, 11, 18, 13, 0, 0).unwrap(),
                WEST_OF_THE_DATE_LINE,
            ),
        ]
    }

    #[test]
    fn deep_fatigue_mornings_are_counted_to_the_athletes_own_today() {
        // Six hard days ending on her yesterday read six deep mornings ending
        // on her today in any zone at any hour, as get_training_history counts
        // them; read on UTC's day the same runs read five, six or seven
        // (carnet#1640).
        for (now, zone) in readings() {
            let snapshot = snapshot_at(now, zone);
            let days: Vec<u32> = snapshot
                .deep_fatigue_mornings
                .iter()
                .map(|m| m.days_ago)
                .collect();
            assert_eq!(days, [0, 1, 2, 3, 4, 5], "{zone} at {now}");
            assert_eq!(
                snapshot.form_reading().map(|r| r.band),
                Some(FormBand::DeepFatigue),
                "{zone} at {now}"
            );
        }
    }

    #[test]
    fn the_load_describes_the_athletes_own_day() {
        // Form is read for the athlete's civil day, the day the roster dates
        // it by, whichever UTC day the reading fell on.
        for (now, zone) in readings() {
            let snapshot = snapshot_at(now, zone);
            let tz = resolve_zone(Some(zone));
            let here = clock_date(now, tz);
            assert_eq!(snapshot.computed_on(), here, "{zone} at {now}");
            let config = AlgorithmConfig::default();
            let activities = hard_week_ending_yesterday(now, tz);
            let calculator = TrainingLoadCalculator::as_of_today(config.clone(), tz, now);
            let daily = ada_thresholds().daily_tss(calculator, &activities);
            let expected = load_on(&config.training_load_algorithm(), &daily, here).unwrap();
            assert_eq!(snapshot.ctl, Some(expected.ctl), "{zone} at {now}");
            assert_eq!(snapshot.tsb, Some(expected.tsb), "{zone} at {now}");
            // Each session on the day it started where she lives.
            assert_eq!(daily.len(), 60, "{zone} at {now}");
            assert_eq!(
                daily.keys().next_back(),
                here.pred_opt().as_ref(),
                "{zone} at {now}: the last run is bucketed on her yesterday"
            );
        }
    }

    #[test]
    fn the_roster_reads_the_load_the_training_history_reads_for_her_day() {
        // One daily split, cageux's: the roster's CTL, ATL and form on her
        // today are the history's row for that day, in every zone at every
        // hour (carnet#1663).
        for (now, zone) in readings() {
            let snapshot = snapshot_at(now, zone);
            let tz = resolve_zone(Some(zone));
            let here = clock_date(now, tz);
            let rows = compute_training_history(
                &hard_week_ending_yesterday(now, tz),
                ada_thresholds(),
                here - Days::new(5),
                here,
                &AlgorithmConfig::default(),
                Some(zone),
            )
            .unwrap();
            let row = rows.last().unwrap();
            assert_eq!(row.date, here, "{zone} at {now}");
            assert_eq!(snapshot.ctl, Some(row.ctl), "{zone} at {now}");
            assert_eq!(snapshot.atl, Some(row.atl), "{zone} at {now}");
            assert_eq!(snapshot.tsb, Some(row.tsb), "{zone} at {now}");
        }
    }

    #[test]
    fn a_session_without_training_stress_leaves_the_roster_load_alone() {
        // A provider-scored zero, two days before her first run: kept, it
        // would open her history there and lower the seed of every load.
        let (now, zone) = readings()[0];
        let tz: Tz = zone.parse().unwrap();
        let mut activities = hard_week_ending_yesterday(now, tz);
        let first = activities.iter().map(Activity::start_date).min().unwrap();
        activities.push(
            ActivityBuilder::new(
                "walk",
                "Marche",
                SportType::Walk,
                first - Duration::days(2),
                1_200,
                "whoop",
            )
            .training_stress_score(0.0)
            .build(),
        );
        let with_walk = build_snapshot_from_activities(
            Uuid::nil(),
            "Ada".to_owned(),
            Some(zone.to_owned()),
            &activities,
            now,
            Some(&ada_thresholds()),
            &AlgorithmConfig::default(),
        );
        let without = snapshot_at(now, zone);
        assert_eq!(
            with_walk.ctl, without.ctl,
            "bug: a zero-TSS session moved the roster CTL"
        );
        assert_eq!(
            with_walk.atl, without.atl,
            "bug: a zero-TSS session moved the roster ATL"
        );
        assert_eq!(
            with_walk.tsb, without.tsb,
            "bug: a zero-TSS session moved the roster form"
        );
    }

    #[test]
    fn days_since_the_last_session_are_the_athletes_calendar_days() {
        // Her last run was at 12:30 on her yesterday: one day ago at any hour,
        // never the 24-hour spans between instants, which read 0 to 2.
        for (now, zone) in readings() {
            let snapshot = snapshot_at(now, zone);
            assert_eq!(
                snapshot.days_since_last_activity,
                Some(1),
                "{zone} at {now}"
            );
        }
        // West of the date line at 23:00 UTC it is 11:00 her today, 22:30
        // after her last run began: still one calendar day.
        let late = Utc.with_ymd_and_hms(2026, 11, 18, 23, 0, 0).unwrap();
        let snapshot = snapshot_at(late, WEST_OF_THE_DATE_LINE);
        assert_eq!(
            snapshot.days_since_last_activity,
            Some(1),
            "{WEST_OF_THE_DATE_LINE} at {late}"
        );
    }

    #[test]
    fn the_weeks_are_the_athletes_calendar_days() {
        // One 6 km run a day at 12:30 on her calendar, the last on her
        // yesterday: this week is her today and the six days before it, six
        // runs; the week before holds the seven before that. Counted as
        // 24-hour spans back from now, the run seven days back moves into
        // this week whenever it is before 12:30 where she lives.
        for (now, zone) in readings() {
            let snapshot = snapshot_at(now, zone);
            assert_eq!(snapshot.weekly_activity_count, 6, "{zone} at {now}");
            assert!(
                (snapshot.weekly_volume_km - 36.0).abs() < 1e-9,
                "{zone} at {now}: {}",
                snapshot.weekly_volume_km
            );
            let previous = snapshot.previous_week_volume_km.unwrap();
            assert!(
                (previous - 42.0).abs() < 1e-9,
                "{zone} at {now}: {previous}"
            );
            let here = clock_date(now, resolve_zone(Some(zone)));
            let days: Vec<NaiveDate> = snapshot
                .recent_activities
                .iter()
                .map(|a| local_date(a.start, resolve_zone(Some(zone))))
                .collect();
            let expected: Vec<NaiveDate> = (1..=6).map(|back| here - Days::new(back)).collect();
            assert_eq!(days, expected, "{zone} at {now}");
        }
    }
}
