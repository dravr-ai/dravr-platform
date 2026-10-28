// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile import screen: the source list's state line, the link check, progress and delete
// ABOUTME: The imports API is mocked; the copy is the shipped English catalogue, so a missing key fails a test

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ArchiveImport } from '@pierre/shared-types';
import { ImportScreen } from '../ImportScreen';
import { importsApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  importsApi: {
    getImports: jest.fn(),
    getCurrentImport: jest.fn(),
    checkLink: jest.fn(),
    startImport: jest.fn(),
    cancelImport: jest.fn(),
    getReport: jest.fn(),
    deleteHistory: jest.fn(),
  },
}));

const api = importsApi as jest.Mocked<typeof importsApi>;

function job(overrides: Partial<ArchiveImport>): ArchiveImport {
  return {
    id: 'job-1',
    source: 'strava_archive',
    status: 'completed',
    total_rows: 3,
    processed_rows: 3,
    imported: 2,
    skipped: 0,
    failed: 0,
    error_code: null,
    earliest_start: '2025-03-01T08:00:00Z',
    latest_start: '2026-03-01T08:00:00Z',
    created_at: '2026-10-27T08:00:00Z',
    updated_at: '2026-10-27T09:00:00Z',
    finished_at: '2026-10-27T09:00:00Z',
    expires_at: null,
    history_held: true,
    ...overrides,
  };
}

function renderScreen(initialSource?: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ImportScreen initialSource={initialSource} />
    </QueryClientProvider>,
  );
}

describe('ImportScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('lists Strava with its import state line', async () => {
    api.getImports.mockResolvedValue({
      enabled: true,
      sources: ['strava_archive'],
      imports: [job({ status: 'importing', total_rows: 3105, processed_rows: 1240 })],
    });
    renderScreen();

    expect(await screen.findByTestId('import-state-strava')).toHaveTextContent('Importing · 1240 of 3105');
  });

  it('says so where imports do not run', async () => {
    api.getImports.mockResolvedValue({ enabled: false, sources: ['strava_archive'], imports: [] });
    renderScreen();

    expect(await screen.findByTestId('import-unavailable')).toBeTruthy();
  });

  it('checks a pasted link and starts the import from it', async () => {
    api.getImports.mockResolvedValue({ enabled: true, sources: ['strava_archive'], imports: [] });
    api.getCurrentImport.mockResolvedValue(null);
    api.checkLink.mockResolvedValue({ accepted: true, refusal: null, size_bytes: 1_200_000_000 });
    api.startImport.mockResolvedValue(job({ status: 'queued', imported: 0 }));
    renderScreen('strava_archive');

    fireEvent.changeText(await screen.findByTestId('import-link-input'), 'https://export.example.com/x.zip');
    fireEvent.press(screen.getByTestId('import-check'));
    expect(await screen.findByTestId('import-link-found')).toHaveTextContent('Export found · about 1.2 GB');
    fireEvent.press(screen.getByTestId('import-start'));

    await waitFor(() => expect(api.startImport).toHaveBeenCalledWith('strava_archive', 'https://export.example.com/x.zip'));
  });

  it('deletes the imported history from its confirmation sheet', async () => {
    api.getImports.mockResolvedValue({ enabled: true, sources: ['strava_archive'], imports: [job({})] });
    api.getCurrentImport.mockResolvedValue(job({}));
    api.deleteHistory.mockResolvedValue(undefined);
    renderScreen('strava_archive');

    fireEvent.press(await screen.findByTestId('import-delete'));
    fireEvent.press(await screen.findByTestId('import-delete-confirm'));

    await waitFor(() => expect(api.deleteHistory).toHaveBeenCalledWith('strava_archive'));
  });

  // carnet#1212: the state line, the history line and the confirmation had one
  // plural form, « 1 activities imported » and « Delete 1 activities ».
  it('counts a single imported activity in the singular on every line', async () => {
    api.getImports.mockResolvedValue({
      enabled: true,
      sources: ['strava_archive'],
      imports: [job({ imported: 1 })],
    });
    api.getCurrentImport.mockResolvedValue(job({ imported: 1 }));
    const list = renderScreen();
    expect(await screen.findByTestId('import-state-strava')).toHaveTextContent(/^1 activity imported · /);
    list.unmount();

    renderScreen('strava_archive');
    expect(await screen.findByText(/^1 activity · imported /)).toBeTruthy();
    fireEvent.press(await screen.findByTestId('import-delete'));
    expect(
      await screen.findByText(
        "Delete 1 imported activity, and the routes and best efforts calculated from it? This can't be undone.",
      ),
    ).toBeTruthy();
    expect(screen.getByTestId('import-delete-confirm')).toHaveTextContent('Delete 1 activity');
  });
});
