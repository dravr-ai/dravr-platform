// ABOUTME: carnet #59 e2e — the mobile coach library imports a markdown coach and exports one back out
// ABOUTME: Picker, filesystem and share sheet are stubbed; the preview/import/export requests are asserted on the wire

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import * as DocumentPicker from 'expo-document-picker';
import * as Sharing from 'expo-sharing';
import type { Coach, ImportCoachResponse, ImportPreviewResponse } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

jest.mock('expo-router', () => {
  const ReactModule = require('react');
  return {
    useRouter: () => ({
      push: jest.fn(),
      replace: jest.fn(),
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => true,
    }),
    useLocalSearchParams: () => ({}),
    useSegments: () => [],
    usePathname: () => '/coaches',
    useFocusEffect: (callback: () => void | (() => void)) => {
      ReactModule.useEffect(() => callback(), [callback]);
    },
  };
});

jest.mock('../../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'user-1' }, isAuthenticated: true }),
}));

import { CoachLibraryScreen } from '../../src/screens/coaches/CoachLibraryScreen';

const PICKED_URI = 'file:///picked/seuil-et-cotes.md';

const COACH_MARKDOWN = [
  '# Seuil & Cotes',
  '',
  '## Purpose',
  'Coach de seuil pour le semi.',
  '',
  '## Instructions',
  'Bloc de cotes le mardi.',
].join('\n');

const EXISTING_COACH: Coach = {
  id: 'coach-existing',
  title: 'Recuperation active',
  description: 'Coach de recuperation',
  system_prompt: 'Tu es un coach de recuperation.',
  category: 'recovery',
  tags: ['recuperation'],
  token_count: 30,
  is_favorite: false,
  use_count: 3,
  last_used_at: null,
  created_at: '2026-06-01T08:00:00Z',
  updated_at: '2026-06-01T08:00:00Z',
  is_system: false,
};

const IMPORTED_COACH: Coach = {
  id: 'coach-imported',
  title: 'Seuil & Cotes',
  description: 'Coach de seuil pour le semi',
  system_prompt: 'Bloc de cotes le mardi.',
  category: 'training',
  tags: ['seuil', 'cotes'],
  token_count: 96,
  is_favorite: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2026-08-23T08:00:00Z',
  updated_at: '2026-08-23T08:00:00Z',
  is_system: false,
};

const PREVIEW: ImportPreviewResponse = {
  valid: true,
  parsed: {
    name: 'seuil-et-cotes',
    title: 'Seuil & Cotes',
    category: 'training',
    tags: ['seuil', 'cotes'],
    purpose: 'Coach de seuil pour le semi.',
    has_instructions: true,
    has_example_inputs: false,
    has_example_outputs: false,
    has_success_criteria: false,
  },
  duplicate_exists: false,
  token_count: 96,
};

const IMPORTED: ImportCoachResponse = {
  coach: IMPORTED_COACH,
  parsed_name: 'seuil-et-cotes',
  token_count: 96,
};

describe('carnet #59 — mobile coach markdown import/export', () => {
  let stub: HttpStub;

  beforeEach(() => {
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    // Seed the in-memory filesystem the expo-file-system mock reads from, so
    // the picked document has real markdown behind its URI.
    const fileSystem = require('expo-file-system') as {
      __mockFileContents: Record<string, string>;
    };
    Object.keys(fileSystem.__mockFileContents).forEach((key) => {
      delete fileSystem.__mockFileContents[key];
    });
    fileSystem.__mockFileContents[PICKED_URI] = COACH_MARKDOWN;

    (DocumentPicker.getDocumentAsync as jest.Mock).mockResolvedValue({
      canceled: false,
      assets: [
        {
          uri: PICKED_URI,
          name: 'seuil-et-cotes.md',
          mimeType: 'text/markdown',
          size: COACH_MARKDOWN.length,
          lastModified: 0,
        },
      ],
    });

    stub = installHttpStub({
      'GET /api/coaches?include_hidden=true': { data: { coaches: [EXISTING_COACH], total: 1 } },
      'GET /api/coaches/hidden': { data: { coaches: [], total: 0 } },
      'POST /api/coaches/import/preview': { data: PREVIEW },
      'POST /api/coaches/import': { data: IMPORTED },
      'GET /api/coaches/coach-existing/export': { data: COACH_MARKDOWN },
    });
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('previews a picked markdown file, then imports the coach it describes', async () => {
    const { getByTestId, getByText, queryByText } = render(<CoachLibraryScreen />);

    await waitFor(() => {
      expect(getByText('Recuperation active')).toBeTruthy();
    });
    expect(queryByText('Seuil & Cotes')).toBeNull();

    fireEvent.press(getByTestId('import-coach-button'));

    // The screen asks where the document is; take the file branch.
    const source = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(source[0]).toBe('Import Coach');
    await act(async () => {
      await source[2].find((button) => button.text === 'From a file')?.onPress?.();
    });

    // The preview is what the server parsed, shown before anything is written.
    await waitFor(() => {
      expect(getByTestId('import-preview-sheet')).toBeTruthy();
    });
    expect(getByText('Seuil & Cotes')).toBeTruthy();
    expect(getByText('Coach de seuil pour le semi.')).toBeTruthy();
    expect(getByText('training · 96 tokens')).toBeTruthy();
    expect(getByText('seuil, cotes')).toBeTruthy();

    // The preview request carried the file's markdown as a text/plain body.
    const previewRequest = stub
      .requestsFor('POST')
      .find((request) => request.url === '/api/coaches/import/preview');
    expect(previewRequest?.body).toBe(COACH_MARKDOWN);
    expect(previewRequest?.headers['content-type']).toBe('text/plain');

    await act(async () => {
      fireEvent.press(getByTestId('confirm-import-button'));
    });

    await waitFor(() => {
      expect(
        stub.requestsFor('POST').map((request) => request.url),
      ).toEqual(['/api/coaches/import/preview', '/api/coaches/import']);
    });

    // The imported coach is in the library, under its real title.
    await waitFor(() => {
      expect(getByTestId('coach-card-coach-imported')).toBeTruthy();
    });
    expect(getByText('Seuil & Cotes')).toBeTruthy();
  });

  it('exports a coach as markdown and hands the file to the share sheet', async () => {
    const { getByTestId, getByText } = render(<CoachLibraryScreen />);

    await waitFor(() => {
      expect(getByText('Recuperation active')).toBeTruthy();
    });

    // Long-press opens the per-coach action menu, where export lives.
    await act(async () => {
      fireEvent(getByTestId('coach-card-coach-existing'), 'longPress');
    });
    await act(async () => {
      fireEvent.press(getByTestId('export-coach-button'));
    });

    await waitFor(() => {
      expect(Sharing.shareAsync as jest.Mock).toHaveBeenCalled();
    });

    expect(stub.requestsFor('GET').map((request) => request.url)).toContain(
      '/api/coaches/coach-existing/export',
    );

    const [sharedUri, options] = (Sharing.shareAsync as jest.Mock).mock.calls[0] as [
      string,
      { mimeType: string; dialogTitle: string },
    ];
    expect(sharedUri).toBe('file:///cache/recuperation-active.md');
    expect(options.mimeType).toBe('text/markdown');

    // The file that left the app is the markdown the server produced.
    const fileSystem = require('expo-file-system') as {
      __mockFileContents: Record<string, string>;
    };
    expect(fileSystem.__mockFileContents[sharedUri]).toBe(COACH_MARKDOWN);
  });
});
