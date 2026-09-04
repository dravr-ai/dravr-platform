// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Core API client factory with platform-agnostic axios configuration
// ABOUTME: Handles auth interceptors, CSRF tokens, and error handling

import * as axiosModule from 'axios';
import type { AxiosInstance, AxiosResponse, AxiosError, InternalAxiosRequestConfig } from 'axios';

// Handle both ESM and CJS imports (for test environment compatibility)
const axios = axiosModule.default ?? axiosModule;
import type { PlatformAdapter, ApiClientOptions } from '../types/platform';
import { readHeader, recoverFromRefusal } from './auth-challenge';

/**
 * Creates an axios instance configured with the platform adapter.
 * Handles authentication, CSRF tokens, and auth failure detection.
 */
export function createAxiosClient(adapter: PlatformAdapter): AxiosInstance {
  const { httpConfig, authStorage, authFailure } = adapter;

  const instance = axios.create({
    baseURL: httpConfig.baseURL,
    timeout: httpConfig.timeout ?? 30000,
    withCredentials: httpConfig.withCredentials ?? false,
    headers: {
      'Content-Type': 'application/json',
      ...httpConfig.defaultHeaders,
    },
  });

  // Request interceptor: Add auth token and CSRF token
  instance.interceptors.request.use(
    async (config: InternalAxiosRequestConfig) => {
      // Add JWT token for mobile (web uses httpOnly cookies)
      if (adapter.platform === 'mobile') {
        const token = await authStorage.getToken();
        if (token && config.headers) {
          config.headers.Authorization = `Bearer ${token}`;
        }
      }

      // Add CSRF token for state-changing requests
      const csrfToken = await authStorage.getCsrfToken();
      if (csrfToken && config.headers) {
        config.headers['X-CSRF-Token'] = csrfToken;
      }

      // Tell the server which first-party surface this request came
      // from so chat slash-command dispatch routes
      // PlatformCommandContext.channel_type correctly (analytics +
      // handler branching on /coach, /group, ...).
      if (config.headers && !config.headers['X-Client-Platform']) {
        config.headers['X-Client-Platform'] = adapter.platform;
      }

      return config;
    },
    (error: AxiosError) => Promise.reject(error)
  );

  // Response interceptor: Handle auth failures and extract CSRF tokens
  instance.interceptors.response.use(
    (response: AxiosResponse) => {
      // Extract CSRF token from response headers if present
      const csrfToken = response.headers['x-csrf-token'];
      if (csrfToken) {
        // Fire and forget - don't await
        authStorage.setCsrfToken(csrfToken).catch(() => {
          // Silently ignore storage errors
        });
      }
      return response;
    },
    async (error: AxiosError) => {
      // A refused request recovers by status AND challenge, never by status
      // alone. Keying recovery on 401 was enough only while 401 was the sole
      // way a credential could become insufficient: an authorization change
      // that narrows a still-valid session answers 403, which fell through
      // here as an ordinary failure and left the session stranded until the
      // token expired on its own (JWT_EXPIRY_HOURS=24).
      //
      // The two 403s are not interchangeable. A grant too narrow for the
      // request is fixed by getting a new one; a role refusal is not, and
      // clearing the session over it signs the athlete out of a permission
      // they were never going to have — then refuses them again after they log
      // back in. So only the challenge the server sent decides.
      //
      // Clearing the session and marking the error are one step, so an app-wide
      // error surface can ask whether this refusal is already being recovered
      // instead of guessing from a symptom.
      await recoverFromRefusal(
        { authStorage, authFailure },
        error.response?.status ?? 0,
        readHeader(error.response?.headers, 'www-authenticate'),
        error
      );
      return Promise.reject(error);
    }
  );

  return instance;
}

/**
 * API client instance with access to the underlying axios instance
 * and platform adapter for domain APIs to use.
 */
export interface ApiClient {
  /** Configured axios instance */
  axios: AxiosInstance;
  /** Platform adapter for auth operations */
  adapter: PlatformAdapter;
}

/**
 * Creates an API client with the given options.
 */
export function createApiClient(options: ApiClientOptions): ApiClient {
  const axiosInstance = options.axiosInstance ?? createAxiosClient(options.adapter);

  return {
    axios: axiosInstance,
    adapter: options.adapter,
  };
}
