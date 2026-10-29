// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the composer's visible "/" affordance, its growth with the draft, and a coach's Everyone / Only <agent> switch
// ABOUTME: Pins that pressing "/" opens the palette, that the field carries a name, and how the switch and the quote chip read (carnet#1249)

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CommandEntry, MessageAudience, RoomQuote } from '@pierre/shared-types';
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

const onStop = vi.fn();

interface ComposerProps {
  disabled?: boolean;
  isStreaming?: boolean;
  isStopping?: boolean;
}

function Composer({ disabled = false, isStreaming = false, isStopping = false }: ComposerProps) {
  const [value, setValue] = useState('');
  return (
    <MessageInput
      value={value}
      onChange={setValue}
      onSend={vi.fn()}
      isStreaming={isStreaming}
      onStop={onStop}
      isStopping={isStopping}
      disabled={disabled}
      conversationId="conv-1"
    />
  );
}

function renderComposer(disabled = false, turn: Pick<ComposerProps, 'isStreaming' | 'isStopping'> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer disabled={disabled} {...turn} />
    </QueryClientProvider>,
  );
}

// carnet#705: while a turn runs the send button is a stop button. It was only
// ever disabled, so an athlete had no way to end a reply they no longer wanted.
describe('MessageInput stop button', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listCommands.mockResolvedValue(CATALOGUE);
  });

  it('shows send and no stop while no turn is running', () => {
    renderComposer();
    expect(screen.getByRole('button', { name: 'Send message' })).toBeInTheDocument();
    expect(screen.queryByTestId('stop-turn-button')).toBeNull();
  });

  it('replaces send with an enabled stop button while a turn streams, and stops on press', async () => {
    const user = userEvent.setup();
    renderComposer(false, { isStreaming: true });

    expect(screen.queryByRole('button', { name: 'Send message' })).toBeNull();
    const stop = screen.getByRole('button', { name: 'Stop this reply' });
    expect(stop).toHaveAttribute('data-testid', 'stop-turn-button');
    expect(stop).toBeEnabled();

    await user.click(stop);
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('holds the stop button disabled once the stop was sent', async () => {
    const user = userEvent.setup();
    renderComposer(false, { isStreaming: true, isStopping: true });

    const stop = screen.getByTestId('stop-turn-button');
    expect(stop).toBeDisabled();
    await user.click(stop);
    expect(onStop).not.toHaveBeenCalled();
  });
});

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

/** The athlete's room message "Adjust plan from this" quoted. */
const QUOTE: RoomQuote = {
  entry_id: 'entry-camille-1',
  author_user_id: 'user-camille',
  author_display_name: 'Camille',
  content: "Legs felt heavy in this morning's swim. Is Thursday's threshold run still a good idea?",
  created_at: '2026-10-29T07:12:00Z',
};

/** A coach's composer in a one-athlete room: the switch and the quote held as the room screen holds them. */
function LaneComposer({ initial = 'room', quote = null }: { initial?: MessageAudience; quote?: RoomQuote | null }) {
  const [value, setValue] = useState('');
  const [audience, setAudience] = useState<MessageAudience>(initial);
  const [held, setHeld] = useState<RoomQuote | null>(quote);
  return (
    <MessageInput
      value={value}
      onChange={setValue}
      onSend={vi.fn()}
      isStreaming={false}
      onStop={onStop}
      conversationId="conv-1"
      focusOnMount={false}
      privateLane={{
        audience,
        agentName: 'Dravr',
        athleteName: 'Camille',
        onToggle: () => setAudience((current) => (current === 'private' ? 'room' : 'private')),
        quote: held,
        onRemoveQuote: () => setHeld(null),
      }}
    />
  );
}

/**
 * WCAG 2.5.3 Label in Name: the switch's accessible name starts with the
 * words it shows, then says what a press does. The name carries the state, so
 * the switch announces no pressed state on top of it.
 */
function expectNamedByItsLabel(toggle: HTMLElement, label: string) {
  expect(toggle.textContent?.trim()).toBe(label);
  expect(toggle.getAttribute('aria-label')?.startsWith(`${label}.`)).toBe(true);
  expect(toggle).not.toHaveAttribute('aria-pressed');
}

function renderLaneComposer(props: { initial?: MessageAudience; quote?: RoomQuote | null } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <LaneComposer {...props} />
    </QueryClientProvider>,
  );
}

// carnet#1249: in a room of one coach and one athlete, the coach can turn the
// composer to the agent alone. A private message sent to the room by mistake
// is the costlier error, so the switch is explicit and the field says so.
describe('MessageInput Everyone / Only <agent> switch', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listCommands.mockResolvedValue(CATALOGUE);
  });

  it('carries no switch outside a coach\'s one-athlete room', () => {
    renderComposer();
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Message Dravr' })).toHaveAttribute('placeholder', 'Message Dravr...');
  });

  it('starts at Everyone: a real button named by its label, over the room placeholder', () => {
    renderLaneComposer();

    const toggle = screen.getByRole('button', { name: 'Everyone. Switch to only Dravr' });
    expect(toggle.tagName).toBe('BUTTON');
    expectNamedByItsLabel(toggle, 'Everyone');
    const field = screen.getByRole('textbox', { name: 'Message Camille and Dravr. Type / for commands' });
    expect(field).toHaveAttribute('placeholder', 'Message Camille and Dravr. Type / for commands');
    expect(field).toHaveAttribute('data-audience', 'room');
    expect(field).not.toHaveClass('border-dashed');
    expect(screen.getByTestId('slash-command-button')).toBeInTheDocument();
  });

  it('turns to Only Dravr on a press: renamed, the field dashed in the private accent, the private placeholder', async () => {
    const user = userEvent.setup();
    renderLaneComposer();

    await user.click(screen.getByTestId('composer-audience-switch'));

    const toggle = screen.getByRole('button', { name: 'Only Dravr. Switch to everyone' });
    expectNamedByItsLabel(toggle, 'Only Dravr');
    const field = screen.getByRole('textbox', { name: "Ask Dravr to change Camille's plan. Camille won't see this." });
    expect(field).toHaveAttribute('data-audience', 'private');
    expect(field).toHaveClass('border-dashed', 'border-warning/60');
    // The private placeholder offers no commands, so the slash steps aside.
    expect(screen.queryByTestId('slash-command-button')).toBeNull();

    await user.click(toggle);
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Everyone. Switch to only Dravr');
    expect(screen.getByTestId('slash-command-button')).toBeInTheDocument();
  });

  it('is reached and pressed from the keyboard', async () => {
    const user = userEvent.setup();
    renderLaneComposer();

    await user.tab();
    expect(screen.getByTestId('composer-audience-switch')).toHaveFocus();
    await user.keyboard(' ');
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone');
    await user.keyboard('{Enter}');
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Everyone. Switch to only Dravr');
  });

  it("shows the quoted message as a chip naming the athlete, hands the caret to the field, and lets the quote go", async () => {
    const user = userEvent.setup();
    renderLaneComposer({ initial: 'private', quote: QUOTE });

    const chip = screen.getByTestId('composer-quote');
    expect(chip).toHaveTextContent("From Camille's message");
    expect(chip).toHaveTextContent("Legs felt heavy in this morning's swim.");
    // "Adjust plan from this" ends with the coach typing.
    expect(screen.getByRole('textbox', { name: /Ask Dravr to change/ })).toHaveFocus();

    await user.click(screen.getByRole('button', { name: 'Remove quote' }));

    expect(screen.queryByTestId('composer-quote')).toBeNull();
    // Taking the quote off leaves the audience where it was.
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone');
  });
});
