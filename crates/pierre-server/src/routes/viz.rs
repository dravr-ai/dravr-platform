// ABOUTME: Serves a coach chart as a PNG for messaging channels that fetch media by URL
// ABOUTME: HMAC-signed short-TTL tokens, because the URL is handed to a third-party fetcher

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chart images for messaging channels.
//!
//! Telegram, `WhatsApp`, Slack and Discord cannot render a vector in a message,
//! but every one of them accepts a media URL and fetches it server-side at send
//! time. That fetch is the whole lifetime of this route: the channel re-hosts
//! the bytes on its own CDN, so the URL needs to survive seconds, not days.
//!
//! That is why there is no bucket. The durable record stays the spec on the
//! message row; the PNG is a delivery artifact regenerated on demand, which
//! means an improvement to the geometry engine reaches charts already sent to a
//! channel the next time anything asks for them, and nothing accumulates in
//! storage waiting for a lifecycle rule.
//!
//! # Why the token is signed
//!
//! The URL is handed to a third party and travels through their infrastructure
//! in the clear. An unauthenticated `/api/viz/<message-id>/<n>.png` would let
//! anyone who saw one URL enumerate every chart in every conversation. The
//! token therefore carries its own authorisation: an HMAC over the exact
//! message, block index, theme and expiry, keyed on the server secret. No
//! session, no cookie — the bearer is a bot fetcher that has neither.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::Utc;
use hmac::{Hmac, Mac};
use photograveur::{resolve, Locale, RenderBlock};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use serde_json::Value;
use sha2::Sha256;
use tracing::{debug, warn};

use crate::mcp::resources::ServerContext;

type HmacSha256 = Hmac<Sha256>;

/// How long a minted URL stays fetchable.
///
/// Long enough for a channel's fetcher to pick it up including a retry, short
/// enough that a leaked URL is worthless by the time it is shared. Channels
/// re-host immediately, so nothing legitimate needs it after this.
const TOKEN_TTL_SECONDS: i64 = 900;

/// Everything a request needs to identify one rendered block.
///
/// Serialised into the URL and covered by the signature, so none of it can be
/// tampered with — swapping the message id or the block index invalidates the
/// MAC rather than serving a different athlete's chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VizToken {
    /// Conversation holding the message.
    ///
    /// Present because the repository reads messages per conversation; there is
    /// no by-id lookup. It is covered by the signature like everything else, so
    /// carrying it costs nothing in safety.
    pub conversation_id: String,
    /// Owner of the conversation, for the tenant-scoped read.
    pub user_id: String,
    /// Tenant the message belongs to.
    pub tenant_id: TenantId,
    /// Message the block belongs to.
    pub message_id: String,
    /// Index into that message's stored block array.
    pub block_index: usize,
    /// Palette to press in.
    pub theme: String,
    /// Locale the axis labels resolve in.
    ///
    /// Carried rather than looked up: the stored record is the spec, so the
    /// Scene is rebuilt on every fetch, and it must be rebuilt in the language
    /// the athlete was replied to in — not whatever their profile says today.
    pub locale: String,
    /// Unix seconds after which the token is refused.
    pub expires_at: i64,
}

impl VizToken {
    /// The exact bytes the signature covers.
    fn payload(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.conversation_id,
            self.user_id,
            self.tenant_id,
            self.message_id,
            self.block_index,
            self.theme,
            self.locale,
            self.expires_at
        )
    }

    /// Mint a signed, URL-safe token.
    #[must_use]
    pub fn mint(&self, secret: &str) -> String {
        let payload = self.payload();
        let signature = sign(&payload, secret);
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload.as_bytes()),
            signature
        )
    }

    /// Build a token for a block, expiring [`TOKEN_TTL_SECONDS`] from now.
    #[must_use]
    pub fn for_block(target: VizTarget, block_index: usize, theme: &str, locale: &str) -> Self {
        Self {
            conversation_id: target.conversation_id,
            user_id: target.user_id,
            tenant_id: target.tenant_id,
            message_id: target.message_id,
            block_index,
            theme: theme.to_owned(),
            locale: locale.to_owned(),
            expires_at: Utc::now().timestamp() + TOKEN_TTL_SECONDS,
        }
    }

    /// Parse and verify a token.
    ///
    /// Returns `None` for anything that is not a well-formed, correctly signed,
    /// unexpired token. The caller turns that into a 404 rather than a 401 —
    /// distinguishing "bad signature" from "no such chart" would tell a prober
    /// which message ids exist.
    #[must_use]
    pub fn verify(raw: &str, secret: &str) -> Option<Self> {
        let (encoded, signature) = raw.split_once('.')?;
        let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()?;

        // Constant-time via the MAC's own verifier: a byte-by-byte string
        // compare here would leak the signature through timing.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
        mac.update(payload.as_bytes());
        let expected = URL_SAFE_NO_PAD.decode(signature).ok()?;
        mac.verify_slice(&expected).ok()?;

        let mut parts = payload.split('|');
        let conversation_id = parts.next()?.to_owned();
        let user_id = parts.next()?.to_owned();
        let tenant_id = TenantId::from_uuid(parse_uuid(parts.next()?).ok()?);
        let message_id = parts.next()?.to_owned();
        let block_index = parts.next()?.parse().ok()?;
        let theme = parts.next()?.to_owned();
        let locale = parts.next()?.to_owned();
        let expires_at: i64 = parts.next()?.parse().ok()?;

        if Utc::now().timestamp() > expires_at {
            debug!(message_id, "viz token expired");
            return None;
        }

        Some(Self {
            conversation_id,
            user_id,
            tenant_id,
            message_id,
            block_index,
            theme,
            locale,
            expires_at,
        })
    }
}

/// Which message a token points at.
///
/// A struct rather than four positional strings, because
/// `(conversation, user, tenant, message)` are all opaque ids and transposing
/// two of them would mint a token that verifies and then finds nothing.
#[derive(Debug, Clone)]
pub struct VizTarget {
    /// Conversation holding the message.
    pub conversation_id: String,
    /// Owner of the conversation.
    pub user_id: String,
    /// Tenant the message belongs to.
    pub tenant_id: TenantId,
    /// Message carrying the block.
    pub message_id: String,
}

/// HMAC-SHA256 of `payload`, URL-safe base64.
fn sign(payload: &str, secret: &str) -> String {
    // An HMAC accepts a key of any length, so this cannot fail in practice;
    // an empty signature would fail every verification rather than pass one.
    HmacSha256::new_from_slice(secret.as_bytes()).map_or_else(
        |_| String::new(),
        |mut mac| {
            mac.update(payload.as_bytes());
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        },
    )
}

/// `GET /api/viz/{token}.png` — press and return one chart.
///
/// # Errors
///
/// 404 for a token that is malformed, wrongly signed, expired, or names a
/// message or block that does not exist. All four are one response on purpose:
/// a distinguishable error would confirm which message ids are real.
pub async fn get_viz_png(
    State(resources): State<Arc<ServerContext>>,
    Path(token): Path<String>,
) -> Result<Response, AppError> {
    let raw = token.strip_suffix(".png").unwrap_or(&token);
    let Some(parsed) = VizToken::verify(raw, &resources.auth.admin_jwt_secret) else {
        return Err(AppError::not_found("chart not found"));
    };

    let block = load_block(&resources, &parsed)
        .await?
        .ok_or_else(|| AppError::not_found("chart not found"))?;

    let png = resources
        .common
        .photograveur
        .press(&block, &parsed.theme)
        .await
        .inspect_err(|e| warn!(error = %e, "photograveur press failed"))?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png".to_owned()),
            // Private and short-lived: the URL is single-purpose and the channel
            // re-hosts immediately, so nothing downstream should keep it.
            (
                header::CACHE_CONTROL,
                format!("private, max-age={TOKEN_TTL_SECONDS}"),
            ),
        ],
        png,
    )
        .into_response())
}

/// Resolve the token's block from the stored spec.
///
/// Reads through the tenant-scoped repository rather than trusting the token to
/// carry content: the token authorises a lookup, it never *is* the data. A
/// token naming a message in another tenant therefore finds nothing rather than
/// serving it.
async fn load_block(
    resources: &ServerContext,
    token: &VizToken,
) -> Result<Option<RenderBlock>, AppError> {
    let messages = resources
        .common
        .repos
        .chat
        .get_messages(&token.conversation_id, &token.user_id, token.tenant_id)
        .await?;

    let Some(message) = messages.into_iter().find(|m| m.id == token.message_id) else {
        return Ok(None);
    };
    let Some(raw) = message.content_blocks else {
        return Ok(None);
    };
    let specs: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
    let Some(spec) = specs.get(token.block_index) else {
        return Ok(None);
    };

    // Resolved here rather than sent as a spec: the press service runs no
    // geometry, which is what makes the PNG and the in-app chart identical.
    match resolve(spec, Locale::from_tag(&token.locale)) {
        Ok(block) => Ok(Some(block)),
        Err(e) => {
            warn!(error = %e, "viz block failed to resolve for raster");
            Ok(None)
        }
    }
}

/// Routes for chart images.
pub struct VizRoutes;

impl VizRoutes {
    /// Unauthenticated by design: the signed token *is* the authorisation, and
    /// the caller is a channel's media fetcher, which carries no session.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/viz/{token}", get(get_viz_png))
            .with_state(resources)
    }
}
