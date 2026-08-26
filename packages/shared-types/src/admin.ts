// ABOUTME: Shared TypeScript types for admin panel and dashboard
// ABOUTME: API keys, admin tokens, A2A protocol, and analytics types

// ========== API KEY TYPES ==========

/** Status of an API key */
export type ApiKeyStatus = 'active' | 'inactive' | 'revoked';

/** An API key for external access */
export interface ApiKey {
  id: string;
  name: string;
  description?: string;
  prefix: string;
  key_prefix: string;
  rate_limit_requests: number;
  status: ApiKeyStatus;
  is_active: boolean;
  created_at: string;
  expires_at?: string;
  last_used_at?: string;
  usage_count: number;
  rate_limit_remaining?: number;
  rate_limit_reset?: string;
}

/** Response for listing API keys */
export interface ApiKeysResponse {
  api_keys: ApiKey[];
  total_count: number;
}

/** Request to create an API key */
export interface CreateApiKeyRequest {
  name: string;
  description?: string;
  rate_limit_requests: number;
  expires_in_days?: number;
}

/** Response for creating an API key (includes secret) */
export interface CreateApiKeyResponse {
  api_key: ApiKey;
  secret_key: string;
}

// ========== ADMIN TOKEN TYPES ==========

/** Permission for admin tokens */
export type AdminPermission =
  | 'provision_keys'
  | 'revoke_keys'
  | 'list_keys'
  | 'manage_admin_tokens'
  | 'view_audit_logs'
  | 'super_admin';

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
}

/** Response for listing admin tokens */
export interface AdminTokensResponse {
  admin_tokens: AdminToken[];
  total_count: number;
}

/** Request to create an admin token */
export interface CreateAdminTokenRequest {
  service_name: string;
  service_description?: string;
  permissions: AdminPermission[];
  is_super_admin?: boolean;
  expires_in_days?: number;
}

/** Response for creating an admin token (includes JWT) */
export interface CreateAdminTokenResponse {
  success: boolean;
  token_id: string;
  service_name: string;
  jwt_token: string;
  token_prefix: string;
  is_super_admin: boolean;
  expires_at?: string;
}

/** Audit log entry for admin token usage */
export interface AdminTokenAudit {
  id: string;
  admin_token_id: string;
  timestamp: string;
  action: string;
  target_resource?: string;
  ip_address?: string;
  success: boolean;
  error_message?: string;
}

/** Usage statistics for an admin token */
export interface AdminTokenUsageStats {
  total_actions: number;
  actions_last_24h: number;
  actions_last_7d: number;
  most_common_actions: Array<{
    action: string;
    count: number;
  }>;
}

// ========== DASHBOARD TYPES ==========

/** Usage statistics by tier */
export interface TierUsage {
  tier: string;
  usage_count: number;
  percentage: number;
  key_count: number;
  total_requests: number;
  average_requests_per_key: number;
}

/** Dashboard overview data */
export interface DashboardOverview {
  total_requests_today: number;
  total_requests_this_month: number;
  active_api_keys: number;
  total_api_keys: number;
  error_rate_today: number;
  rate_limit_status: {
    current: number;
    limit: number;
    reset_time: string;
  };
  current_month_usage_by_tier?: TierUsage[];
}

/** Rate limit overview for an API key */
export interface RateLimitOverview {
  api_key_id: string;
  api_key_name: string;
  tier: string;
  requests_per_minute: number;
  requests_per_hour: number;
  requests_per_day: number;
  limit: number;
  usage_percentage: number;
  current_usage: number;
  reset_times: {
    minute: string;
    hour: string;
    day: string;
  };
}

/** A request log entry */
export interface RequestLog {
  id: string;
  api_key_id: string;
  api_key_name: string;
  timestamp: string;
  tool_name: string;
  status_code: number;
  response_time_ms?: number;
  error_message?: string;
  request_size_bytes?: number;
  response_size_bytes?: number;
  ip_address?: string;
  user_agent?: string;
}

/** Request statistics */
export interface RequestStats {
  total_requests: number;
  successful_requests: number;
  failed_requests: number;
  average_response_time: number;
  min_response_time?: number;
  max_response_time?: number;
  requests_per_minute: number;
  error_rate: number;
}

/** Filter for request logs */
export interface RequestFilter {
  timeRange: string;
  status: string;
  tool: string;
}

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

/** An A2A session */
export interface A2ASession {
  id: string;
  client_id: string;
  user_id?: string;
  granted_scopes: string[];
  created_at: string;
  expires_at: string;
  last_activity: string;
  requests_count: number;
}

/** Rate limit status for A2A client */
export interface A2ARateLimitStatus {
  is_rate_limited: boolean;
  limit?: number;
  remaining?: number;
  reset_at?: string;
  tier: string;
}

/** Usage statistics for an A2A client */
export interface A2AUsageStats {
  client_id: string;
  requests_today: number;
  requests_this_month: number;
  total_requests: number;
  last_request_at?: string;
  rate_limit_tier: string;
  tool_usage_breakdown: Array<{
    tool_name: string;
    usage_count: number;
    percentage: number;
  }>;
  capability_usage: Array<{
    capability: string;
    usage_count: number;
    percentage: number;
  }>;
}

/** A2A usage record (detailed log) */
export interface A2AUsageRecord {
  id: number;
  client_id: string;
  session_token?: string;
  timestamp: string;
  tool_name: string;
  response_time_ms?: number;
  status_code: number;
  error_message?: string;
  request_size_bytes?: number;
  response_size_bytes?: number;
  ip_address?: string;
  user_agent?: string;
  protocol_version: string;
  client_capabilities: string[];
  granted_scopes: string[];
}

/** A2A dashboard overview */
export interface A2ADashboardOverview {
  total_clients: number;
  active_clients: number;
  total_sessions: number;
  active_sessions: number;
  requests_today: number;
  requests_this_month: number;
  most_used_capability: string;
  error_rate: number;
  usage_by_tier: Array<{
    tier: string;
    client_count: number;
    request_count: number;
    percentage: number;
  }>;
}

// ========== SETUP TYPES ==========

/** Response for setup status check */
export interface SetupStatusResponse {
  needs_setup: boolean;
  admin_user_exists: boolean;
  message?: string;
}

/** A provisioned API key record */
export interface ProvisionedKey {
  api_key_id: string;
  user_email: string;
  requested_tier: string;
  key_status: ApiKeyStatus;
  created_at: string;
  expires_at?: string;
}
