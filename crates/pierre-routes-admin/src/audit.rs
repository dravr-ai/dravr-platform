// ABOUTME: The admin audit recorder: one audit_events row for every write an operator makes through the admin API
// ABOUTME: A layer over every admin router; handlers add the target and before/after through an AuditDetail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin audit trail's writer (carnet#1391).
//!
//! [`record_admin_writes`] wraps every admin router — `/admin/*`, the
//! console's `/api/admin/*`, the config and tool-selection routes the
//! composition root mounts beside them — so one place records every write,
//! whichever surface or credential made it: the console's session, a
//! `pierre-cli` device-login token, a service token. A read (`GET`, `HEAD`,
//! `OPTIONS`) passes through untouched, unless its handler records itself
//! through [`record_admin_read`].
//!
//! **Who acted** comes from the response: both admin auth layers put the
//! [`ValidatedAdminToken`] they validated there, since this layer runs
//! outside them. A route that authenticates inside its handler instead (the
//! web-admin user tools, the config API) is resolved here from the request
//! headers, the way the cookie layer would. A request no credential admits is
//! not an operator's action and writes nothing.
//!
//! **What was done** is the route template (never the raw URL) and its path
//! parameters, the status and its outcome — refusals included, since a 409 on
//! a delete is exactly what an operator asks about later. A handler that
//! knows more puts an [`AuditDetail`] on its response: an action key, a
//! sentence, the target, the values before and after. A request body is
//! never read: it carries passwords and client secrets.
//!
//! A failed write is logged and never fails the request it describes.

use std::fmt;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{rejection::RawPathParamsRejection, MatchedPath, RawPathParams, State},
    http::{HeaderMap, Method, Request},
    middleware::Next,
    response::Response,
};
use pierre_core::admin::models::{ValidatedAdminToken, DEVICE_CLI_SERVICE_PREFIX};
use pierre_core::auth_header::extract_bearer_token_owned;
use pierre_database::repositories::{
    AdminAuditRepository, AuditOutcome, AuditSource, NewAuditEvent,
};
use pierre_middleware::{cookie_admin_token, extract_auth_from_headers, require_admin};
use pierre_runtime_context::MiddlewareCtx;
use serde_json::{json, Map, Value};
use tracing::{debug, error};

use crate::auth::service::AdminAuthService;

/// Route prefixes whose writes are not operator actions on the platform: the
/// device-login handshake (a sign-in), first-run setup, the website's magic
/// link, the scheduled canaries.
const UNAUDITED_PREFIXES: &[&str] = &[
    "/admin/device/",
    "/admin/setup",
    "/admin/website/",
    "/admin/diagnostics/",
];

/// What a handler adds to the row its write produces.
///
/// Put on the response (`response.extensions_mut().insert(detail)`); the
/// recorder merges it over what it read from the route.
#[derive(Debug, Clone, Default)]
pub struct AuditDetail {
    /// A stable action key (`user.delete`, `signup.settings.update`).
    pub action: Option<String>,
    /// One line an operator reads in the trail.
    pub summary: Option<String>,
    /// What the action was applied to.
    pub target: Option<Value>,
    /// The value before the write.
    pub before: Option<Value>,
    /// The value after the write.
    pub after: Option<Value>,
    /// Leave the route's path parameters out of the row: a deleted account's
    /// id must not survive in the trail (the deletion policy).
    pub omit_route_params: bool,
}

impl AuditDetail {
    /// A detail with an action key and a sentence.
    #[must_use]
    pub fn new(action: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            action: Some(action.into()),
            summary: Some(summary.into()),
            ..Self::default()
        }
    }

    /// With what the action was applied to.
    #[must_use]
    pub fn target(mut self, target: Value) -> Self {
        self.target = Some(target);
        self
    }

    /// With the values before and after the write.
    #[must_use]
    pub fn change(mut self, before: Value, after: Value) -> Self {
        self.before = Some(before);
        self.after = Some(after);
        self
    }

    /// Without the route's path parameters.
    #[must_use]
    pub const fn omit_route_params(mut self) -> Self {
        self.omit_route_params = true;
        self
    }

    /// Attach to a response.
    #[must_use]
    pub fn attach(self, mut response: Response) -> Response {
        response.extensions_mut().insert(self);
        response
    }
}

/// The recorder's state: the trail, and the server context a handler-auth
/// route's caller is resolved through.
pub struct AuditRecorder<C> {
    audit: Arc<dyn AdminAuditRepository>,
    resources: Arc<C>,
    admin_auth: Option<AdminAuthService>,
}

impl<C> fmt::Debug for AuditRecorder<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuditRecorder")
            .field("admin_auth", &self.admin_auth.is_some())
            .finish_non_exhaustive()
    }
}

impl<C> Clone for AuditRecorder<C> {
    fn clone(&self) -> Self {
        Self {
            audit: Arc::clone(&self.audit),
            resources: Arc::clone(&self.resources),
            admin_auth: self.admin_auth.clone(),
        }
    }
}

impl<C: MiddlewareCtx> AuditRecorder<C> {
    /// A recorder writing to the context's own trail.
    #[must_use]
    pub fn new(resources: Arc<C>) -> Self {
        Self {
            audit: Arc::clone(&resources.repos().admin_audit),
            resources,
            admin_auth: None,
        }
    }

    /// Also admit an admin token on a route that validates it inside its
    /// handler (the config API `pierre-cli config` calls).
    #[must_use]
    pub fn with_admin_auth(mut self, admin_auth: AdminAuthService) -> Self {
        self.admin_auth = Some(admin_auth);
        self
    }
}

/// Where a validated admin credential came from.
fn source_of(token: &ValidatedAdminToken) -> AuditSource {
    if token.token_id.starts_with("cookie:") {
        AuditSource::Console
    } else if token.service_name.starts_with(DEVICE_CLI_SERVICE_PREFIX) {
        AuditSource::Cli
    } else {
        AuditSource::Service
    }
}

/// The caller's address as the edge reported it: the first hop of
/// `X-Forwarded-For`.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|ip| ip.trim().to_owned())
        .filter(|ip| !ip.is_empty())
}

/// Who acted on a route that authenticates inside its handler: the signed-in
/// admin the cookie layer would admit, or `None` when no credential does.
async fn resolve_from_headers<C: MiddlewareCtx>(
    headers: &HeaderMap,
    recorder: &AuditRecorder<C>,
) -> Option<ValidatedAdminToken> {
    if let Some(admin_auth) = &recorder.admin_auth {
        let bearer = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| extract_bearer_token_owned(h).ok());
        if let Some(token) = bearer {
            if let Ok(validated) = admin_auth.authenticate(&token, None).await {
                return Some(validated);
            }
        }
    }
    let resources = &recorder.resources;
    let auth = extract_auth_from_headers(headers, resources).await.ok()?;
    let user = require_admin(auth.user_id, &resources.repos().users)
        .await
        .ok()?;
    Some(cookie_admin_token(&user, auth.active_tenant_id))
}

/// The row a write produces, from its route, its response and the actor.
fn build_event(
    method: &Method,
    route: &str,
    params: Option<Map<String, Value>>,
    status: u16,
    token: &ValidatedAdminToken,
    headers: &HeaderMap,
    detail: Option<AuditDetail>,
) -> NewAuditEvent {
    let detail = detail.unwrap_or_default();
    let outcome = AuditOutcome::from_status(status);
    let source = source_of(token);
    let mut metadata = Map::new();
    metadata.insert("method".to_owned(), json!(method.as_str()));
    metadata.insert("route".to_owned(), json!(route));
    metadata.insert("status".to_owned(), json!(status));
    if !detail.omit_route_params {
        if let Some(params) = params.filter(|p| !p.is_empty()) {
            metadata.insert("params".to_owned(), Value::Object(params));
        }
    }
    // A console session's token id is the admin's own id, already the row's
    // actor; any other names the credential used and the service it was
    // minted for.
    if source != AuditSource::Console {
        metadata.insert("token_id".to_owned(), json!(token.token_id));
        metadata.insert("service".to_owned(), json!(token.service_name));
    }
    for (key, value) in [
        ("target", detail.target),
        ("before", detail.before),
        ("after", detail.after),
    ] {
        if let Some(value) = value {
            metadata.insert(key.to_owned(), value);
        }
    }
    let action = detail
        .action
        .unwrap_or_else(|| format!("{} {route}", method.as_str()));
    let summary = detail
        .summary
        .unwrap_or_else(|| format!("{} {route}", method.as_str()));
    NewAuditEvent {
        action,
        summary,
        outcome,
        source,
        actor_user_id: token.operator_user_id,
        tenant_id: token.tenant_id.clone(),
        ip_address: client_ip(headers),
        user_agent: headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        metadata: Value::Object(metadata),
    }
}

/// Record a read the layer would skip: one a handler judges worth a row because
/// it reaches past the tenant its caller acts in.
///
/// The row names what was read in another tenant, so it belongs to no tenant's
/// trail: a plain admin of the caller's own tenant must not read it. Only a super
/// admin's view of the trail returns it.
///
/// A failed write is logged and never fails the read, like every other row.
pub(crate) async fn record_admin_read(
    audit: &dyn AdminAuditRepository,
    route: &str,
    token: &ValidatedAdminToken,
    headers: &HeaderMap,
    detail: AuditDetail,
) {
    let event = NewAuditEvent {
        tenant_id: None,
        ..build_event(&Method::GET, route, None, 200, token, headers, Some(detail))
    };
    if let Err(e) = audit.record(&event).await {
        error!(error = %e, route, "failed to record an admin read in the audit trail");
    }
}

/// The layer: record every admin write (see the module docs).
pub async fn record_admin_writes<C: MiddlewareCtx>(
    State(recorder): State<AuditRecorder<C>>,
    params: Result<RawPathParams, RawPathParamsRejection>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(request).await;
    }
    let route = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |m| m.as_str().to_owned(),
    );
    if UNAUDITED_PREFIXES.iter().any(|p| route.starts_with(p)) {
        return next.run(request).await;
    }
    let params: Option<Map<String, Value>> = params
        .ok()
        .map(|raw| raw.iter().map(|(k, v)| (k.to_owned(), json!(v))).collect());
    let headers = request.headers().clone();

    let mut response = next.run(request).await;

    let token = match response.extensions_mut().remove::<ValidatedAdminToken>() {
        Some(token) => Some(token),
        None => resolve_from_headers(&headers, &recorder).await,
    };
    let Some(token) = token else {
        debug!(%route, "admin write with no admitted credential: not audited");
        return response;
    };
    let detail = response.extensions_mut().remove::<AuditDetail>();
    let event = build_event(
        &method,
        &route,
        params,
        response.status().as_u16(),
        &token,
        &headers,
        detail,
    );
    if let Err(e) = recorder.audit.record(&event).await {
        error!(error = %e, action = %event.action, "Failed to record an admin audit event");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::admin::models::AdminPermissions;
    use uuid::Uuid;

    fn token(token_id: &str, service: &str) -> ValidatedAdminToken {
        ValidatedAdminToken {
            token_id: token_id.to_owned(),
            service_name: service.to_owned(),
            permissions: AdminPermissions::default_admin(),
            is_super_admin: true,
            tenant_id: None,
            operator_user_id: Some(Uuid::nil()),
            user_info: None,
        }
    }

    #[test]
    fn the_credential_names_the_surface() {
        assert_eq!(
            source_of(&token(&format!("cookie:{}", Uuid::nil()), "a@b.c")),
            AuditSource::Console,
            "bug: a cookie session is the console"
        );
        assert_eq!(
            source_of(&token("t1", &format!("{DEVICE_CLI_SERVICE_PREFIX}a@b.c"))),
            AuditSource::Cli,
            "bug: a device-login token is pierre-cli"
        );
        assert_eq!(
            source_of(&token("t2", "canary")),
            AuditSource::Service,
            "bug: any other token is a service"
        );
    }

    #[test]
    fn a_detail_can_keep_the_route_params_out() {
        let mut params = Map::new();
        params.insert("user_id".to_owned(), json!("6f0c"));
        let tok = token("t1", "device-cli:a@b.c");
        let kept = build_event(
            &Method::POST,
            "/admin/suspend-user/{user_id}",
            Some(params.clone()),
            200,
            &tok,
            &HeaderMap::new(),
            None,
        );
        assert_eq!(
            kept.metadata["params"]["user_id"], "6f0c",
            "bug: the route params are kept by default"
        );
        assert_eq!(
            kept.action, "POST /admin/suspend-user/{user_id}",
            "bug: with no detail the action is the method and route template"
        );
        assert_eq!(
            kept.outcome,
            AuditOutcome::Success,
            "bug: a 200 is a success"
        );

        let omitted = build_event(
            &Method::DELETE,
            "/admin/users/{user_id}",
            Some(params),
            409,
            &tok,
            &HeaderMap::new(),
            Some(AuditDetail::new("user.delete", "Refused").omit_route_params()),
        );
        assert!(
            omitted.metadata.get("params").is_none(),
            "bug: an omitting detail must keep the route params out"
        );
        assert_eq!(
            omitted.action, "user.delete",
            "bug: the detail names the action"
        );
        assert_eq!(
            omitted.outcome,
            AuditOutcome::Refused,
            "bug: a 409 is a refusal"
        );
        assert_eq!(
            omitted.metadata["token_id"], "t1",
            "bug: a non-console row names its token"
        );
        assert_eq!(
            omitted.metadata["service"], "device-cli:a@b.c",
            "bug: a non-console row names the token's service"
        );
    }
}
