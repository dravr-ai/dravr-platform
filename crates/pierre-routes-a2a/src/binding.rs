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
use std::str;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, RawQuery, Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE},
        HeaderMap, HeaderValue, StatusCode,
    },
    middleware::Next,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use pierre_a2a::{
    protocol_types::{
        error_info, retry_after_secs, A2ASpecError, StreamResponse, Task,
        RATE_LIMIT_EXCEEDED_REASON,
    },
    A2ARequest, A2AResponse, A2AServer, JsonRpcError, A2A_VERSION, A2A_VERSION_HEADER,
    INVALID_PARAMS, JSONRPC_VERSION, METHOD_NOT_FOUND, PARSE_ERROR, SERVER_ERROR_START,
};
use pierre_auth::config::{dialed_host, www_authenticate_challenge};
use pierre_auth::dpop::{signing_algorithms, DPOP_SCHEME};
use pierre_middleware::dpop_request::{used_dpop, PresentedCall};
use pierre_runtime_context::{A2ACtx, MiddlewareCtx};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{once, StreamExt};
use tracing::error;

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

/// Attach the RFC 9728 challenge to every 401 the protocol routes answer, on
/// either binding, so a client finds the authorization server from the refusal.
///
/// It names the A2A resource of the host the client dialed, which RFC 9728
/// §3.3 has the client compare to the URL it called. `invalid_token` is said
/// only of a credential that was sent (RFC 6750 §3.1); the JSON-RPC body's own
/// `auth` field is not read here, so a refusal of that one carries no error
/// code, which is still a valid challenge.
pub(crate) async fn challenge_unauthorized<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    request: Request,
    next: Next,
) -> Response {
    let host = dialed_host(|name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    })
    .map(str::to_owned);
    let presented = request.headers().contains_key(AUTHORIZATION);
    // A request that used DPoP (RFC 9449 §7.1) is also told the scheme and
    // the algorithms a proof may use.
    let used_dpop = used_dpop(request.headers());

    let mut response = next.run(request).await;
    if response.status() == StatusCode::UNAUTHORIZED {
        let resource = state
            .ctx
            .oauth2_server_config()
            .a2a_resource_for_host(host.as_deref());
        let bearer = www_authenticate_challenge(&resource, presented.then_some("invalid_token"));
        let challenge = if used_dpop {
            format!(
                "{DPOP_SCHEME} algs=\"{}\", {bearer}",
                signing_algorithms().join(" ")
            )
        } else {
            bearer
        };
        match HeaderValue::from_str(&challenge) {
            Ok(value) => {
                response.headers_mut().insert(WWW_AUTHENTICATE, value);
            }
            Err(e) => error!("The A2A WWW-Authenticate challenge is no header value: {e}"),
        }
    }
    response
}

/// The protocol server for one request, over the routes' shared handles.
fn protocol_server<C: MiddlewareCtx + A2ACtx>(state: &A2ARoutesState<C>) -> A2AServer {
    let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
    A2AServer::new_with_resources(
        ctx,
        state.tool_runtime.clone(),    // Safe: Arc clone of trait object
        state.auth_middleware.clone(), // Safe: Arc clone for the shared auth pipeline
        state.coaching.clone(),        // Safe: Arc clone of trait object
    )
}

/// The wait a rate-limit refusal carries, for its `Retry-After` header.
fn refusal_retry_after(error: &JsonRpcError) -> Option<u64> {
    error.data.as_ref().and_then(retry_after_secs)
}

/// Attach `Retry-After` to `response` when the refusal carried a wait.
fn with_retry_after(mut response: Response, retry_after: Option<u64>) -> Response {
    if let Some(secs) = retry_after {
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(secs));
    }
    response
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

/// Build a bare A2A request for a spec method (REST binding path), carrying
/// the credential as it was presented: its scheme, and the `DPoP` proof, method
/// and path a key-bound token is checked against (RFC 9449).
fn wire_request(method: &str, params: Value, presented: PresentedCall) -> A2ARequest {
    let mut request = A2ARequest::notification(method, Some(params));
    presented.attach(&mut request);
    request
}

/// Serialize one stream frame into an SSE event. With `envelope_id` the
/// frame is wrapped in a full JSON-RPC envelope (JSONRPC binding); without,
/// the bare `StreamResponse` object is the payload (HTTP+JSON binding).
fn frame_event(frame: &StreamResponse, envelope_id: Option<&Value>) -> Event {
    let payload = envelope_id.map_or_else(
        || serde_json::to_value(frame).unwrap_or(Value::Null),
        |id| json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "result": frame }),
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
/// surface as 401 and a spent request budget as 429 (auth is
/// transport-level per spec §4), everything else rides HTTP 200 with the
/// error inside the envelope.
fn jsonrpc_http_status(response: &A2AResponse) -> StatusCode {
    response.error.as_ref().map_or(StatusCode::OK, |error| {
        if has_reason(error, AUTHENTICATION_REQUIRED) {
            StatusCode::UNAUTHORIZED
        } else if has_reason(error, RATE_LIMIT_EXCEEDED_REASON) {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::OK
        }
    })
}

/// A JSON-RPC envelope as its HTTP response: the status
/// [`jsonrpc_http_status`] picks, and `Retry-After` on a rate-limit refusal.
fn jsonrpc_response(response: A2AResponse) -> Response {
    let status = jsonrpc_http_status(&response);
    let retry_after = response.error.as_ref().and_then(refusal_retry_after);
    with_retry_after((status, Json(response)).into_response(), retry_after)
}

/// Read a `POST /a2a/jsonrpc` body into a request, or the error envelope to
/// answer with. A body that is not UTF-8 is a parse error (-32700), like
/// text that is not JSON; the rest is [`A2ARequest::parse`].
fn parse_jsonrpc_body(body: &[u8]) -> Result<A2ARequest, Box<A2AResponse>> {
    str::from_utf8(body)
        .map_err(|e| {
            Box::new(A2AResponse::error(
                None,
                PARSE_ERROR,
                format!("Parse error: body is not UTF-8: {e}"),
            ))
        })
        .and_then(A2ARequest::parse)
}

/// Handle the A2A JSON-RPC transport endpoint (`POST /a2a/jsonrpc`).
///
/// Dispatches non-streaming methods to [`A2AServer::handle_request`] and
/// answers `SendStreamingMessage` / `SubscribeToTask` with SSE.
///
/// The body is read with [`A2ARequest::parse`], which keeps a parse error
/// (-32700) apart from an invalid request (-32600) and never deserializes the
/// `auth`, `headers` or `metadata` extensions. The caller's credential comes
/// only from the `Authorization` header: a body is written by the client, so
/// an `auth` member in it is ignored, and a request without a header token is
/// refused by the protocol layer.
pub(crate) async fn handle_jsonrpc<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    presented: PresentedCall,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    let mut request = match parse_jsonrpc_body(&body) {
        Ok(request) => request,
        Err(response) => return (StatusCode::OK, Json(*response)).into_response(),
    };

    // The credential as presented, `Bearer` or `DPoP` with its proof.
    presented.attach(&mut request);

    if !version_supported(&headers, query.as_deref()) {
        let spec_error = A2ASpecError::VersionNotSupported;
        let response = A2AResponse::error_with_data(
            request.id.clone(), // Safe: JSON value ownership for the error envelope
            spec_error.code(),
            spec_error.to_string(),
            error_info(spec_error.reason()),
        );
        return (StatusCode::OK, Json(response)).into_response();
    }

    let server = protocol_server(&state);

    match request.method.as_str() {
        "SendStreamingMessage" => {
            let envelope_id = request.id.clone(); // Safe: JSON value ownership for SSE frames
            match server.start_streaming_message(request).await {
                Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, envelope_id),
                Err(response) => jsonrpc_response(*response),
            }
        }
        "SubscribeToTask" => {
            let envelope_id = request.id.clone(); // Safe: JSON value ownership for SSE frames
            match server.start_task_subscription(request).await {
                Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, envelope_id),
                Err(response) => jsonrpc_response(*response),
            }
        }
        _ => jsonrpc_response(server.handle_request(request).await),
    }
}

// =========================== HTTP+JSON binding ===========================

/// Map a JSON-RPC error to the HTTP status + `google.rpc.Code` name of the
/// v1.0.1 error-mapping table.
fn rest_error_mapping(error: &JsonRpcError) -> (StatusCode, &'static str) {
    if let Some(spec_error) = A2ASpecError::from_code(error.code) {
        return match spec_error {
            A2ASpecError::TaskNotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            A2ASpecError::TaskNotCancelable
            | A2ASpecError::PushNotificationNotSupported
            | A2ASpecError::UnsupportedOperation
            | A2ASpecError::ExtendedAgentCardNotConfigured
            | A2ASpecError::ExtensionSupportRequired
            | A2ASpecError::VersionNotSupported => (StatusCode::BAD_REQUEST, "FAILED_PRECONDITION"),
            A2ASpecError::ContentTypeNotSupported => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        };
    }
    match error.code {
        INVALID_PARAMS | PARSE_ERROR => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
        METHOD_NOT_FOUND => (StatusCode::NOT_IMPLEMENTED, "UNIMPLEMENTED"),
        SERVER_ERROR_START if has_reason(error, AUTHENTICATION_REQUIRED) => {
            (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED")
        }
        SERVER_ERROR_START if has_reason(error, RATE_LIMIT_EXCEEDED_REASON) => {
            (StatusCode::TOO_MANY_REQUESTS, "RESOURCE_EXHAUSTED")
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
        return with_retry_after(
            rest_error_response(status, code_name, &error.message, &details),
            refusal_retry_after(error),
        );
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
    presented: PresentedCall,
    method: &str,
    params: Value,
) -> Response {
    let request = wire_request(method, params, presented);
    let server = protocol_server(state);
    rest_response(server.handle_request(request).await)
}

/// `POST /a2a/message:send`
pub(crate) async fn rest_message_send<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    presented: PresentedCall,
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
    rest_dispatch(&state, presented, "SendMessage", params).await
}

/// `POST /a2a/message:stream` — SSE with bare `StreamResponse` frames.
pub(crate) async fn rest_message_stream<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    presented: PresentedCall,
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

    let request = wire_request("SendStreamingMessage", params, presented);
    let server = protocol_server(&state);
    match server.start_streaming_message(request).await {
        Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, None),
        Err(response) => rest_response(*response),
    }
}

/// `GET /a2a/tasks/{id}` and `GET /a2a/tasks/{id}:subscribe`.
///
/// The router matches a whole segment, so the path parameter holds any
/// trailing custom verb (AIP-136 style) and this handler splits it, as
/// [`rest_task_action`] does for `POST`. Unsplit, `{id}:subscribe` would read
/// as `GetTask` on an id that never exists. 1.0.1's normative `a2a.proto` binds
/// `SubscribeToTask` to this `GET`; its prose specification says `POST`, so both
/// are served.
pub(crate) async fn rest_get_task<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id): Path<String>,
    Query(query_map): Query<HashMap<String, String>>,
    headers: HeaderMap,
    presented: PresentedCall,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    if let Some((task_id, verb)) = id.rsplit_once(':') {
        return match verb {
            "subscribe" => rest_subscribe(&state, presented, task_id).await,
            _ => unknown_task_action(),
        };
    }
    let mut params = json!({ "id": id });
    if let Some(history_length) = query_map
        .get("historyLength")
        .and_then(|value| value.parse::<u32>().ok())
    {
        params["historyLength"] = json!(history_length);
    }
    rest_dispatch(&state, presented, "GetTask", params).await
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
    presented: PresentedCall,
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

    rest_dispatch(&state, presented, "ListTasks", Value::Object(params)).await
}

/// `POST /a2a/tasks/{id}:cancel` / `POST /a2a/tasks/{id}:subscribe`
pub(crate) async fn rest_task_action<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id_action): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    presented: PresentedCall,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }

    let Some((task_id, action)) = id_action.rsplit_once(':') else {
        return unknown_task_action();
    };

    match action {
        "cancel" => rest_dispatch(&state, presented, "CancelTask", json!({ "id": task_id })).await,
        "subscribe" => rest_subscribe(&state, presented, task_id).await,
        _ => unknown_task_action(),
    }
}

/// `SubscribeToTask` over the HTTP+JSON binding: SSE with bare `StreamResponse`
/// frames.
async fn rest_subscribe<C: MiddlewareCtx + A2ACtx>(
    state: &A2ARoutesState<C>,
    presented: PresentedCall,
    task_id: &str,
) -> Response {
    let request = wire_request("SubscribeToTask", json!({ "id": task_id }), presented);
    let server = protocol_server(state);
    match server.start_task_subscription(request).await {
        Ok((snapshot, receiver)) => sse_task_stream(snapshot, receiver, None),
        Err(response) => rest_response(*response),
    }
}

/// The refusal of a custom verb this route does not serve.
fn unknown_task_action() -> Response {
    rest_error_response(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "Unknown task action",
        &json!([]),
    )
}

/// `POST /a2a/tasks/{id}/pushNotificationConfigs`
pub(crate) async fn rest_push_create<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    presented: PresentedCall,
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
        presented,
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
    presented: PresentedCall,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        presented,
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
    presented: PresentedCall,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query_string(&query_map).as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        presented,
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
    presented: PresentedCall,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    rest_dispatch(
        &state,
        presented,
        "DeleteTaskPushNotificationConfig",
        json!({ "taskId": id, "configId": config_id }),
    )
    .await
}

/// `GET /a2a/extendedAgentCard` — the authenticated extended card
/// (public card enriched with the live tool registry as skills).
pub(crate) async fn rest_extended_card<C: MiddlewareCtx + A2ACtx>(
    State(state): State<A2ARoutesState<C>>,
    headers: HeaderMap,
    presented: PresentedCall,
    RawQuery(query): RawQuery,
) -> Response {
    if let Err(response) = rest_version_guard(&headers, query.as_deref()) {
        return *response;
    }
    rest_dispatch(&state, presented, "GetExtendedAgentCard", Value::Null).await
}

#[cfg(test)]
mod tests {
    use super::{parse_jsonrpc_body, PARSE_ERROR};

    #[test]
    fn a_body_that_is_not_utf8_is_a_parse_error_with_a_null_id() {
        let refused = parse_jsonrpc_body(&[0xff, 0xfe, b'{'])
            .expect_err("a non-UTF-8 body must not parse into a request");
        let error = refused
            .error
            .as_ref()
            .expect("a refused body is answered with an error envelope");
        assert_eq!(
            error.code, PARSE_ERROR,
            "a body that is not UTF-8 is a JSON-RPC parse error (binding bug otherwise)"
        );
        assert!(
            error.message.contains("not UTF-8"),
            "the parse error names the encoding failure: {}",
            error.message
        );
        assert!(
            refused.id.is_none(),
            "an unreadable body has no id to answer with (binding bug otherwise)"
        );
    }

    #[test]
    fn an_auth_member_in_the_body_never_becomes_the_credential() {
        let request = parse_jsonrpc_body(
            br#"{"jsonrpc":"2.0","method":"GetTask","id":3,"auth":"body-token"}"#,
        )
        .expect("a well-formed request parses");
        assert_eq!(
            request.method, "GetTask",
            "the parsed request keeps its method (parser bug otherwise)"
        );
        assert!(
            request.auth_token.is_none(),
            "a client-written `auth` member must never fill the credential (binding bug otherwise)"
        );
    }
}
