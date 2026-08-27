// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the @handle mention palette over the web composer
// ABOUTME: Asserts "@" offers the installed coaches by handle and inserts the handle verbatim, lowercase

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MessageInput from '../chat/MessageInput';

const listCommands = vi.fn();
const listCoaches = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: { listCommands: (...args: unknown[]) => listCommands(...args) },
  coachesApi: { list: (...args: unknown[]) => listCoaches(...args) },
}));

/**
 * What `GET /api/coaches` hands the composer: the athlete's two installed
 * coaches, a personal coach with no handle (never addressable), and a
 * catalogue coach they never installed — which the server would refuse to
 * resolve, so the palette must not offer it either.
 */
const COACHES = [
  { id: 'c-1', title: 'Recovery Coach', handle: 'recovery-coach', is_system: false, is_assigned: true },
  { id: 'c-2', title: 'Marathon Coach', handle: 'marathon-coach', is_system: false, is_assigned: true },
  { id: 'c-3', title: 'My Custom Coach', is_system: false, is_assigned: true },
  // Assigned AND system: `find_installed_by_handle` admits it, so the palette must too.
  { id: 'c-4', title: 'Sleep Coach', handle: 'sleep-coach', is_system: true, is_assigned: true },
  // Listed but never installed — no assignment row, so a mention would not route.
  { id: 'c-5', title: 'Marathon Catalogue', handle: 'marathon-catalogue', is_system: true, is_assigned: false },
];

const onSend = vi.fn();

function Composer() {
  const [value, setValue] = useState('');
  return (
    <MessageInput
      value={value}
      onChange={setValue}
      onSend={onSend}
      isStreaming={false}
      conversationId="conv-1"
    />
  );
}

function renderComposer() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer />
    </QueryClientProvider>,
  );
}

function composer(): HTMLTextAreaElement {
  return screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;
}

describe('@handle mention palette (web composer)', () => {
  beforeEach(() => {
    listCommands.mockReset();
    listCoaches.mockReset();
    onSend.mockReset();
    listCoaches.mockResolvedValue({ coaches: COACHES });
    listCommands.mockResolvedValue([]);
  });

  it('offers the installed coaches by handle when the athlete types "@"', async () => {
    renderComposer();

    await userEvent.type(composer(), '@');

    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());
    const options = screen.getAllByRole('option').map((o) => o.textContent);
    // Handle order, one row per handle. The handle-less personal coach is not
    // addressable, and neither is the catalogue coach with no assignment row —
    // while the assigned system coach is, exactly as the resolver treats them.
    expect(options).toEqual([
      '@marathon-coachMarathon Coach',
      '@recovery-coachRecovery Coach',
      '@sleep-coachSleep Coach',
    ]);
    expect(screen.queryByText('My Custom Coach')).toBeNull();
    expect(screen.queryByText('@marathon-catalogue')).toBeNull();
    expect(listCoaches).toHaveBeenCalledTimes(1);
  });

  it('inserts the handle verbatim, lowercase, followed by a space', async () => {
    renderComposer();
    const input = composer();

    await userEvent.type(input, 'Hey @Rec');
    await waitFor(() => expect(screen.getByTestId('mention-palette-option-recovery-coach')).toBeTruthy());
    expect(screen.queryByTestId('mention-palette-option-marathon-coach')).toBeNull();

    await userEvent.click(screen.getByTestId('mention-palette-option-recovery-coach'));

    expect(input.value).toBe('Hey @recovery-coach ');
    expect(input.selectionStart).toBe('Hey @recovery-coach '.length);
    expect(screen.queryByTestId('mention-palette')).toBeNull();
  });

  it('completes on Enter rather than sending a half-typed handle', async () => {
    renderComposer();
    const input = composer();

    await userEvent.type(input, '@mar');
    await waitFor(() => expect(screen.getByTestId('mention-palette-option-marathon-coach')).toBeTruthy());

    await userEvent.type(input, '{Enter}');

    expect(input.value).toBe('@marathon-coach ');
    expect(onSend).not.toHaveBeenCalled();
  });

  it('walks the rows with the arrow keys and completes the highlighted one on Tab', async () => {
    renderComposer();
    const input = composer();

    await userEvent.type(input, '@');
    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());
    expect(screen.getByTestId('mention-palette-option-marathon-coach').getAttribute('aria-selected')).toBe('true');

    await userEvent.type(input, '{ArrowDown}');
    expect(screen.getByTestId('mention-palette-option-recovery-coach').getAttribute('aria-selected')).toBe('true');

    await userEvent.type(input, '{Tab}');
    expect(input.value).toBe('@recovery-coach ');
  });

  it('lets Enter send a message whose handle is already complete', async () => {
    renderComposer();
    const input = composer();

    await userEvent.type(input, '@recovery-coach');
    await waitFor(() => expect(screen.getByTestId('mention-palette-option-recovery-coach')).toBeTruthy());

    await userEvent.type(input, '{Enter}');

    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('stays closed for an email address — a "@" inside a word is not a mention', async () => {
    renderComposer();

    await userEvent.type(composer(), 'write to jf@dravr');

    expect(screen.queryByTestId('mention-palette')).toBeNull();
    expect(listCoaches).not.toHaveBeenCalled();
  });

  it('closes on Escape and reopens on the next keystroke', async () => {
    renderComposer();
    const input = composer();

    await userEvent.type(input, '@re');
    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());

    await userEvent.type(input, '{Escape}');
    expect(screen.queryByTestId('mention-palette')).toBeNull();

    await userEvent.type(input, 'c');
    await waitFor(() => expect(screen.getByTestId('mention-palette-option-recovery-coach')).toBeTruthy());
  });
});
