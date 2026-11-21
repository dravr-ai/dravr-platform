// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Health records domain API — the served records by kind, the lab review queue and its answers, lab report upload and delete
// ABOUTME: Each answer is checked with its shared-types parser; only the athlete's own session may call these routes (owl phase 5)

import type { AxiosInstance } from 'axios';
import {
  parseHealthRecordReview,
  parseHealthRecordsPage,
  parseLabDocumentList,
  parseLabDocumentView,
  type HealthRecordKind,
  type HealthRecordsPage,
  type LabDocumentView,
  type LabRecord,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';

export type { HealthRecordKind, HealthRecordsPage, LabDocumentView, LabRecord };

/**
 * How long a lab report's upload may take. The server reads it before it
 * answers, which takes up to 120 s when a model is configured, so the shared
 * client's 30 s default would give up on a read still under way.
 */
export const LAB_DOCUMENT_UPLOAD_TIMEOUT_MS = 150_000;

/** The window of a records read; every field is optional. */
export interface HealthRecordsWindow {
  /** From this instant (RFC 3339), inclusive. */
  since?: string;
  /** Until this instant (RFC 3339), exclusive. */
  until?: string;
  /** At most this many, 1 to 200; the server answers 100 when none is asked. */
  limit?: number;
}

/** Creates the health records API methods bound to an axios instance. */
export function createHealthRecordsApi(axios: AxiosInstance) {
  return {
    /** The athlete's served records of one kind, read as they are or confirmed. */
    async listRecords(kind: HealthRecordKind, window: HealthRecordsWindow = {}): Promise<HealthRecordsPage> {
      const endpoint = ENDPOINTS.HEALTH_RECORDS.LIST;
      const params: Record<string, string | number> = { kind };
      if (window.since !== undefined) params.since = window.since;
      if (window.until !== undefined) params.until = window.until;
      if (window.limit !== undefined) params.limit = window.limit;
      const response = await axios.get<unknown>(endpoint, { params });
      return requireShape(parseHealthRecordsPage(response.data), endpoint);
    },

    /** The records read from a lab report that wait for the athlete's answer, newest first. */
    async getReview(): Promise<LabRecord[]> {
      const endpoint = ENDPOINTS.HEALTH_RECORDS.REVIEW;
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseHealthRecordReview(response.data), endpoint);
    },

    /** Confirm a pending record: it is served from now on. Rejects with 404 for any other record. */
    async confirm(id: string): Promise<void> {
      await axios.post(ENDPOINTS.HEALTH_RECORDS.CONFIRM(id));
    },

    /** Reject a pending record: it is never served. Rejects with 404 for any other record. */
    async reject(id: string): Promise<void> {
      await axios.post(ENDPOINTS.HEALTH_RECORDS.REJECT(id));
    },

    /**
     * Upload a lab report's PDF as the raw body. Resolves with the document
     * and the records read from it — `isNew` false when the athlete already
     * held the same file. Rejects with 400 for a body that is not a readable
     * PDF, and past `LAB_DOCUMENT_MAX_BYTES`.
     */
    async uploadLabDocument(file: ArrayBuffer | Uint8Array): Promise<LabDocumentView> {
      const endpoint = ENDPOINTS.HEALTH_RECORDS.LAB_DOCUMENTS;
      const response = await axios.post<unknown>(endpoint, file, {
        headers: { 'Content-Type': 'application/pdf' },
        // The body is the file: no JSON serialization may touch it.
        transformRequest: [(data: unknown) => data],
        timeout: LAB_DOCUMENT_UPLOAD_TIMEOUT_MS,
      });
      return requireShape(parseLabDocumentView(response.data), endpoint);
    },

    /** The athlete's lab reports with their records, newest first. */
    async listLabDocuments(): Promise<LabDocumentView[]> {
      const endpoint = ENDPOINTS.HEALTH_RECORDS.LAB_DOCUMENTS;
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseLabDocumentList(response.data), endpoint);
    },

    /** Delete one lab report and the records read from it. */
    async deleteLabDocument(id: string): Promise<void> {
      await axios.delete(ENDPOINTS.HEALTH_RECORDS.LAB_DOCUMENT(id));
    },
  };
}

export type HealthRecordsApi = ReturnType<typeof createHealthRecordsApi>;
