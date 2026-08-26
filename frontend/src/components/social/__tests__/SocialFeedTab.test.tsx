// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for SocialFeedTab component
// ABOUTME: Tests feed display, reactions, share modal, adapt flow, and the adapted history view

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import '@testing-library/jest-dom';
import SocialFeedTab from '../SocialFeedTab';
import { socialApi } from '../../../services/api';

// Mock the API service
vi.mock('../../../services/api', () => ({
  socialApi: {
    getFeed: vi.fn(),
    getInsightSuggestions: vi.fn(),
    addReaction: vi.fn(),
    removeReaction: vi.fn(),
    shareInsight: vi.fn(),
    adaptInsight: vi.fn(),
    getAdaptedInsights: vi.fn(),
  },
}));

/** The adapted history is a React Query surface, so every render needs a client. */
function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <SocialFeedTab />
    </QueryClientProvider>,
  );
}

const mockFeedItems = {
  items: [
    {
      insight: {
        id: 'insight-1',
        user_id: 'user-2',
        visibility: 'friends_only',
        insight_type: 'achievement',
        sport_type: 'Running',
        content: 'Just completed my first marathon training block! Feeling strong.',
        title: 'Marathon Ready',
        training_phase: 'build',
        reaction_count: 5,
        adapt_count: 2,
        created_at: '2024-01-01T00:00:00Z',
        updated_at: '2024-01-01T00:00:00Z',
        expires_at: null,
      },
      author: {
        user_id: 'user-2',
        display_name: 'Jane Doe',
        email: 'jane@example.com',
      },
      reactions: {
        like: 3,
        celebrate: 2,
        inspire: 0,
        support: 0,
        total: 5,
      },
      user_reaction: null,
      user_has_adapted: false,
    },
  ],
  next_cursor: null,
  has_more: false,
  metadata: { timestamp: '2024-01-01T00:00:00Z', api_version: 'v1' },
};

const adaptedPage = {
  insights: [
    {
      id: 'adapted-1',
      user_id: 'user-1',
      source_insight_id: 'insight-1',
      adapted_content: 'Your own build block peaks at 62km — hold that before adding the long run.',
      adaptation_context: 'Based on your last 8 weeks of running volume',
      created_at: '2024-01-02T00:00:00Z',
    },
  ],
  next_cursor: null,
  has_more: false,
  metadata: { timestamp: '2024-01-02T00:00:00Z', api_version: 'v1' },
};

const emptyAdaptedPage = {
  insights: [],
  next_cursor: null,
  has_more: false,
  metadata: { timestamp: '2024-01-02T00:00:00Z', api_version: 'v1' },
};

describe('SocialFeedTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(socialApi.getFeed).mockResolvedValue(mockFeedItems);
    vi.mocked(socialApi.addReaction).mockResolvedValue({
      reaction: {
        id: 'reaction-1',
        insight_id: 'insight-1',
        user_id: 'user-1',
        reaction_type: 'like',
        created_at: '2024-01-01T00:00:00Z',
      },
      updated_counts: {
        like: 4,
        celebrate: 2,
        inspire: 0,
        support: 0,
        total: 6,
      },
      metadata: { timestamp: '2024-01-01T00:00:00Z', api_version: 'v1' },
    });
    vi.mocked(socialApi.removeReaction).mockResolvedValue(undefined);
    vi.mocked(socialApi.getAdaptedInsights).mockResolvedValue(emptyAdaptedPage);
  });

  it('should render the Social Feed tab with subtitle', async () => {
    renderTab();

    expect(screen.getByText('Coach insights from your friends')).toBeInTheDocument();
  });

  it('should display feed items on mount', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Marathon Ready')).toBeInTheDocument();
    });

    expect(screen.getByText('Just completed my first marathon training block! Feeling strong.')).toBeInTheDocument();
    expect(screen.getByText('Jane Doe')).toBeInTheDocument();
    expect(socialApi.getFeed).toHaveBeenCalled();
  });

  it('should show empty state when no feed items', async () => {
    vi.mocked(socialApi.getFeed).mockResolvedValue({
      items: [],
      next_cursor: null,
      has_more: false,
      metadata: { timestamp: '2024-01-01T00:00:00Z', api_version: 'v1' },
    });

    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Your feed is empty')).toBeInTheDocument();
    });
  });

  it('should display insight type badge', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Achievement')).toBeInTheDocument();
    });
  });

  it('should display context badges for sport type and training phase', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Running')).toBeInTheDocument();
      expect(screen.getByText('build phase')).toBeInTheDocument();
    });
  });

  it('should show Share Insight button', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Share Insight/i })).toBeInTheDocument();
    });
  });

  it('should show reaction buttons with counts', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Marathon Ready')).toBeInTheDocument();
    });

    // Check for reaction counts
    expect(screen.getByText('3')).toBeInTheDocument(); // like count
    expect(screen.getByText('2')).toBeInTheDocument(); // celebrate count
  });

  it('should add a reaction when clicking reaction button', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Marathon Ready')).toBeInTheDocument();
    });

    // Find and click the like button (first reaction button)
    const reactionButtons = screen.getAllByRole('button').filter(
      btn => btn.textContent?.includes('👍')
    );
    fireEvent.click(reactionButtons[0]);

    await waitFor(() => {
      expect(socialApi.addReaction).toHaveBeenCalledWith('insight-1', 'like');
    });
  });

  it('should show Adapt to My Training button', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Adapt to My Training/i })).toBeInTheDocument();
    });
  });

  it('should show Adapted button when user has already adapted', async () => {
    vi.mocked(socialApi.getFeed).mockResolvedValue({
      ...mockFeedItems,
      items: [
        {
          ...mockFeedItems.items[0],
          user_has_adapted: true,
        },
      ],
    });

    renderTab();

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Adapted/i })).toBeInTheDocument();
    });
  });

  it('lists the adapted history when the Adapted view is selected', async () => {
    vi.mocked(socialApi.getAdaptedInsights).mockResolvedValue(adaptedPage);

    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Marathon Ready')).toBeInTheDocument();
    });
    // Nothing is fetched until the view is opened.
    expect(socialApi.getAdaptedInsights).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('tab', { name: 'Adapted' }));

    await waitFor(() => {
      expect(
        screen.getByText(
          'Your own build block peaks at 62km — hold that before adding the long run.',
        ),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByText('Based on your last 8 weeks of running volume'),
    ).toBeInTheDocument();
    expect(socialApi.getAdaptedInsights).toHaveBeenCalledWith({ limit: 20, cursor: undefined });
    // The friends' feed is not showing while the history is.
    expect(screen.queryByText('Marathon Ready')).not.toBeInTheDocument();
  });

  it('pages the adapted history with the cursor the server returned', async () => {
    vi.mocked(socialApi.getAdaptedInsights)
      .mockResolvedValueOnce({
        ...adaptedPage,
        next_cursor: '20',
        has_more: true,
      })
      .mockResolvedValueOnce({
        insights: [
          {
            id: 'adapted-2',
            user_id: 'user-1',
            source_insight_id: 'insight-2',
            adapted_content: 'Second page adaptation: swap Thursday tempo for hills.',
            adaptation_context: null,
            created_at: '2024-01-03T00:00:00Z',
          },
        ],
        next_cursor: null,
        has_more: false,
        metadata: { timestamp: '2024-01-03T00:00:00Z', api_version: 'v1' },
      });

    renderTab();
    fireEvent.click(screen.getByRole('tab', { name: 'Adapted' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Load More/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Load More/i }));

    await waitFor(() => {
      expect(
        screen.getByText('Second page adaptation: swap Thursday tempo for hills.'),
      ).toBeInTheDocument();
    });
    expect(socialApi.getAdaptedInsights).toHaveBeenLastCalledWith({ limit: 20, cursor: '20' });
    // The first page stays on screen — a second page appends, it does not replace.
    expect(
      screen.getByText(
        'Your own build block peaks at 62km — hold that before adding the long run.',
      ),
    ).toBeInTheDocument();
  });

  it('shows the adapted empty state when the history is empty', async () => {
    renderTab();
    fireEvent.click(screen.getByRole('tab', { name: 'Adapted' }));

    await waitFor(() => {
      expect(screen.getByText('No adapted insights yet')).toBeInTheDocument();
    });
  });

  it('should open share modal when clicking Share Insight', async () => {
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('Marathon Ready')).toBeInTheDocument();
    });

    const shareButton = screen.getByRole('button', { name: /Share Insight/i });
    fireEvent.click(shareButton);

    // Modal should appear - look for the modal title "Share Insight" in the modal header
    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Share Insight' })).toBeInTheDocument();
    });
  });
});
