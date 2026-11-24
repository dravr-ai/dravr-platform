// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the shared provider-cards hook holds its list on the one provider-status key, so an invalidation made anywhere reaches it
// ABOUTME: `reload` answers with the rows a new read brought (none when it failed); `revalidate` joins a read in flight instead of doubling it

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { createProviderCardsHook } from '../src/providerCardsHook';

const getProvidersStatus = vi.fn();
const useProviderCards = createProviderCardsHook({ getProvidersStatus });

function card(connected: boolean): ExtendedProviderStatus {
  return {
    provider: 'sciotte_garmin',
    display_name: 'Garmin',
    requires_oauth: false,
    connected,
    capabilities: ['activities'],
  } as ExtendedProviderStatus;
}

function harness() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

describe('useProviderCards', () => {
  beforeEach(() => {
    getProvidersStatus.mockReset();
  });
  afterEach(cleanup);

  it('reads the cards onto the provider-status key, and says when a read has answered', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card(false)] });
    const { client, wrapper } = harness();
    const { result } = renderHook(() => useProviderCards(), { wrapper });

    expect(result.current.loaded).toBe(false);
    expect(result.current.isPending).toBe(true);
    expect(result.current.providers).toEqual([]);
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.providers).toEqual([card(false)]);
    expect(client.getQueryData(QUERY_KEYS.providers.status())).toEqual({ providers: [card(false)] });
  });

  it('shows the answer to an invalidation made elsewhere, without being asked', async () => {
    getProvidersStatus.mockResolvedValueOnce({ providers: [card(false)] });
    const { client, wrapper } = harness();
    const { result } = renderHook(() => useProviderCards(), { wrapper });
    await waitFor(() => expect(result.current.loaded).toBe(true));

    getProvidersStatus.mockResolvedValue({ providers: [card(true)] });
    await act(() => client.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() }));

    await waitFor(() => expect(result.current.providers).toEqual([card(true)]));
  });

  it('holds the read back while disabled', async () => {
    const { wrapper } = harness();
    renderHook(() => useProviderCards({ enabled: false }), { wrapper });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getProvidersStatus).not.toHaveBeenCalled();
  });

  it('reload answers with the rows a new read brought, and none when it failed', async () => {
    getProvidersStatus.mockResolvedValueOnce({ providers: [card(false)] });
    const { wrapper } = harness();
    const { result } = renderHook(() => useProviderCards(), { wrapper });
    await waitFor(() => expect(result.current.loaded).toBe(true));

    getProvidersStatus.mockResolvedValueOnce({ providers: [card(true)] });
    let served: ExtendedProviderStatus[] = [];
    await act(async () => {
      served = await result.current.reload();
    });
    expect(served).toEqual([card(true)]);

    getProvidersStatus.mockRejectedValueOnce(new Error('offline'));
    await act(async () => {
      served = await result.current.reload();
    });
    expect(served).toEqual([]);
    // The list on screen is the last one read, and there is no load error to show over it.
    await waitFor(() => expect(result.current.providers).toEqual([card(true)]));
    expect(result.current.loadError).toBeNull();
  });

  it('says why the cards could not be read while there is no list to show', async () => {
    getProvidersStatus.mockRejectedValue(new Error('offline'));
    const { wrapper } = harness();
    const { result } = renderHook(() => useProviderCards(), { wrapper });
    await waitFor(() => expect(result.current.loadError).toBeInstanceOf(Error));
    expect(result.current.loaded).toBe(false);
  });

  it('revalidate joins a re-read in flight; reload replaces it', async () => {
    getProvidersStatus.mockResolvedValueOnce({ providers: [card(false)] });
    const { wrapper } = harness();
    const { result } = renderHook(() => useProviderCards(), { wrapper });
    await waitFor(() => expect(result.current.loaded).toBe(true));

    let answer: (value: { providers: ExtendedProviderStatus[] }) => void = () => undefined;
    getProvidersStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    act(() => {
      void result.current.revalidate();
    });
    await waitFor(() => expect(getProvidersStatus).toHaveBeenCalledTimes(2));
    act(() => {
      void result.current.revalidate();
    });
    expect(getProvidersStatus).toHaveBeenCalledTimes(2);

    let reloaded: Promise<ExtendedProviderStatus[]> = Promise.resolve([]);
    act(() => {
      reloaded = result.current.reload();
    });
    await waitFor(() => expect(getProvidersStatus).toHaveBeenCalledTimes(3));
    await act(async () => {
      answer({ providers: [card(true)] });
      await reloaded;
    });
    expect(await reloaded).toEqual([card(true)]);
  });
});
