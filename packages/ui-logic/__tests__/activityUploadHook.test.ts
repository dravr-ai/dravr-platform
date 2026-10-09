// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared activity-upload and delete hooks — a stored or deleted upload refreshes Home
// ABOUTME: Each refusal is named by its status, a file past the limit is refused before a byte is sent

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ACTIVITY_UPLOAD_MAX_BYTES, type ActivityUploadResponse } from '@pierre/shared-types';
import {
  activityUploadFailure,
  createActivityUploadHook,
  createDeleteUploadedActivityHook,
  describeActivityUpload,
  isDeletableActivity,
} from '../src/activityUploadHook';

const uploadActivityFile = vi.fn<(bytes: ArrayBuffer | Uint8Array) => Promise<ActivityUploadResponse>>();
const useActivityUpload = createActivityUploadHook({ uploadActivityFile });

const STORED: ActivityUploadResponse = {
  activities: [
    {
      id: `${'b'.repeat(64)}-0`,
      provider: 'upload',
      name: '',
      sport_type: 'ride',
      start_date: '2026-10-07T07:00:00Z',
      duration_seconds: 600,
      distance_meters: 4792,
      elevation_gain_meters: null,
      has_gps: true,
      summary_polyline: null,
      attribution: null,
    },
  ],
  already_held: [],
};

/** An axios-shaped rejection carrying `status`. */
function refused(status: number) {
  return Object.assign(new Error(`status ${status}`), {
    isAxiosError: true,
    response: { status, data: { code: 'x', message: 'refused' } },
  });
}

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const remove = vi.spyOn(client, 'removeQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, remove, wrapper };
}

describe('createActivityUploadHook', () => {
  beforeEach(() => uploadActivityFile.mockReset());

  it('sends the bytes, says the workout was added and refreshes every Home read it joins', async () => {
    uploadActivityFile.mockResolvedValue(STORED);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useActivityUpload(), { wrapper });
    const bytes = new Uint8Array([14, 16, 0, 0]).buffer;

    act(() => result.current.uploadFile({ size: 4, read: async () => bytes }));

    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'uploaded', response: STORED }));
    expect(uploadActivityFile).toHaveBeenCalledWith(bytes);
    const refreshed = invalidate.mock.calls.map(([filters]) => filters?.queryKey);
    expect(refreshed).toEqual(
      expect.arrayContaining([
        ['home', 'recent-activities'],
        ['home', 'calendar'],
        ['home', 'training-volume'],
        ['home', 'training-status'],
      ]),
    );
  });

  it('refuses a file past the limit without reading or sending it', () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useActivityUpload(), { wrapper });
    const read = vi.fn(async () => new ArrayBuffer(1));

    act(() => result.current.uploadFile({ size: ACTIVITY_UPLOAD_MAX_BYTES + 1, read }));

    expect(result.current.outcome).toEqual({ kind: 'refused', failure: 'tooLarge' });
    expect(read).not.toHaveBeenCalled();
    expect(uploadActivityFile).not.toHaveBeenCalled();
  });

  it('says a file that cannot be read failed, and sends nothing', async () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useActivityUpload(), { wrapper });

    act(() => result.current.uploadFile({ read: () => Promise.reject(new Error('gone')) }));

    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'refused', failure: 'failed' }));
    expect(uploadActivityFile).not.toHaveBeenCalled();
  });

  it('names a refusal by its status', async () => {
    const rejecting = vi.fn((_bytes: ArrayBuffer | Uint8Array) => Promise.reject(refused(409)));
    const useRefusedUpload = createActivityUploadHook({ uploadActivityFile: rejecting });
    const { wrapper } = setup();
    const { result } = renderHook(() => useRefusedUpload(), { wrapper });

    act(() => result.current.uploadFile({ read: async () => new ArrayBuffer(1) }));

    await waitFor(() => expect(result.current.outcome).toEqual({ kind: 'refused', failure: 'alreadyHeld' }));
    act(() => result.current.dismiss());
    expect(result.current.outcome).toBeNull();
  });
});

describe('activityUploadFailure', () => {
  it('reads each status the upload route answers', () => {
    expect(activityUploadFailure(refused(413))).toBe('tooLarge');
    expect(activityUploadFailure(refused(400))).toBe('notActivity');
    expect(activityUploadFailure(refused(409))).toBe('alreadyHeld');
    expect(activityUploadFailure(refused(500))).toBe('failed');
    expect(activityUploadFailure(new Error('network down'))).toBe('failed');
  });
});

describe('describeActivityUpload', () => {
  it('says the limit in megabytes, and every other outcome by its key', () => {
    const t = (key: string, params?: Record<string, string | number>) =>
      params ? `${key}:${JSON.stringify(params)}` : key;
    expect(describeActivityUpload({ kind: 'refused', failure: 'tooLarge' }, t)).toBe(
      'home.activities.upload.tooLarge:{"megabytes":16}',
    );
    expect(describeActivityUpload({ kind: 'refused', failure: 'notActivity' }, t)).toBe(
      'home.activities.upload.notActivity',
    );
    expect(describeActivityUpload({ kind: 'uploaded', response: STORED }, t)).toBe(
      'home.activities.upload.uploaded',
    );
  });
});

describe('createDeleteUploadedActivityHook', () => {
  const UPLOAD_ID = `${'b'.repeat(64)}-0`;

  it('deletes the upload, drops its view, refreshes every Home read it left and calls back', async () => {
    const deleteUploadedActivity = vi.fn((_id: string) => Promise.resolve());
    const useDelete = createDeleteUploadedActivityHook({ deleteUploadedActivity });
    const onDeleted = vi.fn();
    const { invalidate, remove, wrapper } = setup();
    const { result } = renderHook(() => useDelete(onDeleted), { wrapper });

    act(() => result.current.deleteActivity(UPLOAD_ID));

    await waitFor(() => expect(onDeleted).toHaveBeenCalledTimes(1));
    expect(deleteUploadedActivity).toHaveBeenCalledWith(UPLOAD_ID);
    expect(remove).toHaveBeenCalledWith({ queryKey: ['home', 'activity-detail', 'upload', UPLOAD_ID] });
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual(
      expect.arrayContaining([
        ['home', 'recent-activities'],
        ['home', 'calendar'],
        ['home', 'training-volume'],
        ['home', 'training-status'],
      ]),
    );
    expect(result.current.failed).toBe(false);
  });

  it('says a failed delete and leaves the view where it is', async () => {
    const deleteUploadedActivity = vi.fn((_id: string) => Promise.reject(refused(500)));
    const useDelete = createDeleteUploadedActivityHook({ deleteUploadedActivity });
    const onDeleted = vi.fn();
    const { wrapper } = setup();
    const { result } = renderHook(() => useDelete(onDeleted), { wrapper });

    act(() => result.current.deleteActivity(UPLOAD_ID));

    await waitFor(() => expect(result.current.failed).toBe(true));
    expect(onDeleted).not.toHaveBeenCalled();
  });
});

describe('isDeletableActivity', () => {
  it("offers Delete on the athlete's uploads alone", () => {
    expect(isDeletableActivity('upload')).toBe(true);
    expect(isDeletableActivity('strava')).toBe(false);
    expect(isDeletableActivity('sciotte_garmin')).toBe(false);
  });
});
