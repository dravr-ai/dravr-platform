// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the mention palette — it offers only coaches the server will resolve for an @handle
// ABOUTME: Resolution needs a coach_assignments row, so a listed-but-uninstalled coach is never suggested

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import type { Coach } from '@pierre/shared-types';
import { useMentionPalette } from '../useMentionPalette';

const list = vi.fn();

vi.mock('../../services/api', () => ({
  coachesApi: {
    list: (...a: unknown[]) => list(...a),
  },
}));

function coach(overrides: Partial<Coach> = {}): Coach {
  return {
    id: 'coach-1',
    title: 'Tempo Coach',
    handle: 'tempo-coach',
    description: 'Threshold work',
    category: 'training',
    is_system: false,
    is_assigned: true,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-20T10:00:00Z',
    ...overrides,
  } as Coach;
}

function wrapperFor(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

describe('useMentionPalette candidates', () => {
  let wrapper: ReturnType<typeof wrapperFor>;

  beforeEach(() => {
    list.mockReset();
    wrapper = wrapperFor(new QueryClient({ defaultOptions: { queries: { retry: false } } }));
  });

  it('offers a coach on the athlete list', async () => {
    list.mockResolvedValue({ coaches: [coach()] });

    const { result } = renderHook(
      () => useMentionPalette({ value: '@', caret: 1, onChange: vi.fn() }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.matches).toHaveLength(1));
    expect(result.current.matches[0].handle).toBe('tempo-coach');
  });

  it('never offers a catalogue coach the athlete has not installed', async () => {
    // `find_installed_by_handle` joins `coach_assignments` for this athlete, so a
    // coach that is merely listed would be a mention that silently does not route.
    list.mockResolvedValue({
      coaches: [
        coach(),
        coach({
          id: 'coach-2',
          title: 'Marathon Coach',
          handle: 'marathon-coach',
          is_assigned: false,
        }),
      ],
    });

    const { result } = renderHook(
      () => useMentionPalette({ value: '@', caret: 1, onChange: vi.fn() }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.matches).toHaveLength(1));
    expect(result.current.matches.map(c => c.handle)).toEqual(['tempo-coach']);
  });

  it('offers an installed system coach — the resolver admits one', async () => {
    // `WHERE c.slug = $2 AND (c.tenant_id = $3 OR c.is_system = 1)`: a system coach
    // the athlete has been assigned resolves, so `is_system` is not the filter.
    list.mockResolvedValue({
      coaches: [
        coach({
          id: 'coach-sys',
          title: 'Sleep Coach',
          handle: 'sleep-coach',
          is_system: true,
          is_assigned: true,
        }),
      ],
    });

    const { result } = renderHook(
      () => useMentionPalette({ value: '@', caret: 1, onChange: vi.fn() }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.matches).toHaveLength(1));
    expect(result.current.matches[0].handle).toBe('sleep-coach');
  });

  it('narrows the offer as the athlete types the handle', async () => {
    list.mockResolvedValue({
      coaches: [coach(), coach({ id: 'coach-3', title: 'Sleep Coach', handle: 'sleep-coach' })],
    });

    // Mount on the bare `@` the way the composer does, then let the athlete
    // type: the palette is driven by re-renders, not by a fresh mount.
    const { result, rerender } = renderHook(
      ({ value, caret }: { value: string; caret: number }) =>
        useMentionPalette({ value, caret, onChange: vi.fn() }),
      { wrapper, initialProps: { value: '@', caret: 1 } },
    );

    await waitFor(() => expect(result.current.matches).toHaveLength(2));

    rerender({ value: '@sle', caret: 4 });

    await waitFor(() =>
      expect(result.current.matches.map(c => c.handle)).toEqual(['sleep-coach']),
    );
  });
});
