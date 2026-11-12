// ABOUTME: The plan push against a calendar whose listing names identity only, as Garmin's does (carnet#1389)
// ABOUTME: An entry is read in detail only before it is overwritten or when it may be a lost write; lost writes are adopted by their writer and date

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc, Weekday};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    Activity, Athlete, CalendarEventRef, CalendarEventSource, CalendarKey, PlannedSession,
    PrescribedWorkout, SportType, Stats, TenantId,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_services::plan_calendar_push::{
    push_active_plan, PushPlanParams, PushReport, SkipReason,
};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

const PROVIDER: &str = "identity_only";

/// The one tag the calendar files every entry of Dravr's under, as Garmin
/// files every workout under the source `Dravr`.
const TAG: &str = "written-by-dravr";

/// One entry on the fake calendar.
#[derive(Debug, Clone)]
struct Entry {
    date: NaiveDate,
    /// Whether Dravr wrote it (the calendar files it under [`TAG`]).
    dravrs: bool,
    /// When someone other than Dravr last changed it, if anyone did.
    edited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct Calendar {
    entries: BTreeMap<String, Entry>,
    next_id: u32,
    detail_reads: Vec<String>,
    /// Entries whose detail read the provider refuses.
    unreadable: HashSet<String>,
    updates: Vec<String>,
}

/// A calendar whose listing names each entry's id and date and nothing else,
/// and whose detail read names the writer and the last change.
///
/// A stand-in rather than the Garmin provider because the reconcile is
/// provider-agnostic and these tests place entries another app or the
/// athlete made, and edits the athlete made, which only a calendar the test
/// owns can hold. Garmin's own detail read and key are pinned against its
/// wire in `pierre-providers`' `garmin_calendar_test`.
struct IdentityOnly {
    config: ProviderConfig,
    calendar: Mutex<Calendar>,
}

impl IdentityOnly {
    fn new() -> Self {
        Self {
            config: ProviderConfig {
                name: PROVIDER.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: Vec::new(),
            },
            calendar: Mutex::new(Calendar::default()),
        }
    }

    fn put(&self, id: &str, date: NaiveDate, dravrs: bool) {
        self.calendar.lock().unwrap().entries.insert(
            id.to_owned(),
            Entry {
                date,
                dravrs,
                edited_at: None,
            },
        );
    }

    fn edit(&self, id: &str, at: DateTime<Utc>) {
        self.calendar
            .lock()
            .unwrap()
            .entries
            .get_mut(id)
            .expect("entry on the calendar")
            .edited_at = Some(at);
    }

    /// Make the provider refuse every detail read of `id`.
    fn refuse_detail(&self, id: &str) {
        self.calendar
            .lock()
            .unwrap()
            .unreadable
            .insert(id.to_owned());
    }

    fn detail_reads(&self) -> Vec<String> {
        self.calendar.lock().unwrap().detail_reads.clone()
    }

    fn updates(&self) -> Vec<String> {
        self.calendar.lock().unwrap().updates.clone()
    }

    fn ids(&self) -> Vec<String> {
        self.calendar
            .lock()
            .unwrap()
            .entries
            .keys()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl FitnessProvider for IdentityOnly {
    fn name(&self) -> &'static str {
        PROVIDER
    }
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }
    async fn is_authenticated(&self) -> bool {
        true
    }
    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }
    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }
    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(Vec::new())
    }
    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(Vec::new(), None, None, false))
    }
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }
    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }

    fn calendar_key(&self, _external_id: &str) -> String {
        TAG.to_owned()
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        Ok(self
            .calendar
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|(_, entry)| (from..=to).contains(&entry.date))
            .map(|(id, entry)| CalendarEventRef {
                provider_event_id: id.clone(),
                external_id: None,
                date: entry.date,
                updated_at: None,
            })
            .collect())
    }

    async fn calendar_event_detail(&self, event: CalendarEventRef) -> AppResult<CalendarEventRef> {
        let mut calendar = self.calendar.lock().unwrap();
        calendar.detail_reads.push(event.provider_event_id.clone());
        if calendar.unreadable.contains(&event.provider_event_id) {
            return Err(AppError::external_service(
                PROVIDER,
                format!("{} could not be read", event.provider_event_id),
            ));
        }
        let entry = calendar
            .entries
            .get(&event.provider_event_id)
            .cloned()
            .ok_or_else(|| AppError::not_found(event.provider_event_id.clone()))?;
        Ok(CalendarEventRef {
            external_id: entry.dravrs.then(|| TAG.to_owned()),
            updated_at: entry.edited_at,
            ..event
        })
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        let mut calendar = self.calendar.lock().unwrap();
        calendar.next_id += 1;
        let id = format!("dravr-{}", calendar.next_id);
        calendar.entries.insert(
            id.clone(),
            Entry {
                date: session.date,
                dravrs: true,
                edited_at: None,
            },
        );
        Ok(id)
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        let mut calendar = self.calendar.lock().unwrap();
        let entry = calendar
            .entries
            .get_mut(provider_event_id)
            .ok_or_else(|| AppError::not_found(provider_event_id.to_owned()))?;
        entry.date = session.date;
        entry.dravrs = true;
        calendar.updates.push(provider_event_id.to_owned());
        Ok(())
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        let mut calendar = self.calendar.lock().unwrap();
        let before = calendar.entries.len();
        calendar
            .entries
            .retain(|id, _| !provider_event_ids.contains(id));
        Ok((before - calendar.entries.len()) as u64)
    }
}

struct Fixture {
    repos: Arc<RepositoryRegistry>,
    user_id: Uuid,
    tenant: TenantId,
    monday: NaiveDate,
}

async fn fixture() -> Result<Fixture> {
    use chrono::Datelike;
    use pierre_tool_runtime::runtime::ToolRuntime;

    common::init_server_config();
    let resources = create_test_server_resources().await?;
    let (user_id, user) = create_test_user(resources.database()).await?;
    let repos = resources.repos().clone();
    let tenant = repos
        .tenants
        .list_for_user(user.id)
        .await?
        .first()
        .expect("user has a tenant")
        .id;
    let mut monday = Utc::now().date_naive() + Duration::days(7);
    while monday.weekday() != Weekday::Mon {
        monday += Duration::days(1);
    }
    Ok(Fixture {
        repos,
        user_id,
        tenant,
        monday,
    })
}

impl Fixture {
    fn day(&self, offset: i64) -> NaiveDate {
        self.monday + Duration::days(offset)
    }

    /// Save the athlete's one plan week, `days` as `(offset, sport, workout)`;
    /// the first save lays the season outline, a later one re-saves the week.
    async fn save_week(&self, outline: bool, days: &[(i64, &str, &str)]) {
        let iso = |offset: i64| self.day(offset).format("%Y-%m-%d").to_string();
        let days: Vec<PlannedDay> = days
            .iter()
            .map(|(offset, sport, workout)| PlannedDay {
                date: iso(*offset),
                sport: (*sport).to_owned(),
                workout: (*workout).to_owned(),
                duration_min: Some(60),
                intensity: String::new(),
                steps: Vec::new(),
                fueling: None,
                template_slug: None,
                template_params: None,
                template_source: None,
                template_preset: None,
                envelope: None,
            })
            .collect();
        let goal = GoalRace {
            name: "Fall race".to_owned(),
            date: iso(70),
            discipline: "gravel".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: iso(0),
            weeks: 10,
            intent: "rebuild volume".to_owned(),
            target_hours: None,
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let week_start = iso(0);
        let weeks = [PlanWeekInput {
            week_start: &week_start,
            focus: "",
            days: &days,
            adjustment_reason: "",
            phase_index: None,
        }];
        self.repos
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.tenant.to_string(),
                user_id: &self.user_id.to_string(),
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: outline.then(|| PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &weeks,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .expect("save plan week");
    }

    async fn push(&self, calendar: &IdentityOnly) -> PushReport {
        push_active_plan(
            &self.repos,
            calendar,
            &PushPlanParams {
                tenant: self.tenant,
                user_id: self.user_id,
                provider: PROVIDER,
                from: Utc::now().date_naive(),
                week_notes: false,
                cancel: None,
            },
        )
        .await
        .expect("push")
    }

    /// The live ledger rows by their key.
    async fn live(&self) -> HashMap<String, PrescribedWorkout> {
        self.repos
            .prescribed_workouts
            .list_live_calendar_events(self.tenant, self.user_id, PROVIDER, None)
            .await
            .expect("live rows")
            .into_iter()
            .filter_map(|row| row.external_id.clone().map(|key| (key, row)))
            .collect()
    }

    fn plan_key(&self, offset: i64) -> String {
        CalendarKey::plan_day(self.user_id, self.day(offset), 0)
    }

    /// A live prescription row naming `event_id` on `date`, as
    /// `prescribe_workout` records one.
    async fn prescription_on(&self, date: NaiveDate, event_id: &str) {
        let now = Utc::now();
        self.repos
            .prescribed_workouts
            .upsert_prescribed_workout(&PrescribedWorkout {
                id: Uuid::new_v4(),
                tenant_id: self.tenant.as_uuid(),
                user_id: self.user_id,
                agent_id: None,
                template_slug: Some("easy-run".to_owned()),
                sport: SportType::Run,
                prescribed_for_date: date,
                provider: PROVIDER.to_owned(),
                provider_event_id: Some(event_id.to_owned()),
                external_id: Some(format!("dravr:prescription:{}", Uuid::new_v4())),
                source: CalendarEventSource::Prescription,
                plan_week_id: None,
                replaces_id: None,
                payload_hash: None,
                payload_json: "{}".to_owned(),
                status: PrescribedWorkout::STATUS_PUSHED.to_owned(),
                created_at: now,
                updated_at: now,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .expect("prescription row");
    }
}

#[tokio::test]
async fn an_entry_is_read_only_before_it_is_overwritten_and_an_athletes_edit_keeps_it() -> Result<()>
{
    let fx = fixture().await?;
    let calendar = IdentityOnly::new();
    fx.save_week(
        true,
        &[(0, "run", "Easy run"), (2, "bike", "Endurance ride")],
    )
    .await;

    let first = fx.push(&calendar).await;
    assert_eq!(
        (first.created, first.updated),
        (2, 0),
        "the first push creates both days: {first:?}"
    );
    let again = fx.push(&calendar).await;
    assert_eq!(again.unchanged, 2, "a re-push changes nothing: {again:?}");
    assert!(
        calendar.detail_reads().is_empty(),
        "an entry created or left unchanged is never read: {:?}",
        calendar.detail_reads()
    );

    // The agent changes both days; the athlete had changed Monday's entry on
    // the provider after Dravr wrote it.
    let live = fx.live().await;
    let monday_event = live[&fx.plan_key(0)].provider_event_id.clone().unwrap();
    let wednesday_event = live[&fx.plan_key(2)].provider_event_id.clone().unwrap();
    calendar.edit(&monday_event, Utc::now() + Duration::hours(1));
    fx.save_week(
        false,
        &[(0, "run", "Tempo run"), (2, "bike", "Sweet spot ride")],
    )
    .await;

    let report = fx.push(&calendar).await;
    assert_eq!(report.updated, 1, "Wednesday is updated: {report:?}");
    assert_eq!(
        report
            .skipped
            .iter()
            .map(|s| (s.external_id.clone(), s.reason))
            .collect::<Vec<_>>(),
        vec![(fx.plan_key(0), SkipReason::EditedOnProvider)],
        "bug: the athlete's edit on Monday was not seen in its detail read"
    );
    let mut reads = calendar.detail_reads();
    reads.sort();
    let mut expected = vec![monday_event.clone(), wednesday_event.clone()];
    expected.sort();
    assert_eq!(reads, expected, "each entry about to change is read once");
    assert_eq!(
        calendar.updates(),
        vec![wednesday_event],
        "the athlete's edit is never overwritten"
    );
    Ok(())
}

#[tokio::test]
async fn a_lost_write_is_adopted_by_its_writer_on_its_date_and_nothing_else_is_read() -> Result<()>
{
    let fx = fixture().await?;
    let calendar = IdentityOnly::new();
    fx.save_week(
        true,
        &[(0, "run", "Easy run"), (2, "bike", "Endurance ride")],
    )
    .await;

    // Monday holds another app's workout and a prescription Dravr's ledger
    // knows; Wednesday holds a write of Dravr's whose ledger row was lost;
    // Saturday, a day the plan leaves empty, holds another app's workout.
    calendar.put("other-monday", fx.day(0), false);
    calendar.put("prescribed-monday", fx.day(0), true);
    fx.prescription_on(fx.day(0), "prescribed-monday").await;
    calendar.put("lost-wednesday", fx.day(2), true);
    calendar.put("other-saturday", fx.day(5), false);

    let report = fx.push(&calendar).await;
    assert_eq!(
        (report.created, report.updated),
        (1, 1),
        "Monday is created, Wednesday adopted: {report:?}"
    );
    let mut reads = calendar.detail_reads();
    reads.sort();
    assert_eq!(
        reads,
        vec!["lost-wednesday".to_owned(), "other-monday".to_owned()],
        "only unclaimed entries on a lost key's date are read"
    );
    assert_eq!(
        calendar.updates(),
        vec!["lost-wednesday".to_owned()],
        "the lost write is the only entry updated"
    );
    let live = fx.live().await;
    assert_eq!(
        live[&fx.plan_key(2)].provider_event_id.as_deref(),
        Some("lost-wednesday"),
        "the lost write is the plan day's entry again, never a twin"
    );
    let monday_event = live[&fx.plan_key(0)].provider_event_id.clone().unwrap();
    assert!(
        !["other-monday", "prescribed-monday"].contains(&monday_event.as_str()),
        "neither another app's workout nor a prescription is taken over"
    );
    assert_eq!(
        calendar.ids().len(),
        5,
        "one entry created beside the four already there: {:?}",
        calendar.ids()
    );
    Ok(())
}

#[tokio::test]
async fn a_day_is_read_until_its_lost_write_is_found_and_no_further() -> Result<()> {
    let fx = fixture().await?;
    let calendar = IdentityOnly::new();
    fx.save_week(
        true,
        &[(0, "run", "Easy run"), (2, "bike", "Endurance ride")],
    )
    .await;
    // Monday holds two other apps' workouts ahead of Dravr's lost write and
    // one after it; Wednesday holds only other apps' workouts.
    calendar.put("a-other", fx.day(0), false);
    calendar.put("b-other", fx.day(0), false);
    calendar.put("c-lost", fx.day(0), true);
    calendar.put("d-other", fx.day(0), false);
    calendar.put("e-other", fx.day(2), false);
    calendar.put("f-other", fx.day(2), false);

    let report = fx.push(&calendar).await;
    assert_eq!(
        (report.updated, report.created),
        (1, 1),
        "Monday's lost write is adopted, Wednesday is created: {report:?}"
    );
    assert_eq!(
        calendar.detail_reads(),
        ["a-other", "b-other", "c-lost", "e-other", "f-other"].map(str::to_owned),
        "bug: a day was not read until its lost key paired, or was read past it"
    );
    assert_eq!(
        fx.live().await[&fx.plan_key(0)]
            .provider_event_id
            .as_deref(),
        Some("c-lost"),
        "bug: the lost write behind other apps' workouts was duplicated"
    );
    Ok(())
}

#[tokio::test]
async fn a_detail_read_that_fails_never_blocks_the_push() -> Result<()> {
    let fx = fixture().await?;
    let calendar = IdentityOnly::new();
    fx.save_week(true, &[(0, "run", "Easy run")]).await;
    let first = fx.push(&calendar).await;
    assert_eq!(first.created, 1, "the first push creates Monday: {first:?}");

    // The agent changes Monday and adds Wednesday, which holds a write of
    // Dravr's whose ledger row was lost; neither entry can be read in detail.
    let monday_event = fx.live().await[&fx.plan_key(0)]
        .provider_event_id
        .clone()
        .unwrap();
    calendar.put("lost-wednesday", fx.day(2), true);
    calendar.refuse_detail(&monday_event);
    calendar.refuse_detail("lost-wednesday");
    fx.save_week(
        false,
        &[(0, "run", "Tempo run"), (2, "bike", "Endurance ride")],
    )
    .await;

    let report = fx.push(&calendar).await;
    assert!(
        report.failed.is_empty(),
        "bug: a refused detail read failed an entry: {report:?}"
    );
    assert_eq!(
        (report.updated, report.created),
        (1, 1),
        "Monday is still updated; Wednesday, unread, is created: {report:?}"
    );
    let mut reads = calendar.detail_reads();
    reads.sort();
    let mut expected = vec!["lost-wednesday".to_owned(), monday_event.clone()];
    expected.sort();
    assert_eq!(reads, expected, "each refused entry was asked for once");
    assert_eq!(
        calendar.updates(),
        vec![monday_event],
        "bug: the entry whose detail read failed was not updated, or the unread one was"
    );
    let live = fx.live().await;
    assert_ne!(
        live[&fx.plan_key(2)].provider_event_id.as_deref(),
        Some("lost-wednesday"),
        "bug: an entry whose writer was never read was adopted"
    );
    Ok(())
}
