// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the "Your coaches" section pinned on Discover — own query, @handle, and the library actions
// ABOUTME: Asserts uninstall vs delete by coach origin, chat start, edit, create, search, hidden, and failure states

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Coach } from '@pierre/shared-types';
import InstalledCoaches from '../InstalledCoaches';

const listCoaches = vi.fn();
const getHidden = vi.fn();
const createCoach = vi.fn();
const updateCoach = vi.fn();
const deleteCoach = vi.fn();
const toggleFavorite = vi.fn();
const hideCoach = vi.fn();
const showCoach = vi.fn();
const forkCoach = vi.fn();
const exportAsMarkdown = vi.fn();
const uninstall = vi.fn();
const createConversation = vi.fn();

vi.mock('../../../services/api', () => ({
  coachesApi: {
    list: (...a: unknown[]) => listCoaches(...a),
    getHidden: (...a: unknown[]) => getHidden(...a),
    create: (...a: unknown[]) => createCoach(...a),
    update: (...a: unknown[]) => updateCoach(...a),
    delete: (...a: unknown[]) => deleteCoach(...a),
    toggleFavorite: (...a: unknown[]) => toggleFavorite(...a),
    hide: (...a: unknown[]) => hideCoach(...a),
    show: (...a: unknown[]) => showCoach(...a),
    fork: (...a: unknown[]) => forkCoach(...a),
    exportAsMarkdown: (...a: unknown[]) => exportAsMarkdown(...a),
  },
  storeApi: {
    uninstall: (...a: unknown[]) => uninstall(...a),
  },
  chatApi: {
    createConversation: (...a: unknown[]) => createConversation(...a),
  },
}));

function coach(overrides: Partial<Coach>): Coach {
  return {
    id: 'coach-1',
    title: 'Coach',
    description: null,
    system_prompt: 'You are a coach.',
    category: 'training',
    tags: [],
    token_count: 100,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-02T00:00:00Z',
    is_system: false,
    ...overrides,
  };
}

/** A copy installed from the store: `forked_from` set, handle inherited. */
const installedCopy = coach({
  id: 'copy-1',
  title: 'Marathon Coach',
  description: 'Long-distance preparation',
  forked_from: 'store-listing-1',
  handle: 'marathon-coach',
  use_count: 4,
});

/** A personal coach that was never published: no handle. */
const personalCoach = coach({
  id: 'own-1',
  title: 'My Custom Coach',
  description: 'Personal training coach',
  purpose: 'Expert in marathon preparation.',
  instructions: 'Personal instructions.',
});

/** An assigned system coach carrying its catalogue handle. */
const systemCoach = coach({
  id: 'sys-1',
  title: 'Recovery Coach',
  category: 'recovery',
  is_system: true,
  handle: 'recovery-coach',
  use_count: 9,
});

function renderSection(props: { searchQuery?: string; onNavigate?: (route: string) => void } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <InstalledCoaches searchQuery={props.searchQuery ?? ''} onNavigate={props.onNavigate} />
    </QueryClientProvider>,
  );
}

describe('InstalledCoaches', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listCoaches.mockResolvedValue({ coaches: [installedCopy, personalCoach, systemCoach], total: 3 });
    getHidden.mockResolvedValue({ coaches: [] });
  });

  it('lists the coach list from its own query, with the @handle of each addressable coach', async () => {
    renderSection();

    expect(await screen.findByText('Marathon Coach')).toBeInTheDocument();
    expect(screen.getByText('My Custom Coach')).toBeInTheDocument();
    expect(screen.getByText('Recovery Coach')).toBeInTheDocument();
    // The section reads the athlete's list, hidden coaches included — not the store.
    expect(listCoaches).toHaveBeenCalledWith({ include_hidden: true, personalize: true });

    const handles = screen.getAllByTestId('coach-handle').map((el) => el.textContent);
    expect(handles).toEqual(['@recovery-coach', '@marathon-coach']);
    expect(screen.getByRole('heading', { name: 'Your coaches (3)' })).toBeInTheDocument();
  });

  it('shows the empty state when the athlete has no coaches', async () => {
    listCoaches.mockResolvedValue({ coaches: [], total: 0 });
    renderSection();

    expect(await screen.findByText('No coaches yet')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create Your First Coach' })).toBeInTheDocument();
    expect(screen.queryAllByTestId('installed-coach-card')).toHaveLength(0);
  });

  it('shows a retry state, not the empty copy, when the list fails', async () => {
    listCoaches.mockRejectedValueOnce(new Error('Network Error'));
    const user = userEvent.setup();
    renderSection();

    expect(await screen.findByText("Couldn't load your coaches")).toBeInTheDocument();
    expect(screen.getByText('Network Error')).toBeInTheDocument();
    expect(screen.queryByText('No coaches yet')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Try Again' }));
    expect(await screen.findByText('Marathon Coach')).toBeInTheDocument();
  });

  it('uninstalls an installed copy through the store, by the copy id', async () => {
    uninstall.mockResolvedValue({ message: 'ok', source_coach_id: 'store-listing-1' });
    const user = userEvent.setup();
    renderSection();

    const card = (await screen.findByText('Marathon Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    await user.click(within(card).getByRole('button', { name: 'Remove' }));
    await user.click(screen.getByRole('dialog').querySelector('button.btn-danger') as HTMLElement);

    await waitFor(() => expect(uninstall).toHaveBeenCalledWith('copy-1'));
    expect(deleteCoach).not.toHaveBeenCalled();
  });

  it('deletes a personal coach through the coaches API', async () => {
    deleteCoach.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderSection();

    const card = (await screen.findByText('My Custom Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    await user.click(within(card).getByRole('button', { name: 'Delete' }));
    await user.click(screen.getByRole('dialog').querySelector('button.btn-danger') as HTMLElement);

    await waitFor(() => expect(deleteCoach).toHaveBeenCalledWith('own-1'));
    expect(uninstall).not.toHaveBeenCalled();
  });

  it('offers no Edit or Delete on a system coach, but Fork and Hide', async () => {
    renderSection();

    const card = (await screen.findByText('Recovery Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    expect(within(card).queryByRole('button', { name: 'Edit' })).toBeNull();
    expect(within(card).queryByRole('button', { name: 'Delete' })).toBeNull();
    expect(within(card).getByRole('button', { name: 'Fork' })).toBeInTheDocument();
    expect(within(card).getByRole('button', { name: 'Hide' })).toBeInTheDocument();
  });

  it('opens a conversation with the coach and routes to it', async () => {
    createConversation.mockResolvedValue({ id: 'conv-9', title: 'Chat' });
    const onNavigate = vi.fn();
    const user = userEvent.setup();
    renderSection({ onNavigate });

    const card = (await screen.findByText('Marathon Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    await user.click(within(card).getByRole('button', { name: 'Chat' }));

    await waitFor(() => expect(createConversation).toHaveBeenCalledTimes(1));
    expect(createConversation.mock.calls[0][0]).toMatchObject({ coach_id: 'copy-1' });
    expect(onNavigate).toHaveBeenCalledWith('chat/conv-9');
  });

  it('edits a personal coach through the shared coach editor', async () => {
    updateCoach.mockResolvedValue({ ...personalCoach, title: 'Renamed Coach' });
    const user = userEvent.setup();
    renderSection();

    const card = (await screen.findByText('My Custom Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    await user.click(within(card).getByRole('button', { name: 'Edit' }));

    const title = screen.getByPlaceholderText(/Marathon Training Coach/i) as HTMLInputElement;
    expect(title.value).toBe('My Custom Coach');
    await user.clear(title);
    await user.type(title, 'Renamed Coach');
    await user.click(screen.getByRole('button', { name: 'Save Changes' }));

    await waitFor(() => expect(updateCoach).toHaveBeenCalledTimes(1));
    expect(updateCoach.mock.calls[0][0]).toBe('own-1');
    expect(updateCoach.mock.calls[0][1]).toMatchObject({ title: 'Renamed Coach' });
  });

  it('creates a coach from the section header', async () => {
    createCoach.mockResolvedValue(coach({ id: 'new-1', title: 'Budget Coach' }));
    const user = userEvent.setup();
    renderSection();
    await screen.findByText('Marathon Coach');

    await user.click(screen.getByRole('button', { name: 'Create Coach' }));
    await user.type(screen.getByPlaceholderText(/Marathon Training Coach/i), 'Budget Coach');
    await user.type(screen.getByPlaceholderText(/Define your coach's personality/i), 'You budget every tool call.');
    await user.type(screen.getByLabelText('Max tool iterations per turn'), '25');
    const submit = screen
      .getAllByRole('button', { name: 'Create Coach' })
      .find((b) => b.getAttribute('type') === 'submit') as HTMLElement;
    await user.click(submit);

    await waitFor(() => expect(createCoach).toHaveBeenCalledTimes(1));
    expect(createCoach).toHaveBeenCalledWith({
      title: 'Budget Coach',
      description: undefined,
      system_prompt: 'You budget every tool call.',
      category: 'Training',
      max_tool_iterations: 25,
    });
  });

  it('narrows to the Discover search, matching handles as well as titles', async () => {
    const { rerender } = renderSection({ searchQuery: 'recovery-co' });
    expect(await screen.findByText('Recovery Coach')).toBeInTheDocument();
    expect(screen.queryByText('Marathon Coach')).toBeNull();

    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    rerender(
      <QueryClientProvider client={queryClient}>
        <InstalledCoaches searchQuery="nothing-matches" />
      </QueryClientProvider>,
    );
    expect(await screen.findByText('No coaches match')).toBeInTheDocument();
  });

  it('reveals hidden coaches only when asked', async () => {
    getHidden.mockResolvedValue({ coaches: [{ id: 'sys-1' }] });
    const user = userEvent.setup();
    renderSection();

    expect(await screen.findByText('Marathon Coach')).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText('Recovery Coach')).toBeNull());

    await user.click(screen.getByRole('button', { name: 'Show hidden coaches' }));
    expect(await screen.findByText('Recovery Coach')).toBeInTheDocument();
    expect(screen.getByText('Hidden')).toBeInTheDocument();
  });

  it('opens the detail sheet with the structured sections', async () => {
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByRole('button', { name: 'Open My Custom Coach' }));

    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByRole('heading', { name: 'Purpose' })).toBeInTheDocument();
    expect(within(dialog).getByText('Expert in marathon preparation.')).toBeInTheDocument();
    expect(within(dialog).queryByRole('heading', { name: 'System Prompt' })).toBeNull();
    expect(within(dialog).getByRole('button', { name: 'Edit' })).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeInTheDocument();
  });

  it('surfaces the server error when export fails', async () => {
    exportAsMarkdown.mockRejectedValueOnce(new Error('Coach export is unavailable'));
    const user = userEvent.setup();
    renderSection();

    const card = (await screen.findByText('Marathon Coach')).closest('[data-testid="installed-coach-card"]') as HTMLElement;
    await user.click(within(card).getByRole('button', { name: 'Export' }));

    expect(await screen.findByText('Coach export is unavailable')).toBeInTheDocument();
  });
});
