// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web slash-command palette over the composer
// ABOUTME: Asserts "/" surfaces a real server command name and selecting it fills the composer

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CommandEntry } from '@pierre/shared-types';
import MessageInput from '../chat/MessageInput';

const listCommands = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    listCommands: (...args: unknown[]) => listCommands(...args),
  },
}));

/** The catalogue rows the server returns, in its domain-then-command order. */
const CATALOGUE: CommandEntry[] = [
  {
    name: 'group-invite',
    command: '/group invite',
    args: '<email>',
    description: 'Invite an athlete to this group',
    domain: 'group',
  },
  {
    name: 'group-status',
    command: '/group status',
    args: null,
    description: 'Show this group and your role in it',
    domain: 'group',
  },
  {
    name: 'plan',
    command: '/plan',
    args: '[week|today]',
    description: 'Show your training plan',
    domain: 'training',
  },
];

/** The composer with real value state, so a fill is observable in the textarea. */
function Composer({ conversationId }: { conversationId?: string | null }) {
  const [value, setValue] = useState('');
  return (
    <MessageInput
      value={value}
      onChange={setValue}
      onSend={onSend}
      isStreaming={false}
      conversationId={conversationId}
    />
  );
}

const onSend = vi.fn();

function renderComposer(conversationId?: string | null) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer conversationId={conversationId} />
    </QueryClientProvider>,
  );
}

describe('slash-command palette (web composer)', () => {
  beforeEach(() => {
    listCommands.mockReset();
    onSend.mockReset();
    listCommands.mockResolvedValue(CATALOGUE);
  });

  // Turns red if "/" stops surfacing the server's catalogue — the exact
  // "23 commands, discoverable only on messaging" gap this closes.
  it('surfaces a real server command name when the athlete types "/"', async () => {
    renderComposer('conv-1');

    await userEvent.type(screen.getByPlaceholderText('Message Dravr...'), '/');

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByTestId('command-palette-option-plan')).toBeTruthy();
    expect(screen.getByText('/plan')).toBeTruthy();
    expect(screen.getByText('Show your training plan')).toBeTruthy();
    // The palette asked the server for THIS conversation, so group-scoped
    // commands are resolved against the group it is bound to.
    expect(listCommands).toHaveBeenCalledWith('conv-1');
  });

  // Turns red if the client ever hardcodes a command list: only what the
  // server returned may appear, so a shorter answer means a shorter palette.
  it('offers only the commands the server returned for this caller', async () => {
    listCommands.mockResolvedValue([CATALOGUE[2]]);
    renderComposer(null);

    await userEvent.type(screen.getByPlaceholderText('Message Dravr...'), '/');

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByTestId('command-palette-option-plan')).toBeTruthy();
    expect(screen.queryByTestId('command-palette-option-group-invite')).toBeNull();
  });

  // Turns red if the prefix filter breaks — typing narrows to the matching
  // commands rather than showing the whole catalogue forever.
  it('narrows the list as the command is typed', async () => {
    renderComposer('conv-1');
    const input = screen.getByPlaceholderText('Message Dravr...');

    await userEvent.type(input, '/group i');

    await waitFor(() => expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy());
    expect(screen.queryByTestId('command-palette-option-plan')).toBeNull();
    expect(screen.queryByTestId('command-palette-option-group-status')).toBeNull();
  });

  // Turns red if selecting stops filling the composer, or fills it without the
  // trailing space a command with arguments needs.
  it('fills the composer with the selected command', async () => {
    renderComposer('conv-1');
    const input = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;

    await userEvent.type(input, '/gro');
    await waitFor(() => expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy());

    await userEvent.click(screen.getByTestId('command-palette-option-group-invite'));

    expect(input.value).toBe('/group invite ');
  });

  // Turns red if Enter starts sending a half-typed command instead of
  // completing it — the palette owns Enter while it is open.
  it('completes on Enter rather than sending a half-typed command', async () => {
    renderComposer('conv-1');
    const input = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;

    await userEvent.type(input, '/pl');
    await waitFor(() => expect(screen.getByTestId('command-palette-option-plan')).toBeTruthy());

    await userEvent.type(input, '{Enter}');

    expect(input.value).toBe('/plan ');
    expect(onSend).not.toHaveBeenCalled();
  });

  // Turns red if the palette opens on prose. A "/" mid-sentence is a slash,
  // not a command, and a palette over ordinary typing is a regression.
  it('stays closed for text that is not a command draft', async () => {
    renderComposer('conv-1');

    await userEvent.type(screen.getByPlaceholderText('Message Dravr...'), 'how many km/h was that');

    expect(screen.queryByTestId('command-palette')).toBeNull();
    expect(listCommands).not.toHaveBeenCalled();
  });


  // Turns red if the domain badge goes back to printing the raw slug: the
  // badge reads the same catalogue key /help heads its domains with, and a
  // slug the catalogue has no word for stays as it is rather than rendering
  // a missing-key placeholder.
  it('names each command domain in the athlete\'s language, and keeps an unknown slug', async () => {
    listCommands.mockResolvedValue([
      CATALOGUE[2],
      {
        name: 'logout',
        command: '/logout',
        args: null,
        description: 'Unlink this messaging account',
        domain: 'account',
      },
      {
        name: 'teleport',
        command: '/teleport',
        args: null,
        description: 'Go somewhere new',
        domain: 'unmapped-domain',
      },
    ]);
    renderComposer('conv-1');

    await userEvent.type(screen.getByPlaceholderText('Message Dravr...'), '/');

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByText('Training')).toBeTruthy();
    expect(screen.getByText('Account')).toBeTruthy();
    expect(screen.getByText('unmapped-domain')).toBeTruthy();
    expect(screen.queryByText('training')).toBeNull();
  });
});
