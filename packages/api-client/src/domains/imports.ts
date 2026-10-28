// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Import-your-own-data domain API — check an export link, start, follow, cancel, read the report, delete the history
// ABOUTME: Each answer is checked with its shared-types parser; only the athlete's own session may call these routes

import type { AxiosInstance } from 'axios';
import {
  parseArchiveImport,
  parseCurrentImport,
  parseImportLinkCheck,
  parseImportReportPage,
  parseImportsOverview,
  type ArchiveImport,
  type ImportLinkCheck,
  type ImportReportPage,
  type ImportsOverview,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';

export type { ArchiveImport, ImportLinkCheck, ImportReportPage, ImportsOverview };

/**
 * Creates the import API methods bound to an axios instance.
 *
 * Every route acts in the athlete's active tenant with their own session; an
 * API key or a delegated grant is refused, and a deployment where imports do
 * not run answers 503 to everything but the overview.
 */
export function createImportsApi(axios: AxiosInstance) {
  return {
    /** Whether imports run here, the offered sources, and each source's running or latest import. */
    async getImports(): Promise<ImportsOverview> {
      const response = await axios.get<unknown>(ENDPOINTS.IMPORTS.LIST);
      return requireShape(parseImportsOverview(response.data), ENDPOINTS.IMPORTS.LIST);
    },

    /** The source's running or latest import, or `null` when it has none. */
    async getCurrentImport(source: string): Promise<ArchiveImport | null> {
      const endpoint = ENDPOINTS.IMPORTS.CURRENT(source);
      const response = await axios.get<unknown>(endpoint);
      const parsed = parseCurrentImport(response.data);
      if (parsed === undefined) {
        throw new Error(`${endpoint} answered with a body that does not match its contract`);
      }
      return parsed;
    },

    /** Whether an import can start from `url`, reading none of the export. */
    async checkLink(source: string, url: string): Promise<ImportLinkCheck> {
      const endpoint = ENDPOINTS.IMPORTS.CHECK(source);
      const response = await axios.post<unknown>(endpoint, { url });
      return requireShape(parseImportLinkCheck(response.data), endpoint);
    },

    /**
     * Start an import: one that downloads `url` now, or, with no link, one
     * waiting for it. Rejects with 409 while another import of the source
     * runs, and 400 naming the refusal code when the link is refused.
     */
    async startImport(source: string, url?: string): Promise<ArchiveImport> {
      const endpoint = ENDPOINTS.IMPORTS.START(source);
      const response = await axios.post<unknown>(endpoint, url ? { url } : {});
      return requireShape(parseArchiveImport(response.data), endpoint);
    },

    /** Cancel an import; whether it was still running. */
    async cancelImport(source: string, importId: string): Promise<boolean> {
      const response = await axios.post<{ cancelled?: unknown }>(ENDPOINTS.IMPORTS.CANCEL(source, importId));
      return response.data?.cancelled === true;
    },

    /** A page of the rows an import did not import. */
    async getReport(source: string, importId: string, offset = 0): Promise<ImportReportPage> {
      const endpoint = ENDPOINTS.IMPORTS.REPORT(source, importId);
      const response = await axios.get<unknown>(endpoint, { params: { offset } });
      return requireShape(parseImportReportPage(response.data), endpoint);
    },

    /** Delete everything imported from the source: a running import is cancelled first. */
    async deleteHistory(source: string): Promise<void> {
      await axios.delete(ENDPOINTS.IMPORTS.DATA(source));
    },
  };
}

/** The import API's methods. */
export type ImportsApi = ReturnType<typeof createImportsApi>;
