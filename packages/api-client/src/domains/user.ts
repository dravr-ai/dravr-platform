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
  ThemePreference,
  User,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { CoachingPersona, McpToken, User };

// Types - aligned with actual backend responses

/**
 * A locale the platform speaks end to end: `SUPPORTED_LOCALES` on
 * `PUT /api/user/locale`, and one column in the five-locale messaging string
 * table. `fr` is the default the server falls back to.
 */
export type SupportedLocale = 'fr' | 'en' | 'es' | 'de' | 'pt';

/** Echo returned by `PUT /api/user/locale` confirming what was stored. */
export interface UpdateLocaleResponse {
  message: string;
  locale: SupportedLocale;
}

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

export interface SystemProviderInfo {
  name: string;
  display_name: string;
  model?: string;
}

export interface LlmSettingsResponse {
  current_provider: string | null;
  providers: LlmProviderStatus[];
  user_credentials: LlmCredentialSummary[];
  tenant_credentials: LlmCredentialSummary[];
  system_provider?: SystemProviderInfo;
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
 * Response from `GET /api/me/onboarding-status`.
 *
 * `needs_provider_connection` is `true` when the caller has zero rows in
 * `provider_connections` and must complete the onboarding flow before the
 * messaging endpoints will accept their requests.
 */
/** A single onboarding step's persisted status (server-driven progress). */
export interface OnboardingStepState {
  /** Step id: `profile_type`, `connect_provider`, `coach_proposal`, `messaging_channel`, `messaging_configure`. */
  step_id: string;
  /** `complete` or `skipped`. */
  status: string;
}

export interface OnboardingStatusResponse {
  needs_provider_connection: boolean;
  /** How many onboarding topics (North Star + 6 pillars) the user has covered. */
  pillars_covered: number;
  /** Total onboarding topics (North Star + 6 pillars = 7). */
  pillars_total: number;
  /** `true` once all pillar context + North Star are captured. */
  onboarding_complete: boolean;
  /** Durable per-step progress; only steps the user has reached appear. */
  steps: OnboardingStepState[];
  /** The messaging channel the user chose during onboarding, if any. */
  chosen_channel: string | null;
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
     * Persist the user's IANA timezone so the chat prompt can render
     * `{{CURRENT_DATE}}` in their local calendar day. Callers pass
     * `Intl.DateTimeFormat().resolvedOptions().timeZone`; the server
     * validates the string parses as a known timezone before storing
     * it. Best-effort — non-fatal failures (e.g. timezone column
     * absent on an older deploy) should not block login flows.
     */
    async setTimezone(timezone: string): Promise<{ timezone: string }> {
      const response = await axios.put<{ timezone: string }>(ENDPOINTS.USER.TIMEZONE, {
        timezone,
      });
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

    // ==================== LOCALE ====================

    /**
     * Set the language the coach answers in (`users.locale`).
     *
     * The server owns reply language; the clients own chrome language. This
     * is the one call that keeps them equal — the language switcher fires it
     * on every change. The server accepts `fr`, `en`, `es`, `de` and `pt` and
     * rejects anything else with `400`, so an unsupported tag surfaces as a
     * failed switch rather than a silently ignored preference.
     */
    async updateLocale(locale: SupportedLocale): Promise<UpdateLocaleResponse> {
      const response = await axios.put<UpdateLocaleResponse>(ENDPOINTS.USER.LOCALE, {
        locale,
      });
      return response.data;
    },

    // ==================== THEME ====================

    /**
     * Persist the viewer's theme choice (`users.theme`) via
     * `PUT /api/user/theme`. `light` / `dark` pin a scheme across devices;
     * `null` means "follow the system". The server answers `204 No Content`,
     * so there is nothing to return — callers fire this alongside the local
     * theme flip and must never let a failed write block that flip.
     */
    async updateTheme(theme: ThemePreference): Promise<void> {
      await axios.put(ENDPOINTS.USER.THEME, { theme });
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

    /** The seven PAR-Q+ pre-participation questions. */
    async getParqQuestions(): Promise<{ questions: Array<{ id: string; text: string }> }> {
      const response = await axios.get<{ questions: Array<{ id: string; text: string }> }>(
        ENDPOINTS.USER.PARQ,
      );
      return response.data;
    },

    /**
     * Submit PAR-Q answers. Each "yes" raises a coach-visible medical flag with a
     * 12-month freshness horizon; a "yes" never blocks sign-up.
     */
    async submitParq(
      answers: Array<{ id: string; yes: boolean }>,
    ): Promise<{ flags_raised: number }> {
      const response = await axios.post<{ flags_raised: number }>(ENDPOINTS.USER.PARQ, {
        answers,
      });
      return response.data;
    },

    /**
     * Persist the about-you onboarding answers as onboarding facts. Every field
     * is optional — a partial answer is worth strictly more than none.
     */
    async saveAboutYou(answers: {
      north_star?: string;
      primary_sport?: string;
      goal?: string;
    }): Promise<{ facts_written: number }> {
      const response = await axios.post<{ facts_written: number }>(
        ENDPOINTS.USER.ABOUT_YOU,
        answers,
      );
      return response.data;
    },

    /**
     * Cheap self-read used by web + mobile to decide, right after login,
     * whether to render the onboarding screen or route to the main UI.
     *
     * Returns `{ needs_provider_connection: true }` when the caller has zero
     * rows in `provider_connections` — the same source of truth the messaging
     * endpoints use, so the gate and the redirect can never drift.
     */
    async getOnboardingStatus(): Promise<OnboardingStatusResponse> {
      const response = await axios.get<OnboardingStatusResponse>(
        ENDPOINTS.USER.ONBOARDING_STATUS,
      );
      return response.data;
    },

    /**
     * Persist an onboarding step's completion status so the flow is durable and
     * follows the user across devices (rather than living only in localStorage).
     * `chosenChannel` is set only for the messaging-channel step. Returns nothing
     * (204); callers treat failure as non-fatal — the local flag still advances
     * the flow, the server write is a durability best-effort.
     */
    async setOnboardingStep(
      stepId: string,
      status: 'complete' | 'skipped',
      chosenChannel?: string,
    ): Promise<void> {
      await axios.put(ENDPOINTS.USER.ONBOARDING_STEP(stepId), {
        status,
        chosen_channel: chosenChannel,
      });
    },
  };
}

/**
 * Wire shape for a single user_facts row served to the memory panel.
 *
 * `kind` is the server's `FactKind` serde string — the nine values
 * `@pierre/shared-constants` lists as `MEMORY_FACT_KINDS`. `coach_title` is
 * the coach behind `coach_id`, joined by the server so the panel can name the
 * coach; a fact no coach authored, or whose coach is gone, carries none.
 */
export interface MemoryFactRow {
  id: string;
  coach_id: string | null;
  coach_title: string | null;
  kind:
    | 'preference'
    | 'physiology'
    | 'injury'
    | 'goal'
    | 'schedule'
    | 'equipment'
    | 'north_star'
    | 'medical'
    | 'other';
  /** What the fact says, as a `PredicateCode` slug (`training_for`, `states`, ...). */
  predicate_code: string;
  /** The athlete's own words for the value, in their language. */
  object: string;
  /** The whole fact as one sentence in the athlete's locale, rendered by the server. */
  sentence: string;
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
