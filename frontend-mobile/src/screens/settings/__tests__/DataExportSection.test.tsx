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

const mockRequestOriginals = jest.fn();
const mockGetOriginals = jest.fn();
const mockGetLatestOriginals = jest.fn(async () => null);
jest.mock('../../../services/api', () => ({
  authStorage: { getToken: jest.fn(async () => 'athlete-token') },
  originalsApi: {
    requestOriginals: (passphrase?: string) => mockRequestOriginals(passphrase),
    getOriginals: (id: string) => mockGetOriginals(id),
    getLatestOriginals: () => mockGetLatestOriginals(),
  },
}));

/** A bundle as the server describes it. */
function bundle(status: 'building' | 'ready' | 'failed', extra: Record<string, unknown> = {}) {
  return {
    bundleId: 'b-1',
    status,
    sealed: false,
    createdAt: '2026-11-14T10:00:00Z',
    expiresAt: '2026-11-21T10:00:00Z',
    files: status === 'ready' ? 3 : null,
    missing: status === 'ready' ? 0 : null,
    damaged: status === 'ready' ? 0 : null,
    byteSize: null,
    sha256: null,
    download: status === 'ready' ? '/api/user/export/originals/b-1/download' : null,
    ...extra,
  };
}
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
    // Reset, not clear: a failed test must leave no queued download behind.
    download.mockReset();
    download.mockImplementation(async (_url: string, destination: { uri: string }) => {
      (jest.requireMock('expo-file-system') as { __mockFileContents: Record<string, string> }).__mockFileContents[destination.uri] =
        '{"format":"dravr-user-export","tables":{},"complete":true,"limit_reached":null,"failed_tables":[],"skipped_tables":[]}';
      return destination;
    });
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

  describe('the originals bundle', () => {
    const files = () =>
      (jest.requireMock('expo-file-system') as { __mockFileContents: Record<string, string> })
        .__mockFileContents;

    beforeEach(() => {
      mockRequestOriginals.mockReset();
      mockGetOriginals.mockReset();
      mockGetLatestOriginals.mockReset();
      mockGetLatestOriginals.mockResolvedValue(null);
    });

    it('finds the bundle again when the screen is reopened, ready to download', async () => {
      mockGetLatestOriginals.mockResolvedValue(bundle('ready') as never);
      renderSection();
      expect(await screen.findByTestId('originals-download')).toBeTruthy();
      expect(mockRequestOriginals).not.toHaveBeenCalled();
    });

    it('still shows the bundle it asked for when the screen is opened again', async () => {
      // The app keeps a query fresh for minutes: the `null` read before the
      // request must not stand for the bundle once the screen mounts again.
      mockRequestOriginals.mockResolvedValue(bundle('ready'));
      const client = new QueryClient({
        defaultOptions: { queries: { retry: false, staleTime: 5 * 60_000 }, mutations: { retry: false } },
      });
      const section = () => (
        <QueryClientProvider client={client}>
          <DataExportSection />
        </QueryClientProvider>
      );
      const first = render(section());
      fireEvent.press(await screen.findByTestId('originals-request'));
      expect(await screen.findByTestId('originals-download')).toBeTruthy();
      first.unmount();

      render(section());
      expect(await screen.findByTestId('originals-download')).toBeTruthy();
      expect(mockGetLatestOriginals).toHaveBeenCalledTimes(1);
    });

    it('follows a bundle until ready, then downloads it from its signed URL and shares it', async () => {
      mockRequestOriginals.mockResolvedValue(bundle('building'));
      mockGetOriginals.mockResolvedValueOnce(bundle('building')).mockResolvedValue(bundle('ready'));
      download
        .mockImplementationOnce(async (_url: string, destination: { uri: string }) => {
          files()[destination.uri] = '{"url":"https://storage.example/signed","expires_at":"2026-11-14T10:15:00Z"}';
          return destination;
        })
        .mockImplementationOnce(async (_url: string, destination: { uri: string }) => {
          files()[destination.uri] = 'PK zip bytes';
          return destination;
        });
      renderSection();

      fireEvent.press(screen.getByTestId('originals-request'));
      expect(await screen.findByText(i18n.t('dataExport.originalsBuilding'))).toBeTruthy();
      expect(mockRequestOriginals).toHaveBeenCalledWith(undefined);
      fireEvent.press(await screen.findByTestId('originals-download', {}, { timeout: 8000 }));

      await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
      const [first, , withToken] = download.mock.calls[0] as [string, unknown, { headers: Record<string, string> }];
      expect(first).toBe('https://api.example.test/api/user/export/originals/b-1/download');
      expect(withToken.headers.Authorization).toBe('Bearer athlete-token');
      const [second, , signed] = download.mock.calls[1] as [string, unknown, { headers?: Record<string, string> }];
      expect(second).toBe('https://storage.example/signed');
      expect(signed.headers).toBeUndefined();
      const [uri, shared] = shareAsync.mock.calls[0] as [string, { mimeType: string }];
      expect(uri).toMatch(/dravr-originals-2026-11-14\.zip$/);
      expect(shared.mimeType).toBe('application/zip');
      await waitFor(() =>
        expect(Object.keys(files()).filter((u) => u.includes('dravr-originals'))).toEqual([]),
      );
    }, 15000);

    it('shares a file the server streamed itself, sealed when the athlete gave a passphrase', async () => {
      mockRequestOriginals.mockResolvedValue(bundle('ready', { sealed: true }));
      download.mockImplementationOnce(async (_url: string, destination: { uri: string }) => {
        files()[destination.uri] = 'age-encryption.org/v1 sealed bytes';
        return destination;
      });
      renderSection();

      fireEvent.changeText(screen.getByTestId('originals-passphrase'), 'short');
      expect(screen.getByText(i18n.t('dataExport.originalsSealTooShort'))).toBeTruthy();
      fireEvent.changeText(screen.getByTestId('originals-passphrase'), 'correct horse battery staple');
      fireEvent.press(screen.getByTestId('originals-request'));
      await waitFor(() =>
        expect(mockRequestOriginals).toHaveBeenCalledWith('correct horse battery staple'),
      );
      fireEvent.press(await screen.findByTestId('originals-download'));
      await waitFor(() => expect(shareAsync).toHaveBeenCalledTimes(1));
      expect(download).toHaveBeenCalledTimes(1);
      const [uri, shared] = shareAsync.mock.calls[0] as [string, { mimeType: string }];
      expect(uri).toMatch(/\.zip\.age$/);
      expect(shared.mimeType).toBe('application/octet-stream');
    });

    it.each([
      [429, 'too_many_bundles', 'dataExport.originalsRateLimited'],
      [503, 'export_busy', 'dataExport.originalsBusy'],
      [409, 'bundle_building', 'dataExport.originalsOtherBuilding'],
    ])('says what a %s refusal means and toasts nothing', async (status, code, key) => {
      mockRequestOriginals.mockRejectedValue({ response: { status, data: { error: code } } });
      renderSection();
      fireEvent.press(screen.getByTestId('originals-request'));
      expect((await screen.findByTestId('originals-failure')).props.children).toBe(i18n.t(key));
      expect(globalMutationError).not.toHaveBeenCalled();
    });
  });
});
