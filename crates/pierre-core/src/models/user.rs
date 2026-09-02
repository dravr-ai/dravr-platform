// ABOUTME: User models for multi-tenant authentication system
// ABOUTME: User, UserTier, UserStatus, and UserPhysiologicalProfile definitions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::profiles::FitnessLevel;
use crate::constants::tiers;
use crate::errors::AppError;
use crate::intelligence::algorithms::maxhr::MaxHrAlgorithm;
use crate::permissions::UserRole;

use super::zones::{HrZoneSet, PowerZoneSet};
use super::{EncryptedToken, SportType};

/// Type-safe wrapper for user identifiers.
///
/// Provides compile-time distinction between user IDs and other UUIDs and
/// bridges the `SQLite` (TEXT) vs `PostgreSQL` (UUID) column-type split that has
/// historically caused `r.get("user_id")` to panic when a row mapper expected
/// `String` but the column was native UUID (or vice versa). The sqlx
/// `Type`/`Encode`/`Decode` impls below encode as hyphenated TEXT for `SQLite`
/// and as native UUID for `PostgreSQL`, so callers never see the backend split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    /// Create a new random `UserId`.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a `UserId` from a UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID value.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Create a nil (all zeros) `UserId`.
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Check if this is a nil `UserId`.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for UserId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<UserId> for Uuid {
    fn from(user_id: UserId) -> Self {
        user_id.0
    }
}

impl Display for UserId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.0)
    }
}

impl FromStr for UserId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

impl AsRef<Uuid> for UserId {
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

// SQLite stores UUIDs as TEXT. Mirror the TenantId bridge so callers can write
// `r.get::<UserId, _>("user_id")` against a TEXT column without panicking.
#[cfg(feature = "sqlx-sqlite")]
mod sqlite_user_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::sqlite::{Sqlite, SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::UserId;

    impl Type<Sqlite> for UserId {
        fn type_info() -> SqliteTypeInfo {
            <String as Type<Sqlite>>::type_info()
        }
    }

    impl<'q> Encode<'q, Sqlite> for UserId {
        fn encode_by_ref(
            &self,
            buf: &mut Vec<SqliteArgumentValue<'q>>,
        ) -> Result<IsNull, BoxDynError> {
            let text = self.0.to_string();
            <String as Encode<'q, Sqlite>>::encode(text, buf)
        }
    }

    impl<'r> Decode<'r, Sqlite> for UserId {
        fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
            let text = <String as Decode<'r, Sqlite>>::decode(value)?;
            let uuid = Uuid::parse_str(&text)?;
            Ok(Self(uuid))
        }
    }
}

// PostgreSQL stores UUIDs natively. Decode reads the binary UUID directly so
// `r.get::<UserId, _>("user_id")` does NOT need a `::text` cast in the SQL.
#[cfg(feature = "sqlx-postgres")]
mod postgres_user_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef, Postgres};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::UserId;

    impl Type<Postgres> for UserId {
        fn type_info() -> PgTypeInfo {
            <Uuid as Type<Postgres>>::type_info()
        }
    }

    impl<'q> Encode<'q, Postgres> for UserId {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            <Uuid as Encode<'q, Postgres>>::encode_by_ref(&self.0, buf)
        }
    }

    impl<'r> Decode<'r, Postgres> for UserId {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            let uuid = <Uuid as Decode<'r, Postgres>>::decode(value)?;
            Ok(Self(uuid))
        }
    }
}

/// User tier for rate limiting - same as `API` key tiers for consistency
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserTier {
    /// Free tier with basic limits
    Starter,
    /// Professional tier with higher limits
    Professional,
    /// Enterprise tier with unlimited access
    Enterprise,
}

impl Display for UserTier {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Starter => write!(f, "Starter"),
            Self::Professional => write!(f, "Professional"),
            Self::Enterprise => write!(f, "Enterprise"),
        }
    }
}

impl UserTier {
    /// Get monthly request limit for this tier
    #[must_use]
    pub const fn monthly_limit(&self) -> Option<u32> {
        match self {
            Self::Starter => Some(10_000),
            Self::Professional => Some(100_000),
            Self::Enterprise => None, // Unlimited
        }
    }

    /// Get display name for this tier
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Starter => "Starter",
            Self::Professional => "Professional",
            Self::Enterprise => "Enterprise",
        }
    }

    /// Convert to string for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Starter => tiers::STARTER,
            Self::Professional => tiers::PROFESSIONAL,
            Self::Enterprise => tiers::ENTERPRISE,
        }
    }
}

impl FromStr for UserTier {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            tiers::STARTER => Ok(Self::Starter),
            tiers::PROFESSIONAL => Ok(Self::Professional),
            tiers::ENTERPRISE => Ok(Self::Enterprise),
            _ => Err(AppError::invalid_input(format!("Invalid user tier: {s}"))),
        }
    }
}

/// User account status for admin approval workflow
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum UserStatus {
    /// Account pending admin approval (new registrations)
    #[default]
    Pending,
    /// Account approved and active
    Active,
    /// Account suspended by admin
    Suspended,
}

impl UserStatus {
    /// Check if user can login
    #[must_use]
    pub const fn can_login(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Get user-friendly status message
    #[must_use]
    pub const fn to_message(&self) -> &'static str {
        match self {
            Self::Pending => "Your account is pending admin approval",
            Self::Active => "Account is active",
            Self::Suspended => "Your account has been suspended",
        }
    }
}

impl Display for UserStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active => write!(f, "active"),
            Self::Suspended => write!(f, "suspended"),
        }
    }
}

/// Coaching persona controlling the format, cadence, and data-density of
/// coach output for this user.
///
/// Persona is **orthogonal** to coach choice: a user with persona
/// [`CoachingPersona::Casual`] talking to the marathon-coach gets the same
/// coach voice as a [`CoachingPersona::PowerAthlete`] talking to the
/// marathon-coach — only the level of structure, citation, and proactive
/// notification cadence changes.
///
/// - [`Self::Casual`] — friendly prose, no framework citations, weekly digest
///   only, P0-only unsolicited push.
/// - [`Self::Enthusiast`] — mixed prose + selective data, framework citations
///   on disagreement or "why?", P0/P1 push.
/// - [`Self::PowerAthlete`] — Endurance discipline: line-by-line, framework
///   citations on every numeric claim, full P0/P1/P2 push ladder.
/// - [`Self::Coach`] — Power-athlete voice + roster tools (paired with
///   [`User::manages_roster`] for permission gating).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoachingPersona {
    /// Friendly prose, no jargon, weekly digest only, P0-only unsolicited push.
    #[default]
    Casual,
    /// Mixed prose + data, citations on request, P0/P1 push.
    Enthusiast,
    /// Endurance discipline — line-by-line, framework citations everywhere,
    /// full P0/P1/P2 push ladder.
    PowerAthlete,
    /// Power-athlete voice + roster management tools. Paired with
    /// [`User::manages_roster`] for permission gating.
    Coach,
}

impl CoachingPersona {
    /// Canonical string representation used for database storage and prompt
    /// placeholder lookup. Matches the [`serde(rename_all = "snake_case")`]
    /// representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Casual => "casual",
            Self::Enthusiast => "enthusiast",
            Self::PowerAthlete => "power_athlete",
            Self::Coach => "coach",
        }
    }
}

impl Display for CoachingPersona {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(self.as_str())
    }
}

impl FromStr for CoachingPersona {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "casual" => Ok(Self::Casual),
            "enthusiast" => Ok(Self::Enthusiast),
            "power_athlete" => Ok(Self::PowerAthlete),
            "coach" => Ok(Self::Coach),
            _ => Err(AppError::invalid_input(format!(
                "Invalid coaching persona: {s}"
            ))),
        }
    }
}

/// A colour scheme pinned across every surface the athlete uses.
///
/// Stored in `users.theme` as `"light"` / `"dark"`, or SQL NULL when the
/// athlete has pinned nothing and each client follows its operating system.
/// A server-side render has no operating system to follow, so
/// [`Self::resolve`] reads an absent pin as [`Self::Dark`] — the scheme
/// messaging clients overwhelmingly draw media bubbles on.
///
/// This is the single allowed set: `PUT /api/user/theme` validates against it
/// before writing, and the chart minter resolves against it before signing a
/// render token.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    /// Light surfaces, dark ink.
    Light,
    /// Dark surfaces, light ink. The default a render falls back to when the
    /// athlete has pinned nothing.
    #[default]
    Dark,
}

impl ColorScheme {
    /// Canonical string form, matching both the `users.theme` column values
    /// and the JSON the clients send.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Resolve a stored pin into the scheme a server-side render paints in.
    ///
    /// `None` — no pin, or a value the column's CHECK constraint should have
    /// made impossible — resolves to [`Self::Dark`] rather than failing: a
    /// chart is worth more in the wrong scheme than not at all.
    #[must_use]
    pub fn resolve(pinned: Option<&str>) -> Self {
        pinned
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }
}

impl Display for ColorScheme {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(self.as_str())
    }
}

impl FromStr for ColorScheme {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(AppError::invalid_input(format!("Invalid theme: {s}"))),
        }
    }
}

/// Represents a user in the multi-tenant system
///
/// Users are authenticated through `OAuth` providers and have encrypted tokens
/// stored securely for accessing their fitness data.
///
/// Multi-tenant membership is managed via the `tenant_users` junction table,
/// allowing users to belong to multiple tenants (like Slack workspaces).
/// The active tenant context is determined per-session via JWT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier
    pub id: Uuid,
    /// User email address (used for identification)
    pub email: String,
    /// Display name
    pub display_name: Option<String>,
    /// Hashed password for authentication
    pub password_hash: String,
    /// User tier for rate limiting
    pub tier: UserTier,
    /// Encrypted Strava tokens
    pub strava_token: Option<EncryptedToken>,
    /// Encrypted Fitbit tokens
    pub fitbit_token: Option<EncryptedToken>,
    /// When the user account was created
    pub created_at: DateTime<Utc>,
    /// Last time user accessed the system
    pub last_active: DateTime<Utc>,
    /// Whether the user account is active
    pub is_active: bool,
    /// User account status for admin approval workflow
    pub user_status: UserStatus,
    /// Whether this user has admin privileges
    pub is_admin: bool,
    /// User role for permission system (`super_admin`, `admin`, `user`)
    pub role: UserRole,
    /// Admin who approved this user (if approved)
    pub approved_by: Option<Uuid>,
    /// When the user was approved by admin
    pub approved_at: Option<DateTime<Utc>>,
    /// Firebase UID if user authenticated via Firebase (Google, Apple, etc.)
    pub firebase_uid: Option<String>,
    /// Authentication provider: "email", "google.com", "apple.com", "github.com"
    pub auth_provider: String,
    /// Whether the user consented to anonymized analytics tracking
    pub analytics_consent: bool,
    /// When the user last updated their analytics consent preference
    pub analytics_consent_at: Option<DateTime<Utc>>,
    /// BCP-47 short locale code for user-facing messaging output
    /// (`"fr"`, `"en"`, `"es"`, `"de"`, `"pt"`). Defaults to
    /// [`default_locale()`] when not explicitly set. Resolved at messaging
    /// dispatch time via `messaging_channel_links.locale` override →
    /// `users.locale` → `DEFAULT_LOCALE`.
    #[serde(default = "default_locale")]
    pub locale: String,
    /// Coach output persona controlling format / citation density /
    /// notification cadence. Defaults to [`CoachingPersona::Casual`] for
    /// new users; users opt up via the post-auth onboarding prompt or
    /// the Settings UI. Persisted serde-side as `snake_case` (`"casual"`,
    /// `"enthusiast"`, `"power_athlete"`, `"coach"`).
    #[serde(default)]
    pub coaching_persona: CoachingPersona,
    /// Whether this user has access to the Coach-tier roster UI (manage
    /// other athletes). Independent from [`Self::coaching_persona`]:
    /// a user can pick the [`CoachingPersona::Coach`] voice without
    /// `manages_roster=true` (they get the voice but not the tools), and
    /// vice versa (admin-granted roster access without picking the
    /// Coach persona).
    #[serde(default)]
    pub manages_roster: bool,
    /// IANA timezone database name (e.g. `"America/Toronto"`,
    /// `"Europe/Paris"`). Captured client-side via
    /// `Intl.DateTimeFormat().resolvedOptions().timeZone` on each
    /// authenticated request and forwarded via the `X-User-Timezone`
    /// header. Server middleware updates this column only when the
    /// header differs from the stored value, so steady-state cost is
    /// one write per genuine TZ change. `None` means no client has
    /// reported yet — readers fall back to UTC. Used by the chat
    /// prompt-assembly stage to resolve `{{CURRENT_DATE}}` to the
    /// user's local calendar day so the coach interprets "today"
    /// correctly.
    #[serde(default)]
    pub timezone: Option<String>,
    /// Colour-scheme preference pinned across the user's devices:
    /// `"light"` or `"dark"`. `None` means no pin — clients follow the
    /// operating system, and server-side renders that cannot see a client
    /// (messaging chart PNGs) fall back to dark, the scheme messaging
    /// clients overwhelmingly draw media bubbles on. Written via
    /// `PUT /api/user/theme`.
    #[serde(default)]
    pub theme: Option<String>,
}

/// Every locale the platform speaks, in the order the language menus list them.
///
/// The one list: the string catalogue ships every key in exactly these, the
/// messaging-strings registry seeds them, `PUT /api/user/locale` accepts
/// exactly these, `GET /api/i18n/{locale}` serves exactly these, and both
/// clients offer exactly these (`SUPPORTED_LANGUAGES` in `@pierre/i18n`). A
/// sixth locale is added here first. The first entry is the default.
pub const SUPPORTED_LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

/// Default locale (`"fr"`) used when deserializing a pre-locale `User`.
///
/// Backward-compat for in-memory JSON payloads (e.g. tests) that predate the
/// `locale` column. The DB column has `NOT NULL DEFAULT 'fr'` so persisted
/// rows always carry a concrete value.
#[must_use]
pub fn default_locale() -> String {
    SUPPORTED_LOCALES[0].to_owned()
}

impl User {
    /// Create a new user with the given email and password hash
    ///
    /// Tenant membership is managed separately via the `tenant_users` table.
    #[must_use]
    pub fn new(email: String, password_hash: String, display_name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            email,
            display_name,
            password_hash,
            tier: UserTier::Starter, // Default to starter tier
            strava_token: None,
            fitbit_token: None,
            created_at: now,
            last_active: now,
            is_active: true,
            user_status: UserStatus::Active, // Callers set Pending when needed (register, Firebase)
            is_admin: false,                 // Regular users are not admins by default
            role: UserRole::User,            // Default to regular user
            approved_by: None,
            approved_at: None,
            firebase_uid: None, // No Firebase UID for email/password users
            auth_provider: "email".to_owned(), // Default to email provider
            analytics_consent: false, // Opt-out by default
            analytics_consent_at: None,
            locale: default_locale(),
            coaching_persona: CoachingPersona::default(),
            manages_roster: false,
            timezone: None,
            theme: None,
        }
    }

    /// Check if user has valid Strava token
    #[must_use]
    pub fn has_strava_access(&self) -> bool {
        self.strava_token
            .as_ref()
            .is_some_and(|token| token.expires_at > Utc::now())
    }

    /// Check if user has valid Fitbit token
    #[must_use]
    pub fn has_fitbit_access(&self) -> bool {
        self.fitbit_token
            .as_ref()
            .is_some_and(|token| token.expires_at > Utc::now())
    }

    /// Get list of available providers for this user
    #[must_use]
    pub fn available_providers(&self) -> Vec<String> {
        let mut providers = Vec::with_capacity(2); // Typically Strava and Fitbit
        if self.has_strava_access() {
            providers.push("strava".into());
        }
        if self.has_fitbit_access() {
            providers.push("fitbit".into());
        }
        providers
    }

    /// Update last active timestamp
    pub fn update_last_active(&mut self) {
        self.last_active = Utc::now();
    }
}

/// User physiological profile for personalized analysis
///
/// Contains physiological data used for calculating personalized heart rate zones,
/// pace zones, and other performance thresholds based on individual fitness metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPhysiologicalProfile {
    /// User `ID` this profile belongs to
    pub user_id: Uuid,
    /// VO2 max in ml/kg/min (if measured or estimated)
    pub vo2_max: Option<f64>,
    /// Resting heart rate in bpm
    pub resting_hr: Option<u16>,
    /// Maximum heart rate in bpm
    pub max_hr: Option<u16>,
    /// Lactate threshold as percentage of VO2 max (0.65-0.95)
    pub lactate_threshold_percentage: Option<f64>,
    /// Age in years
    pub age: Option<u16>,
    /// Weight in kg
    pub weight: Option<f64>,
    /// Overall fitness level
    pub fitness_level: FitnessLevel,
    /// Primary sport for specialized calculations
    pub primary_sport: SportType,
    /// Years of training experience
    pub training_experience_years: Option<u8>,
    /// Functional Threshold Power in watts (cycling / running power).
    ///
    /// Drives Endurance power-zone derivation, intensity-factor
    /// computation, and the polarized-distribution analytics in
    /// `latest.json`. `None` until the athlete supplies it (manually or
    /// via a 20-min FTP test).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_watts: Option<u32>,
    /// Threshold pace in seconds per kilometre.
    ///
    /// Used by the running-side equivalent of FTP to derive Endurance
    /// pace zones and intensity factor for runs without a power meter.
    /// `None` until the athlete supplies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_pace_sec_per_km: Option<f64>,
    /// Heart-rate zone boundaries.
    ///
    /// Per-user definition stored alongside the profile. Distinct from
    /// the per-activity `HeartRateZone` (which carries time-in-zone for
    /// a single activity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr_zones: Option<HrZoneSet>,
    /// Power zone boundaries (cycling / running power).
    ///
    /// Same shape as [`Self::hr_zones`] but for watts. `None` for
    /// athletes without a power meter or saved FTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_zones: Option<PowerZoneSet>,
}

impl UserPhysiologicalProfile {
    /// Create a new physiological profile
    #[must_use]
    pub const fn new(user_id: Uuid, primary_sport: SportType) -> Self {
        Self {
            user_id,
            vo2_max: None,
            resting_hr: None,
            max_hr: None,
            lactate_threshold_percentage: None,
            age: None,
            weight: None,
            fitness_level: FitnessLevel::Recreational,
            primary_sport,
            training_experience_years: None,
            ftp_watts: None,
            threshold_pace_sec_per_km: None,
            hr_zones: None,
            power_zones: None,
        }
    }

    /// Estimate max heart rate from age if not provided using Tanaka formula
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // Safe: HR is constrained to 0-220 range
    #[allow(clippy::cast_sign_loss)] // Safe: HR is always positive from algorithm
    pub fn estimated_max_hr(&self) -> Option<u16> {
        self.max_hr.or_else(|| {
            self.age.map(|age| {
                // Use Tanaka formula via enum (gold standard: 208 - 0.7xage)
                MaxHrAlgorithm::Tanaka
                    .estimate(u32::from(age), None)
                    .ok()
                    .map_or_else(|| 220_u16.saturating_sub(age), |hr| hr.round() as u16)
            })
        })
    }

    /// Check if profile has sufficient data for VO2 max calculations
    #[must_use]
    pub const fn has_vo2_max_data(&self) -> bool {
        self.vo2_max.is_some()
            && self.resting_hr.is_some()
            && (self.max_hr.is_some() || self.age.is_some())
    }

    /// Get fitness level from VO2 max if available
    #[must_use]
    pub fn fitness_level_from_vo2_max(&self) -> FitnessLevel {
        self.vo2_max.map_or(self.fitness_level, |vo2_max| {
            FitnessLevel::from_vo2_max(
                vo2_max, self.age, None, // Gender not stored in this profile
            )
        })
    }
}

/// A standing per-email pre-approval recorded by an operator before the person
/// has an account.
///
/// The registration approval decision consults this allow-list so an allowed
/// address lands [`UserStatus::Active`] without the pending queue, and
/// `allowed_by` feeds the new account's `approved_by` for audit attribution.
/// Managed by `pierre-cli user allow / disallow / list-allowed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreApprovedEmail {
    /// The allowed address, stored lowercase.
    pub email: String,
    /// The operator who recorded the allow, when known (`None` pre-bootstrap).
    pub allowed_by: Option<Uuid>,
    /// Operator note (cohort, reason).
    pub note: Option<String>,
    /// When the allow was recorded.
    pub created_at: DateTime<Utc>,
}
