// ABOUTME: Shared TypeScript types for admin panel and dashboard
// ABOUTME: Admin tokens, A2A client registration and usage, and tool-usage analytics types

// ========== ADMIN TOKEN TYPES ==========

/**
 * Permission for admin tokens — the server's `AdminPermission` names. A
 * super-admin token is minted with `is_super_admin`, not with a permission.
 */
export type AdminPermission =
  | 'provision_keys'
  | 'list_keys'
  | 'revoke_keys'
  | 'update_key_limits'
  | 'manage_admin_tokens'
  | 'view_audit_logs'
  | 'manage_users'
  | 'view_configuration'
  | 'manage_configuration';

/** An admin token for internal services */
export interface AdminToken {
  id: string;
  service_name: string;
  service_description?: string;
  permissions: AdminPermission[];
  is_super_admin: boolean;
  is_active: boolean;
  created_at: string;
  expires_at?: string;
  last_used_at?: string;
  usage_count: number;
  token_prefix: string;
  /** Tenant the token is bound to; null for a platform-wide token */
  tenant_id?: string | null;
  /** The operator a device-login token acts as; null for a service token */
  operator_user_id?: string | null;
}

/**
 * `data` of `GET /api/admin/tokens`. A caller that is not super-admin is not
 * shown super-admin tokens.
 */
export interface AdminTokensResponse {
  tokens: AdminToken[];
  count: number;
}

/** Request to create an admin token */
export interface CreateAdminTokenRequest {
  service_name: string;
  service_description?: string;
  permissions: AdminPermission[];
  is_super_admin?: boolean;
  expires_in_days?: number;
}

/** `data` of creating an admin token; the JWT is shown this once */
export interface CreateAdminTokenResponse {
  token_id: string;
  service_name: string;
  jwt_token: string;
  token_prefix: string;
  is_super_admin: boolean;
  expires_at?: string | null;
}

/** `data` of rotating an admin token: the replacement, shown this once */
export interface RotateAdminTokenResponse extends CreateAdminTokenResponse {
  /** The token the rotation retired */
  old_token_id: string;
}

// ========== DASHBOARD TYPES ==========

/** Breakdown of tool usage */
export interface ToolUsageBreakdown {
  tool_name: string;
  request_count: number;
  success_rate: number;
  average_response_time: number;
  error_count?: number;
  percentage_of_total?: number;
}

// ========== A2A (AGENT-TO-AGENT) PROTOCOL TYPES ==========

/** An A2A client (external agent) */
export interface A2AClient {
  id: string;
  name: string;
  description: string;
  public_key?: string;
  capabilities: string[];
  redirect_uris: string[];
  agent_version?: string;
  contact_email?: string;
  documentation_url?: string;
  is_verified: boolean;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

/** Request to register an A2A client */
export interface A2AClientRegistrationRequest {
  name: string;
  description: string;
  capabilities: string[];
  redirect_uris?: string[];
  contact_email: string;
  agent_version?: string;
  documentation_url?: string;
}

/** Credentials returned after A2A client registration */
export interface A2AClientCredentials {
  client_id: string;
  client_secret: string;
  api_key: string;
}

/**
 * An A2A client's request budget, as `GET /a2a/clients/{id}/rate-limit`
 * serves it: `rate_limit_requests` calls per sliding
 * `rate_limit_window_seconds`, the budget its client-credentials calls spend
 * and are refused on.
 */
export interface A2ARateLimitStatus {
  client_id: string;
  /** The window has admitted its limit, so the next call is refused */
  is_rate_limited: boolean;
  /** Calls the window admits */
  rate_limit_requests: number;
  /** Length of the sliding window, in seconds */
  rate_limit_window_seconds: number;
  /** Calls counted inside the window */
  current_usage: number;
  /** Calls the window still admits */
  remaining: number;
  /** When the window frees its first slot (RFC 3339) */
  reset_at: string;
}

/** One UTC calendar day of an A2A client's calls */
export interface A2ADailyUsage {
  /** The UTC calendar day (YYYY-MM-DD) */
  date: string;
  /** Calls answered below 400 */
  success_count: number;
  /** Calls answered 400 or above */
  error_count: number;
}

/** Usage statistics for an A2A client, counted over its calls */
export interface A2AUsageStats {
  client_id: string;
  /** Calls since the start of the UTC day */
  requests_today: number;
  /** Calls since the start of the UTC month */
  requests_this_month: number;
  /** Every call the client has made */
  total_requests: number;
  /** The most recent call (RFC 3339), null before the first */
  last_request_at: string | null;
  /** One row per UTC day of the last thirty that saw a call, newest first */
  daily_usage: A2ADailyUsage[];
}
