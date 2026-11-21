// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared health records hooks — each write rereads every health records read, failures included
// ABOUTME: The upload's outcome is named by the answer (new, held, no text); a file past 10 MB is refused before a byte is sent

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { LAB_DOCUMENT_MAX_BYTES, type FlatObservation, type LabDocumentView } from '@pierre/shared-types';
import {
  createHealthRecordsHooks,
  describeLabUpload,
  labUploadOutcome,
  observationName,
  observationRange,
  observationValue,
} from '../src/healthRecordsHook';

const api = {
  listRecords: vi.fn(),
  getReview: vi.fn(),
  confirm: vi.fn(),
  reject: vi.fn(),
  uploadLabDocument: vi.fn(),
  listLabDocuments: vi.fn(),
  deleteLabDocument: vi.fn(),
};
const hooks = createHealthRecordsHooks(api);

function view(overrides: Partial<LabDocumentView> = {}, status: 'read' | 'no_text' = 'read'): LabDocumentView {
  return {
    document: {
      id: 'doc-1',
      status,
      pages: 1,
      sizeBytes: 10,
      engine: 'rules',
      flaggedLines: 0,
      skipped: 0,
      dropped: 0,
      modelFailure: null,
      receivedAt: '2026-11-20T10:00:00Z',
    },
    records: [],
    isNew: true,
    ...overrides,
  };
}

const FLAT: FlatObservation = {
  codeSystem: 'http://loinc.org',
  code: '718-7',
  display: 'Hemoglobin',
  value: 150,
  unit: 'g/L',
  valueCode: null,
  interpretation: null,
  refLow: 135,
  refHigh: 175,
  status: 'final',
  components: [],
  effectiveAt: null,
};

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

const healthRecordsRereads = (invalidate: ReturnType<typeof setup>['invalidate']) =>
  invalidate.mock.calls.map(([filters]) => filters?.queryKey);

describe('createHealthRecordsHooks', () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  it('reads one kind under its own key', async () => {
    api.listRecords.mockResolvedValue({ records: [], selfReports: [], readings: [], daily: [] });
    const { wrapper } = setup();
    const { result } = renderHook(() => hooks.useHealthRecords('glucose'), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(api.listRecords).toHaveBeenCalledWith('glucose');
  });

  it('reads nothing while disabled', () => {
    const { wrapper } = setup();
    renderHook(() => hooks.useHealthRecordReview(false), { wrapper });
    renderHook(() => hooks.useLabDocuments(false), { wrapper });
    expect(api.getReview).not.toHaveBeenCalled();
    expect(api.listLabDocuments).not.toHaveBeenCalled();
  });

  it('confirms or rejects by the answer, and rereads every health records read', async () => {
    api.confirm.mockResolvedValue(undefined);
    api.reject.mockResolvedValue(undefined);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => hooks.useReviewAnswer(), { wrapper });
    await act(() => result.current.mutateAsync({ id: 'r1', decision: 'confirm' }));
    await act(() => result.current.mutateAsync({ id: 'r2', decision: 'reject' }));
    expect(api.confirm).toHaveBeenCalledWith('r1');
    expect(api.reject).toHaveBeenCalledWith('r2');
    expect(healthRecordsRereads(invalidate)).toEqual([['health-records'], ['health-records']]);
  });

  it('rereads after a refused answer too, since the record may have left the queue', async () => {
    api.confirm.mockRejectedValue(new Error('404'));
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => hooks.useReviewAnswer(), { wrapper });
    act(() => result.current.mutate({ id: 'r1', decision: 'confirm' }));
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(healthRecordsRereads(invalidate)).toEqual([['health-records']]);
  });

  it('deletes a lab report and rereads', async () => {
    api.deleteLabDocument.mockResolvedValue(undefined);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => hooks.useDeleteLabDocument(), { wrapper });
    await act(() => result.current.mutateAsync('doc-1'));
    expect(api.deleteLabDocument).toHaveBeenCalledWith('doc-1');
    expect(healthRecordsRereads(invalidate)).toEqual([['health-records']]);
  });

  it('uploads the bytes it reads, says how many results were read, and rereads', async () => {
    api.uploadLabDocument.mockResolvedValue(view({ records: [{} as never, {} as never] }));
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => hooks.useLabDocumentUpload(), { wrapper });
    const bytes = new ArrayBuffer(8);
    act(() => result.current.uploadFile({ size: 8, read: async () => bytes }));
    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'uploaded', count: 2 }));
    expect(api.uploadLabDocument).toHaveBeenCalledWith(bytes);
    expect(healthRecordsRereads(invalidate)).toEqual([['health-records']]);

    act(() => result.current.dismiss());
    expect(result.current.outcome).toBeNull();
  });

  it('refuses a file past the limit before reading it', async () => {
    const read = vi.fn();
    const { wrapper } = setup();
    const { result } = renderHook(() => hooks.useLabDocumentUpload(), { wrapper });
    act(() => result.current.uploadFile({ size: LAB_DOCUMENT_MAX_BYTES + 1, read }));
    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'failed' }));
    expect(read).not.toHaveBeenCalled();
    expect(api.uploadLabDocument).not.toHaveBeenCalled();
  });

  it('says a refused upload failed', async () => {
    api.uploadLabDocument.mockRejectedValue(new Error('400'));
    const { wrapper } = setup();
    const { result } = renderHook(() => hooks.useLabDocumentUpload(), { wrapper });
    act(() => result.current.uploadFile({ read: async () => new ArrayBuffer(1) }));
    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'failed' }));
  });
});

describe('labUploadOutcome', () => {
  it('names a held file first, then a scan, else the count', () => {
    expect(labUploadOutcome(view({ isNew: false }, 'no_text'))).toEqual({ kind: 'alreadyUploaded' });
    expect(labUploadOutcome(view({}, 'no_text'))).toEqual({ kind: 'noText' });
    expect(labUploadOutcome(view())).toEqual({ kind: 'uploaded', count: 0 });
  });

  it('says each outcome with its key', () => {
    const t = (key: string, params?: Record<string, string | number>) => `${key}${params ? JSON.stringify(params) : ''}`;
    expect(describeLabUpload({ kind: 'uploaded', count: 3 }, t)).toBe('healthRecords.uploaded{"count":3}');
    expect(describeLabUpload({ kind: 'alreadyUploaded' }, t)).toBe('healthRecords.alreadyUploaded');
    expect(describeLabUpload({ kind: 'noText' }, t)).toBe('healthRecords.noText');
    expect(describeLabUpload({ kind: 'failed' }, t)).toBe('healthRecords.uploadFailed');
  });
});

describe('observation words', () => {
  it('names, values and ranges a flat Observation', () => {
    expect(observationName(FLAT)).toBe('Hemoglobin');
    expect(observationName({ ...FLAT, display: null })).toBe('718-7');
    expect(observationValue(FLAT)).toBe('150 g/L');
    expect(observationValue({ ...FLAT, value: null, valueCode: 'POS' })).toBe('POS');
    expect(observationRange(FLAT)).toBe('135–175');
    expect(observationRange({ ...FLAT, refHigh: null })).toBe('≥ 135');
    expect(observationRange({ ...FLAT, refLow: null })).toBe('≤ 175');
    expect(observationRange({ ...FLAT, refLow: null, refHigh: null })).toBeNull();
    expect(observationValue(null)).toBeNull();
    const pressure = {
      ...FLAT,
      value: null,
      unit: null,
      components: [
        { code: '8480-6', value: 120, unit: 'mm[Hg]' },
        { code: '8462-4', value: 80, unit: 'mm[Hg]' },
      ],
    };
    expect(observationValue(pressure)).toBe('120 / 80 mm[Hg]');
    expect(
      observationValue({ ...pressure, components: [pressure.components[0], { code: 'x', value: 5, unit: 'kg' }] }),
    ).toBe('120 mm[Hg] / 5 kg');
  });
});
