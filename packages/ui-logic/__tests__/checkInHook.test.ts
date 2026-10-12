// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared check-in hook — the read, a choice shown at once, a refused one rolled back, the awaited choice
// ABOUTME: Red if Settings shows a cadence the server did not store, or onboarding advances on a choice that was refused (carnet#831)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CheckInCadence, CheckInPreference } from '@pierre/shared-types';
import { createCheckInHook } from '../src/checkInHook';

const getCheckIn = vi.fn<() => Promise<CheckInPreference>>();
const updateCheckIn = vi.fn<(cadence: CheckInCadence) => Promise<CheckInPreference>>();
const useCheckIn = createCheckInHook({ getCheckIn, updateCheckIn });

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

describe('createCheckInHook', () => {
  beforeEach(() => {
    getCheckIn.mockReset();
    updateCheckIn.mockReset();
  });

  it('is null until the server answers, then the stored cadence — null for no choice', async () => {
    getCheckIn.mockResolvedValue({ cadence: null });
    const { wrapper } = setup();
    const { result } = renderHook(() => useCheckIn(), { wrapper });

    expect(result.current.preference).toBeNull();
    await waitFor(() => expect(result.current.preference).toEqual({ cadence: null }));
  });

  it('shows a choice before the server answers, then settles on what it stored', async () => {
    getCheckIn.mockResolvedValue({ cadence: 'daily' });
    let answer: (stored: CheckInPreference) => void = () => {};
    updateCheckIn.mockImplementation(
      () => new Promise<CheckInPreference>((resolve) => {
        answer = resolve;
      }),
    );
    const { wrapper } = setup();
    const { result } = renderHook(() => useCheckIn(), { wrapper });
    await waitFor(() => expect(result.current.preference).not.toBeNull());

    act(() => result.current.update('off'));
    await waitFor(() => expect(result.current.preference).toEqual({ cadence: 'off' }));
    expect(updateCheckIn).toHaveBeenCalledWith('off');

    act(() => answer({ cadence: 'off' }));
    await waitFor(() => expect(result.current.isUpdating).toBe(false));
    expect(result.current.preference).toEqual({ cadence: 'off' });
    expect(result.current.saveFailed).toBe(false);
  });

  it('rolls a refused choice back to what the server holds, and says it failed', async () => {
    getCheckIn.mockResolvedValue({ cadence: 'weekly' });
    updateCheckIn.mockRejectedValue(new Error('503'));
    const { wrapper } = setup();
    const { result } = renderHook(() => useCheckIn(), { wrapper });
    await waitFor(() => expect(result.current.preference).not.toBeNull());

    act(() => result.current.update('daily'));
    await waitFor(() => expect(result.current.saveFailed).toBe(true));
    await waitFor(() => expect(result.current.preference).toEqual({ cadence: 'weekly' }));
  });

  it('lets onboarding await a choice, and rejects one that was not stored', async () => {
    getCheckIn.mockResolvedValue({ cadence: null });
    updateCheckIn.mockResolvedValueOnce({ cadence: 'three_per_week' }).mockRejectedValueOnce(new Error('503'));
    const { wrapper } = setup();
    const { result } = renderHook(() => useCheckIn(), { wrapper });

    await act(async () => {
      await expect(result.current.choose('three_per_week')).resolves.toEqual({ cadence: 'three_per_week' });
    });
    await act(async () => {
      await expect(result.current.choose('daily')).rejects.toThrow('503');
    });
  });
});
