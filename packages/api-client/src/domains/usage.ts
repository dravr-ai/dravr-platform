// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Usage domain API — the calling user's quota counters, polled by the chat banner and the settings usage cards
// ABOUTME: One method both clients call; the admin consumption analytics stay in the web app

import type { AxiosInstance } from 'axios';
import type { LimitCheckResult, UsageStatusResponse } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type { LimitCheckResult, UsageStatusResponse };

/**
 * Creates the usage API bound to an axios instance.
 */
export function createUsageApi(axios: AxiosInstance) {
  return {
    /** Every counter, its cap, its warning level and its reset instant. */
    async getStatus(): Promise<UsageStatusResponse> {
      const response = await axios.get<UsageStatusResponse>(ENDPOINTS.USAGE.STATUS);
      return response.data;
    },
  };
}

export type UsageApi = ReturnType<typeof createUsageApi>;
