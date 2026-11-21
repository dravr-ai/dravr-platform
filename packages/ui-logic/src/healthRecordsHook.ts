// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks over the athlete's health records — served records by kind, the lab review queue, lab reports
// ABOUTME: Upload, confirm, reject and delete each reread every health records read; the upload's outcome is said by its kind

import { useCallback, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { HealthRecordsApi } from '@pierre/api-client';
import {
  LAB_DOCUMENT_MAX_BYTES,
  type FlatObservation,
  type HealthRecordKind,
  type LabDocumentView,
} from '@pierre/shared-types';

/** The catalogue keys both clients' health records surfaces render. */
export const HEALTH_RECORDS_KEYS = {
  title: 'healthRecords.title',
  blurb: 'healthRecords.blurb',
  uploadLabel: 'healthRecords.uploadLabel',
  uploading: 'healthRecords.uploading',
  uploaded: 'healthRecords.uploaded',
  alreadyUploaded: 'healthRecords.alreadyUploaded',
  noText: 'healthRecords.noText',
  uploadFailed: 'healthRecords.uploadFailed',
  reviewTitle: 'healthRecords.reviewTitle',
  reviewHint: 'healthRecords.reviewHint',
  reviewEmpty: 'healthRecords.reviewEmpty',
  confirm: 'healthRecords.confirm',
  reject: 'healthRecords.reject',
  checksLabel: 'healthRecords.checksLabel',
  failed: 'healthRecords.failed',
  documentsTitle: 'healthRecords.documentsTitle',
  documentsEmpty: 'healthRecords.documentsEmpty',
  deleteDocument: 'healthRecords.deleteDocument',
  deleted: 'healthRecords.deleted',
  recordsTitle: 'healthRecords.recordsTitle',
  recordsEmpty: 'healthRecords.recordsEmpty',
  dailyGlucose: 'healthRecords.dailyGlucose',
} as const;

/** The label key of each kind the records selector offers. */
export const HEALTH_RECORD_KIND_LABEL_KEY: Record<HealthRecordKind, string> = {
  labs: 'healthRecords.kindLabs',
  clinical: 'healthRecords.kindClinical',
  glucose: 'healthRecords.kindGlucose',
  self_reports: 'healthRecords.kindSelfReports',
};

/** What the last lab report upload came to, for the line the surface shows. */
export type LabUploadOutcome =
  | { kind: 'uploaded'; count: number }
  | { kind: 'alreadyUploaded' }
  | { kind: 'noText' }
  | { kind: 'failed' };

/**
 * The outcome a stored answer names: a file the athlete already held, then a
 * scan with no text layer, else the records read from it. 200 and 201 both
 * resolve, so `isNew` is what tells them apart.
 */
export function labUploadOutcome(view: LabDocumentView): LabUploadOutcome {
  if (!view.isNew) return { kind: 'alreadyUploaded' };
  if (view.document.status === 'no_text') return { kind: 'noText' };
  return { kind: 'uploaded', count: view.records.length };
}

/** The line an outcome is said in. */
export function describeLabUpload(
  outcome: LabUploadOutcome,
  t: (key: string, params?: Record<string, string | number>) => string,
): string {
  switch (outcome.kind) {
    case 'uploaded':
      return t(HEALTH_RECORDS_KEYS.uploaded, { count: outcome.count });
    case 'alreadyUploaded':
      return t(HEALTH_RECORDS_KEYS.alreadyUploaded);
    case 'noText':
      return t(HEALTH_RECORDS_KEYS.noText);
    case 'failed':
      return t(HEALTH_RECORDS_KEYS.uploadFailed);
  }
}

/** What a flat Observation is called: the terminology's display, else its code. */
export function observationName(flat: FlatObservation | null): string | null {
  return flat?.display ?? flat?.code ?? null;
}

/** Its value with its unit (`150 g/L`), or its coded result; `null` when it holds neither. */
export function observationValue(flat: FlatObservation | null): string | null {
  if (flat === null) return null;
  if (flat.value !== null) return flat.unit ? `${flat.value} ${flat.unit}` : String(flat.value);
  return flat.valueCode;
}

/** Its reference range (`135–175`, `≥ 35`, `≤ 5.2`), or `null` when the report printed none. */
export function observationRange(flat: FlatObservation | null): string | null {
  if (flat === null) return null;
  const { refLow, refHigh } = flat;
  if (refLow !== null && refHigh !== null) return `${refLow}–${refHigh}`;
  if (refLow !== null) return `≥ ${refLow}`;
  if (refHigh !== null) return `≤ ${refHigh}`;
  return null;
}

/** A file the athlete picked: its size when the picker knows it, and how to read its bytes. */
export interface PickedLabDocument {
  size?: number;
  read: () => Promise<ArrayBuffer>;
}

/** What `useLabDocumentUpload` hands a surface. */
export interface UseLabDocumentUploadResult {
  /** Upload one picked PDF. A file past the limit is refused here, before a byte is sent. */
  uploadFile: (file: PickedLabDocument) => void;
  isUploading: boolean;
  /** The last upload's outcome, or null before the first one. */
  outcome: LabUploadOutcome | null;
  /** Forget the last outcome, so its line leaves the page. */
  dismiss: () => void;
}

/** A pending record and the athlete's answer to it. */
export interface ReviewAnswer {
  id: string;
  decision: 'confirm' | 'reject';
}

function rereadHealthRecords(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: QUERY_KEYS.healthRecords.all });
}

/**
 * Build the health records hooks over one client's API.
 *
 * Every write — an upload, a confirm, a reject, a delete — moves records
 * between the review queue, the served kinds and the documents list, so each
 * one rereads all of them, after a refusal too: a 404 on an answer means the
 * record already left the queue.
 */
export function createHealthRecordsHooks(
  api: Pick<
    HealthRecordsApi,
    'listRecords' | 'getReview' | 'confirm' | 'reject' | 'uploadLabDocument' | 'listLabDocuments' | 'deleteLabDocument'
  >,
) {
  function useHealthRecords(kind: HealthRecordKind, enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.healthRecords.records(kind),
      queryFn: () => api.listRecords(kind),
      enabled,
    });
  }

  function useHealthRecordReview(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.healthRecords.review(),
      queryFn: () => api.getReview(),
      enabled,
    });
  }

  function useLabDocuments(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.healthRecords.documents(),
      queryFn: () => api.listLabDocuments(),
      enabled,
    });
  }

  function useReviewAnswer() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: ({ id, decision }: ReviewAnswer) => (decision === 'confirm' ? api.confirm(id) : api.reject(id)),
      onSettled: () => rereadHealthRecords(queryClient),
    });
  }

  function useDeleteLabDocument() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: (id: string) => api.deleteLabDocument(id),
      onSettled: () => rereadHealthRecords(queryClient),
    });
  }

  function useLabDocumentUpload(): UseLabDocumentUploadResult {
    const queryClient = useQueryClient();
    const [outcome, setOutcome] = useState<LabUploadOutcome | null>(null);
    const { mutate, isPending } = useMutation({
      mutationFn: (bytes: ArrayBuffer | Uint8Array) => api.uploadLabDocument(bytes),
      onSuccess: (view) => setOutcome(labUploadOutcome(view)),
      onError: () => setOutcome({ kind: 'failed' }),
      onSettled: () => rereadHealthRecords(queryClient),
    });
    const uploadFile = useCallback(
      async (file: PickedLabDocument) => {
        const tooLarge = (size: number) => size > LAB_DOCUMENT_MAX_BYTES;
        if (file.size !== undefined && tooLarge(file.size)) {
          setOutcome({ kind: 'failed' });
          return;
        }
        setOutcome(null);
        let bytes: ArrayBuffer;
        try {
          bytes = await file.read();
        } catch {
          setOutcome({ kind: 'failed' });
          return;
        }
        if (tooLarge(bytes.byteLength)) {
          setOutcome({ kind: 'failed' });
          return;
        }
        mutate(bytes);
      },
      [mutate],
    );
    const dismiss = useCallback(() => setOutcome(null), []);
    return useMemo(
      () => ({
        uploadFile: (file: PickedLabDocument) => void uploadFile(file),
        isUploading: isPending,
        outcome,
        dismiss,
      }),
      [uploadFile, isPending, outcome, dismiss],
    );
  }

  return {
    useHealthRecords,
    useHealthRecordReview,
    useLabDocuments,
    useReviewAnswer,
    useDeleteLabDocument,
    useLabDocumentUpload,
  };
}

export type HealthRecordsHooks = ReturnType<typeof createHealthRecordsHooks>;
