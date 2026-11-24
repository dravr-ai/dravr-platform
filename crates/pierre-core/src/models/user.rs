// ABOUTME: User models for multi-tenant authentication system
// ABOUTME: User, UserTier, UserStatus, and UserPhysiologicalProfile definitions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::transport::TransportPolicy;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::str::FromStr;

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::profiles::FitnessLevel;
use crate::constants::tiers;
use crate::errors::AppError;
use crate::intelligence::algorithms::maxhr::MaxHrAlgorithm;
use crate::permissions::UserRole;
use dravr_cageux::algorithms::lthr::LthrAlgorithm;
use dravr_cageux::nutrition_calculator::Gender;

use super::zones::{HrZoneSet, PowerZoneSet};
use super::{
    AnchorProvenance, ProfileVdot, ProvenancedValue, SportType, ThresholdProvenance,
    ThresholdStatedOn,
};

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
/// agent output for this user.
///
/// Persona is **orthogonal** to agent choice: a user with persona
/// [`CoachingPersona::Casual`] talking to the marathon-agent gets the same
/// agent voice as a [`CoachingPersona::PowerAthlete`] talking to the
/// marathon-agent — only the level of structure, citation, and proactive
/// notification cadence changes.
///
/// - [`Self::Casual`] — friendly prose, no framework citations, weekly digest
///   only, P0-only unsolicited push.
/// - [`Self::Enthusiast`] — mixed prose + selective data, framework citations
///   on disagreement or "why?", P0/P1 push.
/// - [`Self::PowerAthlete`] — Endurance discipline: line-by-line, framework
///   citations on every numeric claim, full P0/P1/P2 push ladder.
/// - [`Self::Coach`] — Power-athlete voice. The permission to coach a group
///   is separate: [`User::manages_roster`].
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
    /// Power-athlete voice. The permission to coach a group is separate:
    /// [`User::manages_roster`].
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

/// A `manages_roster` grant an admin made, from the admin console or
/// `pierre-cli user set --manages-roster`.
///
/// The `TrainingPeaks` reconciler takes back only a grant a coach connection
/// earned, never one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminRosterGrant {
    /// When the admin granted it.
    pub granted_at: Timestamp,
    /// The admin who granted it; `None` for a service token, which names
    /// no one, and once that admin's account is deleted.
    pub granted_by: Option<Uuid>,
}

/// Represents a user in the multi-tenant system
///
/// Users are authenticated through `OAuth` providers and have encrypted tokens
/// stored securely for accessing their fitness data.
///
/// Multi-tenant membership is managed via the `tenant_users` junction table,
/// allowing users to belong to multiple tenants (like Slack workspaces).
/// The active tenant context is determined per-session via JWT claims.
///
/// A `User` carries its `password_hash`, so it neither serializes nor
/// deserializes: an API answers with a response type that names the fields
/// it returns, and a row is read by the repository. `Debug` is written by
/// hand for the same reason and prints a fixed placeholder for the hash.
#[derive(Clone)]
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
    /// When the user account was created
    pub created_at: Timestamp,
    /// Last time user accessed the system
    pub last_active: Timestamp,
    /// Whether the user account is active
    pub is_active: bool,
    /// User account status for admin approval workflow
    pub user_status: UserStatus,
    /// User role for permission system (`admin`, `user`): the one source of
    /// truth for who is an admin
    pub role: UserRole,
    /// Admin who approved this user (if approved)
    pub approved_by: Option<Uuid>,
    /// When the user was approved by admin
    pub approved_at: Option<Timestamp>,
    /// Firebase UID if user authenticated via Firebase (Google, Apple, etc.)
    pub firebase_uid: Option<String>,
    /// Authentication provider: "email", "google.com", "apple.com", "github.com"
    pub auth_provider: String,
    /// Whether the user consented to anonymized analytics tracking
    pub analytics_consent: bool,
    /// When the user last updated their analytics consent preference
    pub analytics_consent_at: Option<Timestamp>,
    /// BCP-47 short locale code for user-facing messaging output
    /// (`"fr"`, `"en"`, `"es"`, `"de"`, `"pt"`). Defaults to
    /// [`default_locale()`] when not explicitly set. Resolved at messaging
    /// dispatch time via `messaging_channel_links.locale` override →
    /// `users.locale` → `DEFAULT_LOCALE`.
    pub locale: String,
    /// Agent output persona controlling format / citation density /
    /// notification cadence. Defaults to [`CoachingPersona::Casual`] for
    /// new users; users opt up via the post-auth onboarding prompt or
    /// the Settings UI. Persisted serde-side as `snake_case` (`"casual"`,
    /// `"enthusiast"`, `"power_athlete"`, `"coach"`).
    pub coaching_persona: CoachingPersona,
    /// Whether this user may redeem a coach invite, joining a coaching group
    /// as its human coach (`coaching_groups.coach_user_id`). Independent from
    /// [`Self::coaching_persona`]: a user can pick the
    /// [`CoachingPersona::Coach`] voice without `manages_roster=true`, and
    /// hold the permission without picking that voice. Granted automatically
    /// when the user's TrainingPeaks connection reports a coach account whose
    /// TrainingPeaks email is the user's verified email, and revoked when that
    /// connection is disconnected or signs in as an athlete or as a coach
    /// account that is not the user's, unless another TrainingPeaks coach
    /// connection still holds it. An admin grants or revokes it from the
    /// admin console or `pierre-cli user set --manages-roster`, and an
    /// admin grant ([`AdminRosterGrant`]) is never revoked by the
    /// TrainingPeaks reconciler.
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
    /// user's local calendar day so the agent interprets "today"
    /// correctly.
    pub timezone: Option<String>,
    /// Colour-scheme preference pinned across the user's devices:
    /// `"light"` or `"dark"`. `None` means no pin — clients follow the
    /// operating system, and server-side renders that cannot see a client
    /// (messaging chart PNGs) fall back to dark, the scheme messaging
    /// clients overwhelmingly draw media bubbles on. Written via
    /// `PUT /api/user/theme`.
    pub theme: Option<String>,
}

/// Every field as stored, except `password_hash`, which prints as
/// `[REDACTED]` whatever it holds. A field prints only once it is listed
/// here, so a new one is left out until someone decides it may.
impl Debug for User {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("display_name", &self.display_name)
            .field("password_hash", &"[REDACTED]")
            .field("tier", &self.tier)
            .field("created_at", &self.created_at)
            .field("last_active", &self.last_active)
            .field("is_active", &self.is_active)
            .field("user_status", &self.user_status)
            .field("role", &self.role)
            .field("approved_by", &self.approved_by)
            .field("approved_at", &self.approved_at)
            .field("firebase_uid", &self.firebase_uid)
            .field("auth_provider", &self.auth_provider)
            .field("analytics_consent", &self.analytics_consent)
            .field("analytics_consent_at", &self.analytics_consent_at)
            .field("locale", &self.locale)
            .field("coaching_persona", &self.coaching_persona)
            .field("manages_roster", &self.manages_roster)
            .field("timezone", &self.timezone)
            .field("theme", &self.theme)
            .finish()
    }
}

/// Every locale the platform speaks, in the order the language menus list them.
///
/// The one list: the string catalogue ships every key in exactly these, the
/// messaging-strings registry seeds them, `PUT /api/user/locale` accepts
/// exactly these, `GET /api/i18n/{locale}` serves exactly these, and both
/// clients offer exactly these (`SUPPORTED_LANGUAGES` in `@pierre/i18n`). A
/// sixth locale is added here first. The first entry is the default.
pub const SUPPORTED_LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

/// Default locale (`"fr"`) of a user who has not set one.
///
/// The DB column has `NOT NULL DEFAULT 'fr'`, so persisted rows always carry
/// a concrete value; this is what a row that cannot be read falls back to.
#[must_use]
pub fn default_locale() -> String {
    SUPPORTED_LOCALES[0].to_owned()
}

/// The locale a page with no signed-in user is rendered in, read from the
/// browser's `Accept-Language` header value.
///
/// The primary subtag of the first language the browser accepts, when it is
/// a [`SUPPORTED_LOCALES`] entry; otherwise the default, French, which the
/// platform's majority user base reads.
#[must_use]
pub fn locale_from_accept_language(accept_language: Option<&str>) -> &'static str {
    let primary = accept_language
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default()
        .split(['-', ';'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    SUPPORTED_LOCALES
        .into_iter()
        .find(|locale| *locale == primary)
        .unwrap_or(SUPPORTED_LOCALES[0])
}

/// The first locale of an `OpenID` Connect `ui_locales` list (Core §3.1.2.1:
/// space-separated BCP 47 tags, most preferred first) that the platform
/// speaks, by primary subtag.
#[must_use]
pub fn supported_ui_locale(ui_locales: Option<&str>) -> Option<&'static str> {
    ui_locales?.split_ascii_whitespace().find_map(|tag| {
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        SUPPORTED_LOCALES
            .into_iter()
            .find(|locale| *locale == primary)
    })
}

/// The locale a page with no signed-in user is rendered in: the language the
/// app that opened it asked for (`ui_locales`), else the browser's.
#[must_use]
pub fn page_locale(ui_locales: Option<&str>, accept_language: Option<&str>) -> &'static str {
    supported_ui_locale(ui_locales).unwrap_or_else(|| locale_from_accept_language(accept_language))
}

/// The one form an email is stored and compared in: trimmed, lower-case.
///
/// Emails are case-insensitive across the product, so `Jane@X.com` and
/// `jane@x.com` name one person. Every write stores this form and every
/// lookup compares against it, which is what lets a sign-in, a pre-approval
/// or a coach's roster match an account whatever casing the person typed.
#[must_use]
pub fn normalize_email(email: impl AsRef<str>) -> String {
    email.as_ref().trim().to_lowercase()
}

/// How a member is named to their coach and their group: their display name,
/// else the local part of their email.
///
/// Never the whole address: a roster name reaches tool errors, prompts and
/// list rows, and from there a room.
#[must_use]
pub fn roster_display_name(display_name: Option<&str>, email: &str) -> String {
    display_name.map_or_else(
        || email.split('@').next().unwrap_or_default().to_owned(),
        ToOwned::to_owned,
    )
}

/// The `password_hash` of an account with no password.
///
/// Such an account signs in only through a federated provider (Firebase:
/// Google, Apple, GitHub). The marker is not a bcrypt hash, so no password
/// matches it, and bcrypt refuses to read it at all.
pub const FEDERATED_ONLY_PASSWORD_HASH: &str = "!firebase-auth-only!";

impl User {
    /// Create a new user with the given email and password hash. The email is
    /// kept in its stored form ([`normalize_email`]).
    ///
    /// Tenant membership is managed separately via the `tenant_users` table.
    #[must_use]
    pub fn new(email: String, password_hash: String, display_name: Option<String>) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::new_v4(),
            email: normalize_email(email),
            display_name,
            password_hash,
            tier: UserTier::Starter, // Default to starter tier
            created_at: now,
            last_active: now,
            is_active: true,
            user_status: UserStatus::Active, // Callers set Pending when needed (register, Firebase)
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

    /// Update last active timestamp
    pub fn update_last_active(&mut self) {
        self.last_active = Timestamp::now();
    }

    /// Whether this account may take an admin's decisions now: an admin that
    /// is neither deactivated nor suspended. Suspension changes `user_status`,
    /// never `is_active`, so both are read.
    #[must_use]
    pub fn is_active_admin(&self) -> bool {
        self.is_active && self.user_status == UserStatus::Active && self.role.is_admin()
    }

    /// Whether this user holds coach access: the [`Self::manages_roster`]
    /// flag, or the platform admin role. It is what being attached to a group
    /// as its coach takes, and what every act for that group's athletes asks
    /// again, so access withdrawn after the attachment withdraws the acts.
    #[must_use]
    pub const fn holds_coach_access(&self) -> bool {
        self.manages_roster || self.role.is_admin()
    }
}

/// An athlete's physiological sex, as the published equations use it.
///
/// Keytel et al. (2005) take it for exercise energy expenditure, and
/// Esteve-Lanao et al. (2026) give sex-specific VT1/VT2 speed ratios for the
/// aerobic-threshold pace.
///
/// It selects an equation's coefficients and nothing else. It is recorded
/// only when the athlete states it or a provider reports it, never inferred
/// from a name or anything else; absent, every reader falls back to the
/// pooled or sex-free form, or reports that it needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhysiologicalSex {
    /// Male.
    Male,
    /// Female.
    Female,
}

impl PhysiologicalSex {
    /// The value stored in the `sex` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }

    /// Parse a stored `sex` column value; `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "male" => Some(Self::Male),
            "female" => Some(Self::Female),
            _ => None,
        }
    }

    /// The sex a provider reports as `M` / `F` (Strava's and intervals.icu's
    /// athlete `sex`); `None` for anything else, including no answer.
    #[must_use]
    pub fn from_provider_code(code: &str) -> Option<Self> {
        match code.trim() {
            "M" | "m" => Some(Self::Male),
            "F" | "f" => Some(Self::Female),
            _ => None,
        }
    }

    /// The same sex in cageux's equations' vocabulary.
    #[must_use]
    pub const fn for_equations(self) -> Gender {
        match self {
            Self::Male => Gender::Male,
            Self::Female => Gender::Female,
        }
    }
}

/// User physiological profile for personalized analysis
///
/// Contains physiological data used for calculating personalized heart rate zones,
/// pace zones, and other performance thresholds based on individual fitness metrics.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UserPhysiologicalProfile {
    /// User `ID` this profile belongs to
    pub user_id: Uuid,
    /// VO2 max in ml/kg/min (if measured or estimated)
    pub vo2_max: Option<f64>,
    /// Resting heart rate in bpm
    pub resting_hr: Option<u16>,
    /// Maximum heart rate in bpm
    pub max_hr: Option<u16>,
    /// Lactate threshold as a fraction of `VO2max`, 0.65-0.95: the share of
    /// aerobic capacity the athlete sustains at threshold (fractional
    /// utilisation — about 0.60 untrained, 0.75-0.90 trained, Joyner & Coyle
    /// 2008). Places threshold pace in the VDOT pace zones and, when no
    /// [`Self::threshold_hr`] is measured, estimates the LTHR
    /// ([`Self::lactate_threshold_hr`]).
    pub lactate_threshold_percentage: Option<f64>,
    /// Lactate threshold heart rate (LTHR) in bpm, as measured — a 30-minute
    /// field test, a ramp test or a lab report. Heart-rate-based training
    /// stress is scored against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_hr: Option<u16>,
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
    /// Critical power in watts: the asymptote of the hyperbolic power-duration
    /// relationship, the highest power sustained without a steady drain of
    /// [`Self::w_prime_joules`]. Always carries whether it was measured or
    /// estimated, and by whom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_power_watts: Option<ProvenancedValue<u32>>,
    /// W′ (W prime) in joules: the finite work capacity above critical power.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w_prime_joules: Option<ProvenancedValue<u32>>,
    /// Critical speed in metres per second: the running and swimming analogue
    /// of critical power.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_speed_mps: Option<ProvenancedValue<f64>>,
    /// D′ (D prime) in metres: the finite distance capacity above critical
    /// speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_prime_meters: Option<ProvenancedValue<f64>>,
    /// Where [`Self::ftp_watts`], [`Self::threshold_hr`], [`Self::max_hr`]
    /// and [`Self::threshold_pace_sec_per_km`] came from, for the ones a
    /// provider supplied: each such value is that provider's estimate, quoted
    /// with its origin ("intervals.icu has your FTP at 285 W"). A threshold
    /// not listed is the athlete's own. Serialized with the profile — the
    /// dossier and every tool that echoes the profile carry it — and left out
    /// when every threshold is the athlete's own, so a profile no import
    /// touched reads exactly as before.
    #[serde(default, skip_serializing_if = "ThresholdProvenance::is_empty")]
    pub threshold_provenance: ThresholdProvenance,
    /// The day the athlete stated each of those four thresholds, for the ones
    /// that are their own and were stated since the day was recorded: what
    /// dates an athlete-set anchor, which [`Self::threshold_provenance`]
    /// cannot without reading as an import. Left out when none is recorded.
    #[serde(default, skip_serializing_if = "ThresholdStatedOn::is_empty")]
    pub threshold_stated_on: ThresholdStatedOn,
    /// Where [`Self::resting_hr`], [`Self::vo2_max`] and [`Self::weight`] came
    /// from: the athlete, an import, an estimate, or the weekly promotion from
    /// the athlete's own measurements (DECISION 5). An anchor not listed is
    /// the athlete's own. Left out when none is listed, so a profile nobody
    /// promoted reads exactly as before.
    #[serde(default, skip_serializing_if = "AnchorProvenance::is_empty")]
    pub anchor_provenance: AnchorProvenance,
    /// How [`Self::vo2_max`] was found (`cooper_test`, "lab test"): cleared
    /// with every new `VO2max` saved without one, and by a weekly promotion.
    /// Absent for a value saved before methods were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vo2_max_method: Option<String>,
    /// A VDOT the athlete or their coach stated, from a race or as a number
    /// ([`ProfileVdot`]). Stored apart from [`Self::vo2_max`]: a VDOT is a
    /// performance index, and neither is ever filled from the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vdot: Option<ProfileVdot>,
    /// Physiological sex, for the published equations that take one
    /// ([`PhysiologicalSex`]). Set by the athlete with `set_physiology`, or
    /// filled once from a provider that reports it when none is set; a value
    /// the athlete set is never replaced by a provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<PhysiologicalSex>,
    /// Where this profile may be served (carnet#769): first-party-only when the
    /// turn that wrote it served data whose terms keep it on Dravr's own
    /// surfaces. Profiles written before the stamp read as unstamped.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub transport_policy: TransportPolicy,
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
            threshold_hr: None,
            age: None,
            weight: None,
            fitness_level: FitnessLevel::Recreational,
            primary_sport,
            training_experience_years: None,
            ftp_watts: None,
            threshold_pace_sec_per_km: None,
            hr_zones: None,
            power_zones: None,
            critical_power_watts: None,
            w_prime_joules: None,
            critical_speed_mps: None,
            d_prime_meters: None,
            threshold_provenance: ThresholdProvenance::NONE,
            threshold_stated_on: ThresholdStatedOn::NONE,
            anchor_provenance: AnchorProvenance::NONE,
            vo2_max_method: None,
            vdot: None,
            sex: None,
            transport_policy: TransportPolicy::AnyTransport,
        }
    }

    /// Record the sex a provider `reported`, only when the profile holds
    /// none: a sex the athlete stated, or one recorded earlier, is never
    /// replaced, and no report records nothing. Returns whether it was
    /// recorded.
    pub fn fill_reported_sex(&mut self, reported: Option<PhysiologicalSex>) -> bool {
        match (self.sex, reported) {
            (None, Some(sex)) => {
                self.sex = Some(sex);
                true
            }
            _ => false,
        }
    }

    /// The saved maximum heart rate, else cageux's estimate from the saved age
    /// and sex by `algorithm`, the configured estimator (`algorithm.maxhr`): an
    /// athlete saved as male is never read on Gulati's women's equation; one
    /// with no saved sex is read on the configured one. `None` with neither a
    /// maximum nor an age, or when the estimator refuses the age.
    #[must_use]
    pub fn estimated_max_hr(&self, algorithm: MaxHrAlgorithm) -> Option<u16> {
        self.max_hr.or_else(|| {
            let age = self.age?;
            let sex = self.sex.map(PhysiologicalSex::as_str);
            let estimate = algorithm.estimate(u32::from(age), sex).ok()?;
            // Safe: every estimator returns a heart rate inside 0-220 bpm.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let bpm = estimate.round() as u16;
            Some(bpm)
        })
    }

    /// The lactate threshold heart rate training stress is scored against, in
    /// bpm.
    ///
    /// The measured [`Self::threshold_hr`] when there is one; otherwise the
    /// estimate [`Self::estimated_lactate_threshold_hr`] gives. `None` when
    /// neither is available — heart-rate stress then has no threshold to be
    /// scored against, which is not the same as a threshold of zero.
    #[must_use]
    pub fn lactate_threshold_hr(&self) -> Option<f64> {
        self.threshold_hr
            .map(f64::from)
            .or_else(|| self.estimated_lactate_threshold_hr())
    }

    /// The LTHR the stored lactate threshold implies, in bpm: cageux's
    /// [`LthrAlgorithm::FromVo2maxFraction`], the measured maximum heart rate
    /// times the share of it reached at that fraction of `VO2max` (Swain et
    /// al. 1994).
    ///
    /// Needs a measured `max_hr`; an age-predicted one would stack a second
    /// population estimate on the first. `None` when cageux refuses the
    /// inputs (a fraction outside the regression's 0.40-0.95).
    #[must_use]
    pub fn estimated_lactate_threshold_hr(&self) -> Option<f64> {
        LthrAlgorithm::FromVo2maxFraction {
            max_hr: f64::from(self.max_hr?),
            vo2max_fraction: self.lactate_threshold_percentage?,
        }
        .estimate_lthr()
        .ok()
    }
}

/// What kind of row still points at a user, blocking the account's deletion.
///
/// Most name a foreign key onto `users(id)` that neither cascades nor sets
/// NULL, so deleting the user while one such row exists fails in the database.
/// The rest name something the delete would take from other people or leave
/// running elsewhere: a tenant only this user owns while others belong to it,
/// an agent other users rely on, a billing subscription still live at the
/// provider. An admin reassigns (or removes) the row first; a group owned
/// out from under its members, or an audit trail rewritten to a deleted
/// account, is not something a delete does silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserReferenceKind {
    /// The user owns a coaching group (`coaching_groups.owner_id`).
    OwnsCoachingGroup,
    /// The user is the human coach attached to a group (`coaching_groups.coach_user_id`).
    CoachesCoachingGroup,
    /// The user created a group invite (`group_invites.created_by`).
    CreatedGroupInvite,
    /// The user wrote a runtime config override (`admin_config_overrides.created_by`).
    AdminConfigOverride,
    /// The user is the actor on a config audit entry (`admin_config_audit.admin_user_id`).
    AdminConfigAudit,
    /// The user configured a tenant's provider OAuth app (`tenant_oauth_credentials.configured_by`).
    TenantOAuthCredentials,
    /// The user created LLM credentials held by someone else or by the tenant
    /// (`user_llm_credentials.created_by`).
    LlmCredentials,
    /// The user approved another account (`users.approved_by`).
    ApprovedUser,
    /// The user is the only owner of a tenant other users belong to
    /// (`tenant_users.role = 'owner'`); every tenant read joins its owner, so
    /// the tenant would vanish for those members.
    OwnsTenant,
    /// The user authored an agent other users rely on (`agents.user_id`): one
    /// shared with a tenant or globally, or one another user's conversation, a
    /// group or another user's assignment is bound to. The agent's rows go
    /// with its author.
    AuthoredAgent,
    /// The user holds a billing subscription the provider may still charge
    /// (`subscriptions.user_id`, any status but `canceled` or
    /// `incomplete_expired`); deleting the row cancels nothing there.
    BillingSubscription,
}

impl UserReferenceKind {
    /// The stable identifier the storage query and the admin API both use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnsCoachingGroup => "owns_coaching_group",
            Self::CoachesCoachingGroup => "coaches_coaching_group",
            Self::CreatedGroupInvite => "created_group_invite",
            Self::AdminConfigOverride => "admin_config_override",
            Self::AdminConfigAudit => "admin_config_audit",
            Self::TenantOAuthCredentials => "tenant_oauth_credentials",
            Self::LlmCredentials => "llm_credentials",
            Self::ApprovedUser => "approved_user",
            Self::OwnsTenant => "owns_tenant",
            Self::AuthoredAgent => "authored_agent",
            Self::BillingSubscription => "billing_subscription",
        }
    }

    /// An admin-facing phrase for one reference of this kind; `detail` is
    /// what the storage query names the referencing row by.
    #[must_use]
    pub fn describe(self, detail: &str) -> String {
        match self {
            Self::OwnsCoachingGroup => format!("owns coaching group '{detail}'"),
            Self::CoachesCoachingGroup => format!("coaches coaching group '{detail}'"),
            Self::CreatedGroupInvite => {
                format!("created an invite to coaching group '{detail}'")
            }
            Self::AdminConfigOverride => format!("created admin config override {detail}"),
            Self::AdminConfigAudit => format!("is the actor on admin config audit entry {detail}"),
            Self::TenantOAuthCredentials => {
                format!("configured tenant OAuth credentials for {detail}")
            }
            Self::LlmCredentials => format!("created LLM credentials for {detail}"),
            Self::ApprovedUser => format!("approved the account {detail}"),
            Self::OwnsTenant => {
                format!("is the only owner of tenant '{detail}', which has other members")
            }
            Self::AuthoredAgent => format!("authored agent '{detail}', which other users rely on"),
            Self::BillingSubscription => {
                format!("holds a billing subscription the provider may still charge: {detail}")
            }
        }
    }
}

impl FromStr for UserReferenceKind {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "owns_coaching_group" => Ok(Self::OwnsCoachingGroup),
            "coaches_coaching_group" => Ok(Self::CoachesCoachingGroup),
            "created_group_invite" => Ok(Self::CreatedGroupInvite),
            "admin_config_override" => Ok(Self::AdminConfigOverride),
            "admin_config_audit" => Ok(Self::AdminConfigAudit),
            "tenant_oauth_credentials" => Ok(Self::TenantOAuthCredentials),
            "llm_credentials" => Ok(Self::LlmCredentials),
            "approved_user" => Ok(Self::ApprovedUser),
            "owns_tenant" => Ok(Self::OwnsTenant),
            "authored_agent" => Ok(Self::AuthoredAgent),
            "billing_subscription" => Ok(Self::BillingSubscription),
            other => Err(AppError::internal(format!(
                "Unknown user reference kind: {other}"
            ))),
        }
    }
}

/// One row that still references a user and blocks deleting the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserReference {
    /// Which foreign key the row holds.
    pub kind: UserReferenceKind,
    /// What the referencing row is named by: a group name, a config key, a
    /// provider, or the approved account's email.
    pub detail: String,
}

impl UserReference {
    /// The admin-facing sentence fragment naming this reference.
    #[must_use]
    pub fn describe(&self) -> String {
        self.kind.describe(&self.detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_is_named_by_their_display_name_else_their_email_local_part() {
        assert_eq!(
            roster_display_name(Some("Mira Subject"), "mira@example.test"),
            "Mira Subject"
        );
        assert_eq!(
            roster_display_name(None, "mira.s@example.test"),
            "mira.s",
            "never the whole address"
        );
    }

    #[test]
    fn an_estimated_maximum_never_reads_a_man_on_gulatis_womens_equation() {
        let mut athlete = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        athlete.age = Some(40);
        // Gulati 206 - 0.88 x 40 = 170.8 for her, Tanaka 208 - 0.7 x 40 for him;
        // a saved maximum wins over both.
        for (sex, max_hr, expected) in [
            (PhysiologicalSex::Female, None, 171),
            (PhysiologicalSex::Male, None, 180),
            (PhysiologicalSex::Male, Some(188), 188),
        ] {
            athlete.sex = Some(sex);
            athlete.max_hr = max_hr;
            let got = athlete.estimated_max_hr(MaxHrAlgorithm::Gulati);
            assert_eq!(
                got,
                Some(expected),
                "bug: {sex:?} with saved max {max_hr:?}"
            );
        }
    }

    fn with_role(role: UserRole) -> User {
        let mut user = User::new("op@example.test".to_owned(), String::new(), None);
        user.role = role;
        user
    }

    #[test]
    fn only_an_admin_in_good_standing_is_an_active_admin() {
        assert!(with_role(UserRole::Admin).is_active_admin());
        assert!(!with_role(UserRole::User).is_active_admin());

        for status in [UserStatus::Pending, UserStatus::Suspended] {
            let mut held = with_role(UserRole::Admin);
            held.user_status = status;
            assert!(!held.is_active_admin(), "{status:?}");
        }
        let mut deactivated = with_role(UserRole::Admin);
        deactivated.is_active = false;
        assert!(!deactivated.is_active_admin());
    }

    #[test]
    fn coach_access_is_the_roster_flag_or_the_admin_role() {
        assert!(
            !with_role(UserRole::User).holds_coach_access(),
            "a user without the flag holds no coach access"
        );
        let mut flagged = with_role(UserRole::User);
        flagged.manages_roster = true;
        assert!(flagged.holds_coach_access(), "the flag alone grants it");
        assert!(
            with_role(UserRole::Admin).holds_coach_access(),
            "an admin holds it without the flag"
        );
    }
}
