// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the Discover edit sheet — load, save, delete — and the update request it builds
// ABOUTME: Pins the three-state tool budget on the wire: absent inherits, a number pins, null clears

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Coach } from '@pierre/shared-types';
import CoachEditSheet from '../CoachEditSheet';
import { DEFAULT_COACH_FORM_DATA, formDataToUpdateRequest, type CoachFormData } from '../coachForm';

const getCoach = vi.fn();
const updateCoach = vi.fn();
const deleteCoach = vi.fn();

vi.mock('../../../services/api', () => ({
  coachesApi: {
    get: (...a: unknown[]) => getCoach(...a),
    update: (...a: unknown[]) => updateCoach(...a),
    delete: (...a: unknown[]) => deleteCoach(...a),
  },
}));

const COACH_ID = 'coach-tempo';
const COACH_TITLE = 'Tempo Coach';

function storedCoach(overrides: Partial<Coach> = {}): Coach {
  return {
    id: COACH_ID,
    title: COACH_TITLE,
    description: 'Threshold work and race-week sharpening',
    system_prompt: 'You are a tempo coach.',
    category: 'Training',
    tags: ['tempo'],
    token_count: 40,
    is_favorite: false,
    use_count: 4,
    last_used_at: null,
    created_at: '2026-07-01T00:00:00Z',
    updated_at: '2026-08-01T10:00:00Z',
    is_system: false,
    visibility: 'private',
    is_assigned: true,
    forked_from: 'store-tempo',
    handle: 'tempo-coach',
    max_tool_iterations: 25,
    ...overrides,
  };
}

function renderSheet(onClose = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <CoachEditSheet coachId={COACH_ID} onClose={onClose} />
    </QueryClientProvider>,
  );
  return { onClose, queryClient };
}

describe('CoachEditSheet', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getCoach.mockResolvedValue(storedCoach());
    updateCoach.mockImplementation(async (_id: string, request: Record<string, unknown>) =>
      storedCoach({ ...(request as Partial<Coach>) }),
    );
    deleteCoach.mockResolvedValue(undefined);
  });

  it('loads the coach by id and hydrates the form from it', async () => {
    renderSheet();

    expect(await screen.findByRole('heading', { name: 'Edit Coach' })).toBeInTheDocument();
    expect(getCoach).toHaveBeenCalledWith(COACH_ID);
    expect(screen.getByPlaceholderText('e.g., Marathon Training Coach')).toHaveValue(COACH_TITLE);
    expect(screen.getByLabelText('Max tool iterations per turn')).toHaveValue(25);
  });

  it('saves through coachesApi.update with the update request and closes', async () => {
    const user = userEvent.setup();
    const { onClose } = renderSheet();
    await screen.findByRole('heading', { name: 'Edit Coach' });

    const description = screen.getByPlaceholderText('Brief description of what this coach specializes in');
    await user.clear(description);
    await user.type(description, 'Sharpening only');
    await user.click(screen.getByRole('button', { name: 'Save Changes' }));

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(updateCoach).toHaveBeenCalledTimes(1);
    const [id, request] = updateCoach.mock.calls[0] as [string, Record<string, unknown>];
    expect(id).toBe(COACH_ID);
    expect(request.title).toBe(COACH_TITLE);
    expect(request.description).toBe('Sharpening only');
    // The stored pin rides along untouched — never the workspace default.
    expect(request.max_tool_iterations).toBe(25);
  });

  it('deletes through coachesApi.delete after confirmation and closes', async () => {
    const user = userEvent.setup();
    const { onClose } = renderSheet();
    await screen.findByRole('heading', { name: 'Edit Coach' });

    await user.click(screen.getByRole('button', { name: 'Delete this coach' }));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByText('Delete Coach?')).toBeInTheDocument();
    expect(within(dialog).getByText(`Delete coach "${COACH_TITLE}"? This cannot be undone.`)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(deleteCoach).toHaveBeenCalledWith(COACH_ID));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(updateCoach).not.toHaveBeenCalled();
  });

  it('cancelling the confirmation keeps the coach', async () => {
    const user = userEvent.setup();
    const { onClose } = renderSheet();
    await screen.findByRole('heading', { name: 'Edit Coach' });

    await user.click(screen.getByRole('button', { name: 'Delete this coach' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));

    expect(deleteCoach).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('heading', { name: 'Edit Coach' })).toBeInTheDocument();
  });

  it('shows the load failure instead of an empty form', async () => {
    getCoach.mockRejectedValueOnce(new Error('Coach coach-tempo not found'));
    renderSheet();

    expect(await screen.findByText("Couldn't load this coach")).toBeInTheDocument();
    expect(screen.getByText('Coach coach-tempo not found')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Edit Coach' })).not.toBeInTheDocument();
  });
});

describe('coach form → update request tool budget', () => {
  function filledForm(overrides: Partial<CoachFormData> = {}): CoachFormData {
    return {
      ...DEFAULT_COACH_FORM_DATA,
      title: COACH_TITLE,
      system_prompt: 'You are an expert marathon coach.',
      ...overrides,
    };
  }

  it('omits max_tool_iterations when the field is untouched', () => {
    const request = formDataToUpdateRequest(filledForm());

    expect(request.max_tool_iterations).toBeUndefined();
    expect('max_tool_iterations' in request).toBe(false);
    expect(request.title).toBe(COACH_TITLE);
  });

  it('sends max_tool_iterations when the user entered one', () => {
    const request = formDataToUpdateRequest(filledForm({ max_tool_iterations: 3 }));

    expect(request.max_tool_iterations).toBe(3);
  });

  it('sends an explicit null when the user cleared the box', () => {
    const request = formDataToUpdateRequest(filledForm({ max_tool_iterations: null }));

    // The key has to be PRESENT and null. Merely leaving it out is what the
    // untouched state does, and the server preserves an absent field — so an
    // omitted key would leave the coach's existing pin in place forever.
    expect('max_tool_iterations' in request).toBe(true);
    expect(request.max_tool_iterations).toBeNull();
  });

  it('carries data_requirements only while pre-fetch is enabled', () => {
    const enabled = formDataToUpdateRequest(
      filledForm({ prefetch_enabled: true, activity_count: 30, time_frame: '16w' }),
    );
    expect(enabled.data_requirements?.activities).toMatchObject({
      count: 30,
      time_frame: '16w',
      format: 'toon',
      mode: 'summary',
    });

    const disabled = formDataToUpdateRequest(filledForm({ prefetch_enabled: false }));
    expect('data_requirements' in disabled).toBe(false);
  });
});
