// ABOUTME: carnet #59 e2e — compare mode calls getVersionDiff, which had zero production callers repo-wide
// ABOUTME: Picking two versions renders the server's field changes, so the diff endpoint has a real consumer again

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import type { CoachDiffResponse, ListVersionsResponse } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

jest.mock('expo-router', () => {
  const ReactModule = require('react');
  return {
    useFocusEffect: (callback: () => void | (() => void)) => {
      ReactModule.useEffect(() => callback(), [callback]);
    },
  };
});

import { CoachVersionHistory } from '../../src/components/coaches/CoachVersionHistory';

const PROMPT_V3 = 'Tu es un coach de seuil. Bloc de cotes le mardi.';
const PROMPT_V2 = 'Tu es un coach de seuil. Sortie longue le dimanche.';

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
      content_snapshot: { title: 'Seuil', system_prompt: PROMPT_V2 },
      change_summary: 'Sortie longue deplacee au dimanche',
      created_at: '2026-08-14T08:00:00Z',
      created_by_name: 'ChefFamille',
    },
  ],
  current_version: 3,
  total: 2,
};

const DIFF: CoachDiffResponse = {
  from_version: 2,
  to_version: 3,
  changes: [
    { field: 'title', old_value: 'Seuil', new_value: 'Seuil & Cotes' },
    { field: 'system_prompt', old_value: PROMPT_V2, new_value: PROMPT_V3 },
  ],
};

describe('carnet #59 — mobile coach version compare', () => {
  let stub: HttpStub;

  beforeEach(() => {
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    stub = installHttpStub({
      'GET /api/coaches/coach-e2e-1/versions?limit=50': { data: VERSIONS },
      'GET /api/coaches/coach-e2e-1/versions/2/diff/3': { data: DIFF },
    });
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('compares two versions and renders the server-reported field changes', async () => {
    const { getByTestId, getByText } = render(
      <CoachVersionHistory
        coachId="coach-e2e-1"
        coachTitle="Seuil & Cotes"
        isOpen
        onClose={jest.fn()}
      />,
    );

    await waitFor(() => {
      expect(getByTestId('version-item-3')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('toggle-compare-mode'));
    });
    expect(getByTestId('version-compare-hint')).toBeTruthy();

    await act(async () => {
      fireEvent.press(getByTestId('version-item-2'));
    });
    // One pick is not a comparison — nothing is requested yet.
    expect(
      stub.requestsFor('GET').map((request) => request.url),
    ).toEqual(['/api/coaches/coach-e2e-1/versions?limit=50']);

    await act(async () => {
      fireEvent.press(getByTestId('version-item-3'));
    });

    // The pair is sent oldest → newest, at the diff endpoint.
    await waitFor(() => {
      expect(stub.requestsFor('GET').map((request) => request.url)).toEqual([
        '/api/coaches/coach-e2e-1/versions?limit=50',
        '/api/coaches/coach-e2e-1/versions/2/diff/3',
      ]);
    });

    await waitFor(() => {
      expect(getByText('v2 → v3')).toBeTruthy();
    });
    expect(getByTestId('diff-field-title')).toBeTruthy();
    expect(getByTestId('diff-field-system_prompt')).toBeTruthy();
    expect(getByText('− Seuil')).toBeTruthy();
    expect(getByText('+ Seuil & Cotes')).toBeTruthy();
    expect(getByText(`+ ${PROMPT_V3}`)).toBeTruthy();
  });

  it('leaves compare mode without a stale diff on screen', async () => {
    const { getByTestId, queryByTestId } = render(
      <CoachVersionHistory
        coachId="coach-e2e-1"
        coachTitle="Seuil & Cotes"
        isOpen
        onClose={jest.fn()}
      />,
    );

    await waitFor(() => {
      expect(getByTestId('version-item-3')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('toggle-compare-mode'));
    });
    await act(async () => {
      fireEvent.press(getByTestId('version-item-2'));
    });
    await act(async () => {
      fireEvent.press(getByTestId('version-item-3'));
    });
    await waitFor(() => {
      expect(getByTestId('diff-field-title')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('toggle-compare-mode'));
    });

    expect(queryByTestId('version-compare-panel')).toBeNull();
    expect(queryByTestId('diff-field-title')).toBeNull();
  });
});
