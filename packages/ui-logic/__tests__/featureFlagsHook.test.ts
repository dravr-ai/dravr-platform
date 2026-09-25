// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared feature-flag hook — the shared compile defaults, the merge, and a failed request
// ABOUTME: A failure resolves every flag to off, so a network error can never reveal a gated surface

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, type QueryObserverOptions } from '@tanstack/react-query';
import { FEATURE_KEYS, type FeatureFlagsApi } from '@pierre/api-client';
import { createFeatureFlagsHook } from '../src/featureFlagsHook';

const getMyFeatures = vi.fn();
const useFeatureFlags = createFeatureFlagsHook({ getMyFeatures } as unknown as FeatureFlagsApi);

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

describe('createFeatureFlagsHook', () => {
  beforeEach(() => {
    getMyFeatures.mockReset();
  });

  it('shows the compile defaults while the request is in flight', () => {
    getMyFeatures.mockImplementation(() => new Promise(() => {}));
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.flags).toEqual({ api_tokens: false, billing_header: false });
  });

  it('reads the flags under the shared self key, fresh for five minutes', async () => {
    getMyFeatures.mockResolvedValue({ flags: { api_tokens: true, billing_header: true }, known: [] });
    const { client, wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(true));
    const query = client.getQueryCache().find({ queryKey: ['feature-flags', 'me'], exact: true });
    expect((query?.options as QueryObserverOptions | undefined)?.staleTime).toBe(5 * 60_000);
  });

  it('surfaces the server values and the known-flag registry once they land', async () => {
    getMyFeatures.mockResolvedValue({
      flags: { api_tokens: true, billing_header: true },
      known: [
        { key: 'api_tokens', description: 'Personal MCP bearer tokens', default_enabled: false },
        { key: 'billing_header', description: 'Billing header', default_enabled: false },
      ],
    });
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(true);
    expect(result.current.known.map((flag) => flag.key)).toEqual(['api_tokens', 'billing_header']);
  });

  it('layers a partial server answer over the compile defaults', async () => {
    getMyFeatures.mockResolvedValue({ flags: { api_tokens: true }, known: [] });
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(true));
    // The key the server omitted still has the compile default, not undefined.
    expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(false);
  });

  it('resolves a failed request to the off defaults, never to an open gate', async () => {
    getMyFeatures.mockRejectedValue(new Error('network down'));
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.flags).toEqual({ api_tokens: false, billing_header: false });
  });
});
