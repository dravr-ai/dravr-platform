// ABOUTME: A fellow member's reads of an athlete under peer consent: one peer_data_reads row each, a peer day in the athlete's feed
// ABOUTME: Group views once a day, names gone with the account, the athlete's deletion takes the rows, tenants never leak, retention purges

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1543 Q3. A group member who shares their training with their
//! peers is read by a fellow member through `get_group_member_activities`
//! and through the group views built from every member's training. Each read
//! goes on the member's `peer_data_reads` record before it is made (a group
//! view at most once a UTC day), and their feed (`own_access_feed`,
//! `GET /api/me/access-feed`) lists them as peer days. A peer's in-group
//! sharing never reaches an outside assistant (carnet#1511), so a peer's MCP
//! call is refused and recorded nowhere; a coach's groups-tool read over MCP
//! names the one athlete it read as its usage row's subject.

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::groups::UpdateGroupRequest;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{CoachReadSurface, NewPeerDataRead};
use pierre_services::coach_read_retention::purge_expired_peer_reads;
use pierre_test_support::account::delete_account;
use pierre_test_support::delegation::add_group_member;
use pierre_tool_runtime::coach_group_view::{gate_group_view, record_room_post};
use pierre_tool_runtime::context::SUPPORT_SESSION;
use pierre_tool_runtime::protocols::UniversalResponse;
use serde_json::json;
use sqlx::Row;

use crate::access_feed_test::{feed_of, feed_over_rest, oauth_credential, outside_call};
use crate::coach_athlete_reads_test::{fixture, person, Fixture, Person};
use crate::helpers::impersonation::support_grant;

/// The athlete's roster name, as a member names them.
const MEMBER: &str = "Mira";

/// The fellow member's display name.
const PEER_NAME: &str = "Pia Peer";

/// The fixture's group name.
const GROUP_NAME: &str = "Reads Squad";

/// The fixture, plus a fellow member of the athlete's group, with the
/// group's peer switch on and the athlete sharing with their peers.
async fn peer_fixture() -> (Fixture, Person) {
    let f = fixture().await;
    let peer = person(&f.res, "peer", PEER_NAME).await;
    let repos = f.repos();
    add_group_member(repos, f.group, peer.id, peer.tenant)
        .await
        .unwrap();
    repos
        .groups
        .update_group(
            &f.group.to_string(),
            f.coach.tenant,
            &UpdateGroupRequest {
                name: None,
                description: None,
                agent_id: None,
                peer_data_sharing: Some(true),
                respond_mode: None,
                digest_mode: None,
                is_active: None,
            },
        )
        .await
        .unwrap()
        .expect("the fixture's group");
    assert!(
        repos
            .groups
            .update_peer_sharing_consent(&f.group.to_string(), f.athlete.id, true)
            .await
            .unwrap(),
        "test setup bug: the athlete shares with their peers"
    );
    (f, peer)
}

/// Why a groups-tool read that passed its consent gates answered with an
/// error: the fixture's athletes upload their sessions and hold no provider
/// connection, which the tool reads after the read is on the record.
const NO_SOURCE: &str = "no_source";

/// The `reason` of a call that answered with an error.
fn error_reason(response: &UniversalResponse) -> String {
    assert!(
        !response.success,
        "expected an error: {:?}",
        response.result
    );
    response.result.as_ref().map_or_else(String::new, |result| {
        result["reason"].as_str().unwrap_or_default().to_owned()
    })
}

/// The peer's `get_group_member_activities` of the athlete in their own chat:
/// authorized, recorded, then answered `no_source`.
async fn peer_reads_in_chat(f: &Fixture, peer: Person) {
    let response = f
        .call(
            &f.own_executor(),
            peer,
            "get_group_member_activities",
            json!({ "member": MEMBER }),
        )
        .await;
    assert_eq!(
        error_reason(&response),
        NO_SOURCE,
        "test setup bug: the peer's read passes its gates"
    );
}

/// `(user_id, peer_user_id, tool_name, tenant_id)` of every peer read stored,
/// whoever's.
async fn peer_rows(f: &Fixture) -> Vec<(String, Option<String>, String, String)> {
    const SELECT: &str = "SELECT CAST(user_id AS TEXT), CAST(peer_user_id AS TEXT), tool_name, \
         tenant_id FROM peer_data_reads WHERE CAST(user_id AS TEXT) = $1 \
         OR CAST(peer_user_id AS TEXT) = $1";
    let athlete = f.athlete.id.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(&athlete)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
                .collect()
        };
    }
    match f.res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    }
}

/// `(subject_user_id, outcome)` of every direct-tool-call usage row `caller`
/// owns.
async fn usage_rows(f: &Fixture, caller: Person) -> Vec<(Option<String>, Option<String>)> {
    const SELECT: &str = "SELECT subject_user_id, outcome FROM llm_usage \
         WHERE call_type = 'mcp_tool' AND user_id = $1";
    let caller = caller.id.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(&caller)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect()
        };
    }
    match f.res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    }
}

/// `viewer`'s `/stats` view of the group, as the route gates it.
async fn stats_view(f: &Fixture, viewer: Person) {
    let repos = f.repos();
    let group = repos
        .groups
        .get_group(&f.group.to_string(), f.coach.tenant)
        .await
        .unwrap()
        .expect("the fixture's group");
    let members = repos
        .groups
        .list_members(&f.group.to_string())
        .await
        .unwrap();
    gate_group_view(
        repos,
        &group,
        &members,
        viewer.id,
        viewer.tenant,
        CoachReadSurface::GroupStats,
    )
    .await
    .unwrap();
}

// ── A peer's tool read ───────────────────────────────────────────────────

#[tokio::test]
async fn a_peers_read_is_one_peer_day_with_their_name_and_no_app_read() {
    let (f, peer) = peer_fixture().await;
    peer_reads_in_chat(&f, peer).await;

    assert_eq!(
        peer_rows(&f).await,
        vec![(
            f.athlete.id.to_string(),
            Some(peer.id.to_string()),
            "get_group_member_activities".to_owned(),
            f.athlete.tenant.to_string(),
        )],
        "bug: the peer's read is one row on the athlete's record, under the athlete's tenant"
    );

    let feed = feed_of(&f, f.athlete).await;
    assert!(
        feed.outside_reads.is_empty(),
        "bug: a peer's read is no app read: {feed:?}"
    );
    assert!(
        feed.coach_days.is_empty(),
        "bug: a peer's read is no coach read: {feed:?}"
    );
    assert_eq!(feed.peer_days.len(), 1, "bug: one peer day: {feed:?}");
    let day = &feed.peer_days[0];
    assert_eq!(day.peer, Some(peer.id), "bug: the day names another member");
    assert_eq!(day.group, f.group, "bug: the day names another group");
    assert_eq!((day.reads, day.views), (1, 0), "bug: one read: {day:?}");
    assert!(
        !day.served_to_assistant,
        "bug: a read in Dravr's own chat went to no outside assistant: {day:?}"
    );
    assert!(!day.support_read, "{day:?}");

    let body = feed_over_rest(&f, f.athlete).await;
    let days = body["peer_days"].as_array().expect("a peer_days array");
    assert_eq!(days.len(), 1, "bug: the route lists the day once: {body}");
    assert_eq!(days[0]["member_name"], PEER_NAME, "{body}");
    assert_eq!(days[0]["group_id"], f.group.to_string(), "{body}");
    assert_eq!(days[0]["group_name"], GROUP_NAME, "{body}");
    assert_eq!(days[0]["reads"], 1, "{body}");
    assert_eq!(days[0]["views"], 0, "{body}");
    assert_eq!(
        body["app_reads"].as_array().map(Vec::len),
        Some(0),
        "bug: and no app read: {body}"
    );

    let peers_own = feed_of(&f, peer).await;
    assert!(
        peers_own.peer_days.is_empty(),
        "bug: reading someone is nothing read of the reader: {peers_own:?}"
    );
}

#[tokio::test]
async fn a_member_not_sharing_with_peers_is_refused_and_recorded_nowhere() {
    let (f, peer) = peer_fixture().await;
    f.repos()
        .groups
        .update_peer_sharing_consent(&f.group.to_string(), f.athlete.id, false)
        .await
        .unwrap();
    let response = f
        .call(
            &f.own_executor(),
            peer,
            "get_group_member_activities",
            json!({ "member": MEMBER }),
        )
        .await;
    assert!(
        !response.success,
        "test setup bug: a member who withdrew peer sharing is refused"
    );
    assert!(
        peer_rows(&f).await.is_empty(),
        "bug: a refused read is on nobody's record"
    );
}

// ── Over MCP ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_peers_mcp_call_is_refused_and_writes_no_peer_read() {
    let (f, peer) = peer_fixture().await;
    let response = outside_call(
        &f,
        Some(oauth_credential()),
        Transport::McpHttp,
        peer,
        "get_group_member_activities",
        json!({ "member": MEMBER }),
    )
    .await;
    assert!(
        !response.success,
        "bug: a peer's in-group sharing never reaches an outside assistant: {:?}",
        response.result
    );
    assert!(
        peer_rows(&f).await.is_empty(),
        "bug: a refused read is on nobody's record"
    );
    let feed = feed_of(&f, f.athlete).await;
    assert!(
        feed.peer_days.is_empty(),
        "bug: a refused read is no peer day: {feed:?}"
    );
    // carnet#1543 Q7: answered before its body ran, the call still counts.
    assert_eq!(
        usage_rows(&f, peer).await,
        vec![(Some(peer.id.to_string()), Some("refused".to_owned()))],
        "bug: a refused outside call writes its one row, marked refused, naming the caller"
    );
    let own = feed_of(&f, peer).await;
    assert!(
        own.outside_reads.is_empty(),
        "bug: a refused call served nothing, so no feed lists it: {own:?}"
    );
}

#[tokio::test]
async fn a_coachs_groups_tool_read_over_mcp_names_the_athlete_as_its_subject() {
    let (f, _peer) = peer_fixture().await;
    f.repos()
        .groups
        .set_coach_assistant_read(&f.group.to_string(), f.athlete.id, true, Timestamp::now())
        .await
        .unwrap()
        .expect("a live member");
    let response = outside_call(
        &f,
        Some(oauth_credential()),
        Transport::McpHttp,
        f.coach,
        "get_group_member_activities",
        json!({ "member": MEMBER }),
    )
    .await;
    assert_eq!(
        error_reason(&response),
        NO_SOURCE,
        "test setup bug: the coach's read passes its gates"
    );
    assert_eq!(
        usage_rows(&f, f.coach).await,
        vec![(Some(f.athlete.id.to_string()), Some("error".to_owned()))],
        "bug: the one member the call read is its usage row's subject"
    );
    assert!(
        peer_rows(&f).await.is_empty(),
        "bug: a coach's read is a coach read, never a peer's"
    );
    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(feed.coach_days.len(), 1, "{feed:?}");
    assert!(
        feed.outside_reads.is_empty(),
        "bug: the coach's call is no app read of the athlete's own: {feed:?}"
    );
}

// ── Group views ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_peers_group_view_is_one_view_a_day_and_the_coachs_is_no_peer_read() {
    let (f, peer) = peer_fixture().await;
    stats_view(&f, peer).await;
    stats_view(&f, peer).await;
    stats_view(&f, f.coach).await;

    assert_eq!(
        peer_rows(&f).await,
        vec![(
            f.athlete.id.to_string(),
            Some(peer.id.to_string()),
            CoachReadSurface::GroupStats.as_str().to_owned(),
            f.athlete.tenant.to_string(),
        )],
        "bug: two views of /stats by the peer in one day are one row; the coach's is none"
    );
    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(feed.peer_days.len(), 1, "{feed:?}");
    assert_eq!(
        (feed.peer_days[0].reads, feed.peer_days[0].views),
        (0, 1),
        "bug: a group view counts as a view: {feed:?}"
    );
}

#[tokio::test]
async fn a_support_sessions_group_view_is_recorded_each_time_as_supports() {
    let (f, peer) = peer_fixture().await;
    let support = support_grant(&f.res, peer.id).await;
    for _ in 0..2 {
        SUPPORT_SESSION
            .scope(Some(support.clone()), stats_view(&f, peer))
            .await;
    }

    assert_eq!(
        peer_rows(&f).await.len(),
        2,
        "bug: support's views are recorded each time, never folded into the peer's day"
    );
    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(feed.peer_days.len(), 1, "{feed:?}");
    let day = &feed.peer_days[0];
    assert_eq!((day.reads, day.views), (2, 0), "{day:?}");
    assert!(
        day.support_read,
        "bug: the athlete sees that support read them: {day:?}"
    );
}

#[tokio::test]
async fn the_digest_posted_into_the_chat_is_on_each_sharing_members_record_per_reader() {
    let (f, peer) = peer_fixture().await;
    let repos = f.repos();
    let group = repos
        .groups
        .get_group(&f.group.to_string(), f.coach.tenant)
        .await
        .unwrap()
        .expect("the fixture's group");
    let members = repos
        .groups
        .list_members(&f.group.to_string())
        .await
        .unwrap();
    for _ in 0..2 {
        record_room_post(repos, &group, &members, CoachReadSurface::WeeklyDigest)
            .await
            .unwrap();
    }

    let readers: Vec<Option<String>> = peer_rows(&f)
        .await
        .into_iter()
        .map(|(member, reader, tool, _)| {
            assert_eq!(
                member,
                f.athlete.id.to_string(),
                "bug: only the sharing athlete's week is carried"
            );
            assert_eq!(
                tool,
                CoachReadSurface::WeeklyDigest.as_str(),
                "bug: the chat copy's reads are recorded as the weekly digest"
            );
            reader
        })
        .collect();
    assert_eq!(
        readers,
        vec![Some(peer.id.to_string())],
        "bug: each other member in the chat (the peer; the coach is no member here) reads \
         the athlete's week, once a day"
    );
}

// ── Deletion ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_deleted_peers_reads_stay_on_the_record_with_no_name() {
    let (f, peer) = peer_fixture().await;
    peer_reads_in_chat(&f, peer).await;

    delete_account(f.repos(), peer.id, None).await.unwrap();

    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(
        feed.peer_days.len(),
        1,
        "bug: the athlete keeps the record: {feed:?}"
    );
    assert_eq!(
        feed.peer_days[0].peer, None,
        "bug: a deleted account is named by no one: {feed:?}"
    );
    let body = feed_over_rest(&f, f.athlete).await;
    assert!(
        body["peer_days"][0]["member_name"].is_null(),
        "bug: the route names no one: {body}"
    );
}

#[tokio::test]
async fn deleting_the_athlete_removes_their_record_of_peer_reads() {
    let (f, peer) = peer_fixture().await;
    peer_reads_in_chat(&f, peer).await;
    stats_view(&f, peer).await;
    assert_eq!(peer_rows(&f).await.len(), 2, "test setup bug: two rows");

    delete_account(f.repos(), f.athlete.id, None).await.unwrap();

    assert!(
        peer_rows(&f).await.is_empty(),
        "bug: the athlete's record goes with their account"
    );
}

// ── Tenants and retention ────────────────────────────────────────────────

/// A peer read of the athlete written straight to the store under `tenant`,
/// `age` ago.
async fn insert_peer_read(f: &Fixture, peer: Person, tenant: TenantId, age: Elapsed) {
    f.repos()
        .peer_data_reads
        .record_peer_read(&NewPeerDataRead {
            tenant,
            member: f.athlete.id,
            peer: peer.id,
            group: f.group,
            tool: "get_group_member_activities",
            read_at: Timestamp::now() - age,
            transport: Some(Transport::WebApp),
            impersonator: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn another_tenants_peer_reads_never_appear() {
    let (f, peer) = peer_fixture().await;
    insert_peer_read(&f, peer, f.athlete.tenant, Elapsed::days(1)).await;
    // The same athlete under a tenant they do not belong to.
    insert_peer_read(&f, peer, TenantId::generate(), Elapsed::days(1)).await;

    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(feed.peer_days.len(), 1, "{feed:?}");
    assert_eq!(
        feed.peer_days[0].tenant, f.athlete.tenant,
        "bug: the feed reads only the tenants the athlete belongs to: {feed:?}"
    );
    assert_eq!(feed.peer_days[0].reads, 1, "{feed:?}");
}

#[tokio::test]
async fn the_retention_purges_peer_reads_past_twelve_months_and_keeps_the_rest() {
    let (f, peer) = peer_fixture().await;
    insert_peer_read(&f, peer, f.athlete.tenant, Elapsed::days(400)).await;
    insert_peer_read(&f, peer, f.athlete.tenant, Elapsed::days(1)).await;

    let purged = purge_expired_peer_reads(f.repos().peer_data_reads.as_ref(), Timestamp::now())
        .await
        .unwrap();
    assert!(purged >= 1, "bug: the old read is purged");
    assert_eq!(peer_rows(&f).await.len(), 1, "bug: the recent read stays");
}

/// The membership row's tenant can be the group's rather than the athlete's
/// (an invite joins under the group's tenant): the record is kept under a
/// tenant the athlete belongs to, so their feed finds it.
#[tokio::test]
async fn a_membership_under_the_groups_tenant_records_under_the_athletes() {
    let f = fixture().await;
    let peer = person(&f.res, "peer", PEER_NAME).await;
    let repos = f.repos();
    add_group_member(repos, f.group, peer.id, peer.tenant)
        .await
        .unwrap();
    // A second member who joined under the group's (the coach's) tenant.
    let joiner = person(&f.res, "joiner", "Jo Joiner").await;
    add_group_member(repos, f.group, joiner.id, f.coach.tenant)
        .await
        .unwrap();
    repos
        .groups
        .update_group(
            &f.group.to_string(),
            f.coach.tenant,
            &UpdateGroupRequest {
                name: None,
                description: None,
                agent_id: None,
                peer_data_sharing: Some(true),
                respond_mode: None,
                digest_mode: None,
                is_active: None,
            },
        )
        .await
        .unwrap();
    repos
        .groups
        .update_peer_sharing_consent(&f.group.to_string(), joiner.id, true)
        .await
        .unwrap();

    let response = f
        .call(
            &f.own_executor(),
            peer,
            "get_group_member_activities",
            json!({ "member": "Jo" }),
        )
        .await;
    assert_eq!(
        error_reason(&response),
        NO_SOURCE,
        "test setup bug: the peer's read of the joiner passes its gates"
    );

    let feed = feed_of(&f, joiner).await;
    assert_eq!(
        feed.peer_days.len(),
        1,
        "bug: the read is on the member's record, where their feed finds it: {feed:?}"
    );
    assert_eq!(feed.peer_days[0].tenant, joiner.tenant, "{feed:?}");
}
