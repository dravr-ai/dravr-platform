// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin API methods - user management, tokens, config, coach management, store moderation
// ABOUTME: Handles all administrative functionality for super_admin and admin roles

import { axios } from './client';
import type { Coach } from '@pierre/shared-types';
import type { SocialInsightsConfig } from '../../types/api';

export const adminApi = {
  // ==================== SETUP STATUS ====================
  async getSetupStatus() {
    const response = await axios.get('/admin/setup/status');
    return response.data;
  },

  // ==================== ADMIN TOKEN MANAGEMENT ====================
  async getAdminTokens(params?: { include_inactive?: boolean }) {
    const queryParams = new URLSearchParams();
    if (params?.include_inactive !== undefined) {
      queryParams.append('include_inactive', params.include_inactive.toString());
    }
    const queryString = queryParams.toString();
    const url = queryString ? `/api/admin/tokens?${queryString}` : '/api/admin/tokens';
    const response = await axios.get(url);
    return response.data;
  },

  async createAdminToken(data: {
    service_name: string;
    service_description?: string;
    permissions?: string[];
    is_super_admin?: boolean;
    expires_in_days?: number;
  }) {
    const response = await axios.post('/api/admin/tokens', data);
    return response.data;
  },

  async getAdminTokenDetails(tokenId: string) {
    const response = await axios.get(`/api/admin/tokens/${tokenId}`);
    return response.data;
  },

  async revokeAdminToken(tokenId: string) {
    const response = await axios.post(`/api/admin/tokens/${tokenId}/revoke`);
    return response.data;
  },

  async rotateAdminToken(tokenId: string, data?: { expires_in_days?: number }) {
    const response = await axios.post(`/api/admin/tokens/${tokenId}/rotate`, data || {});
    return response.data;
  },

  async getAdminTokenInfo() {
    const response = await axios.get('/admin/token-info');
    return response.data;
  },

  async getAdminHealth() {
    const response = await axios.get('/admin/health');
    return response.data;
  },

  async getAdminTokenAudit(tokenId: string) {
    const response = await axios.get(`/admin/tokens/${tokenId}/audit`);
    return response.data;
  },

  async getAdminTokenUsageStats(tokenId: string) {
    const response = await axios.get(`/admin/tokens/${tokenId}/usage-stats`);
    return response.data;
  },

  async getAdminTokenProvisionedKeys(tokenId: string) {
    const response = await axios.get(`/admin/tokens/${tokenId}/provisioned-keys`);
    return response.data;
  },

  // ==================== API KEY PROVISIONING ====================
  async provisionApiKey(data: {
    user_email: string;
    tier: string;
    description?: string;
    expires_in_days?: number;
    rate_limit_requests?: number;
    rate_limit_period?: string;
  }) {
    const response = await axios.post('/admin/provision-api-key', data);
    return response.data;
  },

  async revokeApiKey(data: { key_id?: string; user_email?: string }) {
    const response = await axios.post('/admin/revoke-api-key', data);
    return response.data;
  },

  async listApiKeys(params?: {
    user_email?: string;
    active_only?: boolean;
    limit?: number;
    offset?: number;
  }) {
    const queryParams = new URLSearchParams();
    if (params?.user_email) queryParams.append('user_email', params.user_email);
    if (params?.active_only !== undefined) queryParams.append('active_only', params.active_only.toString());
    if (params?.limit) queryParams.append('limit', params.limit.toString());
    if (params?.offset) queryParams.append('offset', params.offset.toString());

    const response = await axios.get(`/admin/list-api-keys?${queryParams}`);
    return response.data;
  },

  // ==================== USER MANAGEMENT ====================
  async getPendingUsers() {
    const response = await axios.get('/api/admin/pending-users');
    return response.data.users || [];
  },

  async approveUser(userId: string, reason?: string) {
    const response = await axios.post(`/api/admin/approve-user/${userId}`, { reason });
    return response.data;
  },

  async suspendUser(userId: string, reason?: string) {
    const response = await axios.post(`/api/admin/suspend-user/${userId}`, { reason });
    return response.data;
  },

  async getAllUsers(params?: {
    status?: 'pending' | 'active' | 'suspended';
    limit?: number;
    offset?: number;
  }) {
    const queryParams = new URLSearchParams();
    if (params?.status) queryParams.append('status', params.status);
    if (params?.limit) queryParams.append('limit', params.limit.toString());
    if (params?.offset) queryParams.append('offset', params.offset.toString());

    const queryString = queryParams.toString();
    const url = queryString ? `/api/admin/users?${queryString}` : '/api/admin/users';
    const response = await axios.get(url);
    return response.data.users || [];
  },

  async resetUserPassword(userId: string): Promise<{
    success: boolean;
    temporary_password: string;
    expires_at: string;
    user_email: string;
  }> {
    const response = await axios.post(`/api/admin/users/${userId}/reset-password`);
    return response.data;
  },

  async getUserRateLimit(userId: string): Promise<{
    user_id: string;
    tier: string;
    rate_limits: {
      daily: { limit: number | null; used: number; remaining: number | null };
      monthly: { limit: number | null; used: number; remaining: number | null };
    };
    reset_times: {
      daily_reset: string;
      monthly_reset: string;
    };
  }> {
    const response = await axios.get(`/api/admin/users/${userId}/rate-limit`);
    return response.data.data;
  },

  async getUserActivity(userId: string, days: number = 30): Promise<{
    user_id: string;
    period_days: number;
    total_requests: number;
    top_tools: Array<{
      tool_name: string;
      call_count: number;
      percentage: number;
    }>;
  }> {
    const response = await axios.get(`/api/admin/users/${userId}/activity?days=${days}`);
    return response.data.data;
  },

  // ==================== PER-USER BILLING USAGE ====================
  async getUserUsage(userId: string, fromIso?: string): Promise<{
    user_id: string;
    from: string;
    by_model: Array<{
      provider: string;
      model: string;
      call_type: string;
      total_tokens: number;
      prompt_tokens: number;
      completion_tokens: number;
      calls: number;
    }>;
    daily: Array<{
      date: string;
      tokens: number;
      prompt_tokens: number;
      completion_tokens: number;
      calls: number;
    }>;
  }> {
    const qs = fromIso ? `?from=${encodeURIComponent(fromIso)}` : '';
    const response = await axios.get(`/api/admin/users/${userId}/usage${qs}`);
    return response.data;
  },

  async getUserCostTimeseries(userId: string, fromIso?: string): Promise<{
    user_id: string;
    from: string;
    daily: Array<{
      date: string;
      tokens: number;
      prompt_tokens: number;
      completion_tokens: number;
      calls: number;
    }>;
  }> {
    const qs = fromIso ? `?from=${encodeURIComponent(fromIso)}` : '';
    const response = await axios.get(`/api/admin/users/${userId}/cost-timeseries${qs}`);
    return response.data;
  },

  async setUserTier(userId: string, tier: 'starter' | 'professional' | 'enterprise'): Promise<{
    user_id: string;
    email: string;
    tier: string;
  }> {
    const response = await axios.post(`/api/admin/users/${userId}/tier`, { tier });
    return response.data.data;
  },

  async exportBillingCsv(period: string, format: 'csv' | 'json' = 'csv', limit = 1000): Promise<Blob | unknown> {
    const response = await axios.get(`/api/admin/billing/export`, {
      params: { period, format, limit },
      responseType: format === 'csv' ? 'blob' : 'json',
    });
    return response.data;
  },

  async getTenantInvoicePreview(tenantId: string, period: string): Promise<{
    tenant_id: string;
    period: string;
    by_model: Array<{
      provider: string;
      model: string;
      call_type: string;
      total_tokens: number;
      prompt_tokens: number;
      completion_tokens: number;
      calls: number;
    }>;
    daily: Array<{
      date: string;
      tokens: number;
      prompt_tokens: number;
      completion_tokens: number;
      calls: number;
    }>;
  }> {
    const response = await axios.get(`/api/admin/tenants/${tenantId}/invoice`, {
      params: { period },
    });
    return response.data;
  },

  // ==================== ADMIN SETTINGS ====================
  async getAutoApprovalSetting(): Promise<{ enabled: boolean; description: string }> {
    const response = await axios.get('/api/admin/settings/auto-approval');
    return response.data.data;
  },

  async updateAutoApprovalSetting(enabled: boolean): Promise<{ enabled: boolean; description: string }> {
    const response = await axios.put('/api/admin/settings/auto-approval', { enabled });
    return response.data.data;
  },

  // ==================== SOCIAL INSIGHTS CONFIG ====================
  async getSocialInsightsConfig(): Promise<SocialInsightsConfig> {
    const response = await axios.get('/api/admin/settings/social-insights');
    return response.data.data;
  },

  async updateSocialInsightsConfig(config: SocialInsightsConfig): Promise<SocialInsightsConfig> {
    const response = await axios.put('/api/admin/settings/social-insights', config);
    return response.data.data;
  },

  async resetSocialInsightsConfig(): Promise<SocialInsightsConfig> {
    const response = await axios.delete('/api/admin/settings/social-insights');
    return response.data.data;
  },

  // ==================== CONFIGURATION MANAGEMENT ====================
  async getConfigCatalog(tenantId?: string): Promise<{
    success: boolean;
    data: {
      total_parameters: number;
      runtime_configurable_count: number;
      static_count: number;
      categories: Array<{
        id: string;
        name: string;
        display_name: string;
        description: string;
        display_order: number;
        is_active: boolean;
        parameters: Array<{
          key: string;
          display_name: string;
          description: string;
          category: string;
          data_type: string;
          current_value: unknown;
          default_value: unknown;
          is_modified: boolean;
          valid_range?: { min?: number; max?: number; step?: number };
          enum_options?: string[];
          units?: string;
          scientific_basis?: string;
          is_runtime_configurable: boolean;
          requires_restart: boolean;
        }>;
      }>;
    };
  }> {
    const params = new URLSearchParams();
    if (tenantId) params.append('tenant_id', tenantId);
    const queryString = params.toString();
    const url = queryString ? `/api/admin/config/catalog?${queryString}` : '/api/admin/config/catalog';
    const response = await axios.get(url);
    return response.data;
  },

  async getConfigAuditLog(params?: {
    category?: string;
    config_key?: string;
    admin_user_id?: string;
    tenant_id?: string;
    limit?: number;
    offset?: number;
  }): Promise<{
    success: boolean;
    data: {
      entries: Array<{
        id: string;
        timestamp: string;
        admin_user_id: string;
        admin_email: string;
        category: string;
        config_key: string;
        old_value: unknown;
        new_value: unknown;
        data_type: string;
        reason?: string;
        tenant_id?: string;
        ip_address?: string;
        user_agent?: string;
      }>;
      total_count: number;
      offset: number;
      limit: number;
    };
  }> {
    const queryParams = new URLSearchParams();
    if (params?.category) queryParams.append('category', params.category);
    if (params?.config_key) queryParams.append('config_key', params.config_key);
    if (params?.admin_user_id) queryParams.append('admin_user_id', params.admin_user_id);
    if (params?.tenant_id) queryParams.append('tenant_id', params.tenant_id);
    if (params?.limit) queryParams.append('limit', params.limit.toString());
    if (params?.offset) queryParams.append('offset', params.offset.toString());

    const queryString = queryParams.toString();
    const url = queryString ? `/api/admin/config/audit?${queryString}` : '/api/admin/config/audit';
    const response = await axios.get(url);
    return response.data;
  },

  async updateConfig(request: {
    parameters: Record<string, unknown>;
    reason?: string;
  }, tenantId?: string): Promise<{
    success: boolean;
    data: {
      updated_count: number;
      requires_restart: boolean;
      validation_errors?: Array<{ parameter: string; error: string }>;
    };
  }> {
    const params = new URLSearchParams();
    if (tenantId) params.append('tenant_id', tenantId);
    const queryString = params.toString();
    const url = queryString ? `/api/admin/config?${queryString}` : '/api/admin/config';
    const response = await axios.put(url, request);
    return response.data;
  },

  async resetConfig(request: {
    category?: string;
    parameters?: string[];
  }, tenantId?: string): Promise<{ success: boolean; data: { reset_count: number } }> {
    const params = new URLSearchParams();
    if (tenantId) params.append('tenant_id', tenantId);
    const queryString = params.toString();
    const url = queryString ? `/api/admin/config/reset?${queryString}` : '/api/admin/config/reset';
    const response = await axios.post(url, request);
    return response.data;
  },

  // ==================== IMPERSONATION ====================
  async startImpersonation(targetUserId: string, reason?: string): Promise<{
    success: boolean;
    session_id: string;
    token: string;
    target_user: {
      id: string;
      email: string;
      display_name?: string;
      role: string;
    };
    message: string;
  }> {
    const response = await axios.post('/api/admin/impersonate', {
      target_user_id: targetUserId,
      reason,
    });
    return response.data;
  },

  async endImpersonation(): Promise<{
    success: boolean;
    message: string;
    session_id: string;
    duration_seconds: number;
  }> {
    const response = await axios.post('/api/admin/impersonate/end');
    return response.data;
  },

  async getImpersonationSessions(): Promise<{
    sessions: Array<{
      id: string;
      impersonator_id: string;
      impersonator_email?: string;
      target_user_id: string;
      target_user_email?: string;
      reason?: string;
      started_at: string;
      ended_at?: string;
      is_active: boolean;
      duration_seconds: number;
    }>;
    total_count: number;
  }> {
    const response = await axios.get('/api/admin/impersonate/sessions');
    return response.data;
  },

  // ==================== SYSTEM COACHES ====================
  async getSystemCoaches(): Promise<{
    coaches: Coach[];
    total: number;
    metadata: { timestamp: string; api_version: string };
  }> {
    const response = await axios.get('/api/admin/coaches');
    return response.data;
  },

  async createSystemCoach(data: {
    title: string;
    description?: string;
    system_prompt: string;
    category?: string;
    tags?: string[];
    visibility?: string;
    purpose?: string;
    when_to_use?: string;
    instructions?: string;
    example_inputs?: string;
    example_outputs?: string;
    success_criteria?: string;
  }): Promise<Coach> {
    const response = await axios.post('/api/admin/coaches', data);
    return response.data;
  },

  async getSystemCoach(coachId: string): Promise<Coach> {
    const response = await axios.get(`/api/admin/coaches/${coachId}`);
    return response.data;
  },

  async updateSystemCoach(coachId: string, data: {
    title?: string;
    description?: string;
    system_prompt?: string;
    category?: string;
    tags?: string[];
    purpose?: string;
    when_to_use?: string;
    instructions?: string;
    example_inputs?: string;
    example_outputs?: string;
    success_criteria?: string;
  }): Promise<Coach> {
    const response = await axios.put(`/api/admin/coaches/${coachId}`, data);
    return response.data;
  },

  async deleteSystemCoach(coachId: string): Promise<void> {
    await axios.delete(`/api/admin/coaches/${coachId}`);
  },

  async assignCoachToUsers(coachId: string, userIds: string[]): Promise<{
    coach_id: string;
    assigned_count: number;
    total_requested: number;
  }> {
    const response = await axios.post(`/api/admin/coaches/${coachId}/assign`, { user_ids: userIds });
    return response.data;
  },

  async unassignCoachFromUsers(coachId: string, userIds: string[]): Promise<{
    coach_id: string;
    removed_count: number;
    total_requested: number;
  }> {
    const response = await axios.delete(`/api/admin/coaches/${coachId}/assign`, {
      data: { user_ids: userIds },
    });
    return response.data;
  },

  async getCoachAssignments(coachId: string): Promise<{
    coach_id: string;
    assignments: Array<{
      user_id: string;
      user_email?: string;
      assigned_at: string;
      assigned_by?: string;
    }>;
  }> {
    const response = await axios.get(`/api/admin/coaches/${coachId}/assignments`);
    return response.data;
  },

  // ==================== TOOL SELECTION ====================
  async getToolCatalog(): Promise<{
    success: boolean;
    message: string;
    data: Array<{
      tool_name: string;
      display_name: string;
      description: string;
      category: string;
      default_enabled: boolean;
      is_globally_disabled: boolean;
      available_in_tiers: string[];
    }>;
  }> {
    const response = await axios.get('/api/admin/tools/catalog');
    return response.data;
  },

  async getToolCatalogEntry(toolName: string): Promise<{
    success: boolean;
    message: string;
    data: {
      tool_name: string;
      display_name: string;
      description: string;
      category: string;
      default_enabled: boolean;
      is_globally_disabled: boolean;
      available_in_tiers: string[];
    };
  }> {
    const response = await axios.get(`/api/admin/tools/catalog/${toolName}`);
    return response.data;
  },

  async getGlobalDisabledTools(): Promise<{
    success: boolean;
    message: string;
    data: { disabled_tools: string[]; count: number };
  }> {
    const response = await axios.get('/api/admin/tools/global-disabled');
    return response.data;
  },

  async getTenantTools(tenantId: string): Promise<{
    success: boolean;
    message: string;
    data: Array<{
      tool_name: string;
      display_name: string;
      description: string;
      category: string;
      is_enabled: boolean;
      source: string;
      min_plan: string;
    }>;
  }> {
    const response = await axios.get(`/api/admin/tools/tenant/${tenantId}`);
    return response.data;
  },

  async setToolOverride(
    tenantId: string,
    toolName: string,
    isEnabled: boolean,
    reason?: string
  ): Promise<{
    success: boolean;
    message: string;
    data: {
      tool_name: string;
      tenant_id: string;
      is_enabled: boolean;
      created_by: string;
      reason?: string;
      created_at: string;
    };
  }> {
    const response = await axios.post(`/api/admin/tools/tenant/${tenantId}/override`, {
      tool_name: toolName,
      is_enabled: isEnabled,
      reason,
    });
    return response.data;
  },

  async removeToolOverride(tenantId: string, toolName: string): Promise<{
    success: boolean;
    message: string;
  }> {
    const response = await axios.delete(`/api/admin/tools/tenant/${tenantId}/override/${toolName}`);
    return response.data;
  },

  async getToolAvailabilitySummary(tenantId: string): Promise<{
    success: boolean;
    message: string;
    data: {
      tenant_id: string;
      total_tools: number;
      enabled_tools: number;
      disabled_tools: number;
      overridden_tools: number;
      globally_disabled_count: number;
      plan_restricted_count: number;
    };
  }> {
    const response = await axios.get(`/api/admin/tools/tenant/${tenantId}/summary`);
    return response.data;
  },

  // ==================== STORE MANAGEMENT ====================
  async getStoreReviewQueue(): Promise<{
    coaches: Array<{
      id: string;
      title: string;
      description: string | null;
      category: string;
      tags: string[];
      sample_prompts: string[];
      token_count: number;
      install_count: number;
      icon_url: string | null;
      published_at: string | null;
      author_id: string | null;
      author_email?: string;
      system_prompt: string;
      created_at: string;
      submitted_at: string;
      publish_status: string;
    }>;
    total: number;
    metadata: { timestamp: string; api_version: string };
  }> {
    const response = await axios.get('/api/admin/store/review-queue');
    return response.data;
  },

  async getPublishedStoreCoaches(options?: {
    sort_by?: 'newest' | 'most_installed';
    limit?: number;
    offset?: number;
  }): Promise<{
    coaches: Array<{
      id: string;
      title: string;
      description: string | null;
      category: string;
      tags: string[];
      sample_prompts: string[];
      token_count: number;
      install_count: number;
      icon_url: string | null;
      published_at: string | null;
      author_id: string | null;
      author_email?: string;
      system_prompt: string;
      created_at: string;
      publish_status: string;
    }>;
    total: number;
    metadata: { timestamp: string; api_version: string };
  }> {
    const params = new URLSearchParams();
    if (options?.sort_by) params.append('sort_by', options.sort_by);
    if (options?.limit) params.append('limit', options.limit.toString());
    if (options?.offset) params.append('offset', options.offset.toString());
    const qs = params.toString();
    const response = await axios.get(`/api/admin/store/published${qs ? `?${qs}` : ''}`);
    return response.data;
  },

  async getRejectedStoreCoaches(options?: {
    limit?: number;
    offset?: number;
  }): Promise<{
    coaches: Array<{
      id: string;
      title: string;
      description: string | null;
      category: string;
      tags: string[];
      sample_prompts: string[];
      token_count: number;
      install_count: number;
      icon_url: string | null;
      published_at: string | null;
      author_id: string | null;
      author_email?: string;
      system_prompt: string;
      created_at: string;
      rejected_at: string;
      rejection_reason: string;
      rejection_notes?: string;
      publish_status: string;
    }>;
    total: number;
    metadata: { timestamp: string; api_version: string };
  }> {
    const params = new URLSearchParams();
    if (options?.limit) params.append('limit', options.limit.toString());
    if (options?.offset) params.append('offset', options.offset.toString());
    const qs = params.toString();
    const response = await axios.get(`/api/admin/store/rejected${qs ? `?${qs}` : ''}`);
    return response.data;
  },

  async getStoreStats(): Promise<{
    pending_count: number;
    published_count: number;
    rejected_count: number;
    total_installs: number;
    rejection_rate: number;
  }> {
    const response = await axios.get('/api/admin/store/stats');
    return response.data;
  },

  async approveStoreCoach(coachId: string): Promise<{
    success: boolean;
    message: string;
    coach_id: string;
  }> {
    const response = await axios.post(`/api/admin/store/coaches/${coachId}/approve`);
    return response.data;
  },

  async rejectStoreCoach(coachId: string, reason: string, notes?: string): Promise<{
    success: boolean;
    message: string;
    coach_id: string;
  }> {
    const response = await axios.post(`/api/admin/store/coaches/${coachId}/reject`, { reason, notes });
    return response.data;
  },

  async unpublishStoreCoach(coachId: string): Promise<{
    success: boolean;
    message: string;
    coach_id: string;
  }> {
    const response = await axios.post(`/api/admin/store/coaches/${coachId}/unpublish`);
    return response.data;
  },

  // ==================== CONTREMAITRE (PROMPT HOT-RELOAD) ====================

  async getContremaitreStatus(): Promise<{
    configured: boolean;
    repo: string | null;
    branch: string | null;
    system_prompt_count: number;
    coach_prompt_count: number;
    compiled_in_count: number;
    contremaitre_count: number;
  }> {
    const response = await axios.get('/api/admin/contremaitre/status');
    return response.data;
  },

  async listSystemPrompts(): Promise<{
    total: number;
    prompts: Array<{
      key: string;
      sha256: string;
      source: 'compiled_in' | 'contremaitre';
      loaded_at: string;
      content_length: number;
    }>;
  }> {
    const response = await axios.get('/api/admin/contremaitre/prompts');
    return response.data;
  },

  async getSystemPrompt(key: string): Promise<{
    key: string;
    content: string;
    sha256: string;
    source: 'compiled_in' | 'contremaitre';
    loaded_at: string;
  }> {
    const response = await axios.get(`/api/admin/contremaitre/prompts/${key}`);
    return response.data;
  },

  async updateSystemPrompt(
    key: string,
    data: { content: string; commit_message?: string },
  ): Promise<{
    success: boolean;
    sha256: string;
    commit_sha: string | null;
  }> {
    const response = await axios.put(`/api/admin/contremaitre/prompts/${key}`, data);
    return response.data;
  },

  async triggerContremaitreSync(): Promise<{
    success: boolean;
    synced: number;
    skipped: number;
    failed: number;
  }> {
    const response = await axios.post('/api/admin/contremaitre/sync');
    return response.data;
  },

  async promoteCoachToContremaitre(coachId: string): Promise<{
    success: boolean;
    path: string;
    commit_sha: string | null;
  }> {
    const response = await axios.post(`/api/admin/contremaitre/coaches/${coachId}/promote`);
    return response.data;
  },

  // ==================== HARNESS CONFIG (Phase B C3) ====================
  async getHarnessConfig(): Promise<HarnessConfigResponse> {
    const response = await axios.get('/api/admin/settings/harness');
    return response.data;
  },

  async putHarnessConfig(config: HarnessConfigDocument): Promise<HarnessConfigResponse> {
    const response = await axios.put('/api/admin/settings/harness', config);
    return response.data;
  },

  // ==================== TIER 5.5 CLAIM VERDICTS ====================
  async listClaimVerdicts(params: {
    tenant_id: string;
    status?: string;
    category?: string;
    coach_id?: string;
    limit?: number;
  }): Promise<{
    verdicts: ClaimVerdictRow[];
    total: number;
  }> {
    const query = new URLSearchParams();
    query.append('tenant_id', params.tenant_id);
    if (params.status) query.append('status', params.status);
    if (params.category) query.append('category', params.category);
    if (params.coach_id) query.append('coach_id', params.coach_id);
    if (params.limit !== undefined) query.append('limit', String(params.limit));
    const response = await axios.get(`/api/admin/claim-verdicts?${query.toString()}`);
    return response.data;
  },

  async listVerdictsForConversation(
    conversationId: string,
    tenantId: string,
  ): Promise<{
    verdicts: ClaimVerdictRow[];
    total: number;
  }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    const response = await axios.get(
      `/api/admin/claim-verdicts/conversations/${conversationId}?${query.toString()}`,
    );
    return response.data;
  },

  // ==================== MEMORY EXTRACTION WORKER (Phase B C6) ====================
  async getMemoryWorkerMetrics(tenantId: string): Promise<MemoryWorkerMetricsResponse> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    const response = await axios.get(`/api/admin/memory/worker-metrics?${query.toString()}`);
    return response.data;
  },

  // ==================== COACH FOLLOWUP TRIAGE (Phase B C7) ====================
  async listPendingFollowups(
    tenantId: string,
    limit?: number,
  ): Promise<{ followups: FollowupRow[]; total: number }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    if (limit !== undefined) {
      query.append('limit', String(limit));
    }
    const response = await axios.get(`/api/admin/coach-followups/pending?${query.toString()}`);
    return response.data;
  },

  async cancelFollowup(
    followupId: string,
    tenantId: string,
  ): Promise<{ cancelled: boolean }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    const response = await axios.post(
      `/api/admin/coach-followups/${followupId}/cancel?${query.toString()}`,
    );
    return response.data;
  },

  // ==================== COACH NOTES AUDIT (Phase B C8) ====================
  async listCoachNoteAudit(
    tenantId: string,
    limit?: number,
  ): Promise<{ notes: CoachNoteAuditRow[]; total: number }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    if (limit !== undefined) {
      query.append('limit', String(limit));
    }
    const response = await axios.get(`/api/admin/coach-notes/audit?${query.toString()}`);
    return response.data;
  },

  // ==================== MYTH-BUSTING SUMMARY (Phase D C13) ====================
  async getMythBustingSummary(
    tenantId: string,
    limit?: number,
  ): Promise<MythBustingSummary> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    if (limit !== undefined) {
      query.append('limit', String(limit));
    }
    const response = await axios.get(`/api/admin/myth-busting/summary?${query.toString()}`);
    return response.data;
  },

  /**
   * Promote a recurring unsupported claim phrase into Harness Config's
   * `blocked_topics` so the chat pipeline rejects future replies that
   * include it. Idempotent: re-promoting an existing topic returns
   * `added: false` and skips the registry write.
   */
  async promoteMythBustingTopic(topic: string): Promise<{
    topic: string;
    added: boolean;
    total_blocked_topics: number;
  }> {
    const response = await axios.post(
      `/api/admin/myth-busting/promote-topic`,
      { topic },
    );
    return response.data;
  },

  /**
   * Suppress an audited coach note so memory recall stops surfacing it
   * to the coach. The audit panel still shows the row (suppressed=true)
   * for review; admins can call `unsuppressCoachNote` to restore it.
   */
  async suppressCoachNote(
    tenantId: string,
    noteId: string,
  ): Promise<{ note_id: string; suppressed: boolean; changed: boolean }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    const response = await axios.post(
      `/api/admin/coach-notes/${encodeURIComponent(noteId)}/suppress?${query.toString()}`,
    );
    return response.data;
  },

  /** Inverse of [[suppressCoachNote]]. */
  async unsuppressCoachNote(
    tenantId: string,
    noteId: string,
  ): Promise<{ note_id: string; suppressed: boolean; changed: boolean }> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    const response = await axios.post(
      `/api/admin/coach-notes/${encodeURIComponent(noteId)}/unsuppress?${query.toString()}`,
    );
    return response.data;
  },

  // ==================== COACH GRADING SUMMARY (Phase D C14) ====================
  async getCoachGradingSummary(
    tenantId: string,
    limit?: number,
  ): Promise<CoachGradingSummary> {
    const query = new URLSearchParams({ tenant_id: tenantId });
    if (limit !== undefined) {
      query.append('limit', String(limit));
    }
    const response = await axios.get(`/api/admin/coach-grading/summary?${query.toString()}`);
    return response.data;
  },

  // ==================== EVAL HARNESS FIXTURE BROWSER (Phase B C16) ====================
  async getEvalFixtureBrowser(): Promise<EvalFixtureBrowserResponse> {
    const response = await axios.get('/api/admin/evals/fixtures');
    return response.data;
  },

  /** Fetch Tier 5.5 verdict calibration stats for the calibration panel (Sprint C20). */
  async getVerdictCalibrationStats(
    tenantId: string,
    windowDays = 30,
  ): Promise<VerdictCalibrationStats> {
    const query = new URLSearchParams({
      tenant_id: tenantId,
      window_days: String(windowDays),
    });
    const response = await axios.get(`/api/admin/evals/verdict-stats?${query.toString()}`);
    return response.data;
  },

  /** Fetch the raw JSONL body of a single fixture for inline editing. */
  async getEvalFixtureContent(name: string): Promise<EvalFixtureContentResponse> {
    const response = await axios.get(
      `/api/admin/evals/fixtures/${encodeURIComponent(name)}`,
    );
    return response.data;
  },

  /** Write the raw JSONL body of a fixture (creating it if missing). */
  async putEvalFixture(name: string, body: string): Promise<EvalFixtureSummary> {
    const response = await axios.put(
      `/api/admin/evals/fixtures/${encodeURIComponent(name)}`,
      { body },
    );
    return response.data;
  },

  /** Delete a fixture by name. */
  async deleteEvalFixture(name: string): Promise<void> {
    await axios.delete(`/api/admin/evals/fixtures/${encodeURIComponent(name)}`);
  },
};

/** Per-case summary row from the eval fixture browser. */
export interface EvalFixtureCaseSummary {
  id: string;
  label: string;
  persona: string;
  turn_count: number;
  must_contain_total: number;
  must_not_contain_total: number;
}

/** Per-fixture summary from the eval fixture browser. */
export interface EvalFixtureSummary {
  name: string;
  path: string;
  case_count: number;
  personas: string[];
  cases: EvalFixtureCaseSummary[];
}

/** Top-level response for `GET /admin/evals/fixtures`. */
export interface EvalFixtureBrowserResponse {
  scanned_dir: string;
  fixture_count: number;
  case_total: number;
  fixtures: EvalFixtureSummary[];
}

/** Response from `GET /admin/evals/fixtures/{name}`. */
export interface EvalFixtureContentResponse {
  name: string;
  body: string;
}

/** Status-level counters emitted by `GET /admin/evals/verdict-stats`. */
export interface VerdictStatusBreakdown {
  supported: number;
  unsupported: number;
  contradicted: number;
  rhetorical: number;
  unverifiable: number;
}

/** One day's verdict counts for the calibration panel. */
export interface VerdictDailyBucket {
  date: string;
  counts: VerdictStatusBreakdown;
}

/** Verdict calibration rollup for a tenant over the last `window_days`. */
export interface VerdictCalibrationStats {
  window_start: string;
  window_days: number;
  totals: VerdictStatusBreakdown;
  daily: VerdictDailyBucket[];
}

/** Compaction tunables persisted with the harness config document. */
export interface HarnessCompactionConfig {
  window_tokens: number;
  warn_threshold: number;
  emergency_threshold: number;
  summarize_oldest_n: number;
  sliding_drop_n: number;
}

/** Tier 6 text guardrail tunables. */
export interface HarnessGuardrailsConfig {
  max_response_chars: number;
  blocked_topics: string[];
  disclaimer_triggers: string[];
  disclaimer_text: string;
}

/** Top-level harness config document persisted under `system_settings`. */
export interface HarnessConfigDocument {
  schema_version: number;
  compaction: HarnessCompactionConfig;
  guardrails: HarnessGuardrailsConfig;
}

/** Wire response wrapper for harness config GET / PUT. */
export interface HarnessConfigResponse {
  config: HarnessConfigDocument;
  source: 'persisted' | 'default';
  updated_at: string | null;
}

/**
 * Aggregate health snapshot for the memory extraction worker.
 *
 * Derived from `user_facts` rows on the server rather than worker instrumentation,
 * because the extraction pipeline is a fire-and-forget background task.
 */
export interface UserFactMetrics {
  total_facts: number;
  facts_last_24h: number;
  facts_last_7d: number;
  distinct_users: number;
  facts_by_kind: Record<string, number>;
  newest_updated_at: string | null;
}

/** Envelope returned by `GET /admin/memory/worker-metrics`. */
export interface MemoryWorkerMetricsResponse {
  tenant_id: string;
  metrics: UserFactMetrics;
}

/** Aggregated stat for a recurring offending claim. */
export interface ClaimPattern {
  claim_excerpt: string;
  occurrences: number;
  coach_count: number;
  last_seen_at: string | null;
}

/** Aggregated stat for a coach with recurring unsupported claims. */
export interface CoachPattern {
  coach_id: string;
  unsupported_total: number;
  categories: string[];
}

/** Aggregated stat for a claim category. */
export interface CategoryPattern {
  category: string;
  flagged_total: number;
  coach_count: number;
}

/** Top-level wire response for `GET /admin/myth-busting/summary`. */
export interface MythBustingSummary {
  tenant_id: string;
  verdicts_scanned: number;
  flagged_total: number;
  top_claims: ClaimPattern[];
  top_coaches: CoachPattern[];
  top_categories: CategoryPattern[];
}

/** Letter grade for a coach based on Tier 5.5 verdict history. */
export type LetterGrade = 'A' | 'B' | 'C' | 'D' | 'F' | 'PROVISIONAL';

/** Per-coach grade row from `GET /admin/coach-grading/summary`. */
export interface CoachGrade {
  coach_id: string;
  total_verdicts: number;
  supported: number;
  unsupported: number;
  contradicted: number;
  rhetorical: number;
  unverifiable: number;
  score: number;
  grade: LetterGrade;
}

/** Top-level wire response for `GET /admin/coach-grading/summary`. */
export interface CoachGradingSummary {
  tenant_id: string;
  verdicts_scanned: number;
  grades: CoachGrade[];
}

/**
 * Wire representation of a coach note row for the admin compliance audit log.
 *
 * Notes are personal context the coach persona derived about a user from
 * conversations, so the audit surface gates behind `ViewAuditLogs`.
 */
export interface CoachNoteAuditRow {
  id: string;
  tenant_id: string;
  user_id: string;
  coach_id: string;
  conversation_id: string | null;
  scope: 'conversation' | 'user' | 'tenant';
  content: string;
  created_at: string;
  updated_at: string;
  /**
   * `true` when an admin has flagged this note as off-policy. Memory
   * recall queries skip suppressed rows; the audit panel still renders
   * them so admins can un-suppress later.
   */
  suppressed: boolean;
}

/**
 * Wire representation of a coach followup row for the admin triage tab.
 *
 * The server flattens the domain `FollowupStatus` enum to its stable
 * snake_case string so the client does not need to map enum variants.
 */
export interface FollowupRow {
  id: string;
  tenant_id: string;
  user_id: string;
  coach_id: string;
  conversation_id: string | null;
  content: string;
  due_at: string | null;
  status: 'pending' | 'delivered' | 'cancelled';
  created_at: string;
  updated_at: string;
  delivered_at: string | null;
}

/** Wire-format representation of a Tier 5.5 claim verdict row. */
export interface ClaimVerdictRow {
  id: string;
  tenant_id: string;
  user_id: string;
  coach_id: string | null;
  conversation_id: string | null;
  message_id: string | null;
  claim_text: string;
  category: 'physiological' | 'training_prescription' | 'nutrition' | 'recovery' | 'supplement' | 'injury_rehab';
  status: 'supported' | 'unsupported' | 'contradicted' | 'rhetorical' | 'unverifiable';
  evidence_strength: 'strong' | 'mixed' | 'weak' | 'none';
  confidence: number;
  layer_fired: 'rhetoric' | 'deterministic' | 'evidence' | 'consistency' | 'judge';
  explanation: string | null;
  evidence_refs: string | null;
  created_at: string;
}
