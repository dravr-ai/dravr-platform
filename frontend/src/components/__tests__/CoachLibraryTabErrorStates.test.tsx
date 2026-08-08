// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for CoachLibraryTab failure surfaces
// ABOUTME: Covers the failed-list error state and the mutation error banner

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachLibraryTab from '../CoachLibraryTab';

const testCoach = {
  id: 'coach-1',
  title: 'Test Coach',
  description: 'A test coach for unit testing',
  system_prompt: 'You are a helpful coach.',
  category: 'Training',
  tags: ['test', 'unit'],
  token_count: 100,
  is_favorite: false,
  use_count: 5,
  is_system: false,
  visibility: 'private',
  created_at: '2025-01-01T00:00:00Z',
  updated_at: '2025-01-01T00:00:00Z',
};

vi.mock('../../services/api', () => ({
  coachesApi: {
    list: vi.fn(),
    getHidden: vi.fn(),
    exportAsMarkdown: vi.fn(),
    delete: vi.fn(),
  },
}));

import { coachesApi } from '../../services/api';

function renderCoachLibraryTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <CoachLibraryTab />
    </QueryClientProvider>
  );
}

describe('CoachLibraryTab failure surfaces', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(coachesApi.list).mockResolvedValue({
      coaches: [testCoach],
      total: 1,
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    });
    vi.mocked(coachesApi.getHidden).mockResolvedValue({
      coaches: [],
      metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
    });
  });

  describe('failed coach list fetch', () => {
    it('should show a retry error state instead of the "no coaches" copy', async () => {
      vi.mocked(coachesApi.list).mockRejectedValueOnce(new Error('Network Error'));

      renderCoachLibraryTab();

      await waitFor(() => {
        expect(screen.getByText("Couldn't load your coaches")).toBeInTheDocument();
      });

      // A failed fetch must not claim the library is empty.
      expect(screen.queryByText('No Coaches Yet')).not.toBeInTheDocument();
      expect(
        screen.queryByText('Create your first coach to customize how Pierre helps you.')
      ).not.toBeInTheDocument();
      expect(screen.getByText('Network Error')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Try Again' })).toBeInTheDocument();
    });

    it('should render the coach list when Try Again succeeds', async () => {
      vi.mocked(coachesApi.list).mockRejectedValueOnce(new Error('Network Error'));

      const user = userEvent.setup();
      renderCoachLibraryTab();

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Try Again' })).toBeInTheDocument();
      });

      await user.click(screen.getByRole('button', { name: 'Try Again' }));

      await waitFor(() => {
        expect(screen.getByText('Test Coach')).toBeInTheDocument();
      });
      expect(screen.queryByText("Couldn't load your coaches")).not.toBeInTheDocument();
    });
  });

  describe('failed mutations', () => {
    it('should surface the server error when export fails', async () => {
      vi.mocked(coachesApi.exportAsMarkdown).mockRejectedValueOnce(
        new Error('Coach export is unavailable')
      );

      const user = userEvent.setup();
      renderCoachLibraryTab();

      await waitFor(() => {
        expect(screen.getByText('Test Coach')).toBeInTheDocument();
      });

      await user.click(screen.getByRole('button', { name: 'Export' }));

      await waitFor(() => {
        expect(screen.getByText('Coach export is unavailable')).toBeInTheDocument();
      });
    });

    it('should surface the server error when delete fails', async () => {
      vi.mocked(coachesApi.delete).mockRejectedValueOnce(
        new Error('Coach is assigned to a group')
      );
      const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

      const user = userEvent.setup();
      renderCoachLibraryTab();

      await waitFor(() => {
        expect(screen.getByText('Test Coach')).toBeInTheDocument();
      });

      await user.click(screen.getByText('Test Coach'));

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Delete' })).toBeInTheDocument();
      });
      await user.click(screen.getByRole('button', { name: 'Delete' }));

      await waitFor(() => {
        expect(screen.getByText('Coach is assigned to a group')).toBeInTheDocument();
      });

      confirmSpy.mockRestore();
    });
  });
});
