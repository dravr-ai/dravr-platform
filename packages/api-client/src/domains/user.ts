// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: User domain API - profile, stats, MCP tokens, OAuth apps
// ABOUTME: Handles user account management and settings

import type { AxiosInstance } from 'axios';
import type {
  CoachingPersona,
  McpToken,
  OAuthApp,
  OAuthAppCredentials,
  User,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { CoachingPersona, McpToken, User };

// Types - aligned with actual backend responses

export interface UserStats {
  connected_providers: number;
  days_active: number;
}

export interface McpTokensResponse {
  tokens: McpToken[];
}

export interface CreateMcpTokenRequest {
  name: string;
  expires_in_days?: number;
}

// Use OAuthApp from shared-types, re-export as UserOAuthApp for backward compat
export type UserOAuthApp = OAuthApp;

export interface UserOAuthAppsResponse {
  apps: UserOAuthApp[];
}

export interface LlmProviderStatus {
  name: string;
  display_name: string;
  has_credentials: boolean;
  credential_source: string | null;
  is_active: boolean;
}

export interface LlmCredentialSummary {
  id: string;
  provider: string;
  user_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface LlmSettingsResponse {
  current_provider: string | null;
  providers: LlmProviderStatus[];
  user_credentials: LlmCredentialSummary[];
  tenant_credentials: LlmCredentialSummary[];
}

export interface SaveLlmCredentialsRequest {
  provider: string;
  api_key: string;
  base_url?: string;
  default_model?: string;
  scope?: 'user' | 'tenant';
}

export interface SaveLlmCredentialsResponse {
  success: boolean;
  id: string | null;
  message: string;
}

export interface UpdateProfileResponse {
  message: string;
  user: { id: string; email: string; display_name?: string };
}

/**
 * Creates the user API methods bound to an axios instance.
 */
export function createUserApi(axios: AxiosInstance) {
  return {
    /**
     * Get user profile.
     */
    async getProfile(): Promise<User> {
      const response = await axios.get<User>(ENDPOINTS.USER.PROFILE);
      return response.data;
    },

    /**
     * Update user profile.
     */
    async updateProfile(data: { display_name: string }): Promise<UpdateProfileResponse> {
      const response = await axios.put<UpdateProfileResponse>(ENDPOINTS.USER.PROFILE, data);
      return response.data;
    },

    /**
     * Get user stats.
     */
    async getStats(): Promise<UserStats> {
      const response = await axios.get<UserStats>(ENDPOINTS.USER.STATS);
      return response.data;
    },

    /**
     * Change password.
     */
    async changePassword(currentPassword: string, newPassword: string): Promise<{ message: string }> {
      const response = await axios.put<{ message: string }>(ENDPOINTS.USER.CHANGE_PASSWORD, {
        current_password: currentPassword,
        new_password: newPassword,
      });
      return response.data;
    },

    // ==================== ANALYTICS CONSENT ====================

    /**
     * Update analytics consent preference.
     */
    async updateAnalyticsConsent(enabled: boolean): Promise<{ message: string; enabled: boolean }> {
      const response = await axios.put<{ message: string; enabled: boolean }>(
        ENDPOINTS.USER.ANALYTICS_CONSENT,
        { enabled }
      );
      return response.data;
    },

    // ==================== COACHING PERSONA ====================

    /**
     * Update the user's coaching persona — output format / citation
     * density / notification cadence preference. Persona is orthogonal to
     * the chosen coach personality; the same coach speaks differently
     * to a Casual user versus a PowerAthlete user.
     */
    async setCoachingPersona(
      persona: CoachingPersona,
    ): Promise<{ message: string; persona: CoachingPersona }> {
      const response = await axios.put<{ message: string; persona: CoachingPersona }>(
        ENDPOINTS.USER.COACHING_PERSONA,
        { persona },
      );
      return response.data;
    },

    // ==================== MCP TOKENS ====================

    /**
     * List MCP tokens.
     */
    async getMcpTokens(): Promise<McpTokensResponse> {
      const response = await axios.get<McpTokensResponse>(ENDPOINTS.USER.MCP_TOKENS);
      return response.data;
    },

    /**
     * Create a new MCP token.
     */
    async createMcpToken(request: CreateMcpTokenRequest): Promise<McpToken> {
      const response = await axios.post<McpToken>(ENDPOINTS.USER.MCP_TOKENS, request);
      return response.data;
    },

    /**
     * Revoke an MCP token.
     */
    async revokeMcpToken(tokenId: string): Promise<{ success: boolean }> {
      const response = await axios.delete<{ success: boolean }>(ENDPOINTS.USER.MCP_TOKEN(tokenId));
      return response.data;
    },

    // ==================== OAUTH APPS ====================

    /**
     * Get user's registered OAuth apps.
     */
    async getOAuthApps(): Promise<UserOAuthAppsResponse> {
      const response = await axios.get<UserOAuthAppsResponse>(ENDPOINTS.USER.OAUTH_APPS);
      return response.data;
    },

    /**
     * Register an OAuth app.
     */
    async registerOAuthApp(credentials: OAuthAppCredentials): Promise<{
      success: boolean;
      provider: string;
      message: string;
    }> {
      const response = await axios.post<{
        success: boolean;
        provider: string;
        message: string;
      }>(ENDPOINTS.USER.OAUTH_APPS, credentials);
      return response.data;
    },

    /**
     * Delete an OAuth app.
     */
    async deleteOAuthApp(provider: string): Promise<void> {
      await axios.delete(ENDPOINTS.USER.OAUTH_APP(provider));
    },

    // ==================== LLM SETTINGS ====================

    /**
     * Get LLM settings.
     */
    async getLlmSettings(): Promise<LlmSettingsResponse> {
      const response = await axios.get<LlmSettingsResponse>(ENDPOINTS.USER.LLM_SETTINGS);
      return response.data;
    },

    /**
     * Save LLM credentials for a provider.
     */
    async saveLlmCredentials(
      data: SaveLlmCredentialsRequest
    ): Promise<SaveLlmCredentialsResponse> {
      const response = await axios.put<SaveLlmCredentialsResponse>(
        ENDPOINTS.USER.LLM_SETTINGS,
        data
      );
      return response.data;
    },

    /**
     * Validate LLM credentials without saving.
     */
    async validateLlmCredentials(
      data: { provider: string; api_key: string; base_url?: string }
    ): Promise<{ valid: boolean; provider?: string; models?: string[]; error?: string }> {
      const response = await axios.post<{ valid: boolean; provider?: string; models?: string[]; error?: string }>(
        ENDPOINTS.USER.LLM_SETTINGS_VALIDATE,
        data
      );
      return response.data;
    },

    /**
     * Delete LLM credentials for a provider.
     */
    async deleteLlmCredentials(
      provider: string
    ): Promise<{ success: boolean; message: string }> {
      const response = await axios.delete<{ success: boolean; message: string }>(
        ENDPOINTS.USER.LLM_SETTINGS_PROVIDER(provider)
      );
      return response.data;
    },

    // Aliases for backward compatibility
    getUserStats() {
      return this.getStats();
    },

    getUserOAuthApps() {
      return this.getOAuthApps();
    },

    registerUserOAuthApp(credentials: OAuthAppCredentials) {
      return this.registerOAuthApp(credentials);
    },

    deleteUserOAuthApp(provider: string) {
      return this.deleteOAuthApp(provider);
    },

    /**
     * List the harness memory facts the platform has stored about the
     * authenticated user. Powers the Sprint C5 "what the coach
     * remembers" panel.
     */
    async listMemoryFacts(params?: {
      coach_id?: string;
      kind?: string;
      limit?: number;
    }): Promise<MemoryFactListResponse> {
      const query = new URLSearchParams();
      if (params?.coach_id) query.append('coach_id', params.coach_id);
      if (params?.kind) query.append('kind', params.kind);
      if (params?.limit !== undefined) query.append('limit', String(params.limit));
      const path = query.toString()
        ? `/api/memory/facts?${query.toString()}`
        : '/api/memory/facts';
      const response = await axios.get<MemoryFactListResponse>(path);
      return response.data;
    },

    /**
     * GDPR-grade Forget for a single stored fact. Returns `{ deleted }`
     * indicating whether a row was actually removed; the post-condition
     * is idempotent ("the fact is gone") regardless.
     */
    async forgetMemoryFact(factId: string): Promise<ForgetMemoryFactResponse> {
      const response = await axios.delete<ForgetMemoryFactResponse>(
        `/api/memory/facts/${encodeURIComponent(factId)}`,
      );
      return response.data;
    },
  };
}

/** Wire shape for a single user_facts row served to the memory panel. */
export interface MemoryFactRow {
  id: string;
  coach_id: string | null;
  kind: 'preference' | 'physiology' | 'injury' | 'goal' | 'schedule' | 'equipment' | 'other';
  subject: string;
  predicate: string;
  object: string;
  confidence: number;
  source_msg_id: string | null;
  updated_at: string;
}

export interface MemoryFactListResponse {
  facts: MemoryFactRow[];
  total: number;
}

export interface ForgetMemoryFactResponse {
  deleted: boolean;
}

export type UserApi = ReturnType<typeof createUserApi>;
