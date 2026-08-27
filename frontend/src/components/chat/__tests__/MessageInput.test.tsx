// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the composer's visible "/" affordance — the discoverable path to the command palette
// ABOUTME: Pins that pressing it types "/" so the same palette a typed slash opens comes up

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CommandEntry } from '@pierre/shared-types';
import MessageInput from '../MessageInput';

const listCommands = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: { listCommands: (...args: unknown[]) => listCommands(...args) },
  coachesApi: { list: vi.fn().mockResolvedValue({ coaches: [] }) },
}));

const CATALOGUE: CommandEntry[] = [
  {
    name: 'coach-list',
    command: '/coach list',
    args: null,
    description: 'List the coaches you can add to a chat',
    domain: 'coach',
  },
  {
    name: 'discover',
    command: '/discover',
    args: '[query|category]',
    description: 'Browse the coach catalogue',
    domain: 'discover',
  },
];

function Composer({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState('');
  return (
    <MessageInput
      value={value}
      onChange={setValue}
      onSend={vi.fn()}
      isStreaming={false}
      disabled={disabled}
      conversationId="conv-1"
    />
  );
}

function renderComposer(disabled = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer disabled={disabled} />
    </QueryClientProvider>,
  );
}

describe('MessageInput slash affordance', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listCommands.mockResolvedValue(CATALOGUE);
  });

  it('types "/" into the composer and opens the command palette', async () => {
    const user = userEvent.setup();
    renderComposer();

    await user.click(screen.getByTestId('slash-command-button'));

    const composer = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;
    expect(composer.value).toBe('/');
    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeInTheDocument());
    expect(screen.getByText('/coach list')).toBeInTheDocument();
    expect(screen.getByText('/discover')).toBeInTheDocument();
  });

  it('carries an accessible name so the affordance is not icon-only', () => {
    renderComposer();
    expect(screen.getByRole('button', { name: 'Commands' })).toBeInTheDocument();
  });

  it('is disabled with the composer', () => {
    renderComposer(true);
    expect(screen.getByTestId('slash-command-button')).toBeDisabled();
  });

  it('no longer offers the "Need ideas?" popover', () => {
    renderComposer();
    expect(screen.queryByText('Need ideas?')).toBeNull();
  });
});
