// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the .fit upload in Home's recent activities — the picked file goes up as bytes, the list is read again, a refusal is said by its reason
// ABOUTME: Red if a picked file is not sent, a stored upload leaves the list stale, or a refused file is reported as added

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  ActivityUploadResponse,
  ProvidersStatusResponse,
  RecentActivitiesResponse,
} from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import { RecentActivities } from '../RecentActivities';
import { activity, providerStatus } from './homeFixtures';

const api = vi.hoisted(() => ({
  getRecentActivities: vi.fn<() => Promise<RecentActivitiesResponse>>(),
  uploadActivityFile: vi.fn<(bytes: ArrayBuffer | Uint8Array) => Promise<ActivityUploadResponse>>(),
  getProvidersStatus: vi.fn<() => Promise<ProvidersStatusResponse>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getRecentActivities: api.getRecentActivities,
    uploadActivityFile: api.uploadActivityFile,
  },
  providersApi: { getProvidersStatus: api.getProvidersStatus },
  featureFlagsApi: { getMyFeatures: async () => ({ flags: {}, known: [] }) },
}));

const UPLOADED = activity({
  id: `${'c'.repeat(64)}-0`,
  provider: 'upload',
  name: '',
  sport_type: 'ride',
  has_gps: false,
});

function renderSection() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <RecentActivities onNavigate={vi.fn()} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

/** A `.fit` file the athlete picks, holding `bytes`. */
function fitFile(bytes: number[]) {
  return new File([new Uint8Array(bytes)], 'ride.fit', { type: 'application/octet-stream' });
}

function refused(status: number) {
  return Object.assign(new Error(`status ${status}`), {
    isAxiosError: true,
    response: { status, data: { code: 'x', message: 'server prose' } },
  });
}

// jsdom's Blob predates `arrayBuffer()`, which every browser the app supports
// has; read it through the FileReader jsdom does implement.
if (typeof Blob.prototype.arrayBuffer !== 'function') {
  Blob.prototype.arrayBuffer = function arrayBuffer(this: Blob) {
    return new Promise<ArrayBuffer>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as ArrayBuffer);
      reader.onerror = () => reject(reader.error);
      reader.readAsArrayBuffer(this);
    });
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getProvidersStatus.mockResolvedValue({
    providers: [providerStatus({ provider: 'strava', display_name: 'Strava' })],
  });
  api.getRecentActivities.mockResolvedValue({ activities: [], as_of: null, sync_failure: null, stale: false });
});

describe('RecentActivities upload', () => {
  it('sends the picked file as bytes, says it was added and reads the list again', async () => {
    api.uploadActivityFile.mockResolvedValue({ activities: [UPLOADED], already_held: [] });
    renderSection();
    const action = await screen.findByRole('button', { name: 'Upload the .fit file of a completed workout' });
    await waitFor(() => expect(action).toBeEnabled());
    api.getRecentActivities.mockResolvedValue({
      activities: [UPLOADED],
      as_of: null,
      sync_failure: null,
      stale: false,
    });

    await userEvent.upload(screen.getByTestId('home-upload-action-input'), fitFile([14, 16, 1, 2]));

    expect(await screen.findByText('Workout added to your activities.')).toBeInTheDocument();
    const [sent] = api.uploadActivityFile.mock.calls[0];
    expect(Array.from(new Uint8Array(sent as ArrayBuffer))).toEqual([14, 16, 1, 2]);
    await waitFor(() => expect(api.getRecentActivities).toHaveBeenCalledTimes(2));
  });

  it('says a refused file is not a workout, in its own words rather than the server’s', async () => {
    api.uploadActivityFile.mockImplementation(() => Promise.reject(refused(400)));
    renderSection();
    await screen.findByTestId('home-upload-action');

    await userEvent.upload(screen.getByTestId('home-upload-action-input'), fitFile([1]));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(
      "This file isn't a completed workout. Choose the .fit file your watch or bike computer recorded.",
    );
    expect(alert).not.toHaveTextContent('server prose');
    expect(screen.queryByText('Workout added to your activities.')).toBeNull();
  });

  it('says a workout already held is already there', async () => {
    api.uploadActivityFile.mockImplementation(() => Promise.reject(refused(409)));
    renderSection();
    await screen.findByTestId('home-upload-action');

    await userEvent.upload(screen.getByTestId('home-upload-action-input'), fitFile([1]));

    expect(await screen.findByRole('alert')).toHaveTextContent('This workout is already in your activities.');
  });
});
