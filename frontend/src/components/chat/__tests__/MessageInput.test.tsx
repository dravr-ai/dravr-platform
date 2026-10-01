// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the composer's visible "/" affordance — the discoverable path to the command palette
// ABOUTME: Pins that pressing it types "/" so the same palette a typed slash opens comes up, that the field carries a name, and that it grows with the draft

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CommandEntry } from '@pierre/shared-types';
import MessageInput from '../MessageInput';

const listCommands = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: { listCommands: (...args: unknown[]) => listCommands(...args) },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
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

  it('names the composer field for a screen reader and for autofill, in the athlete\'s language', () => {
    renderComposer();
    const composer = screen.getByRole('textbox', { name: 'Message Dravr' });
    expect(composer.tagName).toBe('TEXTAREA');
    expect(composer).toHaveAttribute('name', 'message');
    expect(composer.id).not.toBe('');
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

// jsdom lays nothing out and loads no Tailwind, so the field's box is
// modelled here with the numbers the browser produces for `py-2 text-base`
// on a fine pointer: a 23px line and 8px of padding above and below. The
// real geometry is pinned by e2e/chat-composer-geometry.spec.ts.
const LINE_PX = 23;
const PADDING_PX = 8;

describe('MessageInput auto-grow', () => {
  let fieldStyle: HTMLStyleElement;

  beforeEach(() => {
    vi.clearAllMocks();
    listCommands.mockResolvedValue(CATALOGUE);
    fieldStyle = document.createElement('style');
    fieldStyle.textContent = `textarea { line-height: ${LINE_PX}px; padding-top: ${PADDING_PX}px; padding-bottom: ${PADDING_PX}px; }`;
    document.head.appendChild(fieldStyle);
    Object.defineProperty(HTMLTextAreaElement.prototype, 'scrollHeight', {
      configurable: true,
      get(this: HTMLTextAreaElement) {
        return this.value.split('\n').length * LINE_PX + 2 * PADDING_PX;
      },
    });
  });

  afterEach(() => {
    fieldStyle.remove();
    delete (HTMLTextAreaElement.prototype as { scrollHeight?: number }).scrollHeight;
  });

  it('grows one line at a time with the draft', async () => {
    const user = userEvent.setup();
    renderComposer();
    const composer = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;

    expect(composer.style.height).toBe('39px');
    await user.type(composer, 'Je ferai le VTXL{Shift>}{Enter}{/Shift}puis Gravelooza');
    expect(composer.style.height).toBe('62px');
    expect(composer.style.overflowY).toBe('hidden');
  });

  it('stops at eight lines and scrolls past them', async () => {
    const user = userEvent.setup();
    renderComposer();
    const composer = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;

    await user.type(composer, 'ligne{Shift>}{Enter}{/Shift}'.repeat(11) + 'fin');
    expect(composer.style.height).toBe(`${8 * LINE_PX + 2 * PADDING_PX}px`);
    expect(composer.style.overflowY).toBe('auto');
  });

  it('shrinks back to one line when the draft is cleared', async () => {
    const user = userEvent.setup();
    renderComposer();
    const composer = screen.getByPlaceholderText('Message Dravr...') as HTMLTextAreaElement;

    await user.type(composer, 'a{Shift>}{Enter}{/Shift}b{Shift>}{Enter}{/Shift}c');
    expect(composer.style.height).toBe('85px');
    await user.clear(composer);
    expect(composer.style.height).toBe('39px');
  });
});
