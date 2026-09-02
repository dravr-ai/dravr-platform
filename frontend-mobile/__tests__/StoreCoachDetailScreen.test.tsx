// ABOUTME: Unit tests for StoreCoachDetailScreen component
// ABOUTME: Tests coach detail display, install → hint → Open chat, uninstall by copy id, and Edit coach

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

// Per-file expo-router mock override with spyable router methods
const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({ coachId: 'test-coach-id' }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

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

jest.mock('../src/services/api', () => ({
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
import { tabBarBottomOffset } from '../src/components/ui/ExpandableTabBar';
import { CHAT_THREAD_ROUTE, COACH_EDIT_ROUTE } from '../src/navigation/routes';
import type { StoreCoach, StoreCoachDetail, CoachCategory } from '../src/types';

const COACH_HANDLE = 'marathon-training-coach';

const createMockStoreCoachDetail = (overrides: Partial<StoreCoachDetail> = {}): StoreCoachDetail => ({
  id: 'test-coach-id',
  title: 'Marathon Training Coach',
  description: 'A comprehensive marathon training program',
  category: 'training' as CoachCategory,
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
  handle: COACH_HANDLE,
  ...overrides,
});

// An install mints a personal copy with its own id, carrying the listing's
// handle. Both the install response and the installations list return copies.
const installedCopy: StoreCoach = {
  id: 'installed-copy-1',
  title: 'Marathon Training Coach',
  description: 'A comprehensive marathon training program',
  category: 'training' as CoachCategory,
  tags: ['marathon', 'running', 'endurance'],
  sample_prompts: [],
  token_count: 1200,
  install_count: 0,
  icon_url: null,
  published_at: null,
  author_id: null,
  handle: COACH_HANDLE,
};

describe('StoreCoachDetailScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRouter.push.mockClear();
    mockRouter.replace.mockClear();
    mockRouter.back.mockClear();
    mockRouter.navigate.mockClear();
    mockGet.mockResolvedValue(createMockStoreCoachDetail());
    mockGetInstallations.mockResolvedValue({ coaches: [] });
    mockInstall.mockResolvedValue({ message: 'Coach installed successfully', coach: installedCopy });
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

      expect(getByText('Loading coach details...')).toBeTruthy();

      // Cleanup
      resolvePromise!(createMockStoreCoachDetail());
      await waitFor(() => {});
    });

    it('should render coach title', async () => {
      const { getAllByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Coach').length).toBeGreaterThan(0);
      });
    });

    // The action bar is absolute, so its parent's safe-area padding never
    // reaches it. Pinned to the bare 68pt constant it sat inside the floating
    // tab bar, which occupies 34..90 above the screen edge here (carnet#208).
    it('floats its action bar above the tab bar rather than inside it', async () => {
      const { findByTestId } = render(<StoreCoachDetailScreen />);

      const bar = await findByTestId('coach-detail-action-bar');
      const style = bar.props.style as Record<string, unknown> | Array<Record<string, unknown>>;
      const flat = Array.isArray(style) ? Object.assign({}, ...style) : style;

      expect(flat.bottom).toBe(102);
      expect(flat.bottom).toBe(tabBarBottomOffset(34));
    });

    it('should render coach description', async () => {
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
        expect(getByText('1200')).toBeTruthy();
      });
    });

    it('should show error state when coach not found', async () => {
      mockGet.mockResolvedValue(null);

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Coach not found')).toBeTruthy();
      });
    });
  });

  describe('install functionality', () => {
    it('should show Install button when coach is not installed', async () => {
      mockGetInstallations.mockResolvedValue({ coaches: [] });

      const { getByText, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });
      expect(queryByTestId('edit-coach-button')).toBeNull();
    });

    it('should call install when Install button is pressed', async () => {
      mockGetInstallations.mockResolvedValue({ coaches: [] });

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Coach'));

      await waitFor(() => {
        expect(mockInstall).toHaveBeenCalledWith('test-coach-id');
      });
    });

    it('shows the post-install hint that teaches /coach add @handle, not an alert', async () => {
      const { getByText, findByTestId, getByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Coach'));

      expect(await findByTestId('post-install-hint')).toBeTruthy();
      expect(getByTestId('post-install-title')).toHaveTextContent('“Marathon Training Coach” is in your coaches');
      expect(getByTestId('post-install-body')).toHaveTextContent(
        `Use it in any chat: /coach add @${COACH_HANDLE} — or mention @${COACH_HANDLE} for one turn`,
      );
      expect(Alert.alert).not.toHaveBeenCalled();
      // The install is what the funnel counts.
      expect(mockTrackMobile).toHaveBeenCalledWith({ name: 'feature_engaged', props: { feature: 'coach_installed' } });
      // The copy the install minted is what the screen now addresses.
      expect(getByText('Installed')).toBeTruthy();
      expect(getByTestId('edit-coach-button')).toBeTruthy();
    });

    it('Open chat on the hint opens a fresh thread', async () => {
      const { getByText, findByTestId, getByTestId, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Coach'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-open-chat'));

      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: CHAT_THREAD_ROUTE,
        params: { conversationId: 'new' },
      });
      expect(queryByTestId('post-install-hint')).toBeNull();
    });

    it('Dismiss hides the hint and leaves the coach installed', async () => {
      const { getByText, findByTestId, getByTestId, queryByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });
      fireEvent.press(getByText('Install Coach'));
      await findByTestId('post-install-hint');

      fireEvent.press(getByTestId('post-install-dismiss'));

      expect(queryByTestId('post-install-hint')).toBeNull();
      expect(mockRouter.push).not.toHaveBeenCalled();
      expect(getByText('Installed')).toBeTruthy();
    });

    it('should show error alert on installation failure', async () => {
      mockGetInstallations.mockResolvedValue({ coaches: [] });
      mockInstall.mockRejectedValue(new Error('Installation failed'));

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });

      fireEvent.press(getByText('Install Coach'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Error',
          'Failed to install coach. Please try again.'
        );
      });
    });
  });

  describe('installed copy', () => {
    beforeEach(() => {
      // The installations list holds copies (own id, the listing's handle),
      // never the listing itself.
      mockGetInstallations.mockResolvedValue({ coaches: [installedCopy] });
    });

    it('recognises the installed copy by the handle it inherited from the listing', async () => {
      const { getByText, getByTestId, queryByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });
      expect(queryByText('Install Coach')).toBeNull();
      expect(getByTestId('edit-coach-button')).toBeTruthy();
    });

    it('Edit coach opens the edit sheet on the copy, not the listing', async () => {
      const { getByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByTestId('edit-coach-button')).toBeTruthy();
      });

      fireEvent.press(getByTestId('edit-coach-button'));

      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: COACH_EDIT_ROUTE,
        params: { coachId: 'installed-copy-1' },
      });
    });

    it('should show confirmation dialog when Installed button is pressed', async () => {
      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });

      // Pressing "Installed" button triggers uninstall confirmation
      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Uninstall Coach?',
          expect.stringContaining('Marathon Training Coach'),
          expect.any(Array)
        );
      });
    });

    it('uninstalls the copy id, never the store listing id', async () => {
      mockUninstall.mockResolvedValue({ message: 'Uninstalled', source_coach_id: 'test-coach-id' });

      // Mock Alert to automatically call the destructive action
      (Alert.alert as jest.Mock).mockImplementation(
        (title, message, buttons) => {
          const uninstallButton = buttons?.find(
            (b: { text: string }) => b.text === 'Uninstall'
          );
          if (uninstallButton?.onPress) {
            uninstallButton.onPress();
          }
        }
      );

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Installed')).toBeTruthy();
      });

      // Pressing "Installed" button triggers uninstall confirmation
      fireEvent.press(getByText('Installed'));

      await waitFor(() => {
        expect(mockUninstall).toHaveBeenCalledWith('installed-copy-1');
      });
      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });
    });

    it('a listing without a handle cannot be matched to a copy', async () => {
      mockGet.mockResolvedValue(createMockStoreCoachDetail({ handle: undefined }));

      const { getByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        expect(getByText('Install Coach')).toBeTruthy();
      });
    });
  });

  describe('navigation', () => {
    it('should go back when back button is pressed', async () => {
      const { getAllByText, getByTestId } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Coach').length).toBeGreaterThan(0);
      });

      // Find and press back button via testID
      fireEvent.press(getByTestId('back-button'));

      expect(mockRouter.push).toHaveBeenCalledWith('/(app)/(tabs)/(discover)');
    });
  });

  describe('edge cases', () => {
    it('should handle coach with no tags', async () => {
      mockGet.mockResolvedValue(
        createMockStoreCoachDetail({ tags: [] })
      );

      const { queryByText, getAllByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Coach').length).toBeGreaterThan(0);
        // Tags section should not show empty tags
        expect(queryByText('marathon')).toBeNull();
      });
    });

    it('should handle coach with no sample prompts', async () => {
      mockGet.mockResolvedValue(
        createMockStoreCoachDetail({ sample_prompts: [] })
      );

      const { getAllByText, queryByText } = render(
        <StoreCoachDetailScreen />
      );

      await waitFor(() => {
        // Title appears in both header and content area
        expect(getAllByText('Marathon Training Coach').length).toBeGreaterThan(0);
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
          'Failed to load coach details'
        );
      });
    });
  });
});
