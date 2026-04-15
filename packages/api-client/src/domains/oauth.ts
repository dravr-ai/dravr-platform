// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: OAuth domain API - provider connections and authorization
// ABOUTME: Handles OAuth flow initiation and connection status

import type { AxiosInstance, AxiosResponse, AxiosRequestConfig } from 'axios';
import type {
  ProviderStatus,
  ExtendedProviderStatus,
  ProvidersStatusResponse,
  ApiMetadata,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Backpressure retry parameters for the sciotte provider. The platform
// sheds load with HTTP 503 + `Retry-After` when its Chrome queue is
// saturated; clients retry with jittered exponential backoff so a
// thundering herd at t+Retry-After doesn't re-saturate the pod on every
// synchronized tick. Values are fixed per-release — operators tune the
// server-side limit via PIERRE_SCIOTTE_* env vars, not the client.
const SCIOTTE_RETRY_MAX_ATTEMPTS = 3;
const SCIOTTE_RETRY_BASE_MS = 1000;
const SCIOTTE_RETRY_JITTER_MS = 500;
const SCIOTTE_BUSY_STATUS = 503;

interface SciotteBusyErrorShape {
  response?: {
    status?: number;
    headers?: Record<string, string | number | undefined>;
    data?: { error?: string; reason?: string; retry_after_secs?: number };
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * POST wrapper that retries on HTTP 503 from the sciotte backpressure
 * limiter. Honours the server's `Retry-After` header as the lower bound
 * on the delay and adds random jitter so concurrent clients don't
 * synchronise their retries. Any non-503 error propagates immediately;
 * the final axios error is re-thrown after `SCIOTTE_RETRY_MAX_ATTEMPTS`
 * so the caller's existing error handling still surfaces it to the UI.
 */
async function postWithSciotteBackpressureRetry<TResponse, TBody>(
  axios: AxiosInstance,
  path: string,
  body: TBody,
  config: AxiosRequestConfig,
): Promise<AxiosResponse<TResponse>> {
  let attempt = 0;
  while (true) {
    try {
      return await axios.post<TResponse>(path, body, config);
    } catch (err) {
      const typed = err as SciotteBusyErrorShape;
      const status = typed.response?.status;
      if (status !== SCIOTTE_BUSY_STATUS || attempt >= SCIOTTE_RETRY_MAX_ATTEMPTS) {
        throw err;
      }
      const retryAfterHeader = typed.response?.headers?.['retry-after'];
      const retryAfterFromBody = typed.response?.data?.retry_after_secs;
      const retryAfterSecs =
        Number(retryAfterHeader ?? retryAfterFromBody ?? 0) || 0;
      const exponentialMs = SCIOTTE_RETRY_BASE_MS * 2 ** attempt;
      const jitterMs = Math.random() * SCIOTTE_RETRY_JITTER_MS;
      const delayMs = Math.max(retryAfterSecs * 1000, exponentialMs) + jitterMs;
      attempt += 1;
      await sleep(delayMs);
    }
  }
}

// Re-export for consumers
export type { ProviderStatus, ExtendedProviderStatus, ProvidersStatusResponse };

// Extended provider status with additional OAuth details
export interface OAuthProvider extends ProviderStatus {
  connected_at?: string;
  expires_at?: string;
  scopes?: string[];
}

export interface OAuthStatusResponse {
  providers: ProviderStatus[];
  metadata?: ApiMetadata;
}

export interface MobileOAuthInitResponse {
  authorization_url: string;
  state: string;
  code_verifier?: string;
  metadata: ApiMetadata;
}

/**
 * Creates the OAuth API methods bound to an axios instance.
 */
export function createOAuthApi(axios: AxiosInstance, getBaseUrl: () => string) {
  return {
    /**
     * Get the connection status of all OAuth providers.
     * Note: Backend returns array directly, we wrap it for consistency with type.
     */
    async getStatus(): Promise<OAuthStatusResponse> {
      const response = await axios.get<ProviderStatus[]>(ENDPOINTS.OAUTH.STATUS);
      // Backend returns array directly, wrap for consistency
      return { providers: response.data };
    },

    /**
     * Get the OAuth authorization URL for a provider (web).
     */
    getAuthorizeUrl(provider: string, redirectUri?: string): string {
      const baseUrl = getBaseUrl();
      const url = new URL(`${baseUrl}${ENDPOINTS.OAUTH.AUTHORIZE(provider)}`);
      if (redirectUri) {
        url.searchParams.set('redirect_uri', redirectUri);
      }
      return url.toString();
    },

    /**
     * Initialize mobile OAuth flow for a provider.
     * Returns the authorization URL and PKCE parameters.
     */
    async initMobileOAuth(
      provider: string,
      redirectUri?: string
    ): Promise<MobileOAuthInitResponse> {
      const params = new URLSearchParams();
      if (redirectUri) {
        params.set('redirect_uri', redirectUri);
      }

      const queryString = params.toString();
      const url = queryString
        ? `${ENDPOINTS.OAUTH.MOBILE_INIT(provider)}?${queryString}`
        : ENDPOINTS.OAUTH.MOBILE_INIT(provider);

      const response = await axios.get<MobileOAuthInitResponse>(url);
      return response.data;
    },

    /**
     * Get all providers (OAuth and non-OAuth) with connection status.
     * Includes synthetic providers and other non-OAuth data sources.
     */
    async getProvidersStatus(): Promise<ProvidersStatusResponse> {
      const response = await axios.get<ProvidersStatusResponse>(ENDPOINTS.PROVIDERS.STATUS);
      return response.data;
    },

    /**
     * Disconnect a provider by deleting stored OAuth tokens.
     */
    async disconnectProvider(provider: string): Promise<void> {
      await axios.delete(ENDPOINTS.OAUTH.DISCONNECT(provider));
    },

    /**
     * Get the OAuth authorization URL for a provider from the server.
     * Calls the mobile/init endpoint and returns just the URL string.
     * Used by web frontend for popup OAuth flow.
     */
    async getAuthorizeUrlForProvider(provider: string): Promise<string> {
      const response = await this.initMobileOAuth(provider);
      return response.authorization_url;
    },

    /**
     * Start Sciotte credential login flow (headless browser).
     * Returns status: 'connected', 'two_factor_choice', 'otp_required', 'number_match', or error.
     */
    async sciotteLogin(params: {
      email: string;
      password: string;
      method: 'email' | 'google' | 'apple';
      target: 'strava' | 'garmin';
    }): Promise<SciotteLoginResponse> {
      const response = await postWithSciotteBackpressureRetry<
        SciotteLoginResponse,
        typeof params
      >(axios, '/api/providers/sciotte/login', params, { timeout: 1_200_000 });
      return response.data;
    },

    /**
     * Select a 2FA option during Sciotte login.
     */
    async sciotteSelect2FA(optionId: string): Promise<SciotteLoginResponse> {
      const response = await postWithSciotteBackpressureRetry<
        SciotteLoginResponse,
        { option_id: string }
      >(
        axios,
        '/api/providers/sciotte/select-2fa',
        { option_id: optionId },
        { timeout: 1_200_000 },
      );
      return response.data;
    },

    /**
     * Submit OTP code during Sciotte login.
     */
    async sciotteSubmitOTP(code: string): Promise<SciotteLoginResponse> {
      const response = await postWithSciotteBackpressureRetry<
        SciotteLoginResponse,
        { code: string }
      >(
        axios,
        '/api/providers/sciotte/submit-otp',
        { code },
        { timeout: 1_200_000 },
      );
      return response.data;
    },

    // Aliases for backward compatibility
    getOAuthStatus() {
      return this.getStatus();
    },

    getOAuthAuthorizeUrl(provider: string, redirectUri?: string) {
      return this.getAuthorizeUrl(provider, redirectUri);
    },
  };
}

export interface SciotteLoginResponse {
  status: 'connected' | 'two_factor_choice' | 'otp_required' | 'number_match' | 'error';
  error?: string;
  options?: Array<{ id: string; label: string }>;
  number?: string;
}

export type OAuthApi = ReturnType<typeof createOAuthApi>;
