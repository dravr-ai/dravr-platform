// ABOUTME: Unit tests for the mobile data export: the button downloads with the athlete's token straight to a file and opens the share sheet
// ABOUTME: A failed or cut export shows the failure line and opens no share sheet; a partial one is shared and names its limit

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { MutationCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as Sharing from 'expo-sharing';
import { DataExportSection } from '../DataExportSection';
import { File } from 'expo-file-system';
import { i18n } from '@pierre/i18n';
import { EXPORT_PARTIAL_KEYS } from '@pierre/ui-logic';

jest.mock('../../../services/api', () => ({
  authStorage: { getToken: jest.fn(async () => 'athlete-token') },
}));
jest.mock('../../../services/apiUrl', () => ({
  getApiUrl: () => 'https://api.example.test',
}));

const download = File.downloadFileAsync as unknown as jest.Mock;
const shareAsync = Sharing.shareAsync as jest.Mock;

const globalMutationError = jest.fn();

function renderSection() {
  const client = new QueryClient({
    // The app's client toasts every mutation that rejects, as this one records.
    mutationCache: new MutationCache({ onError: globalMutationError }),
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <DataExportSection />
    </QueryClientProvider>,
  );
}

describe('DataExportSection', () => {
  beforeEach(() => {
    shareAsync.mockClear();
    download.mockClear();
    globalMutationError.mockClear();
  });

  function cachedExports(): string[] {
    const { __mockFileContents: files } = jest.requireMock('expo-file-system') as {
      __mockFileContents: Record<string, string>;
    };
    return Object.keys(files).filter((uri) => uri.includes('dravr-export'));
  }

  it('downloads with the token to a dated .json file and shares it', async () => {
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
    const [url, , options] = download.mock.calls[0] as [
      string,
      unknown,
      { headers: Record<string, string>; idempotent: boolean },
    ];
    expect(url).toBe('https://api.example.test/api/user/export');
    expect(options.headers.Authorization).toBe('Bearer athlete-token');
    expect(options.idempotent).toBe(true);
    const [uri, shared] = shareAsync.mock.calls[0] as [string, { mimeType: string }];
    expect(uri).toMatch(/dravr-export-\d{4}-\d{2}-\d{2}\.json$/);
    expect(shared.mimeType).toBe('application/json');
    expect(screen.queryByTestId('data-export-failure')).toBeNull();
    await waitFor(() => expect(cachedExports()).toEqual([]));
  });

  it('leaves no copy of the file in the cache when the share fails', async () => {
    shareAsync.mockRejectedValueOnce(new Error('share failed'));
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    expect(await screen.findByTestId('data-export-failure')).toBeTruthy();
    expect(cachedExports()).toEqual([]);
  });

  // The native layers word it differently: "server returned HTTP 429" on iOS,
  // "response has status: 429" on Android.
  it.each(['server returned HTTP 429', 'response has status: 429'])(
    'says the hourly limit was reached and shares nothing on %s',
    async (message) => {
      download.mockRejectedValueOnce(new Error(message));
      renderSection();

      fireEvent.press(screen.getByTestId('data-export-action'));

      const failure = await screen.findByTestId('data-export-failure');
      expect(failure.props.children).toContain('several times this hour');
      expect(shareAsync).not.toHaveBeenCalled();
    },
  );

  it('says a refusal once, on the section, and not again in the global toast', async () => {
    download.mockRejectedValueOnce(new Error('server returned HTTP 429'));
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    await screen.findByTestId('data-export-failure');
    expect(globalMutationError).not.toHaveBeenCalled();
  });

  // iOS reports a finished download when the connection ends part of the way
  // through: the file holds what arrived, and nothing else says it is short.
  it('shares no file that stops before the end of the document', async () => {
    download.mockImplementationOnce(async (_url: string, destination: { uri: string }) => {
      const { __mockFileContents: files } = jest.requireMock('expo-file-system') as {
        __mockFileContents: Record<string, string>;
      };
      files[destination.uri] = '{"format":"dravr-user-export","tables":{"users":[{"id":"a"';
      return destination;
    });
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    const failure = await screen.findByTestId('data-export-failure');
    expect(failure.props.children).toContain('could not be exported');
    expect(shareAsync).not.toHaveBeenCalled();
    expect(cachedExports()).toEqual([]);
  });

  function serverWrites(text: string) {
    download.mockImplementationOnce(async (_url: string, destination: { uri: string }) => {
      const { __mockFileContents: files } = jest.requireMock('expo-file-system') as {
        __mockFileContents: Record<string, string>;
      };
      files[destination.uri] = text;
      return destination;
    });
  }

  it('says nothing more when the file is complete', async () => {
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('data-export-partial')).toBeNull();
  });

  // The notice is t(key). Each key's text is its own, so the text names the
  // key: a platform copy that lacks a key renders the key itself.
  it('says each partial file in its own words', () => {
    const texts = EXPORT_PARTIAL_KEYS.map((key) => i18n.t(key));
    expect(new Set(texts).size).toBe(EXPORT_PARTIAL_KEYS.length);
  });

  it.each([
    ['the size limit', '"size"', '[]', '["activity_streams"]', 'dataExport.partialSize'],
    ['the time limit', '"time"', '[]', '["activity_streams"]', 'dataExport.partialTime'],
    [
      'the time limit and an unread table',
      '"time"',
      '["recipes"]',
      '["activity_streams"]',
      'dataExport.partialTimeAndFailed',
    ],
    ['an unread table', 'null', '["recipes"]', '[]', 'dataExport.partial'],
  ])(
    'shares a document the server stopped early, and says why (%s)',
    async (_case, limit, failed, skipped, key) => {
      serverWrites(
        `{"tables":{},"complete":false,"limit_reached":${limit},"failed_tables":${failed},"skipped_tables":${skipped}}`,
      );
      renderSection();

      fireEvent.press(screen.getByTestId('data-export-action'));

      await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
      const notice = await screen.findByTestId('data-export-partial');
      expect(notice.props.children).toBe(i18n.t(key));
      expect(screen.queryByTestId('data-export-failure')).toBeNull();
      expect(cachedExports()).toEqual([]);
    },
  );

  it.each([
    ['[]', 'dataExport.partialLimit'],
    ['["recipes"]', 'dataExport.partialLimitAndFailed'],
  ])(
    'shares a partial document from a server that does not name the limit (failed %s)',
    async (failed, key) => {
      serverWrites(
        `{"tables":{},"complete":false,"failed_tables":${failed},"skipped_tables":["goals"]}`,
      );
      renderSection();

      fireEvent.press(screen.getByTestId('data-export-action'));

      await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
      const notice = await screen.findByTestId('data-export-partial');
      expect(notice.props.children).toBe(i18n.t(key));
    },
  );

  it('says the export failed on any other refusal', async () => {
    download.mockRejectedValueOnce(new Error('response has status: 500'));
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    const failure = await screen.findByTestId('data-export-failure');
    expect(failure.props.children).toContain('could not be exported');
  });
});
