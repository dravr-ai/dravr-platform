// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared Home-preferences hook — the read, a change shown at once, and a refused change rolled back
// ABOUTME: A dismissed plan suggestion leaves the page on the tap, and comes back if the server did not store it

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { HomePreferences } from '@pierre/shared-types';
import { createHomePreferencesHook } from '../src/homePreferencesHook';

const getHomePreferences = vi.fn<() => Promise<HomePreferences>>();
const updateHomePreferences = vi.fn<(prefs: HomePreferences) => Promise<HomePreferences>>();
const useHomePreferences = createHomePreferencesHook({ getHomePreferences, updateHomePreferences });

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

describe('createHomePreferencesHook', () => {
  beforeEach(() => {
    getHomePreferences.mockReset();
    updateHomePreferences.mockReset();
  });

  it('is null until the server answers, then the stored choice', async () => {
    getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    const { wrapper } = setup();
    const { result } = renderHook(() => useHomePreferences(), { wrapper });

    expect(result.current.preferences).toBeNull();
    await waitFor(() => expect(result.current.preferences).toEqual({ plan_suggestion_hidden: true }));
  });

  it('shows a change before the server answers, then settles on what it stored', async () => {
    getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    let answer: (prefs: HomePreferences) => void = () => {};
    updateHomePreferences.mockImplementation(
      () => new Promise<HomePreferences>((resolve) => {
        answer = resolve;
      }),
    );
    const { wrapper } = setup();
    const { result } = renderHook(() => useHomePreferences(), { wrapper });
    await waitFor(() => expect(result.current.preferences).not.toBeNull());

    act(() => result.current.update({ plan_suggestion_hidden: true }));
    await waitFor(() => expect(result.current.preferences).toEqual({ plan_suggestion_hidden: true }));
    expect(updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true });

    act(() => answer({ plan_suggestion_hidden: true }));
    await waitFor(() => expect(result.current.isUpdating).toBe(false));
    expect(result.current.preferences).toEqual({ plan_suggestion_hidden: true });
  });

  it('rolls a refused change back to what the server holds', async () => {
    getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    updateHomePreferences.mockRejectedValue(new Error('503'));
    const { wrapper } = setup();
    const { result } = renderHook(() => useHomePreferences(), { wrapper });
    await waitFor(() => expect(result.current.preferences).not.toBeNull());

    act(() => result.current.update({ plan_suggestion_hidden: true }));
    await waitFor(() => expect(result.current.isUpdating).toBe(false));
    await waitFor(() => expect(result.current.preferences).toEqual({ plan_suggestion_hidden: false }));
  });
});
