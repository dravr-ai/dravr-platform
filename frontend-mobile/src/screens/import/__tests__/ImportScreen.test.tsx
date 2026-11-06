// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile import screen: Strava and Garmin rows and state lines, the link check, progress, delete
// ABOUTME: The imports API is mocked; the copy is the shipped English catalogue, so a missing key fails a test

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ArchiveImport } from '@pierre/shared-types';
import * as DocumentPicker from 'expo-document-picker';
import { File } from 'expo-file-system';
import { ImportScreen } from '../ImportScreen';
import { importsApi } from '../../../services/api';

jest.mock('expo-file-system', () => ({ File: jest.fn() }));

jest.mock('../../../services/api', () => ({
  importsApi: {
    getImports: jest.fn(),
    getCurrentImport: jest.fn(),
    checkLink: jest.fn(),
    startImport: jest.fn(),
    cancelImport: jest.fn(),
    getReport: jest.fn(),
    deleteHistory: jest.fn(),
    startUpload: jest.fn(),
    getUploadSession: jest.fn(),
    writeUpload: jest.fn(),
    finishUpload: jest.fn(),
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

    expect(await screen.findByTestId('import-state-strava_archive')).toHaveTextContent('Importing · 1240 of 3105');
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
    expect(await screen.findByTestId('import-state-strava_archive')).toHaveTextContent(/^1 activity imported · /);
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

  // carnet#1337: the Garmin export was built but never offered.
  it('lists Garmin beside Strava, each with its own state line', async () => {
    api.getImports.mockResolvedValue({
      enabled: true,
      sources: ['strava_archive', 'garmin_archive'],
      imports: [job({ source: 'garmin_archive', status: 'awaiting_link', created_at: new Date().toISOString() })],
    });
    renderScreen();

    expect(await screen.findByTestId('import-source-strava_archive')).toHaveTextContent(/Strava/);
    expect(screen.getByTestId('import-source-garmin_archive')).toHaveTextContent(/Garmin Connect/);
    expect(screen.getByTestId('import-state-garmin_archive')).toHaveTextContent(
      "Waiting for Garmin's email · Paste the link",
    );
    expect(screen.queryByTestId('import-state-strava_archive')).toBeNull();
  });

  it('opens the Garmin import from its row, says where to request the export, and checks under Garmin', async () => {
    api.getImports.mockResolvedValue({ enabled: true, sources: ['strava_archive', 'garmin_archive'], imports: [] });
    api.getCurrentImport.mockResolvedValue(null);
    api.checkLink.mockResolvedValue({ accepted: false, refusal: 'link_expired', size_bytes: null });
    renderScreen();

    fireEvent.press(await screen.findByTestId('import-source-garmin_archive'));
    const view = await screen.findByTestId('source-import-garmin_archive');
    expect(view).toHaveTextContent(/Data Management, then Export Your Data/);
    expect(screen.getByTestId('import-link-hint')).toHaveTextContent(/It works for 3 days\./);
    expect(api.getCurrentImport).toHaveBeenCalledWith('garmin_archive');

    fireEvent.changeText(screen.getByTestId('import-link-input'), 'https://s3.amazonaws.com/it-gdpr-bucket/x_1.zip');
    fireEvent.press(screen.getByTestId('import-check'));
    expect(await screen.findByTestId('import-error')).toHaveTextContent(
      'That link has expired. Request a new export from Garmin.',
    );
    expect(api.checkLink).toHaveBeenCalledWith('garmin_archive', 'https://s3.amazonaws.com/it-gdpr-bucket/x_1.zip');
  });
});

describe('ImportScreen, an export the athlete uploads', () => {
  const session = { kind: 'session' as const, url: 'https://storage.example.com/session-secret' };
  const picker = DocumentPicker as jest.Mocked<typeof DocumentPicker>;
  const reads: [number, number][] = [];

  /** A store's session: asked where it stands first, then takes every chunk. */
  function storeFetch() {
    return jest.fn(async (_url: string, init: { headers: Record<string, string> }) => {
      const range = init.headers['Content-Range'];
      return { status: range.startsWith('bytes */') ? 308 : 201, headers: { get: () => null } };
    });
  }

  beforeEach(() => {
    jest.clearAllMocks();
    reads.length = 0;
    (File as unknown as jest.Mock).mockImplementation(() => ({
      size: 9,
      open: () => {
        let offset = 0;
        return {
          set offset(value: number) {
            offset = value;
          },
          get offset() {
            return offset;
          },
          readBytes: (length: number) => {
            reads.push([offset, length]);
            return new Uint8Array(length);
          },
          close: jest.fn(),
        };
      },
    }));
    picker.getDocumentAsync.mockResolvedValue({
      canceled: false,
      assets: [{ uri: 'file:///cache/export.zip', name: 'export.zip', size: 9 }],
    } as never);
    api.getImports.mockResolvedValue({ enabled: true, sources: ['apple_health_archive'], imports: [] });
  });

  it('is listed with the other sources, and opens by its direct link', async () => {
    api.getCurrentImport.mockResolvedValue(null);
    renderScreen();
    expect(await screen.findByTestId('import-source-apple_health_archive')).toBeTruthy();

    renderScreen('apple_health_archive');
    expect(await screen.findByTestId('import-upload-pick')).toBeTruthy();
    expect(screen.queryByTestId('import-link-input')).toBeNull();
  });

  it('opens an upload of the picked file, reads it by chunk and says it landed', async () => {
    api.getCurrentImport.mockResolvedValue(null);
    api.startUpload.mockResolvedValue({ import: job({ id: 'job-9', status: 'awaiting_link' }), upload: session });
    api.finishUpload.mockResolvedValue(job({ id: 'job-9', status: 'downloading' }));
    const fetchMock = storeFetch();
    global.fetch = fetchMock as unknown as typeof fetch;
    renderScreen('apple_health_archive');

    fireEvent.press(await screen.findByTestId('import-upload-pick'));

    await waitFor(() => expect(api.finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-9'));
    expect(api.startUpload).toHaveBeenCalledWith('apple_health_archive', 9);
    expect(reads).toEqual([[0, 9]]);
    expect(fetchMock.mock.calls[1][1].headers).toEqual({ 'Content-Range': 'bytes 0-8/9' });
  });

  it('resumes the import that waits rather than opening another', async () => {
    api.getCurrentImport.mockResolvedValue(job({ id: 'job-7', status: 'awaiting_link', imported: 0 }));
    api.getUploadSession.mockResolvedValue(session);
    api.finishUpload.mockResolvedValue(job({ id: 'job-7', status: 'downloading' }));
    global.fetch = storeFetch() as unknown as typeof fetch;
    renderScreen('apple_health_archive');

    fireEvent.press(await screen.findByTestId('import-upload-pick'));

    await waitFor(() => expect(api.finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-7'));
    expect(api.startUpload).not.toHaveBeenCalled();
  });

  it('starts nothing when the picker is dismissed', async () => {
    api.getCurrentImport.mockResolvedValue(null);
    picker.getDocumentAsync.mockResolvedValue({ canceled: true, assets: null } as never);
    renderScreen('apple_health_archive');

    fireEvent.press(await screen.findByTestId('import-upload-pick'));

    await waitFor(() => expect(picker.getDocumentAsync).toHaveBeenCalled());
    expect(api.startUpload).not.toHaveBeenCalled();
  });

  it('cancels an upload that waits', async () => {
    api.getCurrentImport.mockResolvedValue(job({ id: 'job-7', status: 'awaiting_link', imported: 0 }));
    api.cancelImport.mockResolvedValue(true);
    renderScreen('apple_health_archive');

    fireEvent.press(await screen.findByTestId('import-upload-cancel'));

    await waitFor(() => expect(api.cancelImport).toHaveBeenCalledWith('apple_health_archive', 'job-7'));
  });
});
