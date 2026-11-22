// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The binding of the shared health records hooks over this client's API — served records, review queue, lab reports
// ABOUTME: Every write rereads every health records read; the API is reached when a hook runs, so importing it touches no client

import { createHealthRecordsHooks } from '@pierre/ui-logic';
import { healthRecordsApi } from '../services/api';

export const {
  useHealthRecords,
  useHealthRecordReview,
  useLabDocuments,
  useReviewAnswer,
  useDeleteLabDocument,
  useLabDocumentUpload,
} = createHealthRecordsHooks({
  listRecords: (kind, window) => healthRecordsApi.listRecords(kind, window),
  getReview: () => healthRecordsApi.getReview(),
  confirm: (id) => healthRecordsApi.confirm(id),
  reject: (id) => healthRecordsApi.reject(id),
  uploadLabDocument: (bytes) => healthRecordsApi.uploadLabDocument(bytes),
  listLabDocuments: () => healthRecordsApi.listLabDocuments(),
  deleteLabDocument: (id) => healthRecordsApi.deleteLabDocument(id),
});
