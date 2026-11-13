// ABOUTME: The coach roster's training half: bounded snapshot fetches for a page and the typed per-athlete reading
// ABOUTME: Read by get_roster_overview and GET /api/coach/roster on top of pierre_services::coach_roster
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach roster — training
//!
//! The roster itself (who may read it, who is on it, their consent and the
//! order) is [`pierre_services::coach_roster`]. This module reads training on
//! top of it, for the agent tool (`get_roster_overview`) and the coach's REST
//! read (`GET /api/coach/roster`, carnet#1251) alike: the gate each sharer of
//! a page passes before anything of theirs is read ([`gate_page_reads`]), the
//! snapshots of the sharers it lets through, a bounded number at a time, and
//! the typed reading both project from — form only as the server band on the
//! athlete's own scale.
//!
//! The roster's status says who is listed as sharing; it decides no read.
//! Each read of a sharer asks the one answer every coach read of a coached
//! athlete asks ([`read_subject::coached_read`], training class, carnet#1425)
//! and is recorded on the athlete's own record before it is made
//! (carnet#1445).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::NaiveDate;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use uuid::Uuid;

use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::MemberFitnessSnapshot;
use pierre_core::models::{FormBand, TenantId};
use pierre_database::repositories::CoachReadClass;
use pierre_database::RepositoryRegistry;
use pierre_services::coach_roster::{
    OrderedMember, RosterAthleteStatus, ROSTER_LOOKUP_CONCURRENCY,
};
use pierre_services::coached_athlete::{CoachedAthleteAccess, CoachedAthleteRefusal};

use crate::group_fitness::fetch_member_snapshots;
use crate::read_subject::{self, CoachedReadRefusal, DataClass};
use crate::runtime::ToolRuntime;

/// Snapshots fetched at once. Each is a provider read, so a page of a large
/// roster must not open one per athlete in the same instant.
const SNAPSHOT_CONCURRENCY: usize = 8;

/// A page's sharers, each asked whether the coach may read their training now.
#[derive(Debug, Default)]
pub struct GatedPage {
    /// The sharers the coach may read, in page order, each with their tenant
    /// set to their one home tenant — where their data lives — and what the
    /// gate answered. Each read is already on their record.
    pub readable: Vec<(OrderedMember, CoachedAthleteAccess)>,
    /// The sharers the gate refused, with why; nothing of theirs is read.
    pub(crate) refused: Vec<(OrderedMember, CoachedReadRefusal)>,
}

impl GatedPage {
    /// The readable sharers alone, for [`fetch_page_snapshots`].
    #[must_use]
    pub fn readable_members(&self) -> Vec<OrderedMember> {
        self.readable
            .iter()
            .map(|(member, _)| member.clone())
            .collect()
    }

    /// The athletes the gate refused, for a caller that only leaves their
    /// training out.
    #[must_use]
    pub fn refused_athletes(&self) -> Vec<Uuid> {
        self.refused
            .iter()
            .map(|(member, _)| member.member.user_id)
            .collect()
    }
}

/// Ask each sharer of `page` whether `coach` may read their training now, and
/// record each read the answer lets through.
///
/// The answer is [`read_subject::coached_read`], training class, asked a
/// bounded number at a time. Each read it lets through goes on the athlete's
/// own record under `source` — the tool, or the REST roster's label — before
/// anything of theirs is read (carnet#1445).
///
/// Only `shared` members are asked: the others are listed by status alone and
/// nothing of theirs is read. A sharer always shares through some group; one
/// that does not reads as no member, as a membership that changed would.
///
/// # Errors
///
/// Returns the repository error from the gates or from a record's insert: a
/// read that cannot be recorded is not made.
pub async fn gate_page_reads(
    repos: &RepositoryRegistry,
    coach: Uuid,
    page: Vec<OrderedMember>,
    source: &str,
) -> AppResult<GatedPage> {
    let gated: Vec<(
        OrderedMember,
        Result<CoachedAthleteAccess, CoachedReadRefusal>,
    )> = stream::iter(
        page.into_iter()
            .filter(|m| m.member.status == RosterAthleteStatus::Shared),
    )
    .map(|member| async move {
        let gate = match member.member.shared_via {
            Some(group) => {
                read_subject::coached_read(
                    repos,
                    coach,
                    member.member.user_id,
                    group,
                    DataClass::Training,
                )
                .await?
            }
            None => Err(CoachedReadRefusal::Coaching(
                CoachedAthleteRefusal::NotAMember,
            )),
        };
        AppResult::Ok((member, gate))
    })
    .buffered(ROSTER_LOOKUP_CONCURRENCY)
    .try_collect()
    .await?;
    let mut page = GatedPage::default();
    for (mut member, gate) in gated {
        match gate {
            Ok(access) => {
                read_subject::record_coach_read(
                    repos,
                    &access,
                    coach,
                    source,
                    CoachReadClass::Training,
                )
                .await?;
                member.member.tenant = access.tenant;
                page.readable.push((member, access));
            }
            Err(refused) => page.refused.push((member, refused)),
        }
    }
    Ok(page)
}

/// Fetch the snapshot of every readable sharer in `page`, a bounded number at
/// a time, keyed by athlete. Nothing is fetched for anyone else: the caller
/// passes only the sharers [`gate_page_reads`] let through.
pub async fn fetch_page_snapshots(
    runtime: &Arc<dyn ToolRuntime>,
    page: &[OrderedMember],
) -> HashMap<Uuid, MemberFitnessSnapshot> {
    let wanted: Vec<(Uuid, TenantId)> = page
        .iter()
        .filter(|m| m.member.status == RosterAthleteStatus::Shared)
        .map(|m| (m.member.user_id, m.member.tenant))
        .collect();
    stream::iter(wanted)
        .map(|(user_id, tenant)| {
            let runtime = Arc::clone(runtime);
            async move {
                fetch_member_snapshots(&runtime, &[user_id], tenant)
                    .await
                    .into_iter()
                    .next()
                    .map(|snapshot| (user_id, snapshot))
            }
        })
        .buffer_unordered(SNAPSHOT_CONCURRENCY)
        .filter_map(|found| async move { found })
        .collect()
        .await
}

/// Form, as the coach may see it: the band on the athlete's own scale and
/// when it was read. Never a raw TSB, never a number to compare across
/// athletes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RosterForm {
    /// The band form falls in, as a share of the athlete's own fitness.
    pub band: FormBand,
    /// The athlete's civil day the band was read for.
    pub as_of: NaiveDate,
}

/// The athlete's most recent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RosterLastSession {
    /// The day they trained, on their own clock.
    pub date: NaiveDate,
    /// Sport, as the provider classified it.
    pub sport: String,
    /// Moving time in seconds.
    pub duration_seconds: u64,
}

/// A shared athlete's training, typed: what both consumers read off one
/// snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct RosterReading {
    /// Form on the athlete's own scale; absent when it could not be computed.
    pub form: Option<RosterForm>,
    /// Chronic training load — fitness, rounded; absent when not computed.
    pub ctl: Option<f64>,
    /// Days since their last session; absent with none in the lookback.
    pub days_since_last_activity: Option<i32>,
    /// Their latest session.
    pub last_session: Option<RosterLastSession>,
    /// The day each connected source last delivered a session.
    pub last_activity_per_source: BTreeMap<String, NaiveDate>,
    /// Sources whose connection died and must be reconnected by the athlete.
    pub needs_reconnect: Vec<String>,
    /// True when the data came from a stale cache that could not be
    /// refreshed: quiet recent days are then not evidence they stopped.
    pub stale: bool,
}

/// Read one snapshot into the typed roster reading.
#[must_use]
pub fn read_snapshot(snapshot: &MemberFitnessSnapshot) -> RosterReading {
    let zone = resolve_zone(snapshot.timezone.as_deref());
    RosterReading {
        form: snapshot.form_reading().map(|reading| RosterForm {
            band: reading.band,
            as_of: local_date(snapshot.computed_at, zone),
        }),
        ctl: snapshot.ctl.map(f64::round),
        days_since_last_activity: snapshot.days_since_last_activity,
        last_session: snapshot
            .recent_activities
            .first()
            .map(|a| RosterLastSession {
                date: local_date(a.start, zone),
                sport: a.sport.clone(),
                duration_seconds: a.duration_seconds,
            }),
        last_activity_per_source: snapshot
            .last_activity_per_provider
            .iter()
            .map(|(source, at)| (source.clone(), local_date(*at, zone)))
            .collect(),
        needs_reconnect: snapshot.needs_reauth_providers.clone(),
        stale: snapshot.served_stale,
    }
}
