// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Access feed domain API — the athlete's "Who read your data" list: outside apps' reads, and the coach's reads per day
// ABOUTME: A read of records the server already keeps; revoking an app goes through the OAuth domain's Connected-apps revoke (carnet#1543)

import type { AxiosInstance } from 'axios';
import type { AccessFeed } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type { AccessFeed };

/** Creates the access feed API methods bound to an axios instance. */
export function createAccessFeedApi(axios: AxiosInstance) {
  return {
    /** The caller's own feed, over the window its `days` names; session only. */
    async getAccessFeed(): Promise<AccessFeed> {
      const response = await axios.get<AccessFeed>(ENDPOINTS.ACCESS_FEED.FEED);
      return response.data;
    },
  };
}

export type AccessFeedApi = ReturnType<typeof createAccessFeedApi>;
