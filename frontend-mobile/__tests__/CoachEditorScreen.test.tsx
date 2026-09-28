// ABOUTME: Unit tests for CoachEditorScreen — the edit-only sheet for one of the athlete's own coaches
// ABOUTME: Pins load-by-id, save through update, delete with confirmation, version history (list, compare, revert), no create mode

import React from 'react';
import { render as rtlRender, fireEvent, waitFor, within } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
let mockParams: { agentId?: string } = { agentId: 'coach-1' };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
    useLocalSearchParams: () => mockParams,
  }),
);

const mockGet = jest.fn();
const mockUpdate = jest.fn();
const mockDelete = jest.fn();
const mockCreate = jest.fn();
const mockListVersions = jest.fn();
const mockDiffVersion = jest.fn();
const mockRevertToVersion = jest.fn();

jest.mock('../src/services/api', () => ({
  coachesApi: {
    get: (...args: unknown[]) => mockGet(...args),
    update: (...args: unknown[]) => mockUpdate(...args),
    delete: (...args: unknown[]) => mockDelete(...args),
    create: (...args: unknown[]) => mockCreate(...args),
    listVersions: (...args: unknown[]) => mockListVersions(...args),
    diffVersion: (...args: unknown[]) => mockDiffVersion(...args),
    revertToVersion: (...args: unknown[]) => mockRevertToVersion(...args),
  },
}));

const queryClients: QueryClient[] = [];
afterEach(() => {
  queryClients.splice(0).forEach((client) => client.clear());
});

/** The editor's history group reads server state through React Query. */
function render(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false, gcTime: 0 } },
  });
  queryClients.push(queryClient);
  return rtlRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>);
}

const HISTORY = {
  versions: [
    {
      version: 2,
      content_snapshot: { title: 'Coach Tempo B', system_prompt: 'prompt two' },
      change_summary: null,
      created_at: '2026-09-20T08:30:00Z',
      created_by_name: 'Ada Lovelace',
    },
    {
      version: 1,
      content_snapshot: { title: 'Coach Threshold', system_prompt: 'prompt one' },
      change_summary: null,
      created_at: '2026-09-10T08:30:00Z',
      created_by_name: null,
    },
  ],
  current_version: 2,
  total: 2,
};

jest.spyOn(Alert, 'alert');

import { CoachEditorScreen } from '../src/screens/coaches/CoachEditorScreen';
import type { Agent } from '../src/types';

const storedCoach = (overrides: Partial<Agent> = {}): Agent => ({
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
    mockParams = { agentId: 'coach-1' };
    mockGet.mockResolvedValue(storedCoach());
    mockUpdate.mockImplementation(async (_id: string, request: Partial<Agent>) => storedCoach(request));
    mockDelete.mockResolvedValue(undefined);
    mockListVersions.mockResolvedValue(HISTORY);
    mockDiffVersion.mockResolvedValue({
      version: 1,
      changes: [
        { field: 'title', old_value: 'Coach Threshold', new_value: 'Coach Tempo' },
        { field: 'category', old_value: 'recovery', new_value: 'training' },
      ],
    });
    mockRevertToVersion.mockResolvedValue({
      agent: storedCoach({ title: 'Coach Threshold', system_prompt: 'prompt one' }),
      reverted_to_version: 1,
      new_version: 3,
    });
  });

  it('loads the agent by id and hydrates the form as an edit sheet', async () => {
    const { findByTestId, getByText, queryByTestId, queryByText } = render(<CoachEditorScreen />);

    expect(await findByTestId('coach-editor-screen')).toBeTruthy();
    expect(mockGet).toHaveBeenCalledWith('coach-1');
    expect((await findByTestId('coach-title-input')).props.value).toBe('Coach Tempo');
    // The sheet's title is the native header's, set by the discover layout;
    // the screen draws none of its own.
    expect(queryByText('Edit Agent')).toBeNull();
    expect(getByText('Title *')).toBeTruthy();
    // No create mode, no retired wizard version button, no fork wording. The two strings are
    // the retired wizard's own, as it spelled them before b460057d3 deleted it;
    // neither is in the catalogue now, so the new vocabulary would name text no
    // build can render.
    expect(queryByText('Create Coach')).toBeNull();
    expect(queryByTestId('version-history-button')).toBeNull();
    expect(queryByTestId('forked-from-banner')).toBeNull();
    expect(queryByText('Forked from a system agent')).toBeNull();
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

  it('deletes the agent after confirmation and goes back', async () => {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      const destructive = buttons?.find((b: { text: string }) => b.text === 'Delete');
      destructive?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);
    await findByTestId('coach-editor-screen');

    fireEvent.press(getByTestId('delete-coach-button'));

    expect(Alert.alert).toHaveBeenCalledWith(
      'Delete Agent?',
      'Delete agent "Coach Tempo"? This cannot be undone.',
      expect.any(Array),
    );
    await waitFor(() => expect(mockDelete).toHaveBeenCalledWith('coach-1'));
    await waitFor(() => expect(mockRouter.back).toHaveBeenCalledTimes(1));
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('keeps the agent when the deletion is cancelled', async () => {
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

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', 'Failed to delete agent'));
    expect(mockRouter.back).not.toHaveBeenCalled();
  });

  it('lists the stored versions newest first, with the author when known', async () => {
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);

    const newest = await findByTestId('agent-version-2');
    expect(mockListVersions).toHaveBeenCalledWith('coach-1');
    expect(within(newest).getByText('Version 2')).toBeTruthy();
    expect(within(newest).getByTestId('agent-version-meta').props.children).toContain('Ada Lovelace');
    const oldest = getByTestId('agent-version-1');
    expect(within(oldest).getByTestId('agent-version-meta').props.children).not.toContain('by');
  });

  it('compares a version with the current content and shows each changed field', async () => {
    const { findByTestId } = render(<CoachEditorScreen />);

    fireEvent.press(await findByTestId('agent-version-compare-1'));

    const diff = await findByTestId('agent-version-diff-1');
    expect(mockDiffVersion).toHaveBeenCalledWith('coach-1', 1);
    const title = within(diff).getByTestId('agent-version-change-title');
    expect(within(title).getByTestId('agent-version-old-value').props.children).toBe('Coach Threshold');
    expect(within(title).getByTestId('agent-version-new-value').props.children).toBe('Coach Tempo');
    // Enum values are named for the athlete, never shown raw.
    const category = within(diff).getByTestId('agent-version-change-category');
    expect(within(category).getByTestId('agent-version-old-value').props.children).toBe('Recovery');
    expect(within(diff).queryByTestId('agent-version-change-system_prompt')).toBeNull();
  });

  it('reverts after confirmation, refreshes the history and re-fills the form', async () => {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      buttons?.find((b: { text: string }) => b.text === 'Revert')?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);

    fireEvent.press(await findByTestId('agent-version-revert-1'));

    expect(Alert.alert).toHaveBeenCalledWith(
      'Revert to version 1?',
      expect.stringContaining("this version's content back"),
      expect.any(Array),
    );
    await waitFor(() => expect(mockRevertToVersion).toHaveBeenCalledWith('coach-1', 1));
    await waitFor(() => expect(getByTestId('coach-title-input').props.value).toBe('Coach Threshold'));
    await waitFor(() => expect(mockListVersions).toHaveBeenCalledTimes(2));

    // A save after the revert keeps the restored content.
    fireEvent.press(getByTestId('save-button'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith('coach-1', expect.objectContaining({ title: 'Coach Threshold' })),
    );
  });

  it('reverts nothing when the confirmation is cancelled', async () => {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      buttons?.find((b: { text: string }) => b.text === 'Cancel')?.onPress?.();
    });
    const { findByTestId, getByTestId } = render(<CoachEditorScreen />);

    fireEvent.press(await findByTestId('agent-version-revert-2'));

    expect(mockRevertToVersion).not.toHaveBeenCalled();
    expect(getByTestId('coach-title-input').props.value).toBe('Coach Tempo');
  });

  it('says so when the agent has never been edited', async () => {
    mockListVersions.mockResolvedValue({ versions: [], current_version: 0, total: 0 });
    const { findByTestId, queryByTestId } = render(<CoachEditorScreen />);

    expect(await findByTestId('agent-version-history-empty')).toBeTruthy();
    expect(queryByTestId('agent-version-1')).toBeNull();
  });

  it('shows the not-found state when the route carries no agent id', async () => {
    mockParams = {};
    const { getByTestId, getByText } = render(<CoachEditorScreen />);

    expect(getByTestId('coach-editor-missing')).toBeTruthy();
    expect(getByText('Agent not found')).toBeTruthy();
    expect(mockGet).not.toHaveBeenCalled();
  });
});
