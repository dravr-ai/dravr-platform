// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared command and mention palettes — which rows they offer and which keys they take
// ABOUTME: Both composers hand these the key's name, so one set of cases covers web and mobile

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { renderHook, waitFor, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Agent, CommandEntry } from '@pierre/shared-types';
import { createCommandPaletteHook } from '../src/commandPalette';
import { createMentionPaletteHook } from '../src/mentionPalette';
import { PALETTE_KEYS } from '../src/paletteKeys';

function wrapperFor(client: QueryClient) {
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

function entry(command: string, args: string | null = null): CommandEntry {
  return { name: command.slice(1).replace(/ /g, '-'), command, args, description: command, domain: 'group' };
}

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: 'agent-1',
    title: 'Tempo Coach',
    handle: 'tempo-coach',
    description: 'Threshold work',
    category: 'training',
    is_system: false,
    is_assigned: true,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-20T10:00:00Z',
    ...overrides,
  } as Agent;
}

describe('PALETTE_KEYS', () => {
  it('spells the keys as a DOM KeyboardEvent and a React Native key press both report them', () => {
    expect(PALETTE_KEYS).toEqual({
      down: 'ArrowDown',
      up: 'ArrowUp',
      enter: 'Enter',
      tab: 'Tab',
      escape: 'Escape',
    });
  });
});

describe('createCommandPaletteHook', () => {
  const listCommands = vi.fn();
  const useCommandPalette = createCommandPaletteHook({ listCommands });
  let client: QueryClient;

  beforeEach(() => {
    listCommands.mockReset();
    listCommands.mockResolvedValue([
      entry('/group invite'),
      entry('/group status'),
      entry('/plan', '[week|today]'),
    ]);
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });

  it('asks for the catalogue only once the athlete has typed a slash', () => {
    renderHook(() => useCommandPalette({ value: 'hello', onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });
    expect(listCommands).not.toHaveBeenCalled();
  });

  it('answers for the open conversation, under that conversation key', async () => {
    const { result } = renderHook(
      () => useCommandPalette({ value: '/gr', conversationId: 'conv-9', onChange: vi.fn() }),
      { wrapper: wrapperFor(client) },
    );

    await waitFor(() => expect(result.current.matches).toHaveLength(2));
    expect(listCommands).toHaveBeenCalledWith('conv-9');
    expect(client.getQueryCache().find({ queryKey: ['chat-commands', 'conv-9'], exact: true })).toBeDefined();
    expect(result.current.matches.map((m) => m.command)).toEqual(['/group invite', '/group status']);
  });

  it('moves the highlight with the arrows and wraps', async () => {
    const { result } = renderHook(() => useCommandPalette({ value: '/gr', onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    act(() => {
      expect(result.current.handleKey(PALETTE_KEYS.down)).toBe(true);
    });
    expect(result.current.highlightedIndex).toBe(1);
    act(() => {
      result.current.handleKey(PALETTE_KEYS.down);
    });
    expect(result.current.highlightedIndex).toBe(0);
    act(() => {
      result.current.handleKey(PALETTE_KEYS.up);
    });
    expect(result.current.highlightedIndex).toBe(1);
  });

  it('Tab fills the composer with the highlighted command; an argument command gets a trailing space', async () => {
    const onChange = vi.fn();
    const { result } = renderHook(() => useCommandPalette({ value: '/pl', onChange }), {
      wrapper: wrapperFor(client),
    });
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    let consumed = false;
    act(() => {
      consumed = result.current.handleKey(PALETTE_KEYS.tab);
    });

    expect(consumed).toBe(true);
    expect(onChange).toHaveBeenCalledWith('/plan ');
  });

  it('leaves Enter to the composer once the command is typed in full', async () => {
    const onChange = vi.fn();
    const { result } = renderHook(() => useCommandPalette({ value: '/group status', onChange }), {
      wrapper: wrapperFor(client),
    });
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    expect(result.current.handleKey(PALETTE_KEYS.enter)).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('Escape closes the palette for this draft, and the next keystroke reopens it', async () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useCommandPalette({ value, onChange: vi.fn() }),
      { wrapper: wrapperFor(client), initialProps: { value: '/gr' } },
    );
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    act(() => {
      expect(result.current.handleKey(PALETTE_KEYS.escape)).toBe(true);
    });
    expect(result.current.isOpen).toBe(false);

    rerender({ value: '/gro' });
    expect(result.current.isOpen).toBe(true);
  });

  it('takes no key while closed', () => {
    const { result } = renderHook(() => useCommandPalette({ value: 'hello', onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });
    expect(result.current.handleKey(PALETTE_KEYS.enter)).toBe(false);
    expect(result.current.handleKey(PALETTE_KEYS.down)).toBe(false);
  });
});

describe('createMentionPaletteHook', () => {
  const list = vi.fn();
  const useMentionPalette = createMentionPaletteHook({ list });
  let client: QueryClient;

  beforeEach(() => {
    list.mockReset();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });

  it('asks for the agent list only once the athlete has typed an @', () => {
    renderHook(() => useMentionPalette({ value: 'hello', caret: 5, onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });
    expect(list).not.toHaveBeenCalled();
  });

  it('never offers a catalogue agent the athlete has not installed', async () => {
    // `find_installed_by_handle` joins `coach_assignments` for this athlete, so an
    // agent that is merely listed would be a mention that silently does not route.
    list.mockResolvedValue({
      agents: [
        agent(),
        agent({ id: 'agent-2', title: 'Marathon Coach', handle: 'marathon-coach', is_assigned: false }),
      ],
    });

    const { result } = renderHook(() => useMentionPalette({ value: '@', caret: 1, onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });

    await waitFor(() => expect(result.current.matches).toHaveLength(1));
    expect(result.current.matches.map((c) => c.handle)).toEqual(['tempo-coach']);
  });

  it('offers an installed system agent — the resolver admits one', async () => {
    // `WHERE c.slug = $2 AND (c.tenant_id = $3 OR c.is_system = 1)`: a system agent
    // the athlete has been assigned resolves, so `is_system` is not the filter.
    list.mockResolvedValue({
      agents: [agent({ id: 'agent-sys', title: 'Sleep Coach', handle: 'sleep-coach', is_system: true })],
    });

    const { result } = renderHook(() => useMentionPalette({ value: '@', caret: 1, onChange: vi.fn() }), {
      wrapper: wrapperFor(client),
    });

    await waitFor(() => expect(result.current.matches).toHaveLength(1));
    expect(result.current.matches[0].handle).toBe('sleep-coach');
  });

  it('narrows the offer as the athlete types the handle', async () => {
    list.mockResolvedValue({
      agents: [agent(), agent({ id: 'agent-3', title: 'Sleep Coach', handle: 'sleep-coach' })],
    });

    const { result, rerender } = renderHook(
      ({ value, caret }: { value: string; caret: number }) =>
        useMentionPalette({ value, caret, onChange: vi.fn() }),
      { wrapper: wrapperFor(client), initialProps: { value: '@', caret: 1 } },
    );
    await waitFor(() => expect(result.current.matches).toHaveLength(2));

    rerender({ value: '@sle', caret: 4 });

    await waitFor(() => expect(result.current.matches.map((c) => c.handle)).toEqual(['sleep-coach']));
  });

  it('Enter inserts the highlighted handle and a space, with the caret after it', async () => {
    list.mockResolvedValue({ agents: [agent()] });
    const onChange = vi.fn();
    const { result } = renderHook(() => useMentionPalette({ value: 'hey @Tem', caret: 8, onChange }), {
      wrapper: wrapperFor(client),
    });
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    let consumed = false;
    act(() => {
      consumed = result.current.handleKey(PALETTE_KEYS.enter);
    });

    expect(consumed).toBe(true);
    expect(onChange).toHaveBeenCalledWith('hey @tempo-coach ', 17);
  });

  it('leaves Enter to the composer once the handle is typed in full', async () => {
    list.mockResolvedValue({ agents: [agent()] });
    const onChange = vi.fn();
    const { result } = renderHook(
      () => useMentionPalette({ value: '@tempo-coach', caret: 12, onChange }),
      { wrapper: wrapperFor(client) },
    );
    await waitFor(() => expect(result.current.isOpen).toBe(true));

    expect(result.current.handleKey(PALETTE_KEYS.enter)).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
  });
});
