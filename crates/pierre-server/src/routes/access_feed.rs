// ABOUTME: HTTP boundary of the athlete's "who read my data" feed: GET /api/me/access-feed
// ABOUTME: Outside callers' reads per call or agent turn with the client named, the coach's, peers' and API keys' per day (carnet#1543)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /api/me/access-feed` — who read the caller's data in the last
//! [`FEED_DAYS`] days.
//!
//! Served to the athlete's own screens only, as their health records are: an
//! API key is refused with a 403 ([`served_to_outside_client`]), and REST
//! authentication already refuses an MCP token and a delegated grant.
//!
//! - `app_reads`: each tool call an outside client (an MCP client, an A2A
//!   agent) made reading the caller's own data (`kind: "tool_call"`), and
//!   each turn of Dravr's agent an outside caller drove (`kind:
//!   "agent_turn"`: an A2A text message, a chat message through their API
//!   key), with the client named. The Revoke beside an app is the
//!   Connected-apps one (`DELETE /api/me/oauth-grants/{id}`), matched on
//!   `client_id`.
//! - `api_key_days`: their API keys' successful REST requests per UTC day
//!   and key; a key's `/mcp` calls are its `app_reads` rows instead.
//! - `coach_days`: their human coach's reads per UTC day, per group and
//!   class of data (JF's E4); the per-read list stays in the group's log.
//! - `peer_days`: fellow group members' reads under the caller's peer
//!   sharing, per UTC day, per member and group.
//!
//! Dravr's own agent answering the caller in Dravr's own apps is the caller
//! reading their own data, and is not listed.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::oauth2_server::client_registration::client_display_host;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_core::permissions::impersonation::CredentialKind;
use pierre_middleware::extract_auth_from_headers;
use pierre_services::access_feed::{own_access_feed, CoachReadDay, PeerReadDay};
use pierre_services::athlete_display_name::fetch_user_display_name;
use serde::Serialize;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::own_screens::served_to_outside_client;

/// How many days back the feed reaches.
const FEED_DAYS: i64 = 30;

/// One outside client's call reading the caller's data.
#[derive(Serialize)]
struct AppReadView {
    /// When the call was made (RFC 3339).
    read_at: String,
    /// `tool_call` or `agent_turn`.
    kind: &'static str,
    /// The tool it called; `None` for an agent turn.
    tool: Option<String>,
    /// Every tool it ran: the one called, or the agent turn's.
    tools: Vec<String>,
    /// The client its credential names; `None` for an API key or a token.
    client_id: Option<String>,
    /// The name the client registered (its own choice, shown with
    /// `client_host`); `None` for a support read, whose client is the admin's.
    client_name: Option<String>,
    /// Where an OAuth client is (`client_display_host`).
    client_host: Option<String>,
    /// The kind of credential the call came through.
    credential: Option<String>,
    /// The transport: `mcp_http`, `a2a` or `api_key`.
    transport: Option<String>,
    /// An admin's support session made the call, acting as the caller.
    support_read: bool,
}

/// One day of the coach's reads in one group, of one class of data.
#[derive(Serialize)]
struct CoachDayView {
    /// The UTC day (`YYYY-MM-DD`).
    day: String,
    /// The coach's display name; `None` once their account is deleted.
    coach_name: Option<String>,
    /// The group.
    group_id: String,
    /// The group's name, when it still exists.
    group_name: Option<String>,
    /// `training` or `wellness`.
    data_class: &'static str,
    /// Reads recorded each time they were made.
    reads: u32,
    /// Group views, recorded once a day each.
    views: u32,
    /// Any was served to the coach's outside assistant.
    served_to_assistant: bool,
    /// Any was made by an admin's support session acting as the coach.
    support_read: bool,
}

/// One day of a fellow member's reads in one group.
#[derive(Serialize)]
struct PeerDayView {
    /// The UTC day (`YYYY-MM-DD`).
    day: String,
    /// The member's display name; `None` once their account is deleted.
    member_name: Option<String>,
    /// The group.
    group_id: String,
    /// The group's name, when it still exists.
    group_name: Option<String>,
    /// Reads recorded each time they were made.
    reads: u32,
    /// Group views, recorded once a day each.
    views: u32,
    /// Any was served to the member's outside assistant.
    served_to_assistant: bool,
    /// Any was made by an admin's support session acting as the member.
    support_read: bool,
}

/// One day of one API key's REST requests.
#[derive(Serialize)]
struct ApiKeyDayView {
    /// The UTC day (`YYYY-MM-DD`).
    day: String,
    /// The key.
    key_id: String,
    /// The name the caller gave it.
    key_name: String,
    /// The visible prefix it is recognised by.
    key_prefix: String,
    /// Its requests that day that answered with a result.
    requests: u32,
}

/// The feed.
#[derive(Serialize)]
struct AccessFeedView {
    app_reads: Vec<AppReadView>,
    coach_days: Vec<CoachDayView>,
    peer_days: Vec<PeerDayView>,
    api_key_days: Vec<ApiKeyDayView>,
    days: i64,
}

/// The display name and host of the client `client_id` names, by the kind of
/// credential it came through.
async fn client_display(
    resources: &ServerContext,
    credential: Option<&str>,
    client_id: &str,
) -> Result<(Option<String>, Option<String>), AppError> {
    let repos = &resources.common.repos;
    if credential == Some(CredentialKind::A2aClientCredentials.as_str()) {
        let client = repos.a2a.get_client(client_id).await?;
        return Ok((client.map(|client| client.name), None));
    }
    let client = repos.oauth2_server.get_client(client_id).await?;
    let redirect_uris = client.as_ref().map_or(&[][..], |c| &c.redirect_uris[..]);
    let host = client_display_host(client_id, redirect_uris);
    Ok((client.and_then(|client| client.client_name), host))
}

/// The display and group names a feed resolves, once per request.
#[derive(Default)]
struct Names {
    people: HashMap<Uuid, String>,
    groups: HashMap<Uuid, Option<String>>,
}

impl Names {
    /// `person`'s display name; `None` for an account that is gone.
    async fn person(&mut self, resources: &ServerContext, person: Option<Uuid>) -> Option<String> {
        let person = person?;
        Some(match self.people.entry(person) {
            Entry::Occupied(known) => known.get().clone(),
            Entry::Vacant(slot) => slot
                .insert(fetch_user_display_name(&resources.common.repos, person).await)
                .clone(),
        })
    }

    /// `group`'s name, when it still exists.
    async fn group(
        &mut self,
        resources: &ServerContext,
        group: Uuid,
        tenant: TenantId,
    ) -> Result<Option<String>, AppError> {
        Ok(match self.groups.entry(group) {
            Entry::Occupied(known) => known.get().clone(),
            Entry::Vacant(slot) => slot
                .insert(
                    resources
                        .common
                        .repos
                        .groups
                        .get_group(&group.to_string(), tenant)
                        .await?
                        .map(|group| group.name),
                )
                .clone(),
        })
    }
}

/// A coach day as the feed shows it.
async fn coach_day_view(
    resources: &ServerContext,
    day: CoachReadDay,
    names: &mut Names,
) -> Result<CoachDayView, AppError> {
    let coach_name = names.person(resources, day.coach).await;
    let group_name = names.group(resources, day.group, day.tenant).await?;
    Ok(CoachDayView {
        day: day.day.to_string(),
        coach_name,
        group_id: day.group.to_string(),
        group_name,
        data_class: day.class.as_str(),
        reads: day.reads,
        views: day.views,
        served_to_assistant: day.served_to_assistant,
        support_read: day.support_read,
    })
}

/// A peer day as the feed shows it.
async fn peer_day_view(
    resources: &ServerContext,
    day: PeerReadDay,
    names: &mut Names,
) -> Result<PeerDayView, AppError> {
    let member_name = names.person(resources, day.peer).await;
    let group_name = names.group(resources, day.group, day.tenant).await?;
    Ok(PeerDayView {
        day: day.day.to_string(),
        member_name,
        group_id: day.group.to_string(),
        group_name,
        reads: day.reads,
        views: day.views,
        served_to_assistant: day.served_to_assistant,
        support_read: day.support_read,
    })
}

/// `GET /api/me/access-feed`.
async fn handle_access_feed(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    if served_to_outside_client(&auth) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Who read your data is not served to outside apps. The athlete sees it in Dravr.",
        ));
    }
    let since = Timestamp::now() - Elapsed::days(FEED_DAYS);
    let feed = own_access_feed(&resources.common.repos, auth.user_id, since).await?;

    let mut clients: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut app_reads = Vec::with_capacity(feed.outside_reads.len());
    for read in feed.outside_reads {
        let support_read = read.impersonator_user_id.is_some();
        let (client_name, client_host) = match (&read.client_id, support_read) {
            (Some(client_id), false) => {
                if let Some(known) = clients.get(client_id) {
                    known.clone()
                } else {
                    let display =
                        client_display(&resources, read.credential.as_deref(), client_id).await?;
                    clients.insert(client_id.clone(), display.clone());
                    display
                }
            }
            _ => (None, None),
        };
        app_reads.push(AppReadView {
            read_at: read.read_at.to_rfc3339(),
            kind: read.kind.as_str(),
            tool: read.tool,
            tools: read.tools,
            client_id: read.client_id.filter(|_| !support_read),
            client_name,
            client_host,
            credential: read.credential,
            transport: read.transport,
            support_read,
        });
    }

    let mut names = Names::default();
    let mut coach_days = Vec::with_capacity(feed.coach_days.len());
    for day in feed.coach_days {
        coach_days.push(coach_day_view(&resources, day, &mut names).await?);
    }
    let mut peer_days = Vec::with_capacity(feed.peer_days.len());
    for day in feed.peer_days {
        peer_days.push(peer_day_view(&resources, day, &mut names).await?);
    }
    let api_key_days = feed
        .api_key_days
        .into_iter()
        .map(|day| ApiKeyDayView {
            day: day.day.to_string(),
            key_id: day.key_id,
            key_name: day.key_name,
            key_prefix: day.key_prefix,
            requests: day.requests,
        })
        .collect();
    Ok(Json(AccessFeedView {
        app_reads,
        coach_days,
        peer_days,
        api_key_days,
        days: FEED_DAYS,
    })
    .into_response())
}

/// The owner-feed route.
#[derive(Debug)]
pub struct AccessFeedRoutes;

impl AccessFeedRoutes {
    /// Build `GET /api/me/access-feed`.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/access-feed", get(handle_access_feed))
            .with_state(resources)
    }
}
