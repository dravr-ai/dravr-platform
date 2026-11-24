// ABOUTME: The athlete's "who read my data" feed: outside callers' reads per call or turn, their coach's, peers' and keys' per day
// ABOUTME: A view over records that already exist (llm_usage, coach_data_reads, peer_data_reads, api_key_usage), per tenant (carnet#1543)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The owner feed.
//!
//! Nothing is written for it. An outside client's tool call (an MCP client,
//! an A2A agent) already writes its usage row, which names whose data it read
//! and the client it came through, and so does a turn of Dravr's agent an
//! outside caller drove (an A2A text message, a chat message through the
//! athlete's API key); each REST request an API key makes already writes its
//! `api_key_usage` row, listed per day and key; a human coach's read already writes the
//! athlete's `coach_data_reads` row, and a fellow group member's read under
//! their peer consent their `peer_data_reads` row. The feed lists the first
//! per call and the other two per UTC day (JF's E4), each read under every
//! tenant the athlete
//! belongs to and filtered on that tenant: a read is recorded under the
//! tenant it was made in, which need not be the one the athlete's session is
//! in.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::AppResult;
use pierre_core::models::{ApiKeyRequestDay, OutsideRead, TenantId};
use pierre_database::repositories::{CoachReadClass, PeerDataRead};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::medical_flag;

/// The most outside reads the feed lists.
const MAX_OUTSIDE_READS: u32 = 200;

/// The most coach reads one tenant's record contributes to the day counts:
/// far past any month a coach reads in, so the counts are whole.
const MAX_COACH_READS: u32 = 10_000;

/// The most peer reads one tenant's record contributes to the day counts, on
/// the same reasoning.
const MAX_PEER_READS: u32 = 10_000;

/// One day of a coach's reads in one group, of one class of data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachReadDay {
    /// The UTC day.
    pub day: CivilDate,
    /// The coach; `None` once their account is deleted.
    pub coach: Option<Uuid>,
    /// The group they coach the athlete in.
    pub group: Uuid,
    /// The tenant the reads were recorded under: the group's.
    pub tenant: TenantId,
    /// Which data the reads reached.
    pub class: CoachReadClass,
    /// The reads recorded each time they were made (a tool call, the plan
    /// view, the roster, the coach's thread).
    pub reads: u32,
    /// The group views recorded once a day each (the room's member cards,
    /// `/stats`, `/report`, `/health`, the weekly digest).
    pub views: u32,
    /// Any of them was served to the coach's outside assistant.
    pub served_to_assistant: bool,
    /// Any of them was made by an admin's support session acting as the
    /// coach.
    pub support_read: bool,
}

/// One day of a fellow member's reads of the athlete in one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerReadDay {
    /// The UTC day.
    pub day: CivilDate,
    /// The member who read; `None` once their account is deleted.
    pub peer: Option<Uuid>,
    /// The group whose peer sharing let the reads through.
    pub group: Uuid,
    /// The tenant the reads were recorded under: one the athlete belongs to.
    pub tenant: TenantId,
    /// The reads recorded each time they were made (a tool call).
    pub reads: u32,
    /// The group views recorded once a day each (the room's member cards,
    /// `/stats`, `/report`, `/health`).
    pub views: u32,
    /// Any of them was served to the member's outside assistant.
    pub served_to_assistant: bool,
    /// Any of them was made by an admin's support session acting as the
    /// member.
    pub support_read: bool,
}

/// The athlete's feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessFeed {
    /// Outside clients' calls reading the athlete's own data, newest first.
    pub outside_reads: Vec<OutsideRead>,
    /// Their coach's reads per day, newest day first.
    pub coach_days: Vec<CoachReadDay>,
    /// Their fellow members' reads per day, newest day first.
    pub peer_days: Vec<PeerReadDay>,
    /// Their API keys' REST requests per day and key, newest day first.
    pub api_key_days: Vec<ApiKeyRequestDay>,
}

/// `athlete`'s feed of the reads made since `since`.
///
/// # Errors
///
/// Returns the repository error from the tenant or record reads.
pub async fn own_access_feed(
    repos: &RepositoryRegistry,
    athlete: Uuid,
    since: Timestamp,
) -> AppResult<AccessFeed> {
    let user_id = athlete.to_string();
    let mut outside_reads = Vec::new();
    let mut coach_reads = Vec::new();
    let mut peer_reads = Vec::new();
    for tenant in medical_flag::athlete_tenants(repos, None, athlete).await? {
        outside_reads.extend(
            repos
                .llm_usage
                .list_outside_reads(&tenant.to_string(), &user_id, since, MAX_OUTSIDE_READS)
                .await?,
        );
        coach_reads.extend(
            repos
                .coach_data_reads
                .list_coach_reads_since(tenant, athlete, since, MAX_COACH_READS)
                .await?
                .into_iter()
                .map(|read| (tenant, read)),
        );
        peer_reads.extend(
            repos
                .peer_data_reads
                .list_peer_reads_since(tenant, athlete, since, MAX_PEER_READS)
                .await?
                .into_iter()
                .map(|read| (tenant, read)),
        );
    }
    outside_reads.sort_by_key(|read| Reverse(read.read_at));
    outside_reads.truncate(usize::try_from(MAX_OUTSIDE_READS).unwrap_or(usize::MAX));

    let mut days: BTreeMap<(CivilDate, Option<Uuid>, Uuid, &'static str), CoachReadDay> =
        BTreeMap::new();
    for (tenant, read) in coach_reads {
        let day = read.read_at.utc_date();
        let entry = days
            .entry((day, read.coach, read.group, read.class.as_str()))
            .or_insert(CoachReadDay {
                day,
                coach: read.coach,
                group: read.group,
                tenant,
                class: read.class,
                reads: 0,
                views: 0,
                served_to_assistant: false,
                support_read: false,
            });
        if read.once_a_day {
            entry.views = entry.views.saturating_add(1);
        } else {
            entry.reads = entry.reads.saturating_add(1);
        }
        entry.served_to_assistant |= read.transport.served_to_outside_client();
        entry.support_read |= read.impersonator.is_some();
    }
    let mut coach_days: Vec<CoachReadDay> = days.into_values().collect();
    coach_days.sort_by_key(|day| Reverse(day.day));
    // A key belongs to its owner across tenants, so its days are read once.
    let api_key_days = repos.api_keys.list_request_days(athlete, since).await?;
    Ok(AccessFeed {
        outside_reads,
        coach_days,
        peer_days: peer_days(peer_reads),
        api_key_days,
    })
}

/// `reads` counted per UTC day, peer and group, newest day first.
fn peer_days(reads: Vec<(TenantId, PeerDataRead)>) -> Vec<PeerReadDay> {
    let mut days: BTreeMap<(CivilDate, Option<Uuid>, Uuid), PeerReadDay> = BTreeMap::new();
    for (tenant, read) in reads {
        let day = read.read_at.utc_date();
        let entry = days
            .entry((day, read.peer, read.group))
            .or_insert(PeerReadDay {
                day,
                peer: read.peer,
                group: read.group,
                tenant,
                reads: 0,
                views: 0,
                served_to_assistant: false,
                support_read: false,
            });
        if read.once_a_day {
            entry.views = entry.views.saturating_add(1);
        } else {
            entry.reads = entry.reads.saturating_add(1);
        }
        entry.served_to_assistant |= read.transport.served_to_outside_client();
        entry.support_read |= read.impersonator.is_some();
    }
    let mut peer_days: Vec<PeerReadDay> = days.into_values().collect();
    peer_days.sort_by_key(|day| Reverse(day.day));
    peer_days
}
