// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the health records domain — the routes each call sends, the upload's raw PDF body and its own timeout
// ABOUTME: Runs the real domain over a stub axios, so web and mobile share what is pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createHealthRecordsApi, LAB_DOCUMENT_UPLOAD_TIMEOUT_MS } from '../src/domains/healthRecords';

const DOCUMENT = {
  id: 'doc 1',
  status: 'read',
  pages: 1,
  size_bytes: 10,
  engine: 'rules',
  flagged_lines: 0,
  skipped: 0,
  dropped: 0,
  model_failure: null,
  received_at: '2026-11-20T10:00:00Z',
};

function stubAxios(data: unknown) {
  const get = vi.fn().mockResolvedValue({ data });
  const post = vi.fn().mockResolvedValue({ data });
  const del = vi.fn().mockResolvedValue({ data: undefined });
  const api = createHealthRecordsApi({ get, post, delete: del } as unknown as AxiosInstance);
  return { get, post, del, api };
}

describe('healthRecords domain', () => {
  it('reads one kind with only the window fields it is given', async () => {
    const { get, api } = stubAxios({});
    await expect(api.listRecords('glucose', { limit: 50 })).resolves.toEqual({
      records: [],
      selfReports: [],
      readings: [],
      daily: [],
    });
    expect(get).toHaveBeenCalledWith('/api/me/health-records', { params: { kind: 'glucose', limit: 50 } });
  });

  it('reads the review queue out of its envelope', async () => {
    const { get, api } = stubAxios({ pending: [] });
    await expect(api.getReview()).resolves.toEqual([]);
    expect(get).toHaveBeenCalledWith('/api/me/health-records/review');
  });

  it('confirms and rejects on the encoded id', async () => {
    const { post, api } = stubAxios(undefined);
    await api.confirm('r 1');
    await api.reject('r/2');
    expect(post.mock.calls).toEqual([
      ['/api/me/health-records/r%201/confirm'],
      ['/api/me/health-records/r%2F2/reject'],
    ]);
  });

  it('uploads the PDF as the raw body, with a timeout longer than the read', async () => {
    const { post, api } = stubAxios({ document: DOCUMENT, records: [], new: true });
    const bytes = new Uint8Array([0x25, 0x50, 0x44, 0x46]);
    await expect(api.uploadLabDocument(bytes)).resolves.toMatchObject({ isNew: true });
    const [endpoint, body, config] = post.mock.calls[0];
    expect(endpoint).toBe('/api/me/lab-documents');
    expect(body).toBe(bytes);
    expect(config.headers).toEqual({ 'Content-Type': 'application/pdf' });
    expect(config.timeout).toBe(LAB_DOCUMENT_UPLOAD_TIMEOUT_MS);
    expect(LAB_DOCUMENT_UPLOAD_TIMEOUT_MS).toBeGreaterThan(120_000);
    expect(config.transformRequest[0](bytes)).toBe(bytes);
  });

  it('refuses an upload answer of another shape', async () => {
    const { api } = stubAxios({ document: DOCUMENT });
    await expect(api.uploadLabDocument(new Uint8Array())).rejects.toThrow('/api/me/lab-documents');
  });

  it('lists and deletes lab reports', async () => {
    const { get, del, api } = stubAxios({ documents: [{ document: DOCUMENT, records: [], new: false }] });
    await expect(api.listLabDocuments()).resolves.toHaveLength(1);
    expect(get).toHaveBeenCalledWith('/api/me/lab-documents');
    await api.deleteLabDocument('doc 1');
    expect(del).toHaveBeenCalledWith('/api/me/lab-documents/doc%201');
  });
});
