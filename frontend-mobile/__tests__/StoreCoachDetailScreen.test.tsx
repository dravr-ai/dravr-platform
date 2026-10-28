// ABOUTME: Unit tests for StoreCoachDetailScreen component
// ABOUTME: Tests detail display, install → hint → Open chat, uninstall by installation id, and Edit (fork a membership first)

import React from 'react';
import { act, render as rtlRender, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { Alert } from 'react-native';
import { QUERY_KEYS } from '@pierre/shared-constants';

// Per-file expo-router mock override with spyable router methods
const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
    useLocalSearchParams: () => ({ agentId: 'test-coach-id' }),
  }),
);

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
  }),
}));

const mockTrackMobile = jest.fn();
jest.mock('../src/services/analytics', () => ({
  trackMobile: (...args: unknown[]) => mockTrackMobile(...args),
}));

// Mock API service
const mockGet = jest.fn();
const mockInstall = jest.fn();
const mockUninstall = jest.fn();
const mockGetInstallations = jest.fn();
const mockOpenPersonalTopic = jest.fn();
const mockFork = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    openPersonalTopic: (...args: unknown[]) => mockOpenPersonalTopic(...args),
  },
  coachesApi: {
    fork: (...args: unknown[]) => mockFork(...args),
  },
  storeApi: {
    get: (...args: unknown[]) => mockGet(...args),
    install: (...args: unknown[]) => mockInstall(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    getInstallations: (...args: unknown[]) => mockGetInstallations(...args),
  },
}));

// Mock Alert
jest.spyOn(Alert, 'alert');

import { StoreCoachDetailScreen } from '../src/screens/store/StoreCoachDetailScreen';
import { CHAT_THREAD_ROUTE, COACH_EDIT_ROUTE } from '../src/navigation/routes';
import type { StoreAgent, StoreAgentDetail, AgentCategory } from '../src/types';

const COACH_HANDLE = 'marathon-training-coach';

const createMockStoreCoachDetail = (overrides: Partial<StoreAgentDetail> = {}): StoreAgentDetail => ({
  id: 'test-coach-id',
  title: 'Marathon Training Agent',
  description: 'A comprehensive marathon training program',
  category: 'training' as AgentCategory,
  tags: ['marathon', 'running', 'endurance'],
  sample_prompts: [
    'What should my weekly mileage be?',
    'How do I prevent injuries?',
    'What pace should I run my long runs?',
  ],
  system_prompt: 'You are an expert marathon coach with years of experience...',
  token_count: 1200,
  install_count: 75,
  icon_url: null,
  published_at: '2024-01-15T00:00:00Z',
  author_id: 'author-123',
  created_at: '2024-01-10T00:00:00Z',
  publish_status: 'published',
  is_own: false,
  handle: COACH_HANDLE,
  ...overrides,
});

// Installing a catalogue agent puts it on the athlete's team: the install
// response and the installations list return the listing itself (same id).
const membership: StoreAgent = {
  id: 'test-coach-id',
  title: 'Marathon Training Agent',
  description: 'A comprehensive marathon training program',
  category: 'training' as AgentCategory,
  tags: ['marathon', 'running', 'endurance'],
  sample_prompts: [],
  token_count: 1200,
  install_count: 76,
  icon_url: null,
  published_at: '2024-01-15T00:00:00Z',
  author_id: 'author-123',
  handle: COACH_HANDLE,
};

// A copy — forked when the athlete edited the agent, or installed from another
// workspace — has an id of its own and carries the listing's handle.
const installedCopy: StoreAgent = {
  ...membership,
  id: 'installed-copy-1',
  install_count: 0,
  published_at: null,
  author_id: null,
};

// Open chat refreshes the conversation list the chat tab reads from the
// React Query cache, so every render gets a client of its own.
let queryClient: QueryClient;
function render(ui: React.ReactElement) {
  return rtlRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>);
}

describe('StoreCoachDetailScreen', () => {
  beforeEach(() => {
    queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    jest.clearAllMocks();
    mockRouter.push.mockClear();
    mockRouter.replace.mockClear();
    mockRouter.back.mockClear();
    mockRouter.navigate.mockClear();
    mockGet.mockResolvedValue(createMockStoreCoachDetail());
    mockGetInstallations.mockResolvedValue({ agents: [] });
    mockInstall.mockResolvedValue({ message: 'Coach installed successfully', agent: membership });
    mockFork.mockResolvedValue({ id: 'forked-copy-1', title: 'Marathon Training Agent', forked_from: 'test-coach-id' });
    mockOpenPersonalTopic.mockResolvedValue({
      conversation: { id: 'conv-personal', title: 'Marathon Training Agent', personal: true },
      divider: null,
    });
  });

  describe('rendering', () => {
    it('should show loading state initially', async () => {
      // Delay the API response
      let resolvePromise: (value: unknown) => void;
      mockGet.mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        })
      );

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      expect(getByText('Loading agent details...')).toBeTruthy();

      // Cleanup
      resolvePromise!(createMockStoreCoachDetail());
      await waitFor(() => {});
    });

    it('should render agent title', async () => {
      const { getAllByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Agent').length).toBeGreaterThan(0);
      });
    });

    // The bar used to float over the list and had to clear a floating tab bar
    // by a constant that once put it inside that bar (carnet#208). The system
    // tab bar ends the screen now, so the bar just sits at the bottom of it.
    it('keeps its action bar in the flow, padded for the home indicator', async () => {
      const { findByTestId } = render(<StoreCoachDetailScreen />);

      const bar = await findByTestId('coach-detail-action-bar');
      const style = bar.props.style as Record<string, unknown> | Array<Record<string, unknown>>;
      const flat = Array.isArray(style) ? Object.assign({}, ...style) : style;

      // Nothing floats: the system tab bar sits below the screen, and the bar
      // carries only the device inset the safe-area mock reports.
      expect(flat.bottom).toBeUndefined();
      expect(flat.position).toBeUndefined();
      expect(flat.paddingBottom).toBe(34);
    });

    it('names the native header after the agent', async () => {
      const { findByTestId } = render(<StoreCoachDetailScreen />);

      expect(await findByTestId('stack-header-title')).toHaveTextContent('Marathon Training Agent');
    });

    it('should render agent description', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('A comprehensive marathon training program')).toBeTruthy();
      });
    });

    it('should render category badge', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Training')).toBeTruthy();
      });
    });

    it('should render install count', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('75 users')).toBeTruthy();
      });
    });

    it('should render tags', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('marathon')).toBeTruthy();
        expect(getByText('running')).toBeTruthy();
        expect(getByText('endurance')).toBeTruthy();
      });
    });

    it('should render sample prompts', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('What should my weekly mileage be?')).toBeTruthy();
        expect(getByText('How do I prevent injuries?')).toBeTruthy();
      });
    });

    it('should render system prompt section', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('System Prompt')).toBeTruthy();
      });
    });

    it('should render token count in details', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Token Count')).toBeTruthy();
        expect(getByText('1,200')).toBeTruthy();
      });
    });

    it('writes the token count and the publish date in the app language', async () => {
      mockGet.mockResolvedValue(createMockStoreCoachDetail({ published_at: '2024-01-15T12:00:00Z' }));
      await act(async () => {
        await i18n.changeLanguage('fr');
      });
      try {
        const { getByText } = render(<StoreCoachDetailScreen />);

        await waitFor(() => {
          expect(getByText(/^1\s200$/)).toBeTruthy();
        });
        expect(getByText('15 janv. 2024')).toBeTruthy();
      } finally {
        await act(async () => {
          await i18n.changeLanguage('en');
        });
      }
    });

    // carnet#1212: the count chose its form with an inline `=== 1`, so a
    // listing nobody installed read « 0 utilisateurs ».
    it('counts the listing\'s users in the French singular for 0', async () => {
      mockGet.mockResolvedValue(createMockStoreCoachDetail({ install_count: 0 }));
      await act(async () => {
        await i18n.changeLanguage('fr');
      });
      try {
        const { getByTestId } = render(<StoreCoachDetailScreen />);
        await waitFor(() => {
          expect(getByTestId('install-count').props.children).toBe('0 utilisateur');
        });
      } finally {
        await act(async () => {
          await i18n.changeLanguage('en');
        });
      }
    });

    it('should show error state when agent not found', async () => {
      mockGet.mockResolvedValue(null);

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Agent not found')).toBeTruthy();
      });
    });
  });

  describe('install functionality', () => {
    it('should show Install button when agent is not installed', async () => {
      mockGetInstallations.mockResolvedValue({ agents: [] });

      const { getByText, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
      expect(queryByTestId('edit-coach-button')).toBeNull();
    });

    it('offers neither Install nor Installed on the author\'s own listing', async () => {
      // The author's own agent is on their team as their creation, not as a
      // Store install; installations leave it out, and there is nothing to
      // install or remove.
      mockGet.mockResolvedValue(createMockStoreCoachDetail({ is_own: true }));
      mockGetInstallations.mockResolvedValue({ agents: [] });

      const { getByTestId, queryByText, queryByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('coach-detail-action-bar')).toBeTruthy();
      });
      expect(queryByText('Install Agent')).toBeNull();
      expect(queryByTestId('installed-button')).toBeNull();
      expect(queryByTestId('edit-coach-button')).toBeNull();
    });

    it('should call install when Install button is pressed', async () => {
      mockGetInstallations.mockResolvedValue({ agents: [] });

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Agent'));

      await waitFor(() => {
        expect(mockInstall).toHaveBeenCalledWith('test-coach-id');
      });
    });

    it('shows the post-install hint that teaches /agent add @handle, not an alert', async () => {
      const { getByText, findByTestId, getByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Agent'));

      expect(await findByTestId('post-install-hint')).toBeTruthy();
      expect(getByTestId('post-install-title')).toHaveTextContent('“Marathon Training Agent” is in your agents');
      expect(getByTestId('post-install-body')).toHaveTextContent(
        `Use it in any chat: /agent add @${COACH_HANDLE} — or mention @${COACH_HANDLE} for one turn`,
      );
      expect(Alert.alert).not.toHaveBeenCalled();
      // The install is what the funnel counts.
      expect(mockTrackMobile).toHaveBeenCalledWith({ name: 'feature_engaged', props: { feature: 'coach_installed' } });
      // The agent the install put on the team is what the screen now addresses.
      expect(getByText('Installed')).toBeTruthy();
      expect(getByTestId('edit-coach-button')).toBeTruthy();
    });

    it('Open chat on the hint opens a topic bound to the installed agent in the personal thread', async () => {
      const invalidate = jest.spyOn(queryClient, 'invalidateQueries');
      const { getByText, findByTestId, getByTestId, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Agent'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-open-chat'));

      // Bound with the topic to the agent the install returned — the catalogue
      // agent itself, since no copy is minted — so the server has posted the
      // agent's welcome before the thread is read, in the personal thread.
      await waitFor(() => {
        expect(mockRouter.push).toHaveBeenCalledWith({
          pathname: CHAT_THREAD_ROUTE,
          params: { conversationId: 'conv-personal' },
        });
      });
      expect(mockOpenPersonalTopic).toHaveBeenCalledTimes(1);
      expect(mockOpenPersonalTopic).toHaveBeenCalledWith({ agent_id: 'test-coach-id' });
      expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.chat.conversations() });
      expect(queryByTestId('post-install-hint')).toBeNull();
    });

    it('Open chat opens one topic however often it is tapped', async () => {
      let resolveCreate: (opened: { conversation: { id: string; title: string } }) => void = () => {};
      mockOpenPersonalTopic.mockReturnValue(
        new Promise((resolve) => {
          resolveCreate = resolve;
        }),
      );
      const { getByText, findByTestId, getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Agent'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-open-chat'));
      fireEvent.press(getByTestId('post-install-open-chat'));
      await act(async () => {
        resolveCreate({ conversation: { id: 'conv-personal', title: 'Marathon Training Agent' } });
      });

      expect(mockOpenPersonalTopic).toHaveBeenCalledTimes(1);
      expect(mockRouter.push).toHaveBeenCalledTimes(1);
    });

    it('Open chat that cannot open the topic says so and keeps the hint', async () => {
      mockOpenPersonalTopic.mockRejectedValue(new Error('service unavailable'));
      const { getByText, findByTestId, getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Agent'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-open-chat'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Error', 'Could not open a chat');
      });
      expect(mockRouter.push).not.toHaveBeenCalled();
      expect(getByTestId('post-install-hint')).toBeTruthy();
    });

    it('Dismiss hides the hint and leaves the agent installed', async () => {
      const { getByText, findByTestId, getByTestId, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Agent'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-dismiss'));

      expect(queryByTestId('post-install-hint')).toBeNull();
      expect(mockRouter.push).not.toHaveBeenCalled();
      expect(getByText('Installed')).toBeTruthy();
    });

    it('should show error alert on installation failure', async () => {
      mockGetInstallations.mockResolvedValue({ agents: [] });
      mockInstall.mockRejectedValue(new Error('Installation failed'));

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Agent'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Error',
          'Failed to install agent. Please try again.'
        );
      });
    });
  });

  // Mocks Alert so the confirmation's destructive action runs at once.
  function confirmEveryUninstall() {
    (Alert.alert as jest.Mock).mockImplementation((_title, _message, buttons) => {
      const uninstallButton = buttons?.find((b: { text: string }) => b.text === 'Uninstall');
      if (uninstallButton?.onPress) {
        uninstallButton.onPress();
      }
    });
  }

  describe('membership on the catalogue agent', () => {
    beforeEach(() => {
      // The installations list returns the listing itself for a membership.
      mockGetInstallations.mockResolvedValue({ agents: [membership] });
    });

    it('recognises the membership by the listing handle', async () => {
      const { getByText, getByTestId, queryByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });
      expect(queryByText('Install Agent')).toBeNull();
      expect(getByTestId('edit-coach-button')).toBeTruthy();
    });

    it('Edit agent forks the catalogue agent and opens the editor on the copy', async () => {
      const { getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });

      fireEvent.press(getByTestId('edit-coach-button'));

      await waitFor(() => {
        expect(mockRouter.push).toHaveBeenCalledWith({
          pathname: COACH_EDIT_ROUTE,
          params: { agentId: 'forked-copy-1' },
        });
      });
      expect(mockFork).toHaveBeenCalledTimes(1);
      expect(mockFork).toHaveBeenCalledWith('test-coach-id');
      // The editor never opens on the catalogue agent itself.
      expect(mockRouter.push).not.toHaveBeenCalledWith({
        pathname: COACH_EDIT_ROUTE,
        params: { agentId: 'test-coach-id' },
      });
    });

    it('Edit agent forks once however often it is tapped', async () => {
      let resolveFork: (copy: { id: string }) => void = () => {};
      mockFork.mockReturnValue(
        new Promise((resolve) => {
          resolveFork = resolve;
        }),
      );
      const { getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });
      fireEvent.press(getByTestId('edit-coach-button'));
      fireEvent.press(getByTestId('edit-coach-button'));
      await act(async () => {
        resolveFork({ id: 'forked-copy-1' });
      });

      expect(mockFork).toHaveBeenCalledTimes(1);
      expect(mockRouter.push).toHaveBeenCalledTimes(1);
    });

    it('a fork that fails says so and opens no editor', async () => {
      mockFork.mockRejectedValue(new Error('service unavailable'));
      const { getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });
      fireEvent.press(getByTestId('edit-coach-button'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Error', 'Failed to update agent');
      });
      expect(mockRouter.push).not.toHaveBeenCalled();
    });

    it('after the fork, uninstall removes the copy rather than the listing', async () => {
      mockUninstall.mockResolvedValue({ message: 'Uninstalled', source_agent_id: 'test-coach-id' });
      const { getByTestId, getByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });
      fireEvent.press(getByTestId('edit-coach-button'));
      await waitFor(() => {
        expect(mockRouter.push).toHaveBeenCalledTimes(1);
      });

      confirmEveryUninstall();
      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(mockUninstall).toHaveBeenCalledWith('forked-copy-1');
      });
    });

    it('should show confirmation dialog when Installed button is pressed', async () => {
      const { getByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });

      // Pressing "Installed" button triggers uninstall confirmation
      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Uninstall Agent?',
          expect.stringContaining('Marathon Training Agent'),
          expect.any(Array)
        );
      });
    });

    it('uninstalls the membership by the listing id', async () => {
      mockUninstall.mockResolvedValue({ message: 'Uninstalled', source_agent_id: 'test-coach-id' });
      confirmEveryUninstall();

      const { getByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });

      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(mockUninstall).toHaveBeenCalledWith('test-coach-id');
      });
      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
    });

    it('a listing without a handle cannot be matched to an installation', async () => {
      mockGet.mockResolvedValue(createMockStoreCoachDetail({ handle: undefined }));

      const { getByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
    });
  });

  describe('installed copy', () => {
    beforeEach(() => {
      // An edited agent, or one installed from another workspace, is listed as
      // a copy: its own id, the listing's handle.
      mockGetInstallations.mockResolvedValue({ agents: [installedCopy] });
    });

    it('recognises the installed copy by the handle it inherited from the listing', async () => {
      const { getByText, getByTestId, queryByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });
      expect(queryByText('Install Agent')).toBeNull();
      expect(getByTestId('edit-coach-button')).toBeTruthy();
    });

    it('Edit agent opens the edit sheet on the copy without forking again', async () => {
      const { getByTestId } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });

      fireEvent.press(getByTestId('edit-coach-button'));

      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: COACH_EDIT_ROUTE,
        params: { agentId: 'installed-copy-1' },
      });
      expect(mockFork).not.toHaveBeenCalled();
    });

    it('uninstalls the copy id, never the store listing id', async () => {
      mockUninstall.mockResolvedValue({ message: 'Uninstalled', source_agent_id: 'test-coach-id' });
      confirmEveryUninstall();

      const { getByText } = render(<StoreCoachDetailScreen />);

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });

      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(mockUninstall).toHaveBeenCalledWith('installed-copy-1');
      });
      await waitFor(() => {
        expect(getByText('Install Agent')).toBeTruthy();
      });
    });
  });

  describe('navigation', () => {
    it('draws no back button of its own — the native header carries the chevron', async () => {
      const { getAllByText, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Agent').length).toBeGreaterThan(0);
      });

      expect(queryByTestId('back-button')).toBeNull();
    });
  });

  describe('edge cases', () => {
    it('should handle agent with no tags', async () => {
      mockGet.mockResolvedValue(
        createMockStoreCoachDetail({ tags: [] })
      );

      const { queryByText, getAllByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Agent').length).toBeGreaterThan(0);
        // Tags section should not show empty tags
        expect(queryByText('marathon')).toBeNull();
      });
    });

    it('should handle agent with no sample prompts', async () => {
      mockGet.mockResolvedValue(
        createMockStoreCoachDetail({ sample_prompts: [] })
      );

      const { getAllByText, queryByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Agent').length).toBeGreaterThan(0);
        expect(queryByText('What should my weekly mileage be?')).toBeNull();
      });
    });

    it('should handle singular install count', async () => {
      mockGet.mockResolvedValue(
        createMockStoreCoachDetail({ install_count: 1 })
      );

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('1 user')).toBeTruthy();
      });
    });

    it('should handle API error gracefully', async () => {
      mockGet.mockRejectedValue(new Error('Network error'));

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Error',
          'Failed to load agent details'
        );
      });
    });
  });
});
