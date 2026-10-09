// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The .fit upload in Home's recent activities on the phone — the picked file goes up as its bytes, each outcome said in words
// ABOUTME: Red if a picked file is not sent, a cancelled pick sends anything, or a refused file is reported as added

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as DocumentPicker from 'expo-document-picker';
import type { ActivityUploadResponse } from '@pierre/shared-types';

const mockUploadActivityFile = jest.fn<Promise<ActivityUploadResponse>, [ArrayBuffer | Uint8Array]>();
jest.mock('../src/services/api', () => ({
  athleteApi: { uploadActivityFile: (bytes: ArrayBuffer | Uint8Array) => mockUploadActivityFile(bytes) },
  oauthApi: {},
}));

import { useActivityUpload } from '../src/hooks/useHome';
import { UploadActivityAction, UploadActivityStatus } from '../src/screens/home/UploadActivity';

// eslint-disable-next-line @typescript-eslint/no-require-imports
const { __mockFileContents: files } = require('expo-file-system') as { __mockFileContents: Record<string, string> };

const STORED: ActivityUploadResponse = {
  activities: [
    {
      id: `${'d'.repeat(64)}-0`,
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

function Harness() {
  const uploader = useActivityUpload();
  return (
    <>
      <UploadActivityAction uploader={uploader} />
      <UploadActivityStatus uploader={uploader} />
    </>
  );
}

function renderUpload() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Harness />
    </QueryClientProvider>,
  );
}

/** The picker hands back one file, `ride.fit`, holding `content`. */
function picks(content: string) {
  const uri = 'file:///cache/ride.fit';
  files[uri] = content;
  jest.mocked(DocumentPicker.getDocumentAsync).mockResolvedValueOnce({
    canceled: false,
    assets: [{ uri, name: 'ride.fit', size: content.length, mimeType: 'application/octet-stream', lastModified: 0 }],
  } as DocumentPicker.DocumentPickerResult);
}

function refused(status: number) {
  return Object.assign(new Error(`status ${status}`), {
    isAxiosError: true,
    response: { status, data: { code: 'x', message: 'server prose' } },
  });
}

beforeEach(() => {
  mockUploadActivityFile.mockReset();
  jest.mocked(DocumentPicker.getDocumentAsync).mockClear();
});

describe('Home .fit upload', () => {
  it('sends the picked file as its bytes and says the workout was added', async () => {
    mockUploadActivityFile.mockResolvedValue(STORED);
    picks('FIT!');
    const screen = renderUpload();

    fireEvent.press(screen.getByTestId('home-upload-action'));

    await waitFor(() => expect(screen.getByTestId('home-upload-status')).toHaveTextContent('Workout added to your activities.'));
    const [sent] = mockUploadActivityFile.mock.calls[0];
    expect(Array.from(new Uint8Array(sent as ArrayBuffer))).toEqual([70, 73, 84, 33]);
  });

  it('sends nothing when the athlete cancels the picker', async () => {
    const screen = renderUpload();

    fireEvent.press(screen.getByTestId('home-upload-action'));

    await waitFor(() => expect(DocumentPicker.getDocumentAsync).toHaveBeenCalled());
    expect(mockUploadActivityFile).not.toHaveBeenCalled();
    expect(screen.queryByTestId('home-upload-status')).toBeNull();
    expect(screen.queryByTestId('home-upload-failed')).toBeNull();
  });

  it('says a refused file is not a workout, in its own words, and lets the athlete close it', async () => {
    mockUploadActivityFile.mockImplementation(() => Promise.reject(refused(400)));
    picks('GPX?');
    const screen = renderUpload();

    fireEvent.press(screen.getByTestId('home-upload-action'));

    const alert = await screen.findByTestId('home-upload-failed');
    expect(alert).toHaveTextContent(
      "This file isn't a completed workout. Choose the .fit file your watch or bike computer recorded.Close",
    );
    expect(alert).not.toHaveTextContent('server prose');
    fireEvent.press(screen.getByTestId('home-upload-dismiss'));
    expect(screen.queryByTestId('home-upload-failed')).toBeNull();
  });
});
