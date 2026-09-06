// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Unit tests for StoreScreen component
// ABOUTME: Tests browsing, filtering, searching, install → hint → Open chat, and the edit sheet on an installed agent

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import StoreScreen from '../StoreScreen';

// Mock the store API - define mock data inline to avoid hoisting issues
vi.mock('../../services/api', () => ({
  storeApi: {
    browse: vi.fn().mockResolvedValue({
      coaches: [
        {
          id: 'coach-1',
          title: 'Marathon Training Coach',
          description: 'A comprehensive marathon training program',
          category: 'training',
          tags: ['marathon', 'running', 'endurance'],
          sample_prompts: ['What should my weekly mileage be?'],
          token_count: 1200,
          install_count: 75,
          icon_url: null,
          published_at: '2024-01-15T00:00:00Z',
          author_id: 'author-123',
        },
        {
          id: 'coach-2',
          title: 'Nutrition Expert',
          description: 'Personalized nutrition advice',
          category: 'nutrition',
          tags: ['diet', 'macros', 'meal-planning'],
          sample_prompts: ['How many calories should I eat?'],
          token_count: 800,
          install_count: 120,
          icon_url: null,
          published_at: '2024-01-20T00:00:00Z',
          author_id: 'author-456',
        },
        {
          id: 'coach-3',
          title: 'Recovery Coach',
          description: 'Optimize your recovery',
          category: 'recovery',
          tags: ['sleep', 'stretching', 'rest'],
          sample_prompts: ['How long should I sleep?'],
          token_count: 600,
          install_count: 45,
          icon_url: null,
          published_at: '2024-01-25T00:00:00Z',
          author_id: 'author-789',
        },
      ],
      total: 3,
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    }),
    search: vi.fn().mockResolvedValue({
      coaches: [
        {
          id: 'coach-1',
          title: 'Marathon Training Coach',
          description: 'A comprehensive marathon training program',
          category: 'training',
          tags: ['marathon', 'running', 'endurance'],
          sample_prompts: ['What should my weekly mileage be?'],
          token_count: 1200,
          install_count: 75,
          icon_url: null,
          published_at: '2024-01-15T00:00:00Z',
          author_id: 'author-123',
        },
      ],
      query: 'marathon',
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    }),
    get: vi.fn().mockResolvedValue({
      id: 'coach-1',
      title: 'Marathon Training Coach',
      description: 'A comprehensive marathon training program',
      category: 'training',
      tags: ['marathon', 'running', 'endurance'],
      sample_prompts: ['What should my weekly mileage be?'],
      system_prompt: 'You are an expert marathon coach...',
      token_count: 1200,
      install_count: 75,
      icon_url: null,
      published_at: '2024-01-15T00:00:00Z',
      created_at: '2024-01-10T00:00:00Z',
      author_id: 'author-123',
      publish_status: 'published',
    }),
    install: vi.fn().mockResolvedValue({
      message: 'Coach installed successfully',
      coach: {
        id: 'installed-copy-1',
        title: 'Marathon Training Coach',
        description: 'A comprehensive marathon training program',
        category: 'training',
        tags: ['marathon', 'running', 'endurance'],
        sample_prompts: ['What should my weekly mileage be?'],
        token_count: 1200,
        install_count: 0,
        icon_url: null,
        published_at: null,
        author_id: null,
        handle: 'marathon-training-coach',
      },
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    }),
    uninstall: vi.fn().mockResolvedValue({
      message: 'Coach uninstalled successfully',
      source_coach_id: 'coach-1',
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    }),
  },
  coachesApi: {
    list: vi.fn().mockResolvedValue({
      coaches: [],
      total: 0,
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    }),
    get: vi.fn(),
    update: vi.fn(),
    delete: vi.fn(),
  },
  chatApi: {
    createConversation: vi.fn().mockResolvedValue({ id: 'conv-new', title: 'Chat' }),
  },
}));

import { storeApi, coachesApi, chatApi } from '../../services/api';

// A coach installed from the store is a personal copy: fresh id, `forked_from`
// pointing at the store listing. Mirrors GET /api/coaches on a live server.
const installedCopyOfCoach1 = {
  id: 'installed-copy-1',
  title: 'Marathon Training Coach',
  description: 'A comprehensive marathon training program',
  system_prompt: 'You are an expert marathon coach...',
  category: 'training',
  tags: ['marathon', 'running', 'endurance'],
  token_count: 1200,
  is_favorite: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2024-02-01T00:00:00Z',
  updated_at: '2024-02-01T00:00:00Z',
  is_system: false,
  visibility: 'private',
  is_assigned: true,
  forked_from: 'coach-1',
  handle: 'marathon-training-coach',
};

const mockOnNavigate = vi.fn();

function renderStoreScreen(props: { ownCoachId?: string | null } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <StoreScreen onNavigate={mockOnNavigate} ownCoachId={props.ownCoachId} />
    </QueryClientProvider>
  );
}

/** Open the Marathon listing's detail view from the store grid. */
async function openMarathonListing(user: ReturnType<typeof userEvent.setup>) {
  const grid = await screen.findByTestId('store-coach-grid');
  await user.click(within(grid).getByText('Marathon Training Coach'));
}

describe('StoreScreen', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // clearAllMocks keeps implementations, so re-establish the default of "no
    // agent installed" that the installed-state tests override.
    vi.mocked(coachesApi.list).mockResolvedValue({
      coaches: [],
      total: 0,
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    });
    vi.mocked(coachesApi.get).mockResolvedValue(installedCopyOfCoach1);
  });

  describe('rendering', () => {
    it('should render the header with subtitle', async () => {
      renderStoreScreen();
      expect(screen.getByText('Find AI agents')).toBeInTheDocument();
    });

    it('should render the search input', async () => {
      renderStoreScreen();
      expect(screen.getByPlaceholderText('Search agents...')).toBeInTheDocument();
    });

    it('should render category filter buttons', async () => {
      renderStoreScreen();
      expect(screen.getByText('All')).toBeInTheDocument();
      expect(screen.getByText('Training')).toBeInTheDocument();
      expect(screen.getByText('Nutrition')).toBeInTheDocument();
      expect(screen.getByText('Recovery')).toBeInTheDocument();
    });

    it('should render sort options', async () => {
      renderStoreScreen();
      expect(screen.getByText('Popular')).toBeInTheDocument();
      expect(screen.getByText('Newest')).toBeInTheDocument();
      expect(screen.getByText('A-Z')).toBeInTheDocument();
    });

    it('should render agent cards after loading', async () => {
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
        expect(screen.getByText('Nutrition Expert')).toBeInTheDocument();
        expect(screen.getByText('Recovery Coach')).toBeInTheDocument();
      });
    });

    it('should display agent user counts', async () => {
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('75 users')).toBeInTheDocument();
        expect(screen.getByText('120 users')).toBeInTheDocument();
        expect(screen.getByText('45 users')).toBeInTheDocument();
      });
    });

    it('should display agent categories as badges', async () => {
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getAllByText('Training').length).toBeGreaterThan(0);
        expect(screen.getAllByText('Nutrition').length).toBeGreaterThan(0);
        expect(screen.getAllByText('Recovery').length).toBeGreaterThan(0);
      });
    });

    it('should display agent tags', async () => {
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('marathon')).toBeInTheDocument();
        expect(screen.getByText('running')).toBeInTheDocument();
        expect(screen.getByText('diet')).toBeInTheDocument();
      });
    });
  });

  describe('filtering', () => {
    it('should call browse with popular sort by default', async () => {
      renderStoreScreen();

      await waitFor(() => {
        expect(storeApi.browse).toHaveBeenCalledWith(
          expect.objectContaining({
            sort_by: 'popular',
          })
        );
      });
    });

    it('should filter by category when clicked', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });

      await user.click(screen.getByRole('button', { name: 'Training' }));

      await waitFor(() => {
        expect(storeApi.browse).toHaveBeenCalledWith(
          expect.objectContaining({
            category: 'training',
          })
        );
      });
    });

    it('should change sort when option is clicked', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });

      await user.click(screen.getByRole('button', { name: 'Newest' }));

      await waitFor(() => {
        expect(storeApi.browse).toHaveBeenCalledWith(
          expect.objectContaining({
            sort_by: 'newest',
          })
        );
      });
    });
  });

  describe('search', () => {
    it('should search agents when text is entered', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      const searchInput = screen.getByPlaceholderText('Search agents...');
      await user.type(searchInput, 'marathon');

      await waitFor(
        () => {
          expect(storeApi.search).toHaveBeenCalledWith('marathon', 50);
        },
        { timeout: 1000 }
      );
    });

    it('should clear search when X button is clicked', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      const searchInput = screen.getByPlaceholderText('Search agents...');
      await user.type(searchInput, 'marathon');

      await waitFor(() => {
        expect(searchInput).toHaveValue('marathon');
      });

      // Find and click clear button (it appears after typing)
      const buttons = screen.getAllByRole('button');
      const clearButton = buttons.find(
        (btn) => btn.querySelector('svg path[d*="6 18L18 6"]') !== null
      );
      if (clearButton) {
        await user.click(clearButton);
        await waitFor(() => {
          expect(searchInput).toHaveValue('');
        });
      }
    });
  });

  describe('navigation', () => {
    it('should open detail view when an agent card is clicked', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });

      await user.click(screen.getByText('Marathon Training Coach'));

      // Detail view should show the Add Agent button and System Prompt section
      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Add Agent' })).toBeInTheDocument();
      });
    });
  });

  describe('empty state', () => {
    it('should show empty state when no agents', async () => {
      vi.mocked(storeApi.browse).mockResolvedValueOnce({
        coaches: [],
        total: 0,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });

      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Store is empty')).toBeInTheDocument();
      });
    });

    it('should show search empty state when no search results', async () => {
      vi.mocked(storeApi.search).mockResolvedValueOnce({
        coaches: [],
        query: 'nonexistent',
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });

      const user = userEvent.setup();
      renderStoreScreen();

      const searchInput = screen.getByPlaceholderText('Search agents...');
      await user.type(searchInput, 'nonexistent');

      await waitFor(
        () => {
          expect(screen.getByText('No agents found')).toBeInTheDocument();
        },
        { timeout: 1000 }
      );
    });
  });

  describe('failed store fetch', () => {
    it('should show a retry error state instead of the empty-store copy', async () => {
      vi.mocked(storeApi.browse).mockRejectedValueOnce(new Error('Network Error'));

      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText("Couldn't load the store")).toBeInTheDocument();
      });

      // The confident-but-wrong empty state must not be shown for a failure.
      expect(screen.queryByText('Store is empty')).not.toBeInTheDocument();
      expect(screen.queryByText('No published agents available yet')).not.toBeInTheDocument();
      expect(screen.getByText('Network Error')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Try Again' })).toBeInTheDocument();
    });

    it('should reload the agent grid when Try Again is clicked', async () => {
      vi.mocked(storeApi.browse).mockRejectedValueOnce(new Error('Network Error'));

      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Try Again' })).toBeInTheDocument();
      });

      await user.click(screen.getByRole('button', { name: 'Try Again' }));

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
        expect(screen.getByText('Nutrition Expert')).toBeInTheDocument();
      });
      expect(screen.queryByText("Couldn't load the store")).not.toBeInTheDocument();
    });
  });

  describe('no agent list of its own', () => {
    it('renders the catalogue straight under the search box, with no pinned agents', async () => {
      vi.mocked(coachesApi.list).mockResolvedValue({
        coaches: [installedCopyOfCoach1],
        total: 1,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });

      renderStoreScreen();

      expect(await screen.findByText('Nutrition Expert')).toBeInTheDocument();
      expect(screen.queryByRole('region', { name: /Your agents/ })).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Create Agent' })).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Import Agent' })).not.toBeInTheDocument();
    });
  });

  describe('post-install hint', () => {
    it('replaces the success banner with the hint that teaches /agent add @handle', async () => {
      const user = userEvent.setup();
      renderStoreScreen();
      await openMarathonListing(user);
      await user.click(await screen.findByRole('button', { name: 'Add Agent' }));

      const hint = await screen.findByTestId('post-install-hint');
      expect(hint).toHaveTextContent(
        'Use it in any chat: /agent add @marathon-training-coach — or mention @marathon-training-coach for one turn',
      );
      expect(storeApi.install).toHaveBeenCalledWith('coach-1');
      expect(screen.queryByText(/has been added to your agents/)).not.toBeInTheDocument();
    });

    it('Open chat starts a conversation and routes to it', async () => {
      const user = userEvent.setup();
      renderStoreScreen();
      await openMarathonListing(user);
      await user.click(await screen.findByRole('button', { name: 'Add Agent' }));
      await screen.findByTestId('post-install-hint');

      await user.click(screen.getByRole('button', { name: 'Open chat' }));

      await waitFor(() => {
        expect(chatApi.createConversation).toHaveBeenCalledWith(
          expect.objectContaining({ title: expect.stringMatching(/^Chat /) }),
        );
      });
      await waitFor(() => {
        expect(mockOnNavigate).toHaveBeenCalledWith('chat/conv-new');
      });
      expect(screen.queryByTestId('post-install-hint')).not.toBeInTheDocument();
    });

    it('Dismiss hides the hint and leaves the agent installed', async () => {
      const user = userEvent.setup();
      renderStoreScreen();
      await openMarathonListing(user);
      await user.click(await screen.findByRole('button', { name: 'Add Agent' }));
      await screen.findByTestId('post-install-hint');

      await user.click(screen.getByRole('button', { name: 'Dismiss' }));

      expect(screen.queryByTestId('post-install-hint')).not.toBeInTheDocument();
      expect(chatApi.createConversation).not.toHaveBeenCalled();
      expect(storeApi.uninstall).not.toHaveBeenCalled();
    });
  });

  describe('edit sheet', () => {
    it('opens the edit sheet on the installed copy from the listing detail', async () => {
      vi.mocked(coachesApi.list).mockResolvedValue({
        coaches: [installedCopyOfCoach1],
        total: 1,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });
      const user = userEvent.setup();
      renderStoreScreen();
      await openMarathonListing(user);

      await user.click(await screen.findByRole('button', { name: 'Edit agent' }));

      expect(await screen.findByRole('heading', { name: 'Edit Agent' })).toBeInTheDocument();
      // The copy, never the store listing: the listing is not the athlete's to edit.
      expect(coachesApi.get).toHaveBeenCalledWith('installed-copy-1');
      expect(screen.getByRole('button', { name: 'Delete this agent' })).toBeInTheDocument();
    });

    it('offers no Edit agent on a listing the athlete has not installed', async () => {
      const user = userEvent.setup();
      renderStoreScreen();
      await openMarathonListing(user);

      await screen.findByRole('button', { name: 'Add Agent' });
      expect(screen.queryByRole('button', { name: 'Edit agent' })).not.toBeInTheDocument();
    });

    it('mounts the sheet for the ownCoachId route and hands the route back on close', async () => {
      const user = userEvent.setup();
      renderStoreScreen({ ownCoachId: 'installed-copy-1' });

      expect(await screen.findByRole('heading', { name: 'Edit Agent' })).toBeInTheDocument();
      expect(coachesApi.get).toHaveBeenCalledWith('installed-copy-1');

      await user.click(screen.getByRole('button', { name: 'Cancel' }));

      expect(mockOnNavigate).toHaveBeenCalledWith('discover');
    });
  });

  describe('installed state', () => {
    it('should show Remove for a store agent the user already installed', async () => {
      vi.mocked(coachesApi.list).mockResolvedValue({
        coaches: [installedCopyOfCoach1],
        total: 1,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });

      const user = userEvent.setup();
      renderStoreScreen();

      await openMarathonListing(user);

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument();
      });
      expect(screen.queryByRole('button', { name: 'Add Agent' })).not.toBeInTheDocument();
    });

    it('should uninstall the personal copy id, not the store listing id', async () => {
      vi.mocked(coachesApi.list).mockResolvedValue({
        coaches: [installedCopyOfCoach1],
        total: 1,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      });
      const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

      const user = userEvent.setup();
      renderStoreScreen();

      const grid = await screen.findByTestId('store-coach-grid');
      await user.click(within(grid).getByText('Marathon Training Coach'));

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument();
      });
      await user.click(screen.getByRole('button', { name: 'Remove' }));

      await waitFor(() => {
        expect(storeApi.uninstall).toHaveBeenCalledWith('installed-copy-1');
      });

      confirmSpy.mockRestore();
    });

    it('should show Add Agent when no personal copy points at the listing', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });
      await user.click(screen.getByText('Marathon Training Coach'));

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Add Agent' })).toBeInTheDocument();
      });
      expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument();
    });
  });

  describe('failed install', () => {
    it('should surface the server error instead of failing silently', async () => {
      vi.mocked(storeApi.install).mockRejectedValueOnce(
        new Error('Coach Marathon Training Coach is already installed')
      );

      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });
      await user.click(screen.getByText('Marathon Training Coach'));

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Add Agent' })).toBeInTheDocument();
      });
      await user.click(screen.getByRole('button', { name: 'Add Agent' }));

      await waitFor(() => {
        expect(
          screen.getByText('Coach Marathon Training Coach is already installed')
        ).toBeInTheDocument();
      });
    });
  });

  describe('system prompt preview', () => {
    it('should clamp the preview with a line-clamp class Tailwind emits', async () => {
      const user = userEvent.setup();
      renderStoreScreen();

      await waitFor(() => {
        expect(screen.getByText('Marathon Training Coach')).toBeInTheDocument();
      });
      await user.click(screen.getByText('Marathon Training Coach'));

      const preview = await screen.findByText('You are an expert marathon coach...');
      const clampClass = Array.from(preview.classList).find((c) => c.startsWith('line-clamp-'));

      // Tailwind 3's default lineClamp scale is 1-6 and the project config does
      // not extend it, so anything outside that range is never emitted.
      expect(clampClass).toBeDefined();
      const clampLines = Number(clampClass?.replace('line-clamp-', ''));
      expect(clampLines).toBeGreaterThanOrEqual(1);
      expect(clampLines).toBeLessThanOrEqual(6);
    });
  });
});
