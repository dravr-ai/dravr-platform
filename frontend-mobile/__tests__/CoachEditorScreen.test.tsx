// ABOUTME: Unit tests for CoachEditorScreen — the edit-only sheet for one of the athlete's own coaches
// ABOUTME: Pins load-by-id, save through update, delete with confirmation, and the absence of any create mode

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
let mockParams: { coachId?: string } = { coachId: 'coach-1' };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => mockParams,
}));

const mockGet = jest.fn();
const mockUpdate = jest.fn();
const mockDelete = jest.fn();
const mockCreate = jest.fn();

jest.mock('../src/services/api', () => ({
  coachesApi: {
    get: (...args: unknown[]) => mockGet(...args),
    update: (...args: unknown[]) => mockUpdate(...args),
    delete: (...args: unknown[]) => mockDelete(...args),
    create: (...args: unknown[]) => mockCreate(...args),
  },
}));

jest.spyOn(Alert, 'alert');

import { CoachEditorScreen } from '../src/screens/coaches/CoachEditorScreen';
import type { Coach } from '../src/types';

const storedCoach = (overrides: Partial<Coach> = {}): Coach => ({
  id: 'coach-1',
  title: 'Coach Tempo',
  description: 'Threshold work',
  system_prompt: 'You are a tempo coach.',
  category: 'training',
  tags: ['tempo'],
  token_count: 12,
  is_favorite: false,
  is_system: false,
  use_count: 3,
  last_used_at: null,
  created_at: '2026-08-01T00:00:00Z',
  updated_at: '2026-08-01T00:00:00Z',
  forked_from: 'store-tempo',
  handle: 'coach-tempo',
  startup_query: 'Analyze my tempo runs',
  data_requirements: {
    activities: {
      count: 15,
      time_frame: '8w',
      mode: 'summary',
      format: 'toon',
      analysis_type: 'general_overview',
    },
    athlete_profile: false,
  },
  ...overrides,
});

describe('CoachEditorScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockParams = { coachId: 'coach-1' };
    mockGet.mockResolvedValue(storedCoach());
    mockUpdate.mockImplementation(async (_id: string, request: Partial<Coach>) => storedCoach(request));
    mockDelete.mockResolvedValue(undefined);
  });

  it('loads the coach by id and hydrates the form as an edit sheet', async () => {
    const { findByTestId, getByText, queryByTestId, queryByText } = render(<CoachEditorScreen />);

    expect(await findByTestId('coach-editor-screen')).toBeTruthy();
    expect(mockGet).toHaveBeenCalledWith('coach-1');
    expect((await findByTestId('coach-title-input')).props.value).toBe('Coach Tempo');
    expect(getByText('Edit Coach')).toBeTruthy();
    // No create mode, no version history, no fork wording.
    expect(queryByText('Create Coach')).toBeNull();
    expect(queryByTestId('version-history-button')).toBeNull();
    expect(queryByTestId('forked-from-banner')).toBeNull();
    expect(queryByText('Forked from a system coach')).toBeNull();
  });

  it('saves through coachesApi.update and goes back', async () => {
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);
    await findByTestId('coach-editor-screen');

    fireEvent.changeText(getByTestId('coach-title-input'), 'Coach Tempo v2');
    fireEvent.press(getByTestId('save-button'));

    await waitFor(() => {
      expect(mockUpdate).toHaveBeenCalledWith(
        'coach-1',
        expect.objectContaining({
          title: 'Coach Tempo v2',
          system_prompt: 'You are a tempo coach.',
          startup_query: 'Analyze my tempo runs',
          data_requirements: expect.objectContaining({
            activities: expect.objectContaining({ count: 15, time_frame: '8w' }),
          }),
        }),
      );
    });
    await waitFor(() => expect(mockRouter.back).toHaveBeenCalledTimes(1));
    expect(mockCreate).not.toHaveBeenCalled();
  });

  it('deletes the coach after confirmation and goes back', async () => {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      const destructive = buttons?.find((b: { text: string }) => b.text === 'Delete');
      destructive?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);
    await findByTestId('coach-editor-screen');

    fireEvent.press(getByTestId('delete-coach-button'));

    expect(Alert.alert).toHaveBeenCalledWith(
      'Delete Coach?',
      'Delete coach "Coach Tempo"? This cannot be undone.',
      expect.any(Array),
    );
    await waitFor(() => expect(mockDelete).toHaveBeenCalledWith('coach-1'));
    await waitFor(() => expect(mockRouter.back).toHaveBeenCalledTimes(1));
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('keeps the coach when the deletion is cancelled', async () => {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      const cancel = buttons?.find((b: { text: string }) => b.text === 'Cancel');
      cancel?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);
    await findByTestId('coach-editor-screen');

    fireEvent.press(getByTestId('delete-coach-button'));

    expect(mockDelete).not.toHaveBeenCalled();
    expect(mockRouter.back).not.toHaveBeenCalled();
  });

  it('surfaces a failed delete and stays on the sheet', async () => {
    mockDelete.mockRejectedValue(new Error('boom'));
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      const destructive = buttons?.find((b: { text: string }) => b.text === 'Delete');
      destructive?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);
    await findByTestId('coach-editor-screen');

    fireEvent.press(getByTestId('delete-coach-button'));

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', 'Failed to delete coach'));
    expect(mockRouter.back).not.toHaveBeenCalled();
  });

  it('shows the not-found state when the route carries no coach id', async () => {
    mockParams = {};
    const { getByTestId, getByText } = render(<CoachEditorScreen />);

    expect(getByTestId('coach-editor-missing')).toBeTruthy();
    expect(getByText('Coach not found')).toBeTruthy();
    expect(mockGet).not.toHaveBeenCalled();
  });
});
