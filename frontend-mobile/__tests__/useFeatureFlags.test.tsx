// ABOUTME: carnet #74 — the mobile flag hook reads the shared api-client domain, not a local copy of the types
// ABOUTME: Pins the shared endpoint, the shared compile defaults, and the merge of a partial server answer

import React, { type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { FALLBACK_FEATURE_FLAGS } from '@pierre/api-client';

const mockGetMyFeatures = jest.fn();
jest.mock('../src/services/api', () => ({
  featureFlagsApi: { getMyFeatures: () => mockGetMyFeatures() },
}));

import { useFeatureFlags, FEATURE_KEYS } from '../src/hooks/useFeatureFlags';

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('useFeatureFlags (mobile)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('reads the flag keys the shared domain publishes', () => {
    // A local re-declaration of these strings is exactly the duplication #74
    // removed; they must come from the shared domain.
    expect(FEATURE_KEYS.apiTokens).toBe('api_tokens');
    expect(FEATURE_KEYS.billingHeader).toBe('billing_header');
    expect(FALLBACK_FEATURE_FLAGS).toEqual({ api_tokens: false, billing_header: false });
  });

  it('surfaces the server values once they land', async () => {
    mockGetMyFeatures.mockResolvedValue({
      flags: { api_tokens: true, billing_header: true },
      known: [
        { key: 'api_tokens', description: 'Personal MCP bearer tokens', default_enabled: false },
      ],
    });

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => {
      expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(true);
    });
    expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(true);
    expect(result.current.known).toHaveLength(1);
    expect(result.current.known[0].key).toBe('api_tokens');
  });

  it('layers a partial server answer over the shared compile defaults', async () => {
    mockGetMyFeatures.mockResolvedValue({ flags: { api_tokens: true }, known: [] });

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => {
      expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(true);
    });
    // The key the server omitted still has the compile default, not undefined.
    expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(false);
  });

  it('resolves a failed request to the off defaults, never to an open gate', async () => {
    mockGetMyFeatures.mockRejectedValue(new Error('network down'));

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => {
      expect(result.current.isError).toBe(true);
    });
    expect(result.current.flags).toEqual({ api_tokens: false, billing_header: false });
  });
});
