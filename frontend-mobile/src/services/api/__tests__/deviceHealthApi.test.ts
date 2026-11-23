// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone-only device health API — the route and body a sync sends, the delete's route, and the counts check on every answer
// ABOUTME: Runs the real methods over the app's own axios client with its post and delete stubbed

import type { DeviceSamples } from '@pierre/shared-types';
import { apiClient, deviceHealthApi } from '..';

const COUNTS = { activities: 2, refused: 1, erased: 0, nights: 3, days: 7, self_reports: 4 };

const SAMPLES: DeviceSamples = {
  source: 'apple_health',
  workouts: [
    {
      id: '5D9E9F1A-0B7C-4C55-9A61-3F0B5C2E7D10',
      activity_type: 37,
      start: '2026-09-28T10:15:00.000Z',
      end: '2026-09-28T11:00:00.000Z',
      utc_offset_seconds: -14400,
      recorder: { name: 'Connect' },
    },
  ],
  categories: [],
  quantities: [],
  days: [],
  states_of_mind: [],
  sleep_windows: [],
  quantity_windows: [],
  deleted: { workouts: ['0B7C'], self_reports: [] },
};

afterEach(() => {
  jest.restoreAllMocks();
});

describe('deviceHealthApi', () => {
  it('posts the samples as they are to their store’s sync route', async () => {
    const post = jest.spyOn(apiClient, 'post').mockResolvedValue({ data: COUNTS });
    await expect(deviceHealthApi.syncDeviceSamples(SAMPLES)).resolves.toEqual(COUNTS);
    expect(post).toHaveBeenCalledWith('/api/device-health/apple_health/sync', SAMPLES);
  });

  it('deletes a store’s data on its data route', async () => {
    const del = jest
      .spyOn(apiClient, 'delete')
      .mockResolvedValue({ data: { activities: 5, nights: 2, days: 30, self_reports: 1 } });
    await expect(deviceHealthApi.deleteDeviceData('health_connect')).resolves.toEqual({
      activities: 5,
      refused: 0,
      erased: 0,
      nights: 2,
      days: 30,
      self_reports: 1,
    });
    expect(del).toHaveBeenCalledWith('/api/device-health/health_connect/data');
  });

  it('refuses an answer that is not the counts, naming the endpoint', async () => {
    jest.spyOn(apiClient, 'post').mockResolvedValue({ data: { activities: '2', nights: 0, days: 0, self_reports: 0 } });
    await expect(deviceHealthApi.syncDeviceSamples(SAMPLES)).rejects.toThrow('/api/device-health/apple_health/sync');
  });
});
