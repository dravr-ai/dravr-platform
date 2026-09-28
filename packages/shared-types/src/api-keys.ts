// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's own API keys as /api/keys serves them — the listing, a creation, a revocation, a key's usage
// ABOUTME: Mirrors pierre-server's routes/api_keys service types field for field

/** A key's tier: the plan of the user who minted it. */
export type ApiKeyTier = 'trial' | 'starter' | 'professional' | 'enterprise';

/** One key, without its secret. */
export interface ApiKeyInfo {
  id: string;
  name: string;
  description: string | null;
  tier: ApiKeyTier;
  /** The first characters of the key, enough to recognise it. */
  key_prefix: string;
  /** A revoked key stays listed as inactive. */
  is_active: boolean;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

/** `GET /api/keys`. */
export interface ApiKeyListResponse {
  api_keys: ApiKeyInfo[];
}

/** `POST /api/keys` body. The key's tier and budget are the caller's plan. */
export interface CreateApiKeyRequest {
  name: string;
  description?: string;
  /** Days until the key expires; absent for a key that does not. */
  expires_in_days?: number;
}

/** `POST /api/keys`: the only response that carries the full key. */
export interface ApiKeyCreateResponse {
  api_key: string;
  key_info: ApiKeyInfo;
  warning: string;
}

/** `DELETE /api/keys/{id}`. */
export interface ApiKeyDeactivateResponse {
  message: string;
  deactivated_at: string;
}

/** One key's calls over a window. */
export interface ApiKeyUsageStats {
  api_key_id: string;
  period_start: string;
  period_end: string;
  total_requests: number;
  successful_requests: number;
  failed_requests: number;
  total_response_time_ms: number;
  tool_usage: Record<string, number>;
}

/** `GET /api/keys/{id}/usage`. */
export interface ApiKeyUsageResponse {
  stats: ApiKeyUsageStats;
}
