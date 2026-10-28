// ABOUTME: Short-lived signed link-tokens for channel-initiated provider connection flows
// ABOUTME: Scoped JWT (HS256) that lets a channel bot hand a user a hosted-login URL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Short-lived provider link-tokens for channel-initiated hosted login.
//!
//! A link-token is a JWT that authorizes a user's browser to call a specific
//! provider's hosted-login endpoints (e.g. Sciotte) without an existing
//! Pierre session cookie.
//!
//! ## Flow
//!
//! 1. The platform decides a user must (re)connect a provider — the chat
//!    pipeline's auth recovery, the backfill notifier, or the messaging
//!    connect command — for a `user_id` + `tenant_id` it already resolved.
//! 2. Pierre mints, in-process, an HS256-signed JWT (claims: sub=user_id, tid=tenant_id, tgt=target,
//!    scope=`provider:{provider}:login`, exp=now+TTL, jti=nonce).
//! 3. The hosted-login URL (with `?token=...`) is posted to the user in their channel.
//! 4. User's browser opens the URL. Server validates the token, embeds it in the page.
//! 5. The page's JS calls `/api/providers/{provider}/*` with `Authorization: Bearer <token>`.
//! 6. The sciotte login, 2FA and OTP handlers accept the link-token as an
//!    alternative to normal cookie/JWT auth (via [`verify_link_token`]) and
//!    extract user_id + tenant_id from the claims. No other route accepts one.
//!
//! ## Security
//!
//! - **Scope-clamped**: the `scope` claim restricts the token to a single provider's
//!   login endpoints; any other endpoint must reject it. A provider-specific
//!   token logs in only to the platform its `tgt` names.
//! - **Bounded lifetime, tiered by scope**: provider-specific tokens get a
//!   24-hour TTL (`PROVIDER_LINK_TOKEN_TTL_MINUTES`) so a reconnect nudge the
//!   user reads hours later still resolves — they are nonce-burned on first
//!   page load. Generic connect tokens are **not** nonce-burned (the picker
//!   must survive refreshes and multi-provider connects), so they get a 1-hour
//!   TTL (`CONNECT_LINK_TOKEN_TTL_MINUTES`) to cap the reusable window.
//! - **Signed with `admin_jwt_secret`**: the same server-wide HMAC secret used for
//!   admin-token hashing, so no new secret material is introduced.
//! - **One-time page load**: each token's `jti` is burned on the first
//!   `GET` of the hosted sign-in page ([`HOSTED_SIGNIN_PATH`]) (see [`NonceStore`]);
//!   subsequent page loads receive an error. The token remains valid for the
//!   login, 2FA and OTP POST endpoints within its TTL so the multi-step Sciotte flow
//!   (credentials -> 2FA -> OTP) can complete.
//! - **Minted in-process only**: no HTTP route mints a provider or connect
//!   token, so there is no caller to rate-limit; every mint follows a platform
//!   decision about the user. The one exchange is the hosted import page, which
//!   narrows a valid connect token into an *import* token
//!   ([`mint_import_link_token`]) that reaches only the hosted import
//!   endpoints of one source and never outlives the connect token.
//! - **Replica-safe**: [`NonceStore`] is backed by
//!   the server's [`Cache`] layer (in-memory or Redis), so state is shared across
//!   Cloud Run replicas when Redis is configured.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pierre_cache::{CacheKey, CacheProvider, CacheResource};
use pierre_core::models::TenantId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pierre_core::errors::{AppError, AppResult};

/// Default TTL for provider-specific link-tokens (24 hours).
///
/// These tokens back the channel reconnect nudges a user reads whenever they
/// next open their chat app — often hours after the nudge was sent — so the
/// validity window must outlast a single sitting. The earlier 20-minute value
/// (sized to the sciotte hosted-login completion timeout) expired stale clicks
/// and served the "Link error" page instead of the reconnect form. The
/// hosted-login page burns the token's nonce on first load and the token is
/// scoped to one (user, tenant, provider) in the user's own DM, so the longer
/// window does not widen the blast radius for this scope.
/// `MAX_LINK_TOKEN_LIFETIME_SECS` (the nonce-store TTL) derives from this so the
/// single-use guard stays armed for the token's whole life.
pub const PROVIDER_LINK_TOKEN_TTL_MINUTES: i64 = 24 * 60;

/// TTL for *generic connect* link-tokens (1 hour) — deliberately much shorter
/// than [`PROVIDER_LINK_TOKEN_TTL_MINUTES`].
///
/// The connect picker never nonce-burns its token (the page must survive
/// refreshes and multi-provider connects, see `connect_hosted`), so for the
/// connect scope the token is a **reusable** bearer credential for its whole
/// life — and the connect scope is a superset accepted by every per-provider
/// endpoint. A 24-hour reusable superset credential sitting in chat transcripts
/// and request logs is too wide a window; one hour still covers the in-chat
/// flow (the connect Card is a direct reply to an active conversation, unlike
/// the reauth nudge that must survive being read the next morning). An expired
/// link degrades gracefully: the user asks again and gets a fresh Card.
pub const CONNECT_LINK_TOKEN_TTL_MINUTES: i64 = 60;

/// JWT issuer claim for provider link-tokens
const PROVIDER_LINK_TOKEN_ISSUER: &str = "pierre-provider-link";

/// JWT audience claim for provider link-tokens
const PROVIDER_LINK_TOKEN_AUDIENCE: &str = "pierre-hosted-login";

/// Scope prefix for provider link-token scope claims (e.g. `provider:sciotte:login`)
const PROVIDER_LINK_TOKEN_SCOPE_PREFIX: &str = "provider:";

/// Path of the hosted credential sign-in page a provider link-token opens
/// (`?token=...`): the athlete's own username and password, through Dravr.
///
/// The path names no backend, because the athlete reads it in the browser's
/// address bar and, when a short link cannot be stored, in the chat message
/// itself. Every link this platform mints uses it; the earlier
/// `/providers/sciotte/*` paths stay routed to the same pages so links
/// already sent keep opening.
pub const HOSTED_SIGNIN_PATH: &str = "/providers/signin";

/// Sentinel provider name for a *generic connect* link-token.
///
/// A token minted with this provider carries the scope `provider:connect:login`
/// and authorizes the whole hosted connect picker — both the credential
/// (Sciotte) endpoints and the OAuth-init endpoint — rather than a single
/// provider's login. The concrete provider is chosen by the user on the hosted
/// page. [`verify_link_token`] accepts a connect-scoped token wherever a
/// provider-specific token is accepted (the connect scope is a superset), while
/// a provider-specific token is NOT accepted at the connect/OAuth endpoints.
pub const CONNECT_PROVIDER: &str = "connect";

/// Claims carried in a provider link-token JWT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderLinkTokenClaims {
    /// Subject — the `user_id` the token authorizes actions for
    pub sub: String,
    /// Active `tenant_id` for the session
    pub tid: String,
    /// Target platform (e.g. "strava", "garmin")
    pub tgt: String,
    /// Originating channel (e.g. "slack", "discord") — for audit + phishing defense
    pub channel: String,
    /// Optional channel thread/DM id — for the bot to post a confirmation back
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_thread: Option<String>,
    /// Scope clamp — only handlers matching this scope accept the token
    pub scope: String,
    /// Unique token id (nonce)
    pub jti: String,
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Issued-at (unix seconds)
    pub iat: i64,
    /// Expiry (unix seconds)
    pub exp: i64,
}

/// Arguments for minting a provider link-token
#[derive(Debug)]
pub struct MintProviderLinkTokenArgs<'a> {
    /// Target user
    pub user_id: Uuid,
    /// Active tenant for the session
    pub tenant_id: Uuid,
    /// Provider name (e.g. "sciotte")
    pub provider: &'a str,
    /// Target platform ("strava", "garmin" or "trainingpeaks")
    pub target: &'a str,
    /// Originating channel slug ("slack", "discord", "telegram", ...)
    pub channel: &'a str,
    /// Optional channel thread/DM id for follow-up confirmation
    pub channel_thread: Option<&'a str>,
}

/// Build the scope claim string for a given provider
#[must_use]
pub fn provider_scope(provider: &str) -> String {
    format!("{PROVIDER_LINK_TOKEN_SCOPE_PREFIX}{provider}:login")
}

/// Mint a new HS256 provider link-token signed with the server's admin JWT secret
///
/// # Errors
///
/// Returns an error if JWT encoding fails (invalid claims or HMAC key).
pub fn mint_link_token(
    args: &MintProviderLinkTokenArgs<'_>,
    admin_jwt_secret: &str,
) -> AppResult<String> {
    mint_link_token_with_ttl(args, admin_jwt_secret, PROVIDER_LINK_TOKEN_TTL_MINUTES)
}

/// Mint a link-token with an explicit TTL — the scope-appropriate TTL is chosen
/// by the public mint fns ([`mint_link_token`] = 24h nonce-burned reauth,
/// [`mint_connect_link_token`] = 1h reusable connect).
fn mint_link_token_with_ttl(
    args: &MintProviderLinkTokenArgs<'_>,
    admin_jwt_secret: &str,
    ttl_minutes: i64,
) -> AppResult<String> {
    let now = Utc::now();
    let expiry = now + Duration::minutes(ttl_minutes);

    let claims = ProviderLinkTokenClaims {
        sub: args.user_id.to_string(),
        tid: args.tenant_id.to_string(),
        tgt: args.target.to_owned(),
        channel: args.channel.to_owned(),
        channel_thread: args.channel_thread.map(ToOwned::to_owned),
        scope: provider_scope(args.provider),
        jti: Uuid::new_v4().to_string(),
        iss: PROVIDER_LINK_TOKEN_ISSUER.to_owned(),
        aud: PROVIDER_LINK_TOKEN_AUDIENCE.to_owned(),
        iat: now.timestamp(),
        exp: expiry.timestamp(),
    };

    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(admin_jwt_secret.as_bytes());
    encode(&header, &claims, &key)
        .map_err(|e| AppError::internal(format!("Failed to encode provider link-token: {e}")))
}

/// Mint a *generic connect* link-token for the hosted connect picker.
///
/// Unlike [`mint_link_token`], this is called **in-process** by the platform's
/// own messaging pipeline (which already resolved `user_id` + `tenant_id` for
/// the turn) — there is no admin-authed HTTP round-trip. The resulting token
/// carries the [`CONNECT_PROVIDER`] scope, so the hosted page can route the user
/// to either the credential (Sciotte) flow or the OAuth flow for whichever
/// provider they pick.
///
/// `tgt` is set to [`CONNECT_PROVIDER`] (the concrete platform is decided on the
/// page); `channel` + `channel_thread` are carried through so the
/// connection-complete push can land in the originating conversation.
///
/// # Errors
///
/// Returns an error if JWT encoding fails.
pub fn mint_connect_link_token(
    user_id: Uuid,
    tenant_id: Uuid,
    channel: &str,
    channel_thread: Option<&str>,
    admin_jwt_secret: &str,
) -> AppResult<String> {
    mint_link_token_with_ttl(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id,
            provider: CONNECT_PROVIDER,
            target: CONNECT_PROVIDER,
            channel,
            channel_thread,
        },
        admin_jwt_secret,
        CONNECT_LINK_TOKEN_TTL_MINUTES,
    )
}

/// Verify an HS256 provider link-token and check scope matches the expected provider.
///
/// A token carrying the generic [`CONNECT_PROVIDER`] scope (`provider:connect:login`)
/// is accepted for **any** `expected_provider` — the connect scope is a superset
/// granted by the hosted connect picker. Conversely, asking for
/// `expected_provider == CONNECT_PROVIDER` accepts ONLY a connect-scoped token,
/// so a narrow per-provider token (e.g. the Canot-minted `provider:sciotte:login`)
/// can never reach the connect/OAuth-init endpoints.
///
/// # Errors
///
/// Returns [`AppError::auth_invalid`] if the token is malformed, expired, signature-invalid,
/// or scoped to a different provider.
pub fn verify_link_token(
    token: &str,
    admin_jwt_secret: &str,
    expected_provider: &str,
) -> AppResult<ProviderLinkTokenClaims> {
    let key = DecodingKey::from_secret(admin_jwt_secret.as_bytes());

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_issuer(&[PROVIDER_LINK_TOKEN_ISSUER]);
    validation.set_audience(&[PROVIDER_LINK_TOKEN_AUDIENCE]);

    let data = decode::<ProviderLinkTokenClaims>(token, &key, &validation)
        .map_err(|e| AppError::auth_invalid(format!("Invalid provider link-token: {e}")))?;

    let expected_scope = provider_scope(expected_provider);
    let connect_scope = provider_scope(CONNECT_PROVIDER);
    if data.claims.scope != expected_scope && data.claims.scope != connect_scope {
        return Err(AppError::auth_invalid(format!(
            "Link-token scope '{}' does not match expected '{expected_scope}'",
            data.claims.scope
        )));
    }

    Ok(data.claims)
}

/// Ceiling on an *import* link-token's lifetime (1 hour).
///
/// An import token is minted from a connect token
/// ([`mint_import_link_token`]) and never outlives it: its expiry is the
/// earlier of this ceiling and the connect token's own.
pub const IMPORT_LINK_TOKEN_TTL_MINUTES: i64 = 60;

/// Audience of an import link-token. It differs from the hosted-login
/// audience, so [`verify_link_token`] refuses an import token before it reads
/// the scope, and [`verify_import_link_token`] refuses every other token.
const IMPORT_LINK_TOKEN_AUDIENCE: &str = "pierre-hosted-import";

/// Scope prefix of an import link-token (`import:strava_archive`).
const IMPORT_LINK_TOKEN_SCOPE_PREFIX: &str = "import:";

/// The scope claim of an import link-token for `source`.
#[must_use]
pub fn import_scope(source: &str) -> String {
    format!("{IMPORT_LINK_TOKEN_SCOPE_PREFIX}{source}")
}

/// Mint an *import* link-token for the hosted import page, from the verified
/// claims of the connect token that opened it.
///
/// The import token is narrower than the connect token in every way:
///
/// - **single purpose**: its scope `import:{source}` and its own audience
///   reach only the hosted import endpoints for that one source — starting,
///   following and cancelling an import — and no provider login, OAuth start,
///   import report or history delete;
/// - **single user**: it names the connect token's user and tenant, and the
///   endpoints act in that scope only;
/// - **short-lived**: it expires after [`IMPORT_LINK_TOKEN_TTL_MINUTES`], or
///   with the connect token if that comes first, so an exchange never extends
///   a link's life.
///
/// The hosted import page mints it when a valid connect token opens the page:
/// the one HTTP path that mints a link-token, and only by narrowing one the
/// platform already minted.
///
/// # Errors
///
/// [`AppError::auth_invalid`] when `connect` is not a connect token's claims;
/// an internal error when JWT encoding fails.
pub fn mint_import_link_token(
    connect: &ProviderLinkTokenClaims,
    source: &str,
    admin_jwt_secret: &str,
) -> AppResult<String> {
    if connect.scope != provider_scope(CONNECT_PROVIDER) {
        return Err(AppError::auth_invalid(
            "Only a connect link-token opens the hosted import page",
        ));
    }
    let now = Utc::now();
    let expiry = (now + Duration::minutes(IMPORT_LINK_TOKEN_TTL_MINUTES))
        .timestamp()
        .min(connect.exp);
    let claims = ProviderLinkTokenClaims {
        sub: connect.sub.clone(),
        tid: connect.tid.clone(),
        tgt: source.to_owned(),
        channel: connect.channel.clone(),
        channel_thread: connect.channel_thread.clone(),
        scope: import_scope(source),
        jti: Uuid::new_v4().to_string(),
        iss: PROVIDER_LINK_TOKEN_ISSUER.to_owned(),
        aud: IMPORT_LINK_TOKEN_AUDIENCE.to_owned(),
        iat: now.timestamp(),
        exp: expiry,
    };
    let key = EncodingKey::from_secret(admin_jwt_secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
        .map_err(|e| AppError::internal(format!("Failed to encode import link-token: {e}")))
}

/// Verify an import link-token for `source`.
///
/// Only a token [`mint_import_link_token`] minted for this very source
/// passes: a connect token, a provider token, or an import token for another
/// source is refused. Unlike [`verify_link_token`], no wider scope stands in
/// for this one.
///
/// # Errors
///
/// [`AppError::auth_invalid`] if the token is malformed, expired,
/// signature-invalid, or not an import token for `source`.
pub fn verify_import_link_token(
    token: &str,
    admin_jwt_secret: &str,
    source: &str,
) -> AppResult<ProviderLinkTokenClaims> {
    let key = DecodingKey::from_secret(admin_jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_issuer(&[PROVIDER_LINK_TOKEN_ISSUER]);
    validation.set_audience(&[IMPORT_LINK_TOKEN_AUDIENCE]);

    let data = decode::<ProviderLinkTokenClaims>(token, &key, &validation)
        .map_err(|e| AppError::auth_invalid(format!("Invalid import link-token: {e}")))?;
    if data.claims.scope != import_scope(source) {
        return Err(AppError::auth_invalid(format!(
            "Import link-token scope '{}' does not match '{}'",
            data.claims.scope,
            import_scope(source)
        )));
    }
    Ok(data.claims)
}

/// Extract a Bearer link-token from an `Authorization` header value, if present
#[must_use]
pub fn extract_bearer_link_token(authorization_header: Option<&str>) -> Option<&str> {
    authorization_header.and_then(|h| h.strip_prefix("Bearer "))
}

/// Max link-token TTL in seconds — used as the nonce store's retention window.
const MAX_LINK_TOKEN_LIFETIME_SECS: u64 = (PROVIDER_LINK_TOKEN_TTL_MINUTES as u64) * 60;

/// Cache key provider prefix for nonce entries
const NONCE_PROVIDER_KEY: &str = "_nonce";

/// Cache-backed one-time-use store for link-token `jti` values.
///
/// Records a `jti` the first time [`NonceStore::burn`] is called for it, and
/// returns an error on subsequent calls. Entries auto-expire via the cache TTL
/// after [`MAX_LINK_TOKEN_LIFETIME_SECS`].
///
/// Backed by any [`CacheProvider`] implementation so nonce burns are visible
/// across all Cloud Run replicas when Redis is configured.
#[derive(Debug)]
pub struct NonceStore<C: CacheProvider> {
    cache: Arc<C>,
}

impl<C: CacheProvider> NonceStore<C> {
    /// Create a new nonce store backed by the given cache.
    #[must_use]
    pub fn new(cache: Arc<C>) -> Self {
        Self { cache }
    }

    /// Mark a `jti` as used. Returns `Ok(())` if it was unused, or an error if
    /// it was already recorded (within its TTL window).
    ///
    /// Uses `exists` + `set` on the cache. The TOCTOU window between these two
    /// calls is negligible for this use case (each jti is unique per mint, and
    /// burn happens on user click — simultaneous clicks on the same link across
    /// two replicas within milliseconds is practically impossible).
    ///
    /// # Errors
    ///
    /// Returns [`AppError::auth_invalid`] if the jti has already been burned.
    pub async fn burn(&self, jti: &str) -> AppResult<()> {
        let key = Self::cache_key(jti);
        let ttl = StdDuration::from_secs(MAX_LINK_TOKEN_LIFETIME_SECS);

        if self.cache.exists(&key).await? {
            return Err(AppError::auth_invalid(
                "Link-token has already been used — request a fresh link",
            ));
        }

        // Store a sentinel value with TTL so it auto-expires when the link-token would.
        self.cache.set(&key, &true, ttl).await?;
        Ok(())
    }

    fn cache_key(jti: &str) -> CacheKey {
        CacheKey::new(
            TenantId::nil(),
            Uuid::nil(),
            NONCE_PROVIDER_KEY.to_owned(),
            CacheResource::Custom(format!("jti:{jti}")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "import-link-token-test-secret";
    const SOURCE: &str = "strava_archive";

    fn connect_claims() -> ProviderLinkTokenClaims {
        let token = mint_connect_link_token(Uuid::new_v4(), Uuid::new_v4(), "slack", None, SECRET)
            .expect("mint connect token");
        verify_link_token(&token, SECRET, CONNECT_PROVIDER).expect("verify connect token")
    }

    #[test]
    fn an_import_token_names_the_connect_tokens_athlete_and_one_source() {
        let connect = connect_claims();
        let token = mint_import_link_token(&connect, SOURCE, SECRET).expect("mint");
        let claims = verify_import_link_token(&token, SECRET, SOURCE).expect("verify");

        assert_eq!(
            claims.sub, connect.sub,
            "the import token names the connect token's user"
        );
        assert_eq!(
            claims.tid, connect.tid,
            "the import token names the connect token's tenant"
        );
        assert_eq!(
            claims.scope,
            import_scope(SOURCE),
            "the import token is scoped to its source"
        );
        assert_ne!(
            claims.jti, connect.jti,
            "the import token is a token of its own"
        );
        assert!(
            claims.exp <= connect.exp,
            "never outlives the connect token"
        );
    }

    #[test]
    fn an_import_token_never_outlives_its_connect_token() {
        let mut connect = connect_claims();
        let soon = Utc::now().timestamp() + 120;
        connect.exp = soon;
        let token = mint_import_link_token(&connect, SOURCE, SECRET).expect("mint");
        let claims = verify_import_link_token(&token, SECRET, SOURCE).expect("verify");
        assert_eq!(
            claims.exp, soon,
            "the import token expires with its connect token"
        );
    }

    #[test]
    fn an_import_token_reaches_no_provider_or_connect_endpoint() {
        let token = mint_import_link_token(&connect_claims(), SOURCE, SECRET).expect("mint");
        for provider in [CONNECT_PROVIDER, "sciotte", "strava", SOURCE] {
            assert!(
                verify_link_token(&token, SECRET, provider).is_err(),
                "an import token passed verify_link_token for {provider}"
            );
        }
    }

    #[test]
    fn only_an_import_token_for_the_same_source_passes() {
        let connect = connect_claims();
        let connect_token =
            mint_connect_link_token(Uuid::new_v4(), Uuid::new_v4(), "slack", None, SECRET)
                .expect("mint connect");
        let provider_token = mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id: Uuid::new_v4(),
                tenant_id: Uuid::new_v4(),
                provider: "sciotte",
                target: "strava",
                channel: "slack",
                channel_thread: None,
            },
            SECRET,
        )
        .expect("mint provider");
        let other_source =
            mint_import_link_token(&connect, "garmin_archive", SECRET).expect("mint other");
        let forged = mint_import_link_token(&connect, SOURCE, "another-secret").expect("mint");

        for (what, token) in [
            ("connect", connect_token),
            ("provider", provider_token),
            ("other source", other_source),
            ("other secret", forged),
        ] {
            assert!(
                verify_import_link_token(&token, SECRET, SOURCE).is_err(),
                "a {what} token passed as an import token"
            );
        }
    }

    #[test]
    fn only_a_connect_token_is_narrowed_into_an_import_token() {
        let token = mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id: Uuid::new_v4(),
                tenant_id: Uuid::new_v4(),
                provider: "sciotte",
                target: "strava",
                channel: "slack",
                channel_thread: None,
            },
            SECRET,
        )
        .expect("mint provider");
        let claims = verify_link_token(&token, SECRET, "sciotte").expect("verify");
        assert!(
            mint_import_link_token(&claims, SOURCE, SECRET).is_err(),
            "a provider token was narrowed into an import token"
        );
    }
}
