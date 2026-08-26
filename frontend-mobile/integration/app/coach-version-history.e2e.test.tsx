// ABOUTME: WAVE 0 e2e — the mobile coach editor still lists versions and reverts through the real api-client
// ABOUTME: Web's CoachWizard/CoachVersionHistory were deleted; mobile's callers must keep hitting the same endpoints

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import type { Coach, ListVersionsResponse, RevertVersionResponse } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

// The editor reads the coach id off the route. Everything else in expo-router
// keeps the shape jest.setup.js installs, including the focus effect that
// triggers the version load when the modal opens.
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
    useLocalSearchParams: () => ({ coachId: 'coach-e2e-1' }),
    useSegments: () => [],
    usePathname: () => '/coaches/coach-e2e-1',
    useFocusEffect: (callback: () => void | (() => void)) => {
      ReactModule.useEffect(() => callback(), [callback]);
    },
  };
});

import { CoachEditorScreen } from '../../src/screens/coaches/CoachEditorScreen';

const PROMPT_V3 = 'Tu es un coach de seuil. Bloc de cotes le mardi.';
const PROMPT_V2 = 'Tu es un coach de seuil. Sortie longue le dimanche.';

function coachAt(systemPrompt: string): Coach {
  return {
    id: 'coach-e2e-1',
    title: 'Seuil & Cotes',
    description: 'Coach de seuil pour le semi',
    system_prompt: systemPrompt,
    category: 'training',
    tags: ['seuil'],
    token_count: 42,
    is_favorite: false,
    use_count: 7,
    last_used_at: '2026-08-20T08:00:00Z',
    created_at: '2026-07-01T08:00:00Z',
    updated_at: '2026-08-21T08:00:00Z',
    is_system: false,
  };
}

const VERSIONS: ListVersionsResponse = {
  versions: [
    {
      version: 3,
      content_snapshot: { title: 'Seuil & Cotes', system_prompt: PROMPT_V3 },
      change_summary: 'Bloc de cotes ajoute',
      created_at: '2026-08-21T08:00:00Z',
      created_by_name: 'ChefFamille',
    },
    {
      version: 2,
      content_snapshot: { title: 'Seuil & Cotes', system_prompt: PROMPT_V2 },
      change_summary: 'Sortie longue deplacee au dimanche',
      created_at: '2026-08-14T08:00:00Z',
      created_by_name: 'ChefFamille',
    },
    {
      version: 1,
      content_snapshot: { title: 'Seuil', system_prompt: 'Premier jet.' },
      change_summary: 'Version initiale',
      created_at: '2026-07-01T08:00:00Z',
      created_by_name: 'ChefFamille',
    },
  ],
  current_version: 3,
  total: 3,
};

const REVERTED: RevertVersionResponse = {
  coach: coachAt(PROMPT_V2),
  reverted_to_version: 2,
  new_version: 4,
};

describe('WAVE 0 — mobile coach editor version history', () => {
  let stub: HttpStub;
  // The editor reloads the coach after a revert; the second GET must answer
  // with the reverted content, which is what proves the round trip landed.
  let coachLoads = 0;

  beforeEach(() => {
    coachLoads = 0;
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    stub = installHttpStub({
      'GET /api/coaches/coach-e2e-1': () => {
        coachLoads += 1;
        return { data: coachAt(coachLoads === 1 ? PROMPT_V3 : PROMPT_V2) };
      },
      'GET /api/coaches/coach-e2e-1/versions?limit=50': { data: VERSIONS },
      'POST /api/coaches/coach-e2e-1/versions/2/revert': { data: REVERTED },
    });
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('loads the coach, then lists every stored version with its summary', async () => {
    const { getByTestId, getByText } = render(<CoachEditorScreen />);

    await waitFor(() => {
      expect(getByTestId('system-prompt-input').props.value).toBe(PROMPT_V3);
    });

    await act(async () => {
      fireEvent.press(getByTestId('version-history-button'));
    });

    await waitFor(() => {
      expect(getByText('3 versions saved')).toBeTruthy();
    });

    // Concrete labels, straight off the wire payload.
    expect(getByText('Current: v3')).toBeTruthy();
    expect(getByText('Bloc de cotes ajoute')).toBeTruthy();
    expect(getByText('Sortie longue deplacee au dimanche')).toBeTruthy();
    expect(getByText('Version initiale')).toBeTruthy();
    expect(getByTestId('version-item-1')).toBeTruthy();

    // The endpoint the deleted web copies used to share.
    expect(stub.requestsFor('GET').map((request) => request.url)).toEqual([
      '/api/coaches/coach-e2e-1',
      '/api/coaches/coach-e2e-1/versions?limit=50',
    ]);
  });

  it('reverts to v2 and reloads the editor with the reverted prompt', async () => {
    const { getByTestId, getByText } = render(<CoachEditorScreen />);

    await waitFor(() => {
      expect(getByTestId('system-prompt-input').props.value).toBe(PROMPT_V3);
    });

    await act(async () => {
      fireEvent.press(getByTestId('version-history-button'));
    });
    await waitFor(() => expect(getByTestId('version-item-2')).toBeTruthy());

    // Expand v2 and press its revert control.
    await act(async () => {
      fireEvent.press(getByTestId('version-item-2'));
    });
    expect(getByText('Revert to v2')).toBeTruthy();
    fireEvent.press(getByTestId('revert-button-2'));

    // Confirm through the alert the component raises.
    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Confirm Revert');
    const revertButton = confirm[2].find((button) => button.text === 'Revert');
    await act(async () => {
      await revertButton?.onPress?.();
    });

    // The POST went to the versioned revert endpoint...
    await waitFor(() => {
      expect(stub.requestsFor('POST').map((request) => request.url)).toEqual([
        '/api/coaches/coach-e2e-1/versions/2/revert',
      ]);
    });

    // ...and the editor re-read the coach, now carrying v2's prompt.
    await waitFor(() => {
      expect(getByTestId('system-prompt-input').props.value).toBe(PROMPT_V2);
    });
    expect(coachLoads).toBe(2);
  });
});
