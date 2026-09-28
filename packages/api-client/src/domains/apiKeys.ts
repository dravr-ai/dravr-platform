// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API keys domain API — the athlete's own keys: list, create (the key comes back once), revoke, and one key's usage
// ABOUTME: Both settings panes call it; the operator-side key provisioning stays in the admin console

import type { AxiosInstance } from 'axios';
import type {
  ApiKeyCreateResponse,
  ApiKeyDeactivateResponse,
  ApiKeyListResponse,
  ApiKeyUsageResponse,
  CreateApiKeyRequest,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type {
  ApiKeyCreateResponse,
  ApiKeyDeactivateResponse,
  ApiKeyListResponse,
  ApiKeyUsageResponse,
  CreateApiKeyRequest,
};

/**
 * Creates the API keys API bound to an axios instance.
 */
export function createApiKeysApi(axios: AxiosInstance) {
  return {
    /** Every key the caller holds, revoked ones included (`is_active: false`). */
    async list(): Promise<ApiKeyListResponse> {
      const response = await axios.get<ApiKeyListResponse>(ENDPOINTS.API_KEYS.LIST);
      return response.data;
    },

    /** Create a key. The response is the only place the full key ever appears. */
    async create(request: CreateApiKeyRequest): Promise<ApiKeyCreateResponse> {
      const response = await axios.post<ApiKeyCreateResponse>(ENDPOINTS.API_KEYS.LIST, request);
      return response.data;
    },

    /** Revoke a key: anything using it stops working at once. */
    async revoke(keyId: string): Promise<ApiKeyDeactivateResponse> {
      const response = await axios.delete<ApiKeyDeactivateResponse>(ENDPOINTS.API_KEYS.KEY(keyId));
      return response.data;
    },

    /** One key's calls between two instants (RFC 3339). */
    async usage(keyId: string, startDate: string, endDate: string): Promise<ApiKeyUsageResponse> {
      const params = new URLSearchParams({ start_date: startDate, end_date: endDate });
      const response = await axios.get<ApiKeyUsageResponse>(
        `${ENDPOINTS.API_KEYS.USAGE(keyId)}?${params.toString()}`
      );
      return response.data;
    },
  };
}

export type ApiKeysApi = ReturnType<typeof createApiKeysApi>;
