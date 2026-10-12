// ABOUTME: Constants module with domain-separated organization
// ABOUTME: Pure data constants organized by domain for the Pierre fitness platform
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Constants module
//!
//! This module organizes application constants by domain for better maintainability.
//! Constants are grouped into logical domains rather than being in a single large file.

// Domain-specific modules

/// Cache-related constants (TTL, sizes, etc.)
pub mod cache;
/// Error codes and error-related constants
pub mod errors;
/// OAuth provider constants and configuration
pub mod oauth;
/// Protocol-specific constants for MCP
pub mod protocol;
/// Multi-protocol constants (A2A, MCP, etc.)
pub mod protocols;
/// Tool identifiers and tool-related constants
pub mod tools;
/// Unit conversion and measurement constants
pub mod units;

// Re-export commonly used items for easier access
pub use errors::*;
pub use oauth::*;
/// Tool-related constants re-export
pub use tools::*;
// Note: protocol and protocols are kept as modules to avoid conflicts

/// OAuth provider constants
pub mod oauth_providers {
    /// Re-export all OAuth constants
    pub use super::oauth::*;
}

// Remaining constants organized by domain

/// Network ports
pub mod ports {
    /// Default HTTP port
    pub const DEFAULT_HTTP_PORT: u16 = 8081;
}

/// API routes
pub mod routes {
    /// Health route
    pub const HEALTH: &str = "/health";
    /// Activities route
    pub const ACTIVITIES: &str = "/activities";
    /// Stats route
    pub const STATS: &str = "/stats";
    /// Connect route
    pub const CONNECT: &str = "/connect";
}

/// Default limits
pub mod limits {
    /// Default activities fetch limit.
    ///
    /// Sized for the group-coaching snapshot pipeline: CTL is a 42-day EMA
    /// and the `recent_activities` roster block needs the last 7 days of
    /// activity for every consenting peer. A limit of 20 starved both
    /// (athletes with 1+ activity/day only had ~20 days of training-load
    /// history feeding the EMA, and the newest activities were dropped
    /// when Strava returned ASC under `after`-only). 60 gives ~2 months
    /// of history at one activity/day — well past the 42-day CTL window —
    /// without exceeding Strava's per-page limit (100).
    pub const DEFAULT_ACTIVITIES_LIMIT: usize = 60;
    /// Maximum activities that can be fetched in one request
    pub const MAX_ACTIVITIES_FETCH: usize = 100;
    /// Minutes per hour
    pub const MINUTES_PER_HOUR: u64 = 60;
    /// Seconds per minute
    pub const SECONDS_PER_MINUTE: u64 = 60;
    /// Maximum timeframe days
    pub const MAX_TIMEFRAME_DAYS: u32 = 365;
    /// Activity capacity hint
    pub const ACTIVITY_CAPACITY_HINT: usize = 50;
    /// Meters per kilometer
    pub const METERS_PER_KILOMETER: f64 = 1000.0;
    /// Percentage multiplier
    pub const PERCENTAGE_MULTIPLIER: f64 = 100.0;
    /// Default session hours for JWT tokens
    pub const DEFAULT_SESSION_HOURS: i64 = 24;
    /// User session JWT expiry hours (24 hours for logged-in users)
    pub const USER_SESSION_EXPIRY_HOURS: i64 = 24;
    /// OAuth access token expiry hours (1 hour per RFC 8252 Security Best Practices)
    pub const OAUTH_ACCESS_TOKEN_EXPIRY_HOURS: i64 = 1;
    /// Maximum request size in bytes
    pub const MAX_REQUEST_SIZE: usize = 1_048_576; // 1MB
    /// Maximum response size in bytes
    pub const MAX_RESPONSE_SIZE: usize = 10_485_760; // 10MB
    /// Default backup interval in seconds
    pub const DEFAULT_BACKUP_INTERVAL_SECS: u64 = 86400; // 24 hours
    /// Default backup retention count
    pub const DEFAULT_BACKUP_RETENTION_COUNT: u32 = 7;
}

/// Timeout configurations
pub mod timeouts {
    /// Default HTTP client request timeout in seconds
    pub const HTTP_CLIENT_TIMEOUT_SECS: u64 = 30;
    /// Default HTTP client connect timeout in seconds
    pub const HTTP_CLIENT_CONNECT_TIMEOUT_SECS: u64 = 10;
    /// OAuth client request timeout in seconds
    pub const OAUTH_CLIENT_TIMEOUT_SECS: u64 = 15;
    /// OAuth client connect timeout in seconds
    pub const OAUTH_CLIENT_CONNECT_TIMEOUT_SECS: u64 = 5;
    /// API client request timeout in seconds
    pub const API_CLIENT_TIMEOUT_SECS: u64 = 60;
    /// API client connect timeout in seconds
    pub const API_CLIENT_CONNECT_TIMEOUT_SECS: u64 = 10;
    /// Health check client timeout in seconds
    pub const HEALTH_CHECK_TIMEOUT_SECS: u64 = 5;
    /// SSE cleanup task interval in seconds
    pub const SSE_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes
    /// SSE connection timeout in seconds (inactive connections removed after this duration)
    pub const SSE_CONNECTION_TIMEOUT_SECS: u64 = 600; // 10 minutes
    /// OAuth session cookie Max-Age in seconds (matches JWT expiration)
    pub const SESSION_COOKIE_MAX_AGE_SECS: u64 = 86400; // 24 hours
}

/// Security configurations
pub mod security {
    /// CORS allowed origins
    pub const CORS_ALLOWED_ORIGINS: &str = "*";
}

/// OAuth configuration constants
pub mod oauth_config {
    /// OAuth authorization URL expiration time in minutes
    /// Authorization URLs remain valid for 10 minutes
    pub const AUTHORIZATION_EXPIRES_MINUTES: u32 = 10;
}

/// API key system configuration
pub mod system_config {
    /// Trial tier monthly limit
    pub const TRIAL_MONTHLY_LIMIT: u32 = 1_000;
    /// Starter tier monthly limit
    pub const STARTER_MONTHLY_LIMIT: u32 = 10_000;
    /// Professional tier monthly limit
    pub const PROFESSIONAL_MONTHLY_LIMIT: u32 = 100_000;
    /// Rate limit window in seconds (30 days)
    pub const RATE_LIMIT_WINDOW_SECONDS: u32 = 30 * 24 * 60 * 60;
    /// Default trial period in days
    pub const TRIAL_PERIOD_DAYS: i64 = 14;
}

/// Credential prefixes: the tag that names a raw credential's kind
pub mod key_prefixes {
    /// Live API key prefix
    pub const LIVE: &str = "pk_live_";
    /// Trial API key prefix
    pub const TRIAL: &str = "pk_trial_";
    /// Personal MCP token prefix: the credential an athlete pastes into an
    /// MCP client (Claude Desktop, Cursor), accepted by `/mcp` only
    pub const USER_MCP_TOKEN: &str = "pmcp_";
}

/// API key tiers
pub mod tiers {
    /// Trial tier
    pub const TRIAL: &str = "trial";
    /// Starter tier
    pub const STARTER: &str = "starter";
    /// Professional tier
    pub const PROFESSIONAL: &str = "professional";
    /// Enterprise tier
    pub const ENTERPRISE: &str = "enterprise";
}

/// Default values
pub mod defaults {
    /// Default page size for paginated responses
    pub const PAGE_SIZE: usize = 20;
    /// Default goal timeframe in days
    pub const DEFAULT_GOAL_TIMEFRAME_DAYS: u32 = 30;
    /// Default backup directory
    pub const DEFAULT_BACKUP_DIR: &str = "./backups";
}

/// Tool-loop iteration budget bounds.
///
/// One band shared by every writer and reader of the per-turn tool-call
/// budget: the `agents.max_tool_iterations` column, the
/// `tool_execution.max_iterations` admin configuration parameter, and the
/// chat pipeline that resolves the two into the budget for a turn. Held as
/// `u16` so each consumer widens losslessly — `usize` for the loop counter,
/// `i32` for the agent column, `i64` for the JSON config value.
pub mod tool_execution {
    /// Budget used when neither the agent nor the admin configuration sets one.
    pub const DEFAULT_MAX_TOOL_ITERATIONS: u16 = 10;

    /// Smallest accepted budget — one pass still lets the model call a tool
    /// and answer from its result.
    pub const MIN_MAX_TOOL_ITERATIONS: u16 = 1;

    /// Largest accepted budget. Caps how long one turn can spend fanning out
    /// tool calls before the loop must answer.
    pub const MAX_MAX_TOOL_ITERATIONS: u16 = 50;
}

/// Configuration system constants
pub mod configuration_system {
    /// Number of available configuration parameters in catalog
    ///
    /// Total count of configuration options exposed via MCP configuration tools
    /// Used for catalog size reporting and validation
    pub const AVAILABLE_PARAMETERS_COUNT: usize = 25;
}

/// Database configuration
pub mod database {
    /// Connection timeout in seconds
    pub const CONNECTION_TIMEOUT_SECS: u64 = 30;
    /// Default number of connection retries on startup
    pub const CONNECTION_RETRIES: u32 = 5;
    /// Initial retry delay in milliseconds for exponential backoff
    pub const INITIAL_RETRY_DELAY_MS: u64 = 500;
    /// Maximum retry delay in milliseconds
    pub const MAX_RETRY_DELAY_MS: u64 = 30_000;
}

/// Redis connection configuration
pub mod redis {
    /// Redis connection timeout in seconds
    pub const CONNECTION_TIMEOUT_SECS: u64 = 10;
    /// Redis response timeout in seconds
    pub const RESPONSE_TIMEOUT_SECS: u64 = 5;
    /// Number of reconnection retries
    pub const RECONNECTION_RETRIES: usize = 5;
    /// Exponential backoff base for retry delays
    pub const RETRY_EXPONENT_BASE: u64 = 2;
    /// Maximum retry delay in milliseconds
    pub const MAX_RETRY_DELAY_MS: u64 = 30_000;
    /// Initial connection retry count
    pub const INITIAL_CONNECTION_RETRIES: u32 = 3;
}

/// Status codes and messages
pub mod status {
    /// HTTP status codes
    pub mod http {
        /// OK
        pub const OK: u16 = 200;
        /// Created
        pub const CREATED: u16 = 201;
        /// Bad Request
        pub const BAD_REQUEST: u16 = 400;
        /// Unauthorized
        pub const UNAUTHORIZED: u16 = 401;
        /// Forbidden
        pub const FORBIDDEN: u16 = 403;
        /// Not Found
        pub const NOT_FOUND: u16 = 404;
        /// Internal Server Error
        pub const INTERNAL_SERVER_ERROR: u16 = 500;
    }

    /// MCP status messages
    pub mod mcp {
        /// Connected
        pub const CONNECTED: &str = "connected";
        /// Error
        pub const ERROR: &str = "error";
    }
}

/// Field names for JSON
pub mod json_fields {
    /// User ID field
    pub const USER_ID: &str = "user_id";
    /// Provider field
    pub const PROVIDER: &str = "provider";
    /// Activities field
    pub const ACTIVITIES: &str = "activities";
    /// Limit field
    pub const LIMIT: &str = "limit";
    /// Offset field
    pub const OFFSET: &str = "offset";
    /// Before timestamp field (Unix epoch seconds) - get activities before this time
    pub const BEFORE: &str = "before";
    /// After timestamp field (Unix epoch seconds) - get activities after this time
    pub const AFTER: &str = "after";
    /// Mode field for response detail level (summary vs detailed)
    pub const MODE: &str = "mode";
}

/// Service names
pub mod service_names {
    /// MCP service
    pub const MCP: &str = "mcp";
    /// Audience of the authorization server's own sign-in (the
    /// `/oauth2/login` cookie): accepted by `/oauth2/authorize` and its
    /// consent form only, never by chat, REST, MCP or A2A (carnet#787)
    pub const OAUTH2_AUTHORIZE: &str = "oauth2-authorize";
    /// Auth service
    pub const AUTH: &str = "auth";
    /// OAuth service
    pub const OAUTH: &str = "oauth";
    /// Activity service
    pub const ACTIVITY: &str = "activity";
    /// Health service
    pub const HEALTH: &str = "health";
    /// Pierre MCP Server
    pub const PIERRE_MCP_SERVER: &str = "pierre-mcp-server";
    /// Admin API service
    pub const ADMIN_API: &str = "admin_api";
}

/// Project metadata constants
pub mod project {
    /// Project name
    pub const NAME: &str = "Pierre MCP Server";
    /// Project version (synced from Cargo.toml at compile time)
    pub const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// Project repository URL (synced from Cargo.toml at compile time)
    pub const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");

    /// Builds the HTTP User-Agent string for external API requests
    ///
    /// Format: `Pierre MCP Server/{version} ({repository_url})`
    #[must_use]
    pub fn user_agent() -> String {
        format!("{NAME}/{VERSION} ({REPOSITORY_URL})")
    }
}

/// Time constants
pub mod time_constants {
    /// Seconds per hour
    pub const SECONDS_PER_HOUR: u32 = 3600;
    /// Seconds per minute
    pub const SECONDS_PER_MINUTE: u64 = 60;
    /// Seconds per hour as f64
    pub const SECONDS_PER_HOUR_F64: f64 = 3600.0;
    /// Seconds per day
    pub const SECONDS_PER_DAY: u32 = 86_400;
    /// Seconds per week
    pub const SECONDS_PER_WEEK: u32 = 604_800;
    /// Seconds per month (30 days)
    pub const SECONDS_PER_MONTH: u32 = 2_592_000;
    /// Minutes per hour
    pub const MINUTES_PER_HOUR: u64 = 60;
    /// Hours per day
    pub const HOURS_PER_DAY: u64 = 24;
    /// Days per week
    pub const DAYS_PER_WEEK: u32 = 7;
    /// Days per month (30-day approximation for calculations)
    pub const DAYS_PER_MONTH: u32 = 30;
    /// Days per quarter (90-day approximation for calculations)
    pub const DAYS_PER_QUARTER: u32 = 90;
    /// Days per year (standard calendar year)
    pub const DAYS_PER_YEAR: u32 = 365;
}

/// Error messages
pub mod error_messages {
    /// Invalid credentials message
    pub const INVALID_CREDENTIALS: &str = "Invalid credentials provided";
    /// Unauthorized access message
    pub const UNAUTHORIZED: &str = "Unauthorized access";
    /// Invalid email format message
    pub const INVALID_EMAIL_FORMAT: &str = "Invalid email format";
    /// Password too weak message
    pub const PASSWORD_TOO_WEAK: &str = "Password must be at least 8 characters";
    /// User already exists message
    pub const USER_ALREADY_EXISTS: &str = "User already exists";
    /// Rate limit exceeded
    pub const RATE_LIMIT_EXCEEDED: &str = "Rate limit exceeded";
}

/// Rate limiting constants
pub mod rate_limits {
    /// Strava's read budget per 15-minute window, per app: the standard read
    /// limit the Dravr app holds. `STRAVA_RATE_LIMIT_15MIN` overrides it.
    pub const STRAVA_RATE_LIMIT_15MIN: u32 = 100;
    /// Strava's read budget per day, per app: the standard read limit the
    /// Dravr app holds. `STRAVA_RATE_LIMIT_DAILY` overrides it.
    pub const STRAVA_RATE_LIMIT_DAILY: u32 = 1000;
    /// Garmin default daily rate limit
    pub const GARMIN_DEFAULT_DAILY_RATE_LIMIT: u32 = 1000;
    /// WHOOP default daily rate limit
    pub const WHOOP_DEFAULT_DAILY_RATE_LIMIT: u32 = 1000;
    /// Terra default daily rate limit
    pub const TERRA_DEFAULT_DAILY_RATE_LIMIT: u32 = 1000;
    /// Wahoo's production budget per 5-minute window, per app (Wahoo Cloud
    /// API, "Rate Limiting"; a sandbox app gets 25).
    pub const WAHOO_RATE_LIMIT_5MIN: u32 = 200;
    /// Wahoo's production budget per hour, per app (sandbox: 100).
    pub const WAHOO_RATE_LIMIT_HOURLY: u32 = 1000;
    /// Wahoo's production budget per day, per app (sandbox: 250).
    pub const WAHOO_RATE_LIMIT_DAILY: u32 = 5000;
    /// The requests a day each athlete who granted an Intervals.icu OAuth app
    /// adds to that app's daily pool.
    ///
    /// Intervals.icu's default daily limit for an OAuth app is "100/user per
    /// day up to 500 users (max 50000 requests), with a minimum of 5000"
    /// (forum.intervals.icu/t/609, post 1, as rewritten 2026-06-24): one
    /// pool for the whole app, 100 for each athlete who granted it, never
    /// below [`INTERVALS_ICU_OAUTH_DAILY_MIN`] nor above
    /// [`INTERVALS_ICU_OAUTH_DAILY_MAX`], which any of its athletes may spend.
    pub const INTERVALS_ICU_OAUTH_DAILY_PER_ATHLETE: u32 = 100;
    /// The smallest daily pool of an Intervals.icu OAuth app, however few
    /// athletes granted it: the "minimum of 5000" of that same limit.
    pub const INTERVALS_ICU_OAUTH_DAILY_MIN: u32 = 5_000;
    /// The largest daily pool of an Intervals.icu OAuth app, however many
    /// athletes granted it: the "max 50000 requests" of that same limit. An
    /// app past 500 athletes asks support@intervals.icu for more.
    pub const INTERVALS_ICU_OAUTH_DAILY_MAX: u32 = 50_000;
    /// What an Intervals.icu OAuth app's daily limit is divided by to give
    /// its rolling 15-minute limit.
    ///
    /// "15-minute limit = 1/8 of daily (min 2,500), rolling"
    /// (forum.intervals.icu/t/609, post 1, as the team's 2026-08-21
    /// intervals.icu survey records it), never below
    /// [`INTERVALS_ICU_OAUTH_15MIN_MIN`].
    pub const INTERVALS_ICU_OAUTH_15MIN_DAILY_DIVISOR: u32 = 8;
    /// The smallest rolling 15-minute limit of an Intervals.icu OAuth app:
    /// the "min 2,500" of that same rule.
    pub const INTERVALS_ICU_OAUTH_15MIN_MIN: u32 = 2_500;
    /// Intervals.icu's limit for one personal API key: "2500 requests per
    /// rolling 15 minute window" (forum.intervals.icu/t/609).
    pub const INTERVALS_ICU_API_KEY_15MIN: u32 = 2_500;
    /// Intervals.icu's limit for one personal API key: "5000 requests per
    /// day" (forum.intervals.icu/t/609).
    pub const INTERVALS_ICU_API_KEY_DAILY: u32 = 5_000;
    /// Default burst limit
    pub const DEFAULT_BURST_LIMIT: u32 = 10;
}

/// Provider athlete-seat capacities.
///
/// Distinct from request rate limits: this is the number of distinct athletes
/// an OAuth application may have connected at once, enforced by the upstream
/// provider (Strava) and not exposed via any API response header. The value
/// here is the default the platform assumes; an operator overrides it via the
/// `STRAVA_OAUTH_SEAT_CAP` environment variable after self-upgrading the app's
/// tier in the Strava API settings dashboard.
pub mod provider_seats {
    /// Default athlete capacity for the shared Dravr Strava OAuth app.
    ///
    /// Matches the Strava Standard Tier entry-level cap (10 athletes) that a
    /// developer can self-upgrade to with no review. Overridable via
    /// `STRAVA_OAUTH_SEAT_CAP`.
    pub const STRAVA_OAUTH_SEAT_CAP_DEFAULT: u32 = 10;
}

/// Capture versions: which generation of a provider's capture wrote its history.
///
/// Deep history is immutable once backfilled — the historical gate serves a
/// covered window from the durable cache without calling the provider again.
/// That is only safe while the rows it vouches for are as complete as the
/// capture can make them. A capture fix that starts reading a field the old
/// capture missed leaves every previously backfilled row permanently short of
/// it unless something invalidates the coverage that protects them.
///
/// Bump a provider's version in the same change that ships such a fix. Coverage
/// rows stamped below the current version read as not covered, so the next deep
/// ask re-runs the backfill through the corrected capture and its upserts
/// replace the old rows. Providers absent from this table sit at the baseline.
pub mod provider_capture {
    use super::oauth_providers::SCIOTTE;

    /// The version every provider starts at.
    ///
    /// Coverage rows written before the column existed carry it too.
    pub const BASELINE_CAPTURE_VERSION: u32 = 0;

    /// Strava via sciotte.
    ///
    /// 1: elevation is read by the unit its value carries rather than by the
    /// feed's localized label (dravr-sciotte 0.12.1). Rows the dedicated
    /// service captured before that carry no elevation.
    pub const SCIOTTE_STRAVA_CAPTURE_VERSION: u32 = 1;

    /// The capture version a backfill of `provider` writes today.
    #[must_use]
    pub fn current_capture_version(provider: &str) -> u32 {
        if provider == SCIOTTE {
            SCIOTTE_STRAVA_CAPTURE_VERSION
        } else {
            BASELINE_CAPTURE_VERSION
        }
    }
}

/// API provider limits
pub mod api_provider_limits {
    /// Strava specific limits
    pub mod strava {
        /// Estimated rate-limit block duration (seconds).
        ///
        /// Strava publishes a 15-minute rolling window, so a caller that has
        /// exhausted it is clear again within one window.
        pub const ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS: u64 = 900;
        /// Default activities per page
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 30;
        /// Maximum activities per request
        pub const MAX_ACTIVITIES_PER_REQUEST: usize = 200;
    }

    /// WHOOP API limits
    pub mod whoop {
        /// Estimated rate-limit block duration (seconds).
        pub const ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS: u64 = 60;
        /// Default workouts per page request
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 25;
        /// Maximum workouts per single API request
        pub const MAX_ACTIVITIES_PER_REQUEST: usize = 25;
    }

    /// Wahoo Cloud API limits
    pub mod wahoo {
        /// Estimated rate-limit block duration (seconds): Wahoo's shortest
        /// window is five minutes.
        pub const ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS: u64 = 300;
        /// Default workouts per list request when the caller names no limit
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 30;
        /// Workouts asked for per `GET /v1/workouts` page (Wahoo's default is 30)
        pub const WORKOUTS_PER_PAGE: usize = 50;
    }

    /// COROS API limits
    pub mod coros {
        /// Estimated rate-limit block duration (seconds).
        pub const ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS: u64 = 60;
        /// Default workouts per page request
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 25;
        /// Maximum workouts per single API request
        pub const MAX_ACTIVITIES_PER_REQUEST: usize = 50;
    }

    /// Intervals.icu API limits
    pub mod intervals_icu {
        /// Default activities per page request
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 30;
        /// Maximum activities per single API request
        pub const MAX_ACTIVITIES_PER_REQUEST: usize = 200;
    }

    /// Garmin Connect API limits
    ///
    /// **IMPORTANT**: These limits are based on community observations of unofficial API endpoints.
    /// Garmin does not publicly document rate limits for their unofficial API.
    pub mod garmin {
        /// Default number of activities per page request
        pub const DEFAULT_ACTIVITIES_PER_PAGE: usize = 20;
        /// Maximum activities per single API request
        pub const MAX_ACTIVITIES_PER_REQUEST: usize = 100;
        /// Recommended maximum requests per hour per user
        pub const RECOMMENDED_MAX_REQUESTS_PER_HOUR: usize = 100;
        /// Recommended minimum interval between login attempts (seconds)
        pub const RECOMMENDED_MIN_LOGIN_INTERVAL_SECS: u64 = 300;
        /// Estimated rate limit block duration (seconds)
        pub const ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS: u64 = 3600;
    }
}

/// Time-related constants
pub mod time {
    /// Default token expiry in seconds (1 hour)
    pub const DEFAULT_TOKEN_EXPIRY_SECONDS: i64 = 3600;
    /// How long before its expiry a provider access token is refreshed.
    ///
    /// The one pre-expiry window for every provider token, read through
    /// [`crate::models::refresh_due`]. Garmin asks for a refresh at least
    /// 600 s before expiry (Garmin Connect Developer Program OAuth2.0 PKCE
    /// Specification); ten minutes meets that and is a small share of every
    /// provider's token lifetime (Strava 6 h, Garmin 24 h).
    pub const TOKEN_REFRESH_WINDOW_MINUTES: i64 = 10;
    /// Seconds in a minute
    pub const MINUTE_SECONDS: i64 = 60;
    /// Seconds in an hour
    pub const HOUR_SECONDS: i64 = 3600;
    /// Unix epoch start
    pub const UNIX_EPOCH: &str = "1970-01-01T00:00:00Z";
}

/// Network configuration
pub mod network_config {
    /// TCP keep alive timeout in seconds
    pub const TCP_KEEP_ALIVE_SECS: u64 = 60;
    /// `SO_REUSEADDR`
    pub const SO_REUSEADDR: bool = true;
    /// OAuth code verifier length
    pub const OAUTH_CODE_VERIFIER_LENGTH: usize = 128;
    /// SSE broadcast channel size
    pub const SSE_BROADCAST_CHANNEL_SIZE: usize = 1000;
    /// Maximum concurrent SSE connections per user (`DoS` prevention)
    pub const SSE_MAX_CONNECTIONS_PER_USER: usize = 5;
}

/// Physiology constants
pub mod physiology {
    /// Default resting heart rate
    pub const DEFAULT_RESTING_HR: u16 = 60;
    /// Default maximum heart rate
    pub const DEFAULT_MAX_HR: u16 = 190;
}

/// HTTP status codes
pub mod http_status {
    /// HTTP 200 OK (success range minimum)
    pub const SUCCESS_MIN: u16 = 200;
    /// HTTP 299 (success range maximum)
    pub const SUCCESS_MAX: u16 = 299;
    /// HTTP 400 Bad Request
    pub const BAD_REQUEST: u16 = 400;
    /// HTTP 401 Unauthorized
    pub const UNAUTHORIZED: u16 = 401;
    /// HTTP 403 Forbidden
    pub const FORBIDDEN: u16 = 403;
    /// HTTP 404 Not Found
    pub const NOT_FOUND: u16 = 404;
    /// HTTP 409 Conflict
    pub const CONFLICT: u16 = 409;
    /// HTTP 429 Too Many Requests
    pub const TOO_MANY_REQUESTS: u16 = 429;
    /// HTTP 500 Internal Server Error
    pub const INTERNAL_SERVER_ERROR: u16 = 500;
    /// HTTP 502 Bad Gateway
    pub const BAD_GATEWAY: u16 = 502;
    /// HTTP 503 Service Unavailable
    pub const SERVICE_UNAVAILABLE: u16 = 503;
}

/// System monitoring constants
pub mod system_monitoring {
    /// Memory warning threshold percentage
    pub const MEMORY_WARNING_THRESHOLD: f64 = 80.0;
    /// Disk warning threshold percentage
    pub const DISK_WARNING_THRESHOLD: f64 = 85.0;
}

/// OAuth 2.0 rate limiting configurations
pub mod oauth_rate_limiting {
    /// Authorization endpoint rate limit (requests per minute)
    pub const AUTHORIZE_RPM: u32 = 60;
    /// Token endpoint rate limit (requests per minute)
    pub const TOKEN_RPM: u32 = 30;
    /// Registration endpoint rate limit (requests per minute)
    pub const REGISTER_RPM: u32 = 10;
    /// Password re-confirmations a signed-in account may attempt per window
    /// (`change-password`, account deletion), counted per account: a stolen
    /// session cannot be turned into a password-guessing oracle
    pub const PASSWORD_CONFIRM_RPM: u32 = 5;
    /// Refused password sign-ins (every password sign-in surface) one client
    /// address may make per window before its next attempt is refused
    /// unchecked: credential stuffing from one address stops here
    pub const PASSWORD_LOGIN_RPM: u32 = 20;
    /// Refused password sign-ins naming one account per window, from any
    /// address: guessing one athlete's password from many addresses stops here
    pub const PASSWORD_LOGIN_ACCOUNT_RPM: u32 = 10;
    /// Rate limit window duration in seconds
    pub const WINDOW_SECS: u64 = 60;
}

/// Retention of RFC 7591 dynamic client registrations.
///
/// `POST /oauth2/register` is anonymous, so the rows it writes are bounded by
/// these rather than by who calls it. A registration no user has authorized is
/// "pending": at most [`MAX_PENDING_REGISTRATIONS`] exist at once, and each is
/// deleted [`ABANDONED_AFTER_SECS`] after it was made. An expired registration
/// is deleted [`EXPIRED_GRACE_SECS`] after its `expires_at`.
pub mod oauth2_client_retention {
    /// How long an expired registration is kept before the sweep deletes it: 30 days.
    ///
    /// `validate_client` already refuses it the moment it expires; the grace
    /// keeps the row readable to an operator asking why a client stopped
    /// working, without holding it for another year.
    pub const EXPIRED_GRACE_SECS: u64 = 30 * 24 * 60 * 60;
    /// Age at which a registration no user has authorized is deleted: 24 hours.
    ///
    /// An MCP client registers and opens the consent screen in one motion, so a
    /// registration still unauthorized a day later is one nobody finished.
    pub const ABANDONED_AFTER_SECS: u64 = 24 * 60 * 60;
    /// How often the retention sweep runs: hourly.
    ///
    /// Pending rows live a day, so an hourly pass holds each within an hour of
    /// its deadline.
    pub const SWEEP_INTERVAL_SECS: u64 = 60 * 60;
    /// Registrations no user has authorized that may exist at once: 10,000.
    ///
    /// At a few hundred bytes a row that bounds the anonymous part of the table
    /// to a few megabytes. Holding it full takes about 7 registrations a minute
    /// sustained (10,000 per day-long pending lifetime), within the 10 a minute
    /// the register rate limit already grants a single address.
    pub const MAX_PENDING_REGISTRATIONS: u64 = 10_000;
}

/// Bounds on the values a client hands the `OAuth2` authorization endpoint.
pub mod oauth2_authorization {
    /// Longest `state` accepted, in bytes: 1,024.
    ///
    /// RFC 6749 leaves `state` opaque and unbounded, but the server stores it
    /// as the primary key of `oauth2_states`, and a `PostgreSQL` btree entry
    /// tops out near 2,700 bytes. A client's CSRF token is tens of bytes; a
    /// kilobyte leaves room for one that encodes its own context.
    pub const MAX_STATE_BYTES: usize = 1024;
}

/// Cache configuration constants
pub mod cache_config {
    /// Default cache capacity for LRU cache
    pub const DEFAULT_CAPACITY: usize = 1000;
}

/// MCP transport configuration
pub mod mcp_transport {
    /// Notification broadcast channel size
    pub const NOTIFICATION_CHANNEL_SIZE: usize = 100;
}

/// Sleep analysis and recovery constants
pub mod sleep_recovery {
    /// Number of recent activities to fetch for sleep/recovery analysis
    pub const ACTIVITY_LIMIT: u32 = 42;
    /// Minimum number of days of sleep history required for trend analysis
    pub const TREND_MIN_DAYS: usize = 7;
    /// Sleep trend improving threshold (hours increase over previous period)
    pub const TREND_IMPROVING_THRESHOLD: f64 = 5.0;
    /// Sleep trend declining threshold (hours decrease below previous period)
    pub const TREND_DECLINING_THRESHOLD: f64 = 5.0;
    /// Additional sleep hours recommended when athlete is fatigued (TSB negative)
    pub const FATIGUE_BONUS_HOURS: f64 = 0.5;
    /// ATL (Acute Training Load) threshold indicating high training load
    pub const HIGH_LOAD_ATL_THRESHOLD: f64 = 100.0;
    /// Additional sleep hours recommended during high training load periods
    pub const HIGH_LOAD_BONUS_HOURS: f64 = 0.25;
    /// Buffer time in minutes before target sleep time for wind-down routine
    pub const WIND_DOWN_MINUTES: i64 = 15;
    /// Minutes per day constant for time calculations and day wrapping
    pub const MINUTES_PER_DAY: i64 = 1440;
}

/// Goal management and feasibility constants
pub mod goal_management {
    /// Minimum number of activities required to establish training history
    pub const MIN_ACTIVITIES_FOR_TRAINING_HISTORY: usize = 2;
    /// Activities per week threshold for advanced fitness level classification
    pub const ADVANCED_FITNESS_ACTIVITIES_PER_WEEK: f64 = 5.0;
    /// Minimum training weeks required for advanced fitness level
    pub const ADVANCED_FITNESS_MIN_WEEKS: f64 = 26.0;
    /// Activities per week threshold for intermediate fitness level classification
    pub const INTERMEDIATE_FITNESS_ACTIVITIES_PER_WEEK: f64 = 3.0;
    /// Minimum training weeks required for intermediate fitness level
    pub const INTERMEDIATE_FITNESS_MIN_WEEKS: f64 = 12.0;
    /// Default training time availability per week (hours)
    pub const DEFAULT_TIME_AVAILABILITY_HOURS: f64 = 3.0;
    /// Default preferred activity duration (minutes)
    pub const DEFAULT_PREFERRED_DURATION_MINUTES: u32 = 30;
    /// Average days per month for monthly calculations (365.25/12)
    pub const DAYS_PER_MONTH_AVERAGE: f64 = 30.44;
}
