// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the web Home's provider-status read — which connected providers have to be reconnected
// ABOUTME: Home's shared reads (the stale-answer schedule, the route policy, the plan) are tested in @pierre/ui-logic

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider, notifyManager } from '@tanstack/react-query';
import type { ProvidersStatusResponse } from '@pierre/shared-types';
import { useProviderConnection } from '../useHome';

const api = vi.hoisted(() => ({
  getProvidersStatus: vi.fn<() => Promise<ProvidersStatusResponse>>(),
}));

vi.mock('../../services/api', () => ({
  athleteApi: {},
  providersApi: { getProvidersStatus: api.getProvidersStatus },
}));

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/** Let every resolved promise and zero-delay timer run, under act. */
async function flush() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  // React Query batches its notifications on a zero-delay timeout; run them
  // inline so an answer reaches the hook without a clock tick.
  notifyManager.setScheduler((callback) => callback());
});

afterEach(() => {
  vi.useRealTimers();
  notifyManager.setScheduler((callback) => setTimeout(callback, 0));
});

describe('useProviderConnection', () => {
  function status(
    provider: string,
    display_name: string,
    connected: boolean,
    needs_reauth: boolean,
  ): ProvidersStatusResponse['providers'][number] {
    return {
      provider,
      display_name,
      requires_oauth: true,
      connected,
      needs_reauth,
      capabilities: ['activities'],
      consent_required: false,
    };
  }

  it('names the connected providers that have to be reconnected, and only those', async () => {
    api.getProvidersStatus.mockResolvedValue({
      providers: [
        status('strava', 'Strava', true, false),
        status('garmin', 'Garmin', true, true),
        // The flag means nothing on a provider that is not connected.
        status('whoop', 'WHOOP', false, true),
        status('coros', 'COROS', true, true),
      ],
    });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    expect(result.current).toEqual({ loaded: false, connected: false, needsReconnect: [] });
    await flush();

    expect(result.current).toEqual({ loaded: true, connected: true, needsReconnect: ['Garmin', 'COROS'] });
  });

  it('names a provider once when its mirror and native cards both need reconnecting', async () => {
    api.getProvidersStatus.mockResolvedValue({
      providers: [status('strava', 'Strava', true, true), status('sciotte', 'Strava', true, true)],
    });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    await flush();

    expect(result.current.needsReconnect).toEqual(['Strava']);
  });

  it('names nobody when every connection is healthy', async () => {
    api.getProvidersStatus.mockResolvedValue({ providers: [status('strava', 'Strava', true, false)] });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    await flush();

    expect(result.current).toEqual({ loaded: true, connected: true, needsReconnect: [] });
  });

  it('never names a provider whose sign-in is paused, since it cannot be reconnected', async () => {
    api.getProvidersStatus.mockResolvedValue({
      providers: [
        { ...status('sciotte_garmin', 'Garmin', true, true), connect_paused: true },
        status('coros', 'COROS', true, true),
      ],
    });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    await flush();

    expect(result.current.needsReconnect).toEqual(['COROS']);
  });
});
