// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: i18n domain API - the live string catalogue for one locale, revalidated by ETag
// ABOUTME: The one request a client makes before login, so it admits the 304 the shared instance would reject

import type { AxiosInstance } from 'axios';
import type { I18nBundle, I18nBundleResult } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { I18nBundle, I18nBundleResult };

/**
 * Creates the i18n API methods bound to an axios instance.
 */
export function createI18nApi(axios: AxiosInstance) {
  return {
    /**
     * Fetch the live catalogue for `locale`.
     *
     * `etag` is the digest of the bundle already held; sending it back turns
     * an unchanged catalogue into a bodiless 304, which this method reports as
     * `unchanged` rather than throwing — the shared axios instance treats any
     * non-2xx as an error, and a 304 is the success case here.
     */
    async bundle(locale: string, etag?: string): Promise<I18nBundleResult> {
      const response = await axios.get<I18nBundle>(ENDPOINTS.I18N.BUNDLE(locale), {
        headers: etag === undefined ? undefined : { 'If-None-Match': `"${etag}"` },
        validateStatus: (status) => status === 200 || status === 304,
      });
      if (response.status === 304) {
        return { status: 'unchanged' };
      }
      return { status: 'fresh', bundle: response.data };
    },
  };
}

export type I18nApi = ReturnType<typeof createI18nApi>;
