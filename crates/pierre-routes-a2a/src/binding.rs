// ABOUTME: A2A 1.0 transport bindings — JSON-RPC endpoint (SSE-capable) and the HTTP+JSON REST surface
// ABOUTME: Enforces A2A-Version negotiation, maps errors to google.rpc.Status, frames SSE per binding
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A 1.0 protocol bindings.
//!
//! Two functionally equivalent interfaces are exposed (both declared in the
//! agent card's `supportedInterfaces`):
//!
//! - **JSONRPC** at `POST /a2a/jsonrpc` — JSON-RPC 2.0 envelopes; the two
//!   streaming methods answer with SSE whose `data:` lines are full
//!   JSON-RPC envelopes wrapping `StreamResponse` frames.
//! - **HTTP+JSON** under `/a2a` — the spec's fixed REST paths
//!   (`/message:send`, `/tasks/{id}`, `/tasks/{id}:cancel`, …), responses
//!   in `application/a2a+json`, errors as `google.rpc.Status` JSON per the
//!   v1.0.1 mapping table, SSE `data:` lines carrying bare `StreamResponse`
//!   frames.
//!
//! Version negotiation (spec §3.6): every protocol request must carry
//! `A2A-Version: 1.0` (header, or `?A2A-Version=` query parameter). An
//! absent header means 0.3 semantics, which this server does not carry —
//! such requests receive `VersionNotSupportedError` (`-32009`).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, RawQuery, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, StatusCode,
    },
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use pierre_a2a::{
    protocol_types::{error_info, A2ASpecError, StreamResponse, Task},
    A2ARequest, A2AResponse, A2AServer, JsonRpcError, A2A_VERSION, A2A_VERSION_HEADER,
};
use pierre_core::auth_header::extract_bearer_token;
use pierre_runtime_context::{A2ACtx, MiddlewareCtx};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{once, StreamExt};

use crate::A2ARoutesState;

/// Media type of HTTP+JSON binding responses (registered in spec §14.1.1;
/// preferred over plain `application/json` since v1.0.1).
pub const A2A_MEDIA_TYPE: &str = "application/a2a+json";

/// `google.rpc.ErrorInfo` reason attached to authentication failures by the
/// protocol layer; both bindings map it to HTTP 401.
const AUTHENTICATION_REQUIRED: &str = "AUTHENTICATION_REQUIRED";

// ================================ helpers ================================

/// Extract the requested A2A version from header or query string.
fn requested_version(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(value) = headers
        .get(A2A_VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        return Some(value.trim().to_owned());
    }
    query.and_then(|query| {
        query.split('&').find_map(|pair| {
            pair.strip_prefix("A2A-Version=")
                .map(|version| version.trim().to_owned())
        })
    })
}

/// Whether the request negotiates the protocol version this server carries.
fn version_supported(headers: &HeaderMap, query: Option<&str>) -> bool {
    requested_version(headers, query).is_some_and(|version| version == A2A_VERSION)
}

/// Bearer token from the `Authorization` header, if present.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| extract_bearer_token(header).ok())
        .map(str::to_owned)
}

/// Whether a JSON-RPC error carries the given `ErrorInfo` reason.
fn has_reason(error: &JsonRpcError, reason: &str) -> bool {
    error
        .data
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|details| {
            details
                .iter()
                .any(|detail| detail.get("reason").and_then(Value::as_str) == Some(reason))
        })
}

/// Build a bare A2A request for a spec method (REST binding path).
fn wire_request(method: &str, params: Value, auth_token: Option<String>) -> A2ARequest {
    A2ARequest {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params: Some(params),
        id: None,
        auth_token,
        headers: None,
        metadata: HashMap::new(),
    }
}

/// Serialize one stream frame into an SSE event. With `envelope_id` the
/// frame is wrapped in a full JSON-RPC envelope (JSONRPC binding); without,
/// the bare `StreamResponse` object is the payload (HTTP+JSON binding).
fn frame_event(frame: &StreamResponse, envelope_id: Option<&Value>) -> Event {
    let payload = envelope_id.map_or_else(
        || serde_json::to_value(frame).unwrap_or(Value::Null),
        |id| json!({ "jsonrpc": "2.0", "id": id, "result": frame }),
    );
    Event::default().data(payload.to_string())
}

/// Build the SSE response for a task stream: the snapshot first (spec
/// §9.4.2), then live events until the executor closes the channel at the
/// terminal state.
fn sse_task_stream(
    snapshot: Task,
    receiver: broadcast::Receiver<StreamResponse>,
    envelope_id: Option<Value>,
) -> Response {
    let first = frame_event(&StreamResponse::Task(snapshot), envelope_id.as_ref());
    let live = BroadcastStream::new(receiver).filter_map(move |item| match item {
        Ok(frame) => Some(Ok::<Event, Infallible>(frame_event(
            &frame,
            envelope_id.as_ref(),
        ))),
        // A lagged subscriber skips missed frames; the terminal state is
        // still guaranteed via stream closure.
        Err(BroadcastStreamRecvError::Lagged(_)) => None,
    });
    let stream = once(Ok::<Event, Infallible>(first)).chain(live);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ============================ JSONRPC binding ============================

/// HTTP status for a JSON-RPC response envelope: authentication failures
/// surface as 401 (auth is transport-level per spec §4), everything else
/// rides HTTP 200 with the error inside the envelope.
fn jsonrpc_http_status(response: &A2AResponse) -> StatusCode {
    response.error.as_ref().map_or(StatusCode::OK, |error| {
        if has_reason(error, AUTHENTICATION_REQUIRED) {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::OK
        }
    })
}

/// JSON-RPC error envelope helper.
fn jsonrpc_error(code: i32, message: &str, data: Option<Value>, id: Option<Value>) -> A2AResponse {
    A2AResponse {
        jsonrpc: "2.0".into(),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_owned(),
            data,
        }),
        id,
    }
}

/// Handle the A2A JSON-RPC transport endpoint (`POST /a2a/jsonrpc`).
///
/// Dispatches non-streaming methods to [`A2AServer::handle_request`] and
/// answers `SendStreamingMessage` / `SubscribeToTask` with SSE. A bearer
/// token supplied via the `Authorization` header is folded into the
/// request's `auth` field when the body omits it.
pub(crate) async fn handle_jsonrpc<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    let mut request: A2ARequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            let response = jsonrpc_error(-32700, &format!("Parse error: {e}"), None, None);
            return (StatusCode::OK, Json(response)).into_response();
        }
    };

    if request.auth_token.is_none() {
        request.auth_token = bearer_token(&headers);
    }

    if !version_supported(&headers, query.as_deref()) {
        let spec_error = A2ASpecError::VersionNotSupported;
        let response = jsonrpc_error(
            spec_error.code(),
            &spec_error.to_string(),
            Some(error_info(spec_error.reason())),
            request.id.clone(),
        );
        return (StatusCode::OK, Json(response)).into_response();
    }

    let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
    let server = A2AServer::new_with_resources(ctx, state.tool_runtime.clone()); // Safe: Arc clone of trait object

    match request.method.as_str() {
        "SendStreamingMessage" => {
            let envelope_id = request.id.clone(); // Safe: JSON value ownership for SSE frames
            match server.start_streaming_message(request).await {
                Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, envelope_id),
                Err(response) => {
                    let status = jsonrpc_http_status(&response);
                    (status, Json(*response)).into_response()
                }
            }
        }
        "SubscribeToTask" => {
            let envelope_id = request.id.clone(); // Safe: JSON value ownership for SSE frames
            match server.start_task_subscription(request).await {
                Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, envelope_id),
                Err(response) => {
                    let status = jsonrpc_http_status(&response);
                    (status, Json(*response)).into_response()
                }
            }
        }
        _ => {
            let response = server.handle_request(request).await;
            let status = jsonrpc_http_status(&response);
            (status, Json(response)).into_response()
        }
    }
}

// =========================== HTTP+JSON binding ===========================

/// Map a JSON-RPC error to the HTTP status + `google.rpc.Code` name of the
/// v1.0.1 error-mapping table.
fn rest_error_mapping(error: &JsonRpcError) -> (StatusCode, &'static str) {
    match error.code {
        -32001 => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        -32002 | -32003 | -32004 | -32007 | -32008 | -32009 => {
            (StatusCode::BAD_REQUEST, "FAILED_PRECONDITION")
        }
        -32005 | -32602 | -32700 => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
        -32601 => (StatusCode::NOT_IMPLEMENTED, "UNIMPLEMENTED"),
        -32000 if has_reason(error, AUTHENTICATION_REQUIRED) => {
            (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED")
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    }
}

/// Build a `google.rpc.Status` error response (HTTP+JSON binding).
fn rest_error_response(
    status: StatusCode,
    code_name: &str,
    message: &str,
    details: &Value,
) -> Response {
    let body = json!({
        "error": {
            "code": status.as_u16(),
            "status": code_name,
            "message": message,
            "details": details,
        }
    });
    (status, [(CONTENT_TYPE, A2A_MEDIA_TYPE)], Json(body)).into_response()
}

/// Convert a protocol-layer response into the REST binding's HTTP shape.
fn rest_response(response: A2AResponse) -> Response {
    if let Some(error) = &response.error {
        let (status, code_name) = rest_error_mapping(error);
        let details = error.data.clone().unwrap_or_else(|| json!([])); // Safe: JSON value ownership for body
        return rest_error_response(status, code_name, &error.message, &details);
    }
    let result = response.result.unwrap_or(Value::Null);
    (
        StatusCode::OK,
        [(CONTENT_TYPE, A2A_MEDIA_TYPE)],
        Json(result),
    )
        .into_response()
}

/// Reject requests that negotiate an unsupported protocol version.
fn rest_version_guard(headers: &HeaderMap, query: Option<&str>) -> Result<(), Box<Response>> {
    if version_supported(headers, query) {
        return Ok(());
    }
    let spec_error = A2ASpecError::VersionNotSupported;
    Err(Box::new(rest_error_response(
        StatusCode::BAD_REQUEST,
        "FAILED_PRECONDITION",
        &spec_error.to_string(),
        &error_info(spec_error.reason()),
    )))
}

/// Reject request bodies with a non-JSON content type.
fn rest_content_type_guard(headers: &HeaderMap) -> Result<(), Box<Response>> {
    let Some(content_type) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    if content_type.contains("json") {
        return Ok(());
    }
    let spec_error = A2ASpecError::ContentTypeNotSupported;
    Err(Box::new(rest_error_response(
        StatusCode::BAD_REQUEST,
        "INVALID_ARGUMENT",
        &spec_error.to_string(),
        &error_info(spec_error.reason()),
    )))
}

/// Parse a REST JSON body, mapping failures to `INVALID_ARGUMENT`.
fn rest_parse_body(body: &Bytes) -> Result<Value, Box<Response>> {
    serde_json::from_slice(body).map_err(|e| {
        Box::new(rest_error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            &format!("Invalid JSON body: {e}"),
            &json!([]),
        ))
    })
}

/// Dispatch one REST call through the protocol layer.
async fn rest_dispatch<C: MiddlewareCtx + A2ACtx>(
    state: &A2ARoutesState<C>,
    headers: &HeaderMap,
    method: &str,
    params: Value,
) -> Response {
    let request = wire_request(method, params, bearer_token(headers));
    let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
    let server = A2AServer::new_with_resources(ctx, state.tool_runtime.clone()); // Safe: Arc clone of trait object
    rest_response(server.handle_request(request).await)
}

/// `POST /a2a/message:send`
pub(crate) async fn rest_message_send<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    if let Err(response) = rest_content_type_guard(&headers) {
        return *response;
    }
    let params = match rest_parse_body(&body) {
        Ok(params) => params,
        Err(response) => return *response,
    };
    rest_dispatch(&state, &headers, "SendMessage", params).await
}

/// `POST /a2a/message:stream` — SSE with bare `StreamResponse` frames.
pub(crate) async fn rest_message_stream<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    if let Err(response) = rest_content_type_guard(&headers) {
        return *response;
    }
    let params = match rest_parse_body(&body) {
        Ok(params) => params,
        Err(response) => return *response,
    };

    let request = wire_request("SendStreamingMessage", params, bearer_token(&headers));
    let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
    let server = A2AServer::new_with_resources(ctx, state.tool_runtime.clone()); // Safe: Arc clone of trait object
    match server.start_streaming_message(request).await {
        Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, None),
        Err(response) => rest_response(*response),
    }
}

/// `GET /a2a/tasks/{id}`.
///
/// Shares the path parameter with `POST /a2a/tasks/{id}:action`, whose
/// handler splits the trailing `:cancel` / `:subscribe` custom verb
/// (AIP-136 style, per the spec's REST mapping).
pub(crate) async fn rest_get_task<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id): Path<String>,
    Query(query_map): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    let mut params = json!({ "id": id });
    if let Some(history_length) = query_map
        .get("historyLength")
        .and_then(|value| value.parse::<u32>().ok())
    {
        params["historyLength"] = json!(history_length);
    }
    rest_dispatch(&state, &headers, "GetTask", params).await
}

/// Rebuild a raw query string from the parsed map (for the version guard).
fn query_string(query_map: &HashMap<String, String>) -> Option<String> {
    query_map
        .get(A2A_VERSION_HEADER)
        .map(|version| format!("{A2A_VERSION_HEADER}={version}"))
}

/// `GET /a2a/tasks` — `ListTasks` with `camelCase` query parameters.
pub(crate) async fn rest_list_tasks<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Query(query_map): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }

    let mut params = serde_json::Map::new();
    if let Some(page_size) = query_map
        .get("pageSize")
        .and_then(|value| value.parse::<u32>().ok())
    {
        params.insert("pageSize".into(), json!(page_size));
    }
    for key in ["pageToken", "contextId", "status", "statusTimestampAfter"] {
        if let Some(value) = query_map.get(key) {
            params.insert(key.into(), json!(value));
        }
    }
    if let Some(include_artifacts) = query_map
        .get("includeArtifacts")
        .and_then(|value| value.parse::<bool>().ok())
    {
        params.insert("includeArtifacts".into(), json!(include_artifacts));
    }
    if let Some(history_length) = query_map
        .get("historyLength")
        .and_then(|value| value.parse::<u32>().ok())
    {
        params.insert("historyLength".into(), json!(history_length));
    }

    rest_dispatch(&state, &headers, "ListTasks", Value::Object(params)).await
}

/// `POST /a2a/tasks/{id}:cancel` / `POST /a2a/tasks/{id}:subscribe`
pub(crate) async fn rest_task_action<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id_action): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }

    let Some((task_id, action)) = id_action.rsplit_once(':') else {
        return rest_error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Unknown task action",
            &json!([]),
        );
    };

    match action {
        "cancel" => rest_dispatch(&state, &headers, "CancelTask", json!({ "id": task_id })).await,
        "subscribe" => {
            let request = wire_request(
                "SubscribeToTask",
                json!({ "id": task_id }),
                bearer_token(&headers),
            );
            let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
            let server = A2AServer::new_with_resources(ctx, state.tool_runtime.clone()); // Safe: Arc clone of trait object
            match server.start_task_subscription(request).await {
                Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, None),
                Err(response) => rest_response(*response),
            }
        }
        _ => rest_error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Unknown task action",
            &json!([]),
        ),
    }
}

/// `POST /a2a/tasks/{id}/pushNotificationConfigs`
pub(crate) async fn rest_push_create<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    if let Err(response) = rest_content_type_guard(&headers) {
        return *response;
    }
    let config = match rest_parse_body(&body) {
        Ok(config) => config,
        Err(response) => return *response,
    };
    rest_dispatch(
        &state,
        &headers,
        "CreateTaskPushNotificationConfig",
        json!({ "taskId": id, "config": config }),
    )
    .await
}

/// `GET /a2a/tasks/{id}/pushNotificationConfigs`
pub(crate) async fn rest_push_list<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id): Path<String>,
    Query(query_map): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        &headers,
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": id }),
    )
    .await
}

/// `GET /a2a/tasks/{id}/pushNotificationConfigs/{config_id}`
pub(crate) async fn rest_push_get<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path((id, config_id)): Path<(String, String)>,
    Query(query_map): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        &headers,
        "GetTaskPushNotificationConfig",
        json!({ "taskId": id, "configId": config_id }),
    )
    .await
}

/// `DELETE /a2a/tasks/{id}/pushNotificationConfigs/{config_id}`
pub(crate) async fn rest_push_delete<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path((id, config_id)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        &headers,
        "DeleteTaskPushNotificationConfig",
        json!({ "taskId": id, "configId": config_id }),
    )
    .await
}

/// `GET /a2a/extendedAgentCard` — no extended card is configured.
pub(crate) async fn rest_extended_card(headers: HeaderMap, RawQuery(query): RawQuery) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    let spec_error = A2ASpecError::ExtendedAgentCardNotConfigured;
    rest_error_response(
        StatusCode::BAD_REQUEST,
        "FAILED_PRECONDITION",
        &spec_error.to_string(),
        &error_info(spec_error.reason()),
    )
}
