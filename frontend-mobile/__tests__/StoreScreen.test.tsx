// ABOUTME: Unit tests for StoreScreen component
// ABOUTME: Tests coach store browsing, filtering, search, and navigation

import React from 'react';
import { render as rtlRender, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// Per-file expo-router mock override with spyable router methods
const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({}),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(() => { return cb(); }, [cb]); },
}));

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
  }),
}));

// Mock API service
const mockBrowseStoreCoaches = jest.fn();
const mockSearchStoreCoaches = jest.fn();
const mockGetStoreCategories = jest.fn();
const mockListInstalledCoaches = jest.fn();

jest.mock('../src/services/api', () => ({
  storeApi: {
    browse: (...args: unknown[]) => mockBrowseStoreCoaches(...args),
    search: (...args: unknown[]) => mockSearchStoreCoaches(...args),
    getCategories: (...args: unknown[]) => mockGetStoreCategories(...args),
  },
  coachesApi: {
    list: (...args: unknown[]) => mockListInstalledCoaches(...args),
  },
}));

import { StoreScreen } from '../src/screens/store/StoreScreen';
import type { StoreCoach, CoachCategory, Coach } from '../src/types';

/** Discover pins the athlete's installed coaches through react-query, so every render needs a client. */
function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const createInstalledCoach = (overrides: Partial<Coach> = {}): Coach => ({
  id: 'coach-1',
  title: 'Coach Tempo',
  description: null,
  system_prompt: '',
  category: 'training',
  tags: [],
  token_count: 0,
  is_favorite: false,
  is_system: false,
  is_hidden: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2026-08-01T00:00:00Z',
  updated_at: '2026-08-01T00:00:00Z',
  ...overrides,
});

const createMockStoreCoach = (overrides: Partial<StoreCoach> = {}): StoreCoach => ({
  id: 'store-coach-1',
  title: 'Test Store Coach',
  description: 'A published coach for the store',
  category: 'training' as CoachCategory,
  tags: ['running', 'marathon'],
  sample_prompts: ['How do I improve my pace?'],
  token_count: 800,
  install_count: 25,
  icon_url: null,
  published_at: '2024-01-15T00:00:00Z',
  author_id: null,
  ...overrides,
});

describe('StoreScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRouter.push.mockClear();
    mockRouter.replace.mockClear();
    mockRouter.back.mockClear();
    mockRouter.navigate.mockClear();
    mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });
    mockListInstalledCoaches.mockResolvedValue({ coaches: [] });
    mockGetStoreCategories.mockResolvedValue({
      categories: [
        { category: 'training', count: 5 },
        { category: 'nutrition', count: 3 },
        { category: 'recovery', count: 2 },
      ],
    });
  });

  describe('rendering', () => {
    it('should render header with Discover title', async () => {
      const { getByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('Discover')).toBeTruthy();
      });
    });

    it('should render category filters', async () => {
      const { getByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('All')).toBeTruthy();
        expect(getByText('Training')).toBeTruthy();
        expect(getByText('Nutrition')).toBeTruthy();
        expect(getByText('Recovery')).toBeTruthy();
      });
    });

    it('should render sort options', async () => {
      const { getByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('Popular')).toBeTruthy();
        expect(getByText('Newest')).toBeTruthy();
        expect(getByText('A-Z')).toBeTruthy();
      });
    });

    it('should render search input', async () => {
      const { getByPlaceholderText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByPlaceholderText('Search coaches...')).toBeTruthy();
      });
    });

    it('should render empty state when no coaches', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });
      const { getByText } = render(
        <StoreScreen />
      );
      await waitFor(() => {
        expect(getByText('No coaches available')).toBeTruthy();
      });
    });
  });

  describe('coach list', () => {
    it('should render coach cards', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Marathon Training Coach' }),
        createMockStoreCoach({ id: '2', title: 'Nutrition Guide', category: 'nutrition' as CoachCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 2 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Marathon Training Coach')).toBeTruthy();
        expect(getByText('Nutrition Guide')).toBeTruthy();
      });
    });

    it('should show install count on coach cards', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Popular Coach', install_count: 150 }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('150 installs')).toBeTruthy();
      });
    });

    it('should show category badge on coach cards', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Training Coach', category: 'training' as CoachCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('training')).toBeTruthy();
      });
    });

    it('should show tags on coach cards', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Tagged Coach', tags: ['beginner', 'cardio'] }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('beginner')).toBeTruthy();
        expect(getByText('cardio')).toBeTruthy();
      });
    });
  });

  describe('filtering', () => {
    it('should filter by category when chip is pressed', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Training Coach', category: 'training' as CoachCategory }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Training')).toBeTruthy();
      });

      // Clear previous calls and press Training filter
      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByText('Training'));

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: 'training' })
        );
      });
    });

    it('should clear category filter when All is pressed', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });

      const { getByText } = render(
        <StoreScreen />
      );

      // Wait for initial load
      await waitFor(() => {
        expect(getByText('All')).toBeTruthy();
      });

      // First select a category and wait for the load triggered by that
      fireEvent.press(getByText('Training'));
      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: 'training' })
        );
      });

      // Then clear with All
      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByText('All'));

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ category: undefined })
        );
      });
    });
  });

  describe('sorting', () => {
    it('should sort by popular by default', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });

      render(<StoreScreen />);

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ sort_by: 'popular' })
        );
      });
    });

    it('should change sort when option is pressed', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Newest')).toBeTruthy();
      });

      mockBrowseStoreCoaches.mockClear();
      fireEvent.press(getByText('Newest'));

      await waitFor(() => {
        expect(mockBrowseStoreCoaches).toHaveBeenCalledWith(
          expect.objectContaining({ sort_by: 'newest' })
        );
      });
    });
  });

  describe('search', () => {
    it('should search coaches when text is entered', async () => {
      mockBrowseStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });
      mockSearchStoreCoaches.mockResolvedValue({ coaches: [], total: 0 });

      const { getByPlaceholderText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByPlaceholderText('Search coaches...')).toBeTruthy();
      });

      const searchInput = getByPlaceholderText('Search coaches...');
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
    it('should navigate to StoreCoachDetail when coach is pressed', async () => {
      const coaches = [
        createMockStoreCoach({ id: 'coach-123', title: 'Clickable Coach' }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

      const { getByText } = render(
        <StoreScreen />
      );

      await waitFor(() => {
        expect(getByText('Clickable Coach')).toBeTruthy();
      });

      fireEvent.press(getByText('Clickable Coach'));

      expect(mockRouter.push).toHaveBeenCalledWith({ pathname: '/(app)/(tabs)/(discover)/[coachId]', params: { coachId: 'coach-123' } });
    });

  });

  describe('loading states', () => {
    it('should show loading indicator while fetching coaches', async () => {
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
      resolvePromise!({ coaches: [], total: 0 });

      await waitFor(() => {
        // Loading should be done
      });
    });
  });

  describe('pull to refresh', () => {
    it('should refresh coaches on pull down', async () => {
      const coaches = [
        createMockStoreCoach({ id: '1', title: 'Initial Coach' }),
      ];
      mockBrowseStoreCoaches.mockResolvedValue({ coaches, total: 1 });

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

  describe('installed coaches pinned above the catalogue', () => {
    // Turns red if the strip stops reading the athlete's own coach list — it
    // must be its own query, never a re-rank of the catalogue page, because
    // handle_browse grades each cursor page and an installed coach on page
    // three could never surface on page one.
    it('lists the installed coaches with their @handle, from the coach list not the catalogue', async () => {
      mockListInstalledCoaches.mockResolvedValue({
        coaches: [
          createInstalledCoach({ id: 'coach-1', title: 'Coach Tempo', handle: 'coach-tempo' }),
          createInstalledCoach({ id: 'coach-2', title: 'My private coach' }),
        ],
      });

      const { findByTestId, getByTestId, getByText } = render(<StoreScreen />);

      expect(await findByTestId('installed-coach-coach-1')).toBeTruthy();
      expect(getByTestId('installed-coach-handle-coach-1').props.children).toEqual(['@', 'coach-tempo']);
      expect(getByText('Coach Tempo')).toBeTruthy();
      // A personal coach with no catalogue handle is still installed; it shows its category instead.
      expect(getByTestId('installed-coach-coach-2')).toBeTruthy();
      expect(getByText('Installed · 2')).toBeTruthy();
      // The catalogue returned nothing, so these rows can only have come from
      // the coach list — the strip's own query, refetched on focus.
      expect(mockListInstalledCoaches).toHaveBeenCalled();
      expect(mockBrowseStoreCoaches).toHaveBeenCalled();
      // The strip sits above the category filters, which stay as they were.
      expect(getByText('All')).toBeTruthy();
      expect(getByText('Training')).toBeTruthy();
    });

    it('opens the installed coach detail under Discover', async () => {
      mockListInstalledCoaches.mockResolvedValue({
        coaches: [createInstalledCoach({ id: 'coach-1', title: 'Coach Tempo', handle: 'coach-tempo' })],
      });
      const { findByTestId } = render(<StoreScreen />);

      fireEvent.press(await findByTestId('installed-coach-coach-1'));

      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: '/(app)/(tabs)/(discover)/library/[coachId]',
        params: { coachId: 'coach-1' },
      });
    });

    it('reaches the coach library and the coach editor from the strip', async () => {
      const { findByTestId, getByTestId } = render(<StoreScreen />);

      fireEvent.press(await findByTestId('manage-coaches-button'));
      expect(mockRouter.push).toHaveBeenCalledWith('/(app)/(tabs)/(discover)/library');

      fireEvent.press(getByTestId('discover-create-coach-button'));
      expect(mockRouter.push).toHaveBeenCalledWith('/(app)/(tabs)/(discover)/library/editor');
    });

    it('says so when nothing is installed yet', async () => {
      const { findByTestId } = render(<StoreScreen />);
      expect(await findByTestId('installed-coaches-empty')).toBeTruthy();
    });
  });
});
