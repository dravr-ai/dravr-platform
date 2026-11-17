// ABOUTME: name_place tool — the athlete names, renames or clears the name of a place they train from
// ABOUTME: Addressed by a row's place id or by an activity id; answers {place: {id, name}}, never a location

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `name_place` (carnet#1484, slice 6)
//!
//! A place is where an athlete keeps starting, and `get_activities` serves
//! each row's place as `{id, name}`. This tool names one as the athlete asks
//! ("call where I rode Saturday Gai-Luron"), renames it, or clears the name
//! so the place goes back to its automatic one. By activity, an activity with
//! a recorded route and no place yet becomes a place of its own: the athlete
//! asserted the spot (`pierre_services::athlete_places::name_place`).
//!
//! The answer is the place as a row serves it (`served_place_name`): over MCP
//! and A2A only a name the athlete gave is shown, so a clear that leaves an
//! automatic name answers `name: null` there. Nothing here ever returns a
//! centroid, a radius or a count.
//!
//! The tool acts for the caller only: it is in no data class, so a coach's
//! thread about an athlete withholds it.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::place_strings::{
    KEY_NAME_PLACE_ACTIVITY_HAS_NO_ROUTE, KEY_NAME_PLACE_ACTIVITY_NOT_FOUND,
    KEY_NAME_PLACE_INVALID_NAME, KEY_NAME_PLACE_PLACE_NOT_FOUND,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{PlaceRef, TenantId};
use pierre_services::athlete_places::{
    name_place, PlaceRefusal, PlaceTarget, MAX_PLACE_NAME_CHARS,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::activity_detail_store::served_place_name;
use crate::annotations::{annotations, Effect, Reach};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// Tool name.
const TOOL: &str = "name_place";

/// What `name_place` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NamePlaceResult {
    /// The place named, as a row's `place` serves it.
    pub place: NamedPlaceView,
}

/// A place as `name_place` answers it: its id and its name.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NamedPlaceView {
    /// The place id, as rows' `place.id` serves it.
    pub id: String,
    /// The place's name as this reader may see it: the athlete's own name
    /// everywhere, an automatic one only on Dravr's own surfaces; `null`
    /// otherwise.
    pub name: Option<String>,
}

/// `name_place` — name, rename or clear the name of one of the athlete's
/// places.
#[derive(Debug)]
pub struct NamePlaceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for NamePlaceTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "place_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("A row's place.id.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Or an activity id.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(format!("1-{MAX_PLACE_NAME_CHARS} characters.")),
                ..Default::default()
            },
        );
        properties.insert(
            "clear".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("true removes their name.".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        answers_with::<NamePlaceResult>(tool_definition(
            "name_place",
            "Name where the athlete trains, as they asked: by a row's place.id or an \
             activity id (a first visit too). clear=true restores the automatic name.",
            schema,
            annotations("Name place", Effect::IdempotentWrite, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let (target, name) = parse_args(&args)?;
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            match name_place(
                context.resources.repos(),
                &tenant_id,
                context.user_id,
                target,
                name,
            )
            .await?
            {
                Ok(named) => {
                    let place = PlaceRef {
                        id: named.id,
                        name: named.name,
                        athlete_named: named.athlete_named,
                    };
                    ok_typed(
                        TOOL,
                        NamePlaceResult {
                            place: NamedPlaceView {
                                name: served_place_name(&place),
                                id: place.id,
                            },
                        },
                    )
                }
                Err(refused) => Ok(refusal(&context, refused).await),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The place the call names, and the name it gives (`None` to clear).
fn parse_args(args: &Value) -> AppResult<(PlaceTarget, Option<String>)> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let target = match (text("place_id"), text("activity_id")) {
        (Some(place_id), None) => PlaceTarget::Place(place_id),
        (None, Some(activity_id)) => PlaceTarget::Activity(activity_id),
        _ => {
            return Err(AppError::invalid_input(
                "give exactly one of place_id or activity_id",
            ));
        }
    };
    let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
    // The name is passed as written: the service defangs it and refuses
    // one that is empty or too long, in the athlete's language.
    let name = args.get("name").and_then(Value::as_str).map(str::to_owned);
    match (name, clear) {
        (Some(name), false) => Ok((target, Some(name))),
        (None, true) => Ok((target, None)),
        _ => Err(AppError::invalid_input(
            "give exactly one of name or clear=true",
        )),
    }
}

/// Why nothing was named, in the reader's language, with its stable code.
async fn refusal(context: &ToolExecutionContext, refused: PlaceRefusal) -> ToolResult {
    let locale = context.render_locale(context.user_id).await;
    let strings = context.resources.messaging_strings_registry();
    let longest = MAX_PLACE_NAME_CHARS.to_string();
    let message = match refused {
        PlaceRefusal::PlaceNotFound => strings.render(KEY_NAME_PLACE_PLACE_NOT_FOUND, &locale, &[]),
        PlaceRefusal::ActivityNotFound => {
            strings.render(KEY_NAME_PLACE_ACTIVITY_NOT_FOUND, &locale, &[])
        }
        PlaceRefusal::ActivityHasNoRoute => {
            strings.render(KEY_NAME_PLACE_ACTIVITY_HAS_NO_ROUTE, &locale, &[])
        }
        PlaceRefusal::InvalidName => {
            strings.render(KEY_NAME_PLACE_INVALID_NAME, &locale, &[&longest])
        }
    };
    ToolResult::error(json!({ "reason": refused.as_str(), "error": message }))
}

/// Factory for the place tools.
#[must_use]
pub fn create_place_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(NamePlaceTool)]
}

// Writes a name in Dravr's own database; nothing leaves, and a clear brings
// the automatic name back.
crate::declare_security!(NamePlaceTool => empty);
