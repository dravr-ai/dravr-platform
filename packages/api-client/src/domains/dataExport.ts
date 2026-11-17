// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Data export domain API — ask for the originals bundle the JSON export leaves out, optionally sealed, find it again, follow it
// ABOUTME: Each answer is checked with its shared-types parser; only the athlete's own session may call these routes

import type { AxiosInstance } from 'axios';
import { parseOriginalsBundle, type OriginalsBundle } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';

export type { OriginalsBundle };

/**
 * Creates the data export API methods bound to an axios instance.
 *
 * The bytes of a bundle are not read here: a download can be gigabytes, so
 * each client fetches `ENDPOINTS.USER.EXPORT_ORIGINALS_DOWNLOAD` natively.
 */
export function createDataExportApi(axios: AxiosInstance) {
  return {
    /**
     * Ask for the originals bundle, sealed with `passphrase` when one is
     * given; answers the bundle started, the one already building, or an
     * unsealed one ready for less than an hour.
     */
    async requestOriginals(passphrase?: string): Promise<OriginalsBundle> {
      const endpoint = ENDPOINTS.USER.EXPORT_ORIGINALS;
      const response = await axios.post<unknown>(endpoint, passphrase ? { passphrase } : {});
      return requireShape(parseOriginalsBundle(response.data), endpoint);
    },

    /** The athlete's latest bundle, or `null` when they never asked for one. */
    async getLatestOriginals(): Promise<OriginalsBundle | null> {
      const endpoint = ENDPOINTS.USER.EXPORT_ORIGINALS;
      const response = await axios.get<unknown>(endpoint);
      if (response.data === null) return null;
      return requireShape(parseOriginalsBundle(response.data), endpoint);
    },

    /** One of the athlete's bundles, as it stands now. */
    async getOriginals(bundleId: string): Promise<OriginalsBundle> {
      const endpoint = ENDPOINTS.USER.EXPORT_ORIGINALS_BUNDLE(bundleId);
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseOriginalsBundle(response.data), endpoint);
    },
  };
}

export type DataExportApi = ReturnType<typeof createDataExportApi>;
