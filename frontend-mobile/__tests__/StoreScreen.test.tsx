// ABOUTME: Unit tests for StoreScreen component
// ABOUTME: Tests agent store browsing, filtering, search, navigation, and that Discover keeps no agent list of its own

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { ActionSheetIOS } from 'react-native';

// Per-file expo-router mock override with spyable router methods
const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
  }),
);

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
  }),
}));

// Mock API service
const mockBrowseStoreCoaches = jest.fn();
const mockSearchStoreCoaches = jest.fn();
const mockListCoaches = jest.fn();

jest.mock('../src/services/api', () => ({
  storeApi: {
    browse: (...args: unknown[]) => mockBrowseStoreCoaches(...args),
    search: (...args: unknown[]) => mockSearchStoreCoaches(...args),
  },
  coachesApi: {
    list: (...args: unknown[]) => mockListCoaches(...args),
  },
}));

import { StoreScreen } from '../src/screens/store/StoreScreen';
import type { StoreAgent, AgentCategory } from '../src/types';

const createMockStoreCoach = (overrides: Partial<StoreAgent> = {}): StoreAgent => ({
  id: 'store-coach-1',
  title: 'Test Store Coach',
  description: 'A published coach for the store',
  category: 'training' as AgentCategory,
  tags: ['running', 'marathon'],
  sample_prompts: ['How do I improve my pace?'],
  token_count: 800,
  install_count: 25,
  icon_url: null,
  published_at: '2024-01-15T00:00:00Z',
  author_id: null,
  ...overrides,
});

/** The rows the platform sheet last offered, and a way to pick one by label. */
function presentedSortMenu() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [options, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [
    { options: string[]; cancelButtonIndex?: number },
    (index: number) => void,
  ];
  return {
    labels: options.options,
    cancelButtonIndex: options.cancelButtonIndex,
    pick: (label: string) => callback(options.options.indexOf(label)),
  };
}

describe('StoreScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRouter.push.mockClear();
    mockRouter.replace.mockClear();
    mockRouter.back.mockClear();
    mockRouter.navigate.mockClear();
    mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });
    mockListCoaches.mockResolvedValue({ agents: [] });
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
  });

  describe('rendering', () => {
    it('draws no header of its own — the large title is the native one', async () => {
      const { getByTestId, queryByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByTestId('store-screen')).toBeTruthy();
      });
      expect(queryByText('Discover')).toBeNull();
    });

    it('should render category filters', async () => {
      const { getByText, getAllByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('All')).toBeTruthy();
        expect(getAllByText('Training').length).toBeGreaterThan(0);
        expect(getByText('Nutrition')).toBeTruthy();
        expect(getByText('Recovery')).toBeTruthy();
      });
    });

    it('offers the sort choices behind the header button, not an inline chip row', async () => {
      const { getByTestId, queryByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByTestId('discover-sort-button')).toBeTruthy();
      });
      // The always-visible "Sort by:" row and its chips are gone.
      expect(queryByText('Sort by:')).toBeNull();

      fireEvent.press(getByTestId('discover-sort-button'));
      expect(presentedSortMenu().labels).toEqual(['Popular', 'Newest', 'A-Z', 'Cancel']);
    });

    it('puts the search field in the native header', async () => {
      const { getByTestId } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByTestId('header-search-input').props.placeholder).toBe('Search agents...');
      });
    });

    it('should render empty state when no agents', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });
      const { getByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('No agents available')).toBeTruthy();
      });
    });
  });

  describe('agent list', () => {
    it('should render agent cards', async () => {
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Marathon Training Agent' }),
        createMockStoreCoach({ id: '2', title: 'Nutrition Guide', category: 'nutrition' as AgentCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 2 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Marathon Training Agent')).toBeTruthy();
        expect(getByText('Nutrition Guide')).toBeTruthy();
      });
    });

    it('should show install count on agent cards', async () => {
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Popular Coach', install_count: 150 }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('150 users')).toBeTruthy();
      });
    });

    it('should show the category dot on agent rows', async () => {
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Training Coach', category: 'training' as AgentCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByTestId('category-badge').props.accessibilityLabel).toBe('Training');
      });
    });

    it('should show the install action on agent rows', async () => {
      // Tags moved to the detail page (D7): the row itself carries only the
      // glyph, name, category dot, description, install count and this action.
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Tagged Coach', tags: ['beginner', 'cardio'] }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByTestId('install-action-0')).toHaveTextContent('Install');
      });
    });
  });

  describe('filtering', () => {
    it('should filter by category when a tab is pressed', async () => {
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Training Coach', category: 'training' as AgentCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByTestId('discover-category-tabs-training')).toBeTruthy();
      });

      // Clear previous calls and press the Training tab
      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByTestId('discover-category-tabs-training'));

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: 'training' })
        );
      });
    });

    it('should clear category filter when All is pressed', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      // Wait for initial load
      await waitFor(() => {
        expect(getByTestId('discover-category-tabs-all')).toBeTruthy();
      });

      // First select a category and wait for the load triggered by that
      fireEvent.press(getByTestId('discover-category-tabs-training'));
      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: 'training' })
        );
      });

      // Then clear with All
      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByTestId('discover-category-tabs-all'));

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: undefined })
        );
      });
    });
  });

  describe('sorting', () => {
    it('should sort by popular by default', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });

      render(<StoreScreen />);

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ sort_by: 'popular' })
        );
      });
    });

    it('should change sort when a menu option is picked', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByTestId('discover-sort-button')).toBeTruthy();
      });

      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByTestId('discover-sort-button'));
      presentedSortMenu().pick('Newest');

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ sort_by: 'newest' })
        );
      });
    });
  });

  describe('search', () => {
    it('should search agents when text is entered', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ agents: [], total: 0 });
      mockSearchStoreCoaches.mockResolvedValue({ agents: [], total: 0 });

      const { getByPlaceholderText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByPlaceholderText('Search agents...')).toBeTruthy();
      });

      const searchInput = getByPlaceholderText('Search agents...');
      fireEvent.changeText(searchInput, 'marathon');

      // Search is debounced, so wait for it
      await waitFor(
        () => {
          expect(mockSearchStoreCoaches).toHaveBeenCalledWith('marathon', expect.any(Number));
        },
        { timeout: 1000 }
      );
    });
  });

  describe('navigation', () => {
    it('should navigate to StoreAgentDetail when coach is pressed', async () => {
      const agents = [
        createMockStoreCoach({ id: 'coach-123', title: 'Clickable Coach' }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Clickable Coach')).toBeTruthy();
      });

      fireEvent.press(getByText('Clickable Coach'));

      expect(mockRouter.push).toHaveBeenCalledWith({ pathname: '/(app)/(tabs)/(discover)/[agentId]', params: { agentId: 'coach-123' } });
    });

  });

  describe('loading states', () => {
    it('should show loading indicator while fetching agents', async () => {
      // Create a promise that doesn't resolve immediately
      let resolvePromise: (value: unknown) => void;
      const pendingPromise = new Promise((resolve) => {
        resolvePromise = resolve;
      });
      mockBrowseStoreCoaches.mockReturnValue(pendingPromise);

      const { getByTestId } = render(
        <StoreScreen />
      );

      // Should show loading state
      expect(getByTestId('loading-indicator')).toBeTruthy();

      // Resolve the promise
      resolvePromise!({ agents: [], total: 0 });

      await waitFor(() => {
        // Loading should be done
      });
    });
  });

  describe('pull to refresh', () => {
    it('should refresh agents on pull down', async () => {
      const agents = [
        createMockStoreCoach({ id: '1', title: 'Initial Coach' }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { getByTestId } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByTestId('coach-list')).toBeTruthy();
      });

      // Clear mock to track refresh call
      mockBrowseStoreCoaches.mockClear();

      // Trigger refresh
      const flatList = getByTestId('coach-list');
      const { refreshControl } = flatList.props;
      if (refreshControl?.props?.onRefresh) {
        refreshControl.props.onRefresh();
      }

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalled();
      });
    });
  });

  describe('no agent list of its own', () => {
    // Discover is the catalogue. The athlete's agents are reached from chat
    // (`/agent list`, `@handle`), and editing lives on the store detail of an
    // installed listing — no strip, no library, no create button here.
    it('renders the catalogue with no pinned agents, library link or create button', async () => {
      const agents = [createMockStoreCoach({ id: '1', title: 'Marathon Training Agent' })];
      mockBrowseStoreCoaches.mockResolvedValue({ agents, total: 1 });

      const { findByText, queryByTestId } = render(<StoreScreen />);

      expect(await findByText('Marathon Training Agent')).toBeTruthy();
      expect(queryByTestId('installed-coaches-strip')).toBeNull();
      expect(queryByTestId('manage-coaches-button')).toBeNull();
      expect(queryByTestId('discover-create-coach-button')).toBeNull();
      // The catalogue is the only query Discover makes.
      expect(mockListCoaches).not.toHaveBeenCalled();
    });
  });
});
