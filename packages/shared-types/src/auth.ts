// ABOUTME: Shared TypeScript types for authentication and user management
// ABOUTME: User types, login responses, OAuth types

// ========== USER TYPES ==========

/** User role */
export type UserRole = 'super_admin' | 'admin' | 'user';

/** User account status */
export type UserStatus = 'pending' | 'active' | 'suspended';

/** User subscription tier */
export type UserTier = 'starter' | 'professional' | 'enterprise';

/**
 * Coaching persona — orthogonal to the chosen coach personality.
 *
 * Controls output format / citation density / notification cadence:
 * - `casual` — friend-texting prose, no jargon, P0-only push
 * - `enthusiast` — prose with optional data citations, P0/P1 push
 * - `power_athlete` — Section 11 discipline, line-by-line numeric reports,
 *   framework citations on every numeric claim, full P0/P1/P2 push
 * - `coach` — inherits power_athlete + roster framing for managing other athletes
 *
 * Persisted as snake_case on the backend.
 */
export type CoachingPersona = 'casual' | 'enthusiast' | 'power_athlete' | 'coach';

/** A user in the system */
export interface User {
  /** Primary user identifier */
  id: string;
  /** Alternative identifier (alias for id) */
  user_id?: string;
  email: string;
  display_name?: string;
  is_admin: boolean;
  role: UserRole;
  /** Account status (use user_status or status) */
  user_status?: UserStatus;
  status?: UserStatus;
  /**
   * Whether the user has confirmed their email address.
   *
   * Absent means the surface that produced this user did not resolve it —
   * deliberately not `false`, which would claim the address is unconfirmed when
   * the server simply didn't look. Populated on login and session restore, which
   * is where the clients decide between "confirm your email" and "awaiting review".
   */
  email_verified?: boolean;
  /** Subscription tier (always present in user listings) */
  tier: UserTier;
  /** Active tenant identifier for multi-tenant operations */
  tenant_id?: string;
  /** Account creation timestamp (always present) */
  created_at: string;
  /** Last activity timestamp */
  last_active?: string;
  /** Admin who approved this user */
  approved_by?: string;
  /** Approval timestamp */
  approved_at?: string;
  /** Whether the user consented to anonymized analytics tracking */
  analytics_consent?: boolean;
  /** When the user last updated their analytics consent preference */
  analytics_consent_at?: string;
  /** Output-format / cadence preference. Defaults to `casual` for new users. */
  coaching_persona?: CoachingPersona;
  /** True when the user has access to the Coach-tier roster UI. */
  manages_roster?: boolean;
  /**
   * BCP-47 short locale the athlete is answered in (`fr`, `en`, `es`, `de`,
   * `pt`). Sent by `/oauth/token`, `/api/auth/session` and the Firebase login
   * as `UserInfo.locale`; absent on the surfaces that build a `User` without
   * reading the column (admin listings).
   */
  locale?: string;
}

/** Extended user for admin views (deprecated: use User directly) */
export type AdminUser = User;

// ========== THEME PREFERENCE ==========

/**
 * The theme the user pinned across their devices (`users.theme`).
 * `null` means "follow the system" — the clients resolve the OS scheme
 * locally and the server stores that no pin exists.
 */
export type ThemePreference = 'light' | 'dark' | null;

/** Body of `PUT /api/user/theme`; the server answers `204 No Content`. */
export interface UpdateThemeRequest {
  theme: ThemePreference;
}

// ========== AUTH RESPONSE TYPES ==========

/** Response from login endpoint */
export interface LoginResponse {
  access_token: string;
  token_type: string;
  expires_in?: number;
  refresh_token?: string;
  user: User;
  csrf_token: string;
}

/** Response from registration endpoint */
export interface RegisterResponse {
  user_id: string;
  email: string;
  /** Account status at creation time ("pending" or "active") */
  user_status: 'pending' | 'active' | 'suspended';
  /** Display name, if the user supplied one at registration */
  display_name?: string;
  message: string;
}

/** Response from Firebase login */
export interface FirebaseLoginResponse {
  csrf_token: string;
  jwt_token: string;
  user: User;
  is_new_user: boolean;
}

/** Response from session restore endpoint */
export interface SessionResponse {
  user: User;
  access_token: string;
  csrf_token: string;
}

/** Response from forgot-password endpoint */
export interface ForgotPasswordResponse {
  message: string;
}

/** Response from complete-reset endpoint */
export interface ResetPasswordResponse {
  message: string;
}

// ========== OAUTH TYPES ==========

/** Status of a provider connection */
export interface ProviderStatus {
  provider: string;
  connected: boolean;
  last_sync: string | null;
}

/** Extended provider status from /api/providers endpoint */
export interface ExtendedProviderStatus {
  provider: string;
  display_name: string;
  requires_oauth: boolean;
  connected: boolean;
  /**
   * A connected provider whose session is no longer usable and needs the user
   * to reconnect (a dead sciotte scrape session or a non-recoverable OAuth
   * refresh). Only meaningful when `connected` is true — render "Reconnect
   * needed" instead of a healthy "Connected".
   */
  needs_reauth: boolean;
  capabilities: string[];
  /**
   * For a user-facing provider with more than one auth backend (Strava can be
   * served by shared-app OAuth or by the Sciotte mirror), the backend a NEW
   * connection should use: `"oauth"` while shared seats remain, otherwise
   * `"mirror"`. Absent for single-backend providers.
   */
  recommended_backend?: 'oauth' | 'mirror';
  /**
   * Remaining shared-app OAuth athlete seats for this provider when it is
   * seat-limited (Strava). Absent for providers without a seat cap.
   */
  seats_left?: number;
}

/** Response from /api/providers endpoint */
export interface ProvidersStatusResponse {
  providers: ExtendedProviderStatus[];
}

/** OAuth app credentials (user-provided) */
export interface OAuthApp {
  provider: string;
  client_id: string;
  redirect_uri: string;
  created_at: string;
}

/** OAuth app credentials with secret (only for registration) */
export interface OAuthAppCredentials {
  provider: string;
  client_id: string;
  client_secret: string;
  redirect_uri: string;
}

/** Known OAuth providers */
export interface OAuthProvider {
  id: string;
  name: string;
  color: string;
}

/**
 * A connected MCP OAuth app — an external MCP client (e.g. Claude Desktop) the
 * user approved on the OAuth consent screen. Surfaced in the "Connected apps"
 * settings section; revoking one forces that client to re-consent on its next
 * authorization. Mirrors the server's `GrantView` in `routes/oauth_grants.rs`.
 */
export interface OAuthGrant {
  /** Grant id — pass to the revoke endpoint. */
  id: string;
  /** OAuth client the user approved (shown as the app name). */
  client_id: string;
  /** Scope the grant covers (space-delimited). */
  scope: string;
  /** When the grant was made (RFC 3339). */
  granted_at: string;
}

// ========== MCP TOKEN TYPES ==========

/** An MCP token for API access */
export interface McpToken {
  id: string;
  name: string;
  token_prefix: string;
  /** Only returned once on creation */
  token_value?: string;
  expires_at: string | null;
  last_used_at: string | null;
  usage_count: number;
  is_revoked: boolean;
  created_at: string;
}

// ========== USER MANAGEMENT TYPES ==========

/** Response for user management operations */
export interface UserManagementResponse {
  success: boolean;
  message: string;
  user?: AdminUser;
}

/** Request to approve a user */
export interface ApproveUserRequest {
  reason?: string;
}

/** Request to suspend a user */
export interface SuspendUserRequest {
  reason?: string;
}
