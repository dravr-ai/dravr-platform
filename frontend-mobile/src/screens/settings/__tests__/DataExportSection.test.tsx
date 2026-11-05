// ABOUTME: Unit tests for the mobile data export: the button downloads with the athlete's token straight to a file and opens the share sheet
// ABOUTME: A failed export shows the failure line and opens no share sheet

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as Sharing from 'expo-sharing';
import { DataExportSection } from '../DataExportSection';
import { File } from 'expo-file-system';

jest.mock('../../../services/api', () => ({
  authStorage: { getToken: jest.fn(async () => 'athlete-token') },
}));
jest.mock('../../../services/apiUrl', () => ({
  getApiUrl: () => 'https://api.example.test',
}));

const download = File.downloadFileAsync as unknown as jest.Mock;
const shareAsync = Sharing.shareAsync as jest.Mock;

function renderSection() {
  const client = new QueryClient({
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

  it('says the export failed on any other refusal', async () => {
    download.mockRejectedValueOnce(new Error('response has status: 500'));
    renderSection();

    fireEvent.press(screen.getByTestId('data-export-action'));

    const failure = await screen.findByTestId('data-export-failure');
    expect(failure.props.children).toContain('could not be exported');
  });
});
