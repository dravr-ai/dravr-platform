// ABOUTME: Group-aware MCP tool — fetch a consenting peer member's activities.
// ABOUTME: Gated by shared group membership + per-member consent + group kill-switch.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Group tools
//!
//! [`GetGroupMemberActivitiesTool`] lets the AI coach pull a *consenting* group
//! member's activities on demand (e.g. a named past race) inside a group chat.
//!
//! The single-user data tools (`get_activities`) always execute as the
//! requesting user and silently return the requester's own data when asked for
//! a peer, so this is the only path that reads a peer's data. It hard-gates on
//! three independent conditions, all required:
//! 1. the requester and the target share a group (`list_groups_for_user` is
//!    requester-scoped, so a match proves co-membership without trusting the
//!    conversation tenant),
//! 2. the target member's `peer_sharing_consent` is `true`, and
//! 3. the group's `peer_data_sharing` kill-switch is `true`.
//!
//! The peer's activities are fetched under the peer's OWN connection tenant
//! (cross-tenant lookup), matching how their 1-1 chat resolves data — the same
//! correction applied to the group snapshot fetcher in [`crate::group_fitness`].

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::GroupMember;
use pierre_core::models::Activity;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;
use pierre_tools_core::ToolResult;
use uuid::Uuid;

use crate::activity_dedup::{ActivityDeduplicator, TimeWindowDeduplicator};
use crate::activity_fetch::fetch_provider_activities;
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::group_fitness::fetch_user_display_name;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;

/// Number of peer activities returned when `limit` is omitted.
const DEFAULT_PEER_ACTIVITY_LIMIT: usize = 30;
/// Hard cap on returned peer activities (token-budget guard).
const MAX_PEER_ACTIVITY_LIMIT: usize = 100;

fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Factory for group-scoped tools, registered under the `tools-groups` feature.
#[must_use]
pub fn create_group_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(GetGroupMemberActivitiesTool)]
}

/// A group member that matched the caller's `member` query, carried with the
/// group context needed to authorize and explain the result.
struct MemberMatch {
    group_name: String,
    group_allows_sharing: bool,
    display_name: String,
    member: GroupMember,
}

/// Scan every group the requester belongs to for members whose display name
/// contains `query_lower`. Returns the peer matches plus whether the
/// requester themself matched — the signal for the self-query redirect.
///
/// `list_groups_for_user` is requester-scoped and cross-tenant, so any match
/// proves shared membership without trusting a conversation tenant.
async fn scan_shared_rosters(
    data: &DataContext,
    requester: Uuid,
    query_lower: &str,
) -> AppResult<(Vec<MemberMatch>, bool)> {
    let groups = data.repos().groups.list_groups_for_user(requester).await?;
    let mut matches: Vec<MemberMatch> = Vec::new();
    let mut requester_matched = false;
    for group in &groups {
        let members = data
            .repos()
            .groups
            .list_members(&group.id.to_string())
            .await?;
        for member in members {
            if member.left_at.is_some() {
                continue;
            }
            let display_name = fetch_user_display_name(data, member.user_id).await;
            if !display_name.to_lowercase().contains(query_lower) {
                continue;
            }
            if member.user_id == requester {
                requester_matched = true;
                continue;
            }
            matches.push(MemberMatch {
                group_name: group.name.clone(),
                group_allows_sharing: group.peer_data_sharing,
                display_name,
                member,
            });
        }
    }
    Ok((matches, requester_matched))
}

/// Collapse the match set to a single peer, or an error payload explaining why
/// resolution failed (none found / ambiguous across distinct users).
fn resolve_unique_peer<'a>(
    matches: &'a [MemberMatch],
    query: &str,
) -> Result<&'a MemberMatch, Value> {
    let Some(first) = matches.first() else {
        return Err(json!({
            "error": format!(
                "No group member matching '{query}' was found in your group(s). \
                 They must be a member of a group you share."
            )
        }));
    };

    // Ambiguous only when DISTINCT users match — the same peer appearing in
    // several shared groups is not ambiguous.
    let first_id = first.member.user_id;
    if matches.iter().any(|m| m.member.user_id != first_id) {
        let mut names: Vec<String> = matches.iter().map(|m| m.display_name.clone()).collect();
        names.sort();
        names.dedup();
        return Err(json!({
            "error": format!(
                "'{query}' is ambiguous — multiple members match: {}. Use a more specific name.",
                names.join(", ")
            )
        }));
    }

    // Same peer (possibly across multiple groups): prefer a group where both the
    // kill-switch and the member's consent are on, so the authorization gates
    // below report the most permissive shared context.
    let best = matches
        .iter()
        .find(|m| m.group_allows_sharing && m.member.peer_sharing_consent)
        .unwrap_or(first);
    Ok(best)
}

/// Project an activity to the compact shape the coach reasons over. Mirrors the
/// fields a coach asks about (distance, duration, climbing, power, HR) without
/// the full activity payload's timeseries/laps token cost.
fn project_activity(a: &Activity) -> Value {
    json!({
        "id": a.id(),
        "name": a.name(),
        "sport": format!("{:?}", a.sport_type()),
        "start_date": a.start_date().to_rfc3339(),
        "distance_km": a.distance_meters().map(|m| m / 1000.0),
        "duration_minutes": a.duration_seconds() / 60,
        "elevation_gain_m": a.elevation_gain(),
        "average_heart_rate": a.average_heart_rate(),
        "max_heart_rate": a.max_heart_rate(),
        "average_power": a.average_power(),
        "max_power": a.max_power(),
        "calories": a.calories(),
        "provider": a.provider(),
    })
}

/// Fetch a consenting group member's activities (consent-gated peer fetch).
pub struct GetGroupMemberActivitiesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetGroupMemberActivitiesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "member".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Display name of the group member whose activities to fetch, as shown in the \
                     group roster (e.g. 'Raph'). Required."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sport_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Optional sport filter (e.g. 'ride', 'run').".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "after".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Optional Unix epoch-second lower bound. Pair with `before` to target a \
                     specific past race or date range."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "before".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Optional Unix epoch-second upper bound.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Max activities to return (1-100, default 30).".to_owned()),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["member".to_owned()]),
        };

        tool_definition(
            "get_group_member_activities",
            "Fetch a CONSENTING group member's recent or past activities. This is the ONLY way to \
             read a peer's data in a group chat — `get_activities` always returns YOUR own data, \
             never a peer's. Identify the member by their roster display name. For a specific past \
             race or date range, pass `after`/`before` epoch-second bounds. Returns an error \
             (not data) if the member has not shared their data via `/group consent yes`.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let Some(member_query) = args
                .get("member")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Ok(ToolResult::error(json!({
                    "error": "Missing required 'member' argument (the group member's roster display name)."
                })));
            };

            let requester = context.user_id;
            let runtime = &context.resources;
            let data = runtime.data();

            // Resolve the target peer across the groups the requester belongs to.
            let query_lower = member_query.to_lowercase();
            let (matches, requester_matched) =
                scan_shared_rosters(&data, requester, &query_lower).await?;

            // A self-lookup used to fall through to the generic "no member
            // matching" error — a dead end with no gradient out, since no
            // retry of your own name can ever succeed. Name the actual
            // mistake and the tool that serves it instead.
            if matches.is_empty() && requester_matched {
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "'{member_query}' is you, the requester. This tool reads a \
                         consenting PEER's activities; your own activities come from \
                         `get_activities`."
                    )
                })));
            }

            let resolved = match resolve_unique_peer(&matches, member_query) {
                Ok(r) => r,
                Err(payload) => return Ok(ToolResult::error(payload)),
            };

            // Authorization gates — both must hold.
            if !resolved.group_allows_sharing {
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "Peer data sharing is disabled for group '{}'.",
                        resolved.group_name
                    )
                })));
            }
            if !resolved.member.peer_sharing_consent {
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "{} hasn't shared their data with the group yet. They can opt in with `/group consent yes`.",
                        resolved.display_name
                    )
                })));
            }

            let peer_id = resolved.member.user_id;
            info!(
                requester = %requester,
                peer = %peer_id,
                "get_group_member_activities: authorized consent-gated peer fetch"
            );

            // Query params.
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(DEFAULT_PEER_ACTIVITY_LIMIT)
                .clamp(1, MAX_PEER_ACTIVITY_LIMIT);
            let before = args.get("before").and_then(Value::as_i64);
            let after = args.get("after").and_then(Value::as_i64);
            let sport_filter = args
                .get("sport_type")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            let params = ActivityQueryParams {
                limit: Some(limit),
                offset: None,
                before,
                after,
            };

            // Fetch under the peer's OWN connection tenant(s) — cross-tenant lookup,
            // so a peer whose Strava lives in a non-host tenant still resolves.
            // Errors propagate: a repo failure answered with an empty `ok` would
            // read downstream as "the peer didn't train".
            let connections = data
                .repos()
                .provider_connections
                .get_for_user(peer_id, None)
                .await?;
            if connections.is_empty() {
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "{} has no connected training data source, so there are no \
                         activities to fetch for them.",
                        resolved.display_name
                    )
                })));
            }
            // `fetch_provider_activities` returns `None` when the live fetch
            // fails and the stale cache is empty. Every connection failing is an
            // outage, and it must be reported as one: the old unconditional
            // `ok(count: 0)` taught the coach the peer had not trained.
            // A successful fetch that genuinely finds nothing in the window
            // still answers `ok` with a zero count — that emptiness is real.
            let mut activities = Vec::new();
            let mut any_fetch_succeeded = false;
            for conn in &connections {
                if let Some(fetched) =
                    fetch_provider_activities(runtime, &conn.provider, peer_id, &conn.tenant_id, &params)
                        .await
                {
                    any_fetch_succeeded = true;
                    activities.extend(fetched);
                }
            }
            if !any_fetch_succeeded {
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "Couldn't fetch {}'s activities right now — their data source \
                         did not respond. This is a temporary failure, not evidence \
                         they haven't trained.",
                        resolved.display_name
                    )
                })));
            }

            let mut activities = TimeWindowDeduplicator::from_env().deduplicate(activities);
            if let Some(sport) = &sport_filter {
                activities.retain(|a| format!("{:?}", a.sport_type()).to_lowercase().contains(sport));
            }
            activities.sort_by_key(|a| Reverse(a.start_date()));
            activities.truncate(limit);

            let rendered: Vec<Value> = activities.iter().map(project_activity).collect();
            Ok(ToolResult::ok(json!({
                "member": resolved.display_name,
                "group": resolved.group_name,
                "count": rendered.len(),
                "activities": rendered,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetGroupMemberActivitiesTool => UNTRUSTED_OUTPUT);
