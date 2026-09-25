// ABOUTME: Shared TypeScript types for admin panel and dashboard
// ABOUTME: Admin tokens, A2A client registration and usage, and tool-usage analytics types

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
